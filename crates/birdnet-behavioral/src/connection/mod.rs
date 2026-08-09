//! `DuckDB` file-based connection management.
//!
//! Provides a durable, file-backed `DuckDB` database for behavioral analytics.
//! Data is synced from the operational `SQLite` database (OLTP) into `DuckDB`
//! (OLAP) for complex analytical queries using the `duckdb-behavioral` extension.
//!
//! # Module layout
//!
//! | Sub-module   | Contents                                                     |
//! |--------------|--------------------------------------------------------------|
//! | `sync`       | `sync_from_sqlite`, `insert_detection`, count helpers       |
//! | `analytics`  | `sessionize`, `retention`, `funnel`, `next_species`         |

mod analytics;
#[cfg(test)]
mod live;
mod sync;

use duckdb::{Connection, Error as DuckDbError};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::queries;

// Optional compile-time embed of the `behavioral` extension binary; the
// generated file declares `EMBEDDED_EXTENSION: Option<&[u8]>`. `Some(bytes)`
// when the release pipeline (or a maintainer's `vendor/`) supplies the
// extension, `None` otherwise — see `build.rs`.
include!(concat!(env!("OUT_DIR"), "/embedded_extension.rs"));

/// Errors from `DuckDB` operations.
#[derive(Debug)]
pub enum AnalyticsError {
    /// `DuckDB` connection or query error.
    Database(DuckDbError),
    /// Failed to load the behavioral extension.
    ExtensionLoad(String),
    /// Query returned unexpected data.
    InvalidData(String),
}

impl fmt::Display for AnalyticsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(f, "DuckDB error: {e}"),
            Self::ExtensionLoad(msg) => write!(f, "extension load error: {msg}"),
            Self::InvalidData(msg) => write!(f, "invalid data: {msg}"),
        }
    }
}

impl std::error::Error for AnalyticsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::ExtensionLoad(_) | Self::InvalidData(_) => None,
        }
    }
}

impl From<DuckDbError> for AnalyticsError {
    fn from(e: DuckDbError) -> Self {
        Self::Database(e)
    }
}

/// Default cap on `DuckDB`'s buffer-pool memory.
///
/// Without an explicit limit `DuckDB` sizes its buffer pool at ~80% of system
/// RAM, which OOM-kills the whole process during a heavy analytics query on a
/// 1–4 GB Raspberry Pi. This conservative default leaves room for inference,
/// the web server, and the OS within the unit's memory budget; larger hosts
/// can raise it via the `BIRDNET_DUCKDB_MEMORY_LIMIT` environment variable.
const DEFAULT_DUCKDB_MEMORY_LIMIT: &str = "256MB";

/// Resolve the `DuckDB` memory limit from an optional configured value,
/// falling back to [`DEFAULT_DUCKDB_MEMORY_LIMIT`] when unset or malformed.
///
/// The value is interpolated into a `SET memory_limit='…'` statement, so only
/// well-formed literals (a leading digit then digits / unit letters / `.` /
/// `%`) are accepted — anything else falls back to the default. That both
/// keeps untrusted text out of the statement and avoids a startup failure from
/// an operator typo.
fn resolve_memory_limit(configured: Option<&str>) -> String {
    match configured.map(str::trim) {
        Some(v) if is_valid_memory_limit(v) => v.to_owned(),
        _ => DEFAULT_DUCKDB_MEMORY_LIMIT.to_owned(),
    }
}

/// Whether `v` is a safe `DuckDB` memory-limit literal: a leading ASCII digit
/// followed only by alphanumerics, `.`, or `%` (e.g. `512MB`, `2GB`, `80%`,
/// `1073741824`).
fn is_valid_memory_limit(v: &str) -> bool {
    v.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'%')
}

/// How [`AnalyticsDb::open_or_quarantine`] obtained its handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// The existing database opened and read cleanly.
    Opened,
    /// The existing database was unusable and was moved aside; this handle is
    /// a fresh, empty one that the caller should repopulate.
    Rebuilt {
        /// Where the unusable file was moved to.
        quarantined: PathBuf,
    },
}

/// Move an unusable analytics database (and its `.wal` sidecar) aside.
///
/// Returns the path it was moved to. The suffix carries a Unix-seconds stamp so
/// repeated failures accumulate rather than overwrite one another.
fn quarantine_file(path: &Path) -> Result<PathBuf, AnalyticsError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".corrupt.{stamp}"));
    let quarantine = path.with_file_name(name);

    std::fs::rename(path, &quarantine).map_err(|e| {
        AnalyticsError::InvalidData(format!(
            "analytics database at {} is unusable and could not be moved aside ({e}); \
             analytics stay disabled until it is removed by hand",
            path.display()
        ))
    })?;

    // Best-effort: DuckDB's write-ahead log sits alongside as `<file>.wal`.
    // Leaving it next to a freshly-created database would have DuckDB try to
    // replay a log belonging to the file we just quarantined.
    let mut wal = path.as_os_str().to_os_string();
    wal.push(".wal");
    let wal = PathBuf::from(wal);
    if wal.exists() {
        let mut to = quarantine.as_os_str().to_os_string();
        to.push(".wal");
        let _ = std::fs::rename(&wal, PathBuf::from(to));
    }

    Ok(quarantine)
}

/// A file-backed `DuckDB` connection for behavioral analytics.
#[derive(Debug)]
pub struct AnalyticsDb {
    pub(super) conn: Connection,
    path: PathBuf,
    extension_loaded: bool,
}

/// A build-time embedded extension that targets the wrong `DuckDB` engine.
///
/// Produced by [`AnalyticsDb::embedded_extension_mismatch`]. Both versions are
/// carried so the report names what was embedded *and* what it had to match,
/// which is the difference between an actionable packaging error and "analytics
/// is empty again".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionMismatch {
    /// `DuckDB` version the embedded extension was built for, e.g. `v1.5.3`.
    pub embedded_for: &'static str,
    /// `DuckDB` version actually linked into this binary, e.g. `v1.5.5`.
    pub engine: String,
}

impl AnalyticsDb {
    /// Open or create a file-based `DuckDB` database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or the schema
    /// cannot be created.
    pub fn open(path: &Path) -> Result<Self, AnalyticsError> {
        // Open with `allow_unsigned_extensions=true` so the embedded-extension
        // fallback in `load_extension()` can `LOAD '<path>'` from a temp file.
        // DuckDB rejects changes to this setting once the connection is open,
        // so it must be a connection config at open time.
        let config = duckdb::Config::default()
            .allow_unsigned_extensions()
            .map_err(AnalyticsError::Database)?;
        let conn = Connection::open_with_flags(path, config)?;

        // Bound DuckDB's buffer memory before any query runs, so a heavy
        // analytics query cannot OOM the process on a small Pi (DuckDB
        // otherwise targets ~80% of system RAM).
        let memory_limit =
            resolve_memory_limit(std::env::var("BIRDNET_DUCKDB_MEMORY_LIMIT").ok().as_deref());
        conn.execute_batch(&format!("SET memory_limit='{memory_limit}';"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS detections (
                Date TEXT NOT NULL,
                Time TEXT NOT NULL,
                Sci_Name TEXT NOT NULL,
                Com_Name TEXT NOT NULL,
                Confidence DOUBLE NOT NULL,
                Lat DOUBLE,
                Lon DOUBLE,
                Cutoff DOUBLE,
                Week INTEGER,
                Sens DOUBLE,
                Overlap DOUBLE,
                File_Name TEXT
            );",
        )?;
        conn.execute_batch(queries::CREATE_DETECTIONS_TS_VIEW)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            extension_loaded: false,
        })
    }

    /// Open the analytics database, quarantining and rebuilding it if it is
    /// unusable.
    ///
    /// The `DuckDB` store is **purely derived**: every row in it is a copy of a
    /// `SQLite` row, so throwing it away costs nothing but the time to sync it
    /// back. That makes "start over" always safe here, and it is the right
    /// answer for an unattended station — previously a corrupt or
    /// version-incompatible analytics file was logged once as "not available
    /// (non-fatal)" and then ignored forever, leaving every analytics page
    /// silently empty until a human noticed and deleted the file by hand.
    ///
    /// Opening is not enough of a check on its own: `DuckDB` can attach to a
    /// damaged file and only fail once a query touches the broken block, so a
    /// probe query runs before the handle is handed back.
    ///
    /// On failure the file — and its `.wal` sidecar — are moved aside with a
    /// timestamped `.corrupt.<unix-seconds>` suffix and a fresh database is
    /// created in their place. The caller's usual startup sync then repopulates
    /// it from `SQLite`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database is unusable *and* cannot be quarantined,
    /// or if the freshly-created replacement also fails to open — either of
    /// which means the analytics directory itself is not writable.
    pub fn open_or_quarantine(path: &Path) -> Result<(Self, OpenOutcome), AnalyticsError> {
        let probe_failure = match Self::open(path) {
            Ok(db) => match db.probe() {
                Ok(()) => return Ok((db, OpenOutcome::Opened)),
                // Drop the handle before renaming the file underneath it.
                Err(e) => {
                    drop(db);
                    e
                }
            },
            Err(e) => e,
        };

        tracing::error!(
            path = %path.display(),
            error = %probe_failure,
            "analytics database is unusable; quarantining it and rebuilding from SQLite"
        );

        let quarantined = quarantine_file(path)?;

        let db = Self::open(path)?;
        db.probe()?;

        tracing::warn!(
            quarantined_to = %quarantined.display(),
            "analytics database rebuilt; it will repopulate from SQLite on this start. \
             The quarantined file can be deleted once you are satisfied nothing is wrong"
        );

        Ok((db, OpenOutcome::Rebuilt { quarantined }))
    }

    /// Cheap read that touches the detections table, so a file `DuckDB` opened
    /// lazily but cannot actually read fails here rather than on the first
    /// analytics request.
    fn probe(&self) -> Result<(), AnalyticsError> {
        let _: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))?;
        Ok(())
    }

    /// Get the database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the behavioral extension has been loaded.
    pub const fn extension_loaded(&self) -> bool {
        self.extension_loaded
    }

    /// Load the `duckdb-behavioral` extension.
    ///
    /// Three-stage fallback so the station works fully out of the box:
    ///   1. `LOAD behavioral` from DuckDB's local extension cache — offline,
    ///      succeeds when a prior install (or a previous run) already cached
    ///      the binary.
    ///   2. `INSTALL behavioral FROM community; LOAD behavioral` — fetches
    ///      from the community registry on first run when network is up.
    ///   3. A build-time-embedded copy of the extension binary, staged into a
    ///      temp file and loaded by path — the offline guarantee on a fresh
    ///      install whose Pi cannot reach the registry. Only available when
    ///      `build.rs` was given a binary (via `BIRDNET_BUNDLED_EXTENSION_FILE`
    ///      or `crates/birdnet-behavioral/vendor/behavioral.duckdb_extension`).
    ///
    /// Non-fatal at the caller: basic DuckDB queries still work without the
    /// extension; only the behavioural-specific functions (`sessionize`,
    /// `retention`, `window_funnel`, `sequence_*`) require it.
    ///
    /// # Errors
    ///
    /// Returns an error only when all three stages fail.
    pub fn load_extension(&mut self) -> Result<(), AnalyticsError> {
        // Report a build-time/engine version mismatch *before* trying anything,
        // so it is visible even when stage 2 masks it. A station with network
        // will happily INSTALL the correct build from the community registry
        // and look healthy, while the embedded copy it would need offline is
        // unusable — which is precisely how the v1.5.3-into-v1.5.5 defect
        // survived a fully green CI matrix.
        if let Some(mismatch) = self.embedded_extension_mismatch() {
            tracing::error!(
                embedded_for = mismatch.embedded_for,
                engine = %mismatch.engine,
                "the behavioral extension embedded at build time targets a different DuckDB \
                 version than the engine linked into this binary; it can never load. Offline \
                 stations will have no behavioural analytics. This is a packaging defect — \
                 rebuild with the extension published for the engine version"
            );
        }

        // 1) Try the locally-cached LOAD (offline-safe).
        if self
            .conn
            .execute_batch(queries::LOAD_BEHAVIORAL_CACHED)
            .is_ok()
        {
            self.extension_loaded = true;
            return Ok(());
        }
        // 2) Try INSTALL FROM community (needs network on first run).
        if self.conn.execute_batch(queries::INSTALL_BEHAVIORAL).is_ok() {
            self.extension_loaded = true;
            return Ok(());
        }
        // 3) Final fallback: embedded binary from build.rs.
        if let Some(bytes) = EMBEDDED_EXTENSION {
            return self.load_embedded(bytes);
        }
        Err(AnalyticsError::ExtensionLoad(
            "behavioral extension not loaded: LOAD from cache failed, \
             INSTALL FROM community failed, and no embedded extension was bundled"
                .to_string(),
        ))
    }

    /// Stage embedded extension bytes to a temp file and `LOAD '<path>'`.
    ///
    /// The bytes themselves are the upstream community-signed build, but
    /// `LOAD` from an ad-hoc path bypasses DuckDB's signature check by design;
    /// `allow_unsigned_extensions=true` is set at open time (see `open`)
    /// because DuckDB refuses to change it on an already-open connection.
    fn load_embedded(&mut self, bytes: &[u8]) -> Result<(), AnalyticsError> {
        let dir = std::env::temp_dir().join("birdnet-behavioral-ext");
        std::fs::create_dir_all(&dir).map_err(|e| {
            AnalyticsError::ExtensionLoad(format!("create temp dir for embedded extension: {e}"))
        })?;
        let path = dir.join("behavioral.duckdb_extension");
        std::fs::write(&path, bytes)
            .map_err(|e| AnalyticsError::ExtensionLoad(format!("write embedded extension: {e}")))?;
        let escaped = path.display().to_string().replace('\'', "''");
        let sql = format!("LOAD '{escaped}';");
        self.conn
            .execute_batch(&sql)
            .map_err(|e| AnalyticsError::ExtensionLoad(format!("load embedded: {e}")))?;
        self.extension_loaded = true;
        tracing::info!(
            path = %path.display(),
            bytes = bytes.len(),
            "loaded behavioral extension from embedded bundle"
        );
        Ok(())
    }

    /// A build-time embedded extension that can never load into this engine.
    ///
    /// `Some` only when an extension was embedded (see `build.rs`) **and** the
    /// `DuckDB` version it declares differs from the engine actually linked in.
    /// `None` when nothing is embedded, when the versions agree, or when the
    /// engine version cannot be read.
    ///
    /// A `DuckDB` extension is version-locked — the engine refuses to `LOAD` a
    /// build targeting any other version, and `allow_extensions_metadata_mismatch`
    /// does not bypass that check — so a mismatch here is fatal to the offline
    /// load path and nothing else can rescue it at run time.
    pub fn embedded_extension_mismatch(&self) -> Option<ExtensionMismatch> {
        let embedded_for = EMBEDDED_EXTENSION_DUCKDB_VERSION?;
        let engine = self.duckdb_version()?;
        (engine != embedded_for).then_some(ExtensionMismatch {
            embedded_for,
            engine,
        })
    }

    /// The `DuckDB` version the build-time embedded extension targets.
    ///
    /// `None` when no extension was embedded. Read out of the extension's own
    /// metadata footer by `build.rs`, not configured anywhere.
    pub const fn embedded_extension_duckdb_version() -> Option<&'static str> {
        EMBEDDED_EXTENSION_DUCKDB_VERSION
    }

    /// The version of the build-time embedded `behavioral` extension itself
    /// (e.g. `v0.9.1`). `None` when no extension was embedded.
    pub const fn embedded_extension_version() -> Option<&'static str> {
        EMBEDDED_EXTENSION_VERSION
    }

    /// The platform the build-time embedded extension targets (e.g.
    /// `linux_amd64`). `None` when no extension was embedded.
    pub const fn embedded_extension_platform() -> Option<&'static str> {
        EMBEDDED_EXTENSION_PLATFORM
    }

    /// The bundled `DuckDB` engine version (e.g. `v1.5.5`).
    ///
    /// The `behavioral` community extension is version-locked to this exact
    /// `DuckDB` version — a build for any other version will not `LOAD` — so
    /// this is the value the published extension must target. Returns `None`
    /// only if the version string cannot be read.
    pub fn duckdb_version(&self) -> Option<String> {
        self.conn
            .query_row("SELECT version()", [], |r| r.get::<_, String>(0))
            .ok()
    }

    /// The loaded `behavioral` extension version (e.g. `v0.9.1`).
    ///
    /// Returns `None` when the extension is not loaded in this connection or its
    /// version is unavailable. Best-effort — any query error maps to `None` — so
    /// it is safe to call for status reporting regardless of extension state.
    pub fn extension_version(&self) -> Option<String> {
        self.conn
            .query_row(queries::BEHAVIORAL_EXTENSION_VERSION, [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .ok()
            .flatten()
    }

    /// Force-reinstall the `behavioral` extension from the community registry
    /// and load it.
    ///
    /// Always re-downloads the latest build for the bundled `DuckDB` version,
    /// even when a cached copy exists. Backs the `--refresh-extension`
    /// maintenance command. Requires network access.
    ///
    /// # Errors
    ///
    /// Returns an error if the extension cannot be reinstalled or loaded — for
    /// example, offline, or the community registry has no build matching the
    /// bundled `DuckDB` version yet.
    pub fn refresh_extension(&mut self) -> Result<(), AnalyticsError> {
        self.conn
            .execute_batch(queries::FORCE_INSTALL_BEHAVIORAL)
            .map_err(|e| AnalyticsError::ExtensionLoad(e.to_string()))?;
        self.extension_loaded = true;
        Ok(())
    }

    /// Get a reference to the underlying `DuckDB` connection.
    pub const fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_db() -> (AnalyticsDb, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();
        (db, dir)
    }

    #[test]
    fn embedded_extension_loads_when_bundled() {
        // Only meaningful when the build embedded a binary (via
        // `BIRDNET_BUNDLED_EXTENSION_FILE` or `vendor/`). Skips quietly
        // otherwise so the test is safe to run on un-bundled dev builds.
        let Some(bytes) = EMBEDDED_EXTENSION else {
            eprintln!("skipped — build did not embed an extension binary");
            return;
        };
        let (mut db, _tmp) = make_db();
        // Call the embedded path directly so the test exercises stage + LOAD
        // without depending on DuckDB's extension cache or network reachability.
        db.load_embedded(bytes)
            .expect("embedded extension should load via LOAD '<path>'");
        assert!(db.extension_loaded());
        assert!(
            db.extension_version().is_some(),
            "extension_version should report a version after the embedded load"
        );
    }

    #[test]
    fn embedded_extension_targets_the_linked_engine() {
        // The invariant nothing used to assert. A DuckDB extension is
        // version-locked, so an embedded copy built for a different engine can
        // never load — but on a station with network the community-registry
        // INSTALL masks that completely, and everything looks healthy right up
        // until the box is deployed somewhere without network.
        //
        // That is exactly how `Dockerfile` shipped a v1.5.3 extension inside a
        // v1.5.5 binary through a fully green CI matrix.
        let Some(embedded_for) = AnalyticsDb::embedded_extension_duckdb_version() else {
            eprintln!("skipped — build did not embed an extension binary");
            return;
        };
        let (db, _tmp) = make_db();
        let engine = db
            .duckdb_version()
            .expect("version() should be readable from a freshly opened database");

        assert_eq!(
            embedded_for, engine,
            "embedded behavioral extension targets DuckDB {embedded_for} but this binary links \
             DuckDB {engine}; it can never LOAD. Point the build at the extension published for \
             {engine} (community-extensions.duckdb.org/{engine}/<platform>/)."
        );
        assert_eq!(db.embedded_extension_mismatch(), None);
    }

    #[test]
    fn embedded_extension_metadata_is_all_or_nothing() {
        // build.rs writes the bytes and the three metadata constants from the
        // same parse, so a half-populated set means the generator drifted.
        let present = [
            EMBEDDED_EXTENSION.is_some(),
            AnalyticsDb::embedded_extension_duckdb_version().is_some(),
            AnalyticsDb::embedded_extension_version().is_some(),
            AnalyticsDb::embedded_extension_platform().is_some(),
        ];
        assert!(
            present.iter().all(|p| *p) || present.iter().all(|p| !*p),
            "embedded-extension constants disagree about whether an extension was embedded: \
             {present:?}"
        );
    }

    #[test]
    fn embedded_extension_metadata_is_well_formed() {
        let Some(ddb) = AnalyticsDb::embedded_extension_duckdb_version() else {
            eprintln!("skipped — build did not embed an extension binary");
            return;
        };
        // Parsed out of the extension's own footer, so these shapes are the
        // upstream contract, not our formatting choice.
        assert!(
            ddb.starts_with('v'),
            "DuckDB version from the footer should look like `v1.5.5`, got {ddb:?}"
        );
        let platform = AnalyticsDb::embedded_extension_platform()
            .expect("platform is written alongside the DuckDB version");
        assert!(
            !platform.is_empty(),
            "platform field should be populated, got {platform:?}"
        );
    }

    #[test]
    fn open_creates_file() {
        let (db, _tmp) = make_db();
        assert!(db.path().exists());
        assert!(!db.extension_loaded());
    }

    #[test]
    fn duckdb_version_reports_bundled_engine() {
        let (db, _tmp) = make_db();
        let v = db.duckdb_version().expect("version() should be readable");
        // The bundled DuckDB is pinned to 1.5.x so it matches the DuckDB the
        // published `behavioral` community extension targets (see the duckdb
        // pin in the workspace Cargo.toml). If this trips, the pin moved and
        // the extension compatibility must be re-checked before shipping.
        assert!(v.starts_with("v1.5"), "unexpected DuckDB version: {v:?}");
    }

    #[test]
    fn extension_version_is_none_before_load() {
        // Not loaded in this connection, so there is no active version to
        // report — even if a build is cached in DuckDB's shared extension dir.
        let (db, _tmp) = make_db();
        assert!(!db.extension_loaded());
        assert_eq!(db.extension_version(), None);
    }

    #[test]
    fn memory_limit_defaults_when_unset_or_malformed() {
        assert_eq!(resolve_memory_limit(None), DEFAULT_DUCKDB_MEMORY_LIMIT);
        assert_eq!(resolve_memory_limit(Some("")), DEFAULT_DUCKDB_MEMORY_LIMIT);
        assert_eq!(
            resolve_memory_limit(Some("MB")),
            DEFAULT_DUCKDB_MEMORY_LIMIT
        ); // no leading digit
        // Anything that could break out of the SET statement falls back.
        assert_eq!(
            resolve_memory_limit(Some("256MB';DROP TABLE detections;--")),
            DEFAULT_DUCKDB_MEMORY_LIMIT
        );
    }

    #[test]
    fn memory_limit_accepts_well_formed_values() {
        assert_eq!(resolve_memory_limit(Some("512MB")), "512MB");
        assert_eq!(resolve_memory_limit(Some("2GB")), "2GB");
        assert_eq!(resolve_memory_limit(Some(" 80% ")), "80%");
        assert_eq!(resolve_memory_limit(Some("1073741824")), "1073741824");
    }

    #[test]
    fn open_applies_a_bounded_memory_limit() {
        let (db, _tmp) = make_db();
        let limit: String = db
            .conn()
            .query_row("SELECT current_setting('memory_limit')", [], |r| r.get(0))
            .unwrap();
        // Our 256MB cap reads back sub-GiB (e.g. "244.1 MiB"); crucially it is
        // NOT the multi-GiB ~80%-of-RAM default, so a big analytics query can't
        // OOM a small Pi.
        assert!(
            !limit.is_empty() && !limit.contains("GiB"),
            "expected a bounded sub-GiB memory_limit, got {limit:?}"
        );
    }
}
