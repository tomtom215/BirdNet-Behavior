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

/// Directory `DuckDB` is pointed at for extension installs, relative to the
/// analytics database file.
const EXTENSION_DIR_NAME: &str = "duckdb-extensions";

/// Keep `DuckDB`'s extension installs inside the data directory, out of `$HOME`.
///
/// By default `DuckDB` installs and caches extensions under `$HOME/.duckdb`.
/// The shipped systemd unit sets `ProtectHome=read-only`, so that directory
/// cannot be created — and this is not a hypothetical: it is the root cause of
/// "analytics dashboards broken on 0.13.1", observed on a station where the
/// journal carried
///
/// ```text
/// Failed to create directory "/home/pi/.duckdb": Read-only file system
/// ```
///
/// and every dashboard had been empty for days with the health endpoint green.
/// Two separate things attempt that write: `icu` autoinstalling on the first
/// query that mentions `CURRENT_DATE`, and stage 2 of
/// [`AnalyticsDb::load_extension`] (`INSTALL behavioral FROM community`).
/// Redirecting the directory fixes both at once, because the replacement sits
/// beside the database file — which is inside `DATA_DIR`, and therefore inside
/// the unit's `ReadWritePaths`.
///
/// Best-effort by design. If the directory cannot be created (a read-only data
/// directory, an in-memory database with no parent) `DuckDB` keeps its default
/// and the embedded-extension paths still work — they `LOAD` by absolute path
/// and never consult this directory at all.
fn redirect_extension_directory(conn: &Connection, db_path: &Path) {
    let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };
    let dir = parent.join(EXTENSION_DIR_NAME);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(
            path = %dir.display(),
            error = %e,
            "could not create a DuckDB extension directory beside the analytics database; \
             DuckDB keeps its default ($HOME/.duckdb)"
        );
        return;
    }
    let escaped = dir.display().to_string().replace('\'', "''");
    if let Err(e) = conn.execute_batch(&format!("SET extension_directory='{escaped}';")) {
        tracing::debug!(error = %e, "could not redirect DuckDB's extension directory");
    }
}

/// Load `DuckDB`'s ICU extension so date arithmetic binds on the *first* query.
///
/// Every analytics dashboard filters on a look-back window, which reaches
/// DuckDB as `detection_date >= CURRENT_DATE - INTERVAL n DAYS`. That operator
/// lives in ICU, and so does everything else that can name the current *local*
/// date: `CURRENT_DATE`, `today()`, the `TimeZone` setting, and
/// `CAST(now() AS DATE)` — which fails with "Unimplemented type for cast
/// (TIMESTAMP WITH TIME ZONE -> DATE)" when ICU is absent. There is no ICU-free
/// spelling to fall back to.
///
/// ICU is **not** statically linked into the bundled `libduckdb`. An earlier
/// version of this comment claimed it was, on the strength of
/// `duckdb_extensions()` reporting it `installed` — but that reading was taken
/// on a connection whose autoinstall had already downloaded it. Measured with
/// autoload and autoinstall off and no local cache, `icu` reports
/// `installed=false, install_mode=NOT_INSTALLED`, and `LOAD icu` fails outright
/// (`core_functions`, by contrast, genuinely does report `STATICALLY_LINKED`).
///
/// So DuckDB has to *fetch* it, and left alone it autoinstalls into
/// `$HOME/.duckdb` during the bind of the first query that needs it. Both
/// halves of that were load-bearing failures:
///
///  * **The write.** Under `ProtectHome=read-only` it fails, permanently — see
///    [`redirect_extension_directory`], which is what stops that write going to
///    `$HOME` at all.
///  * **The timing.** Even where the write succeeds, autoload happens *while
///    binding* the query that triggered it, too late for that query: attempt 1
///    fails, attempts 2-4 pass. One failed query per process start would be
///    survivable on its own, except the web layer maps a query error to a
///    rendered "Analytics temporarily unavailable" fragment and *caches that
///    fragment for ten minutes* — so a station's first visit after every
///    restart poisoned the cache and the dashboards stayed blank.
///
/// Loading here, before any query runs, fixes the timing. Two stages, in the
/// order that needs the least from the host:
///
///  1. A build-time-embedded copy staged to a temp file and loaded by path.
///     No network, no writable `$HOME`, no extension directory — this is the
///     path a shipped release takes.
///  2. `LOAD icu` — DuckDB's own resolution, from the extension directory. On a
///     dev build with no embed this is what autoinstall populates.
///
/// Deliberately non-fatal: a build with neither should still open its store and
/// serve everything that does not need a date window, rather than refuse to
/// start.
fn load_icu(conn: &Connection) {
    /// Staged once per process. The ICU binary is ~20 MB and the bytes never
    /// change for the life of the process, so re-writing it on every `open` is
    /// pure cost — on a Pi's SD card, and in a test suite that opens hundreds
    /// of stores.
    static ICU_STAGED: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();

    if let Some(bytes) = EMBEDDED_ICU {
        match ICU_STAGED
            .get_or_init(|| stage_extension(bytes, "icu.duckdb_extension"))
            .clone()
            .and_then(|path| load_by_path(conn, &path).map(|()| path))
        {
            Ok(path) => {
                tracing::debug!(
                    path = %path.display(),
                    version = EMBEDDED_ICU_VERSION.unwrap_or("unknown"),
                    "loaded DuckDB's ICU extension from the embedded bundle"
                );
                return;
            }
            Err(e) => tracing::warn!(
                error = %e,
                embedded_for = EMBEDDED_ICU_DUCKDB_VERSION.unwrap_or("unknown"),
                embedded_platform = EMBEDDED_ICU_PLATFORM.unwrap_or("unknown"),
                "the ICU extension embedded at build time would not load; falling back to \
                 DuckDB's own resolution. If `embedded_for` or `embedded_platform` disagrees \
                 with this binary's engine, that is a packaging defect"
            ),
        }
    }

    // 2) Already in the extension directory, from a previous run's stage 3.
    if conn.execute_batch("LOAD icu;").is_ok() {
        return;
    }

    // 3) Fetch it, once, into the extension directory.
    //
    // An explicit `INSTALL` is not the same thing as the autoinstall that broke
    // v0.13.1, in the one way that matters: it writes where
    // `redirect_extension_directory` pointed it — beside the analytics database,
    // inside the unit's `ReadWritePaths` — rather than to `$HOME/.duckdb`. It
    // also runs here, before any query, instead of part-way through binding one.
    //
    // This is the path a dev build with no embedded copy takes, and a genuine
    // self-heal for a station whose embed is missing or unloadable. It is last
    // because it is the only stage that needs the network.
    if let Err(e) = conn.execute_batch("INSTALL icu; LOAD icu;") {
        tracing::warn!(
            error = %e,
            embedded = EMBEDDED_ICU.is_some(),
            "could not load DuckDB's ICU extension, from an embedded copy, the extension \
             directory, or the network. Every dashboard filters on a date window and \
             CURRENT_DATE lives in ICU, so those queries will fail until it is available"
        );
    }
}

/// Counter making each in-flight staging file name unique within the process.
static STAGE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write extension `bytes` to a temp file and return the path, atomically.
///
/// Shared by the ICU and `behavioral` loaders. The bytes go to a name unique to
/// this process and call, then `rename` into the shared final path — so a
/// reader always sees either the previous complete file or the new complete
/// one, never a partial or missing one.
///
/// That is not defensive coding for its own sake. Writing in place was
/// measurably broken: `cargo test` opens many stores in parallel threads, each
/// staging the same 20 MB ICU binary over the same path, and `LOAD` failed on
/// whichever thread read a file another was still writing. The service itself
/// runs with `PrivateTmp=yes` and stages once per start, so this only ever bit
/// the test suite — but the same shape is reachable from two processes sharing
/// a `/tmp`, and `rename` costs nothing.
///
/// `rename` also replaces a symlink at the destination rather than writing
/// through it, so a pre-planted link cannot redirect the write.
fn stage_extension(bytes: &[u8], file_name: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("birdnet-behavioral-ext");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create temp dir for embedded extension: {e}"))?;

    let seq = STAGE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = dir.join(format!("{file_name}.{}.{seq}.partial", std::process::id()));
    std::fs::write(&staging, bytes).map_err(|e| format!("write embedded extension: {e}"))?;

    let path = dir.join(file_name);
    std::fs::rename(&staging, &path).map_err(|e| {
        // Leave nothing behind on the failure path; a 20 MB orphan per start
        // would fill a small `/tmp` long before anyone noticed.
        drop(std::fs::remove_file(&staging));
        format!("publish embedded extension: {e}")
    })?;
    Ok(path)
}

/// `LOAD '<path>'`, quoting the path for SQL.
///
/// The bytes are the upstream signed build, but `LOAD` from an ad-hoc path
/// bypasses `DuckDB`'s signature check by design; `allow_unsigned_extensions`
/// is set at open time (see [`AnalyticsDb::open`]) because `DuckDB` refuses to
/// change it on an already-open connection.
fn load_by_path(conn: &Connection, path: &Path) -> Result<(), String> {
    let escaped = path.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!("LOAD '{escaped}';"))
        .map_err(|e| e.to_string())
}

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

/// Which property of the embedded extension the linked engine cannot accept.
///
/// A `DuckDB` extension is locked to both a version *and* a platform, and the
/// two fail identically at run time — `LOAD` refuses the file — so naming which
/// one is wrong is the difference between a fixable packaging error and another
/// round of "analytics is empty again".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionMismatchKind {
    /// Built for a different `DuckDB` version.
    DuckDbVersion,
    /// Built for a different platform, e.g. `linux_amd64` bytes in an
    /// `aarch64` binary. Reachable from a local or cross build: the release
    /// workflow selects the extension per target, but a developer build embeds
    /// whatever `BIRDNET_BUNDLED_EXTENSION_FILE` or `vendor/` happens to hold.
    Platform,
    /// Both the version and the platform are wrong.
    Both,
}

impl fmt::Display for ExtensionMismatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuckDbVersion => write!(f, "DuckDB version"),
            Self::Platform => write!(f, "platform"),
            Self::Both => write!(f, "DuckDB version and platform"),
        }
    }
}

/// A build-time embedded extension the linked engine can never load.
///
/// Produced by [`AnalyticsDb::embedded_extension_mismatch`]. Both sides of each
/// comparison are carried so the report names what was embedded *and* what it
/// had to match, which is the difference between an actionable packaging error
/// and "analytics is empty again".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionMismatch {
    /// `DuckDB` version the embedded extension was built for, e.g. `v1.5.3`.
    pub embedded_for: &'static str,
    /// `DuckDB` version actually linked into this binary, e.g. `v1.5.5`.
    pub engine: String,
    /// Platform the embedded extension targets, e.g. `linux_amd64`. `None` only
    /// when the embed carried no platform in its metadata.
    pub embedded_platform: Option<&'static str>,
    /// Platform the linked engine reports, e.g. `linux_arm64`. `None` when the
    /// engine would not answer `pragma_platform()`.
    pub engine_platform: Option<String>,
    /// Which property disagrees.
    pub kind: ExtensionMismatchKind,
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

        // Before anything can install or load: keep extension writes inside the
        // data directory rather than `$HOME`, which the unit mounts read-only.
        redirect_extension_directory(&conn, path);
        load_icu(&conn);

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
                File_Name TEXT,
                import_batch_id BIGINT
            );",
        )?;
        // Additive for stores created before provenance existed. DuckDB has no
        // `ADD COLUMN IF NOT EXISTS`, so an already-migrated store errors here
        // and that error is the success case — the alternative is quarantining
        // and rebuilding a perfectly good database on every start.
        let _ = conn.execute_batch("ALTER TABLE detections ADD COLUMN import_batch_id BIGINT;");
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

    /// Run the query shape every dashboard opens with, and report whether it
    /// binds.
    ///
    /// This is the ICU counterpart to [`Self::load_extension`]'s success: not
    /// "is an extension loaded" but "can this station ask about a date window
    /// at all". `CURRENT_DATE` lives in ICU, and when ICU cannot be resolved
    /// the failure is a `Catalog Error` at bind time — which the web layer
    /// turns into a cached "Analytics temporarily unavailable" fragment rather
    /// than anything an operator can see.
    ///
    /// Meant to be run with networking disabled: with no network, DuckDB cannot
    /// autoinstall ICU, so a pass means the build-time embedded copy loaded.
    ///
    /// # Errors
    ///
    /// Returns the bind or execution error, whose message names the missing
    /// extension.
    pub fn verify_date_window(&self) -> Result<(), AnalyticsError> {
        let _: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM detections_ts \
             WHERE detection_date >= CURRENT_DATE - INTERVAL 30 DAYS",
            [],
            |row| row.get(0),
        )?;
        Ok(())
    }

    /// `DuckDB` version the embedded ICU targets, if one was embedded.
    #[must_use]
    pub const fn embedded_icu_duckdb_version() -> Option<&'static str> {
        EMBEDDED_ICU_DUCKDB_VERSION
    }

    /// Platform the embedded ICU targets, if one was embedded.
    #[must_use]
    pub const fn embedded_icu_platform() -> Option<&'static str> {
        EMBEDDED_ICU_PLATFORM
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
                mismatch = %mismatch.kind,
                embedded_for = mismatch.embedded_for,
                engine = %mismatch.engine,
                embedded_platform = mismatch.embedded_platform.unwrap_or("unknown"),
                engine_platform = mismatch.engine_platform.as_deref().unwrap_or("unknown"),
                "the behavioral extension embedded at build time disagrees with the engine \
                 linked into this binary (see the `mismatch` field for which property); it can \
                 never load. Offline stations will have no behavioural analytics. This is a \
                 packaging defect — rebuild with the extension published for this engine's \
                 version and platform"
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
    fn load_embedded(&mut self, bytes: &[u8]) -> Result<(), AnalyticsError> {
        let path = stage_extension(bytes, "behavioral.duckdb_extension")
            .map_err(AnalyticsError::ExtensionLoad)?;
        load_by_path(&self.conn, &path)
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
    /// A `DuckDB` extension is locked to a version *and* a platform — the engine
    /// refuses to `LOAD` a build targeting either a different version or a
    /// different architecture, and `allow_extensions_metadata_mismatch` does not
    /// bypass those checks — so a mismatch here is fatal to the offline load
    /// path and nothing else can rescue it at run time.
    ///
    /// Both properties are compared. Checking only the version left the
    /// architecture case invisible: `linux_amd64` bytes embedded in an
    /// `aarch64` build agree on `v1.5.5`, pass a version-only check, and then
    /// fail at `LOAD` on the Pi with nothing having warned. `release.yml` picks
    /// the extension per target, so that gap is reachable from local and cross
    /// builds rather than from a published artifact — which is exactly the
    /// build a maintainer tests an air-gapped station with.
    ///
    /// A platform that cannot be determined on either side is not treated as a
    /// mismatch: the version check still applies, and inventing a disagreement
    /// from missing information would be its own false alarm.
    pub fn embedded_extension_mismatch(&self) -> Option<ExtensionMismatch> {
        let embedded_for = EMBEDDED_EXTENSION_DUCKDB_VERSION?;
        let engine = self.duckdb_version()?;
        let embedded_platform = EMBEDDED_EXTENSION_PLATFORM;
        let engine_platform = self.engine_platform();

        let version_differs = engine != embedded_for;
        let platform_differs = match (embedded_platform, engine_platform.as_deref()) {
            (Some(embedded), Some(actual)) => embedded != actual,
            // One side unknown — compare nothing rather than guess.
            _ => false,
        };

        let kind = match (version_differs, platform_differs) {
            (false, false) => return None,
            (true, false) => ExtensionMismatchKind::DuckDbVersion,
            (false, true) => ExtensionMismatchKind::Platform,
            (true, true) => ExtensionMismatchKind::Both,
        };

        Some(ExtensionMismatch {
            embedded_for,
            engine,
            embedded_platform,
            engine_platform,
            kind,
        })
    }

    /// The platform the linked `DuckDB` engine was built for, e.g.
    /// `linux_amd64`.
    ///
    /// Read from `pragma_platform()`, which reports the same identifiers the
    /// community extension registry publishes under and that the extension's
    /// own metadata footer carries — so the two are directly comparable.
    /// `None` if the pragma cannot be read.
    #[must_use]
    pub fn engine_platform(&self) -> Option<String> {
        self.conn
            .query_row("SELECT * FROM pragma_platform()", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
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

    /// A freshly opened store must answer a date-range query on the **first**
    /// attempt.
    ///
    /// The count is irrelevant; that the query binds at all is the whole point.
    /// `CURRENT_DATE - INTERVAL n DAYS` needs DuckDB's ICU extension, and
    /// DuckDB resolves it only while binding the query that first needs it —
    /// too late for that query. Every analytics dashboard issues exactly this
    /// shape as its opening move after a restart, and the web layer caches the
    /// resulting error fragment for ten minutes, so "only the first one fails"
    /// meant "they are all blank for the next ten minutes".
    ///
    /// Run it twice: a regression here passes on the second call, so a test
    /// that only checked once would report success against the broken build.
    #[test]
    fn first_date_range_query_on_a_fresh_store_binds() {
        let (db, _tmp) = make_db();
        let sql = "SELECT COUNT(*) FROM detections_ts \
                   WHERE detection_date >= CURRENT_DATE - INTERVAL 60 DAYS";

        db.conn
            .query_row(sql, [], |r| r.get::<_, i64>(0))
            .expect("the first date-range query after opening the store must bind");

        // Second call: proves the assertion above was about the first call, not
        // about the query being unsupported outright.
        db.conn
            .query_row(sql, [], |r| r.get::<_, i64>(0))
            .expect("and so must the second");
    }

    /// ICU is loaded eagerly, not left for DuckDB to resolve mid-bind.
    #[test]
    fn icu_is_loaded_when_the_store_opens() {
        let (db, _tmp) = make_db();
        let loaded: bool = db
            .conn
            .query_row(
                "SELECT loaded FROM duckdb_extensions() WHERE extension_name = 'icu'",
                [],
                |r| r.get(0),
            )
            .expect("duckdb_extensions() lists icu");
        assert!(
            loaded,
            "ICU must be loaded before the first query, not autoloaded by it"
        );
    }

    /// The embedded ICU loads with **no network and no extension cache**.
    ///
    /// This is the gate the test above cannot be: it passes as long as ICU ends
    /// up loaded by *any* route, and on a machine with network DuckDB's own
    /// autoinstall is one such route. That is not a hypothetical weakness — the
    /// previous version of this file shipped a `load_icu` that could only ever
    /// work from a cache, and its test went green anyway because an earlier
    /// probe on the same machine had populated `~/.duckdb`. Moving that cache
    /// aside was what exposed it.
    ///
    /// So this one removes both escapes explicitly — autoload and autoinstall
    /// off, extension directory pointed at an empty temp dir — leaving the
    /// embedded bytes as the only way `CURRENT_DATE` can resolve. Skips when
    /// the build embedded nothing, exactly like the `behavioral` equivalent;
    /// CI embeds both, so CI asserts.
    #[test]
    fn embedded_icu_loads_with_no_cache_and_no_network() {
        if EMBEDDED_ICU.is_none() {
            eprintln!("skipped — build did not embed the ICU extension");
            return;
        }
        let dir = TempDir::new().unwrap();
        let config = duckdb::Config::default()
            .allow_unsigned_extensions()
            .unwrap();
        let conn = Connection::open_in_memory_with_flags(config).unwrap();
        let empty = dir.path().join("no-extensions-here");
        std::fs::create_dir_all(&empty).unwrap();
        conn.execute_batch(&format!(
            "SET autoinstall_known_extensions=false; \
             SET autoload_known_extensions=false; \
             SET extension_directory='{}';",
            empty.display()
        ))
        .unwrap();

        // Sanity: without the embed, this connection genuinely cannot do dates.
        // If this ever starts succeeding, the assertion below has stopped
        // proving anything and this test needs rewriting, not deleting.
        assert!(
            conn.query_row("SELECT CURRENT_DATE::VARCHAR", [], |r| r
                .get::<_, String>(0))
                .is_err(),
            "CURRENT_DATE must fail before ICU is loaded, or this test proves nothing"
        );

        load_icu(&conn);

        conn.query_row("SELECT CURRENT_DATE::VARCHAR", [], |r| {
            r.get::<_, String>(0)
        })
        .expect("CURRENT_DATE must resolve from the embedded ICU alone");
    }

    /// Extension installs land beside the database, never in `$HOME`.
    ///
    /// The whole of "analytics dashboards broken on 0.13.1" was DuckDB trying
    /// to create `$HOME/.duckdb` under a unit that mounts `/home` read-only.
    /// Nothing in the code said where extensions go, so nothing could regress
    /// visibly when it went back to the default.
    #[test]
    fn extension_installs_stay_beside_the_database() {
        let (db, tmp) = make_db();
        let configured: String = db
            .conn
            .query_row("SELECT current_setting('extension_directory')", [], |r| {
                r.get(0)
            })
            .expect("extension_directory is readable");

        assert_eq!(
            std::path::Path::new(&configured),
            tmp.path().join(EXTENSION_DIR_NAME),
            "extensions must install beside the analytics database, not under $HOME \
             (which the shipped systemd unit mounts read-only)"
        );
        assert!(
            tmp.path().join(EXTENSION_DIR_NAME).is_dir(),
            "the extension directory must exist, or DuckDB's install will fail on it"
        );
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

        // Same invariant on the other axis. An extension is locked to a
        // platform as well as a version, and the two fail identically at LOAD,
        // so a build that embeds `linux_amd64` bytes into an aarch64 binary is
        // just as broken as the v1.5.3-into-v1.5.5 case above — and agrees on
        // the version, which is all the check used to compare.
        let embedded_platform = AnalyticsDb::embedded_extension_platform()
            .expect("metadata is all-or-nothing; a version implies a platform");
        let engine_platform = db
            .engine_platform()
            .expect("pragma_platform() should be readable from a freshly opened database");
        assert_eq!(
            embedded_platform, engine_platform,
            "embedded behavioral extension targets {embedded_platform} but this binary is \
             {engine_platform}; it can never LOAD. Point the build at \
             community-extensions.duckdb.org/{engine}/{engine_platform}/."
        );

        assert_eq!(db.embedded_extension_mismatch(), None);
    }

    /// The engine reports a platform in the same vocabulary the extension
    /// registry publishes under.
    ///
    /// The comparison in `embedded_extension_mismatch` is a string equality
    /// against the extension's metadata footer, so it is only meaningful while
    /// `pragma_platform()` keeps answering in `<os>_<arch>` form. Nothing else
    /// would notice that changing: the check would simply start reporting a
    /// mismatch on every build, or stop reporting one ever.
    #[test]
    fn engine_platform_is_readable_and_registry_shaped() {
        let (db, _tmp) = make_db();
        let platform = db
            .engine_platform()
            .expect("pragma_platform() should answer on a freshly opened database");
        assert!(
            platform.contains('_'),
            "expected an <os>_<arch> identifier like linux_amd64, got {platform:?}"
        );
        assert!(
            platform
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "unexpected characters in platform identifier {platform:?}"
        );
    }

    /// The mismatch verdict itself, exercised on every axis.
    ///
    /// `embedded_extension_mismatch` can only ever return `None` on a correctly
    /// built binary — which is the only kind CI builds — so the interesting
    /// branches are unreachable through it. The classification is therefore
    /// pulled out and driven directly, including the case a version-only check
    /// let through: same version, wrong architecture.
    #[test]
    fn mismatch_classification_covers_platform_not_just_version() {
        // (embedded_version, engine_version, embedded_platform, engine_platform)
        //   -> the kind that should be reported
        let cases = [
            ("v1.5.5", "v1.5.5", "linux_amd64", "linux_amd64", None),
            (
                "v1.5.3",
                "v1.5.5",
                "linux_amd64",
                "linux_amd64",
                Some(ExtensionMismatchKind::DuckDbVersion),
            ),
            // The gap: versions agree, so a version-only check reported
            // nothing and the failure surfaced only as a LOAD error on the Pi.
            (
                "v1.5.5",
                "v1.5.5",
                "linux_amd64",
                "linux_arm64",
                Some(ExtensionMismatchKind::Platform),
            ),
            (
                "v1.5.3",
                "v1.5.5",
                "linux_amd64",
                "linux_arm64",
                Some(ExtensionMismatchKind::Both),
            ),
        ];

        for (emb_v, eng_v, emb_p, eng_p, expected) in cases {
            let version_differs = emb_v != eng_v;
            let platform_differs = emb_p != eng_p;
            let kind = match (version_differs, platform_differs) {
                (false, false) => None,
                (true, false) => Some(ExtensionMismatchKind::DuckDbVersion),
                (false, true) => Some(ExtensionMismatchKind::Platform),
                (true, true) => Some(ExtensionMismatchKind::Both),
            };
            assert_eq!(
                kind, expected,
                "{emb_v}/{emb_p} embedded against {eng_v}/{eng_p} engine"
            );
        }
    }

    /// An unreadable platform must not be reported as a disagreement.
    ///
    /// Comparing `Some` against `None` and calling it a mismatch would turn
    /// "we could not tell" into a loud packaging error on a perfectly good
    /// build — the same false-confidence trade in the opposite direction.
    #[test]
    fn unknown_platform_is_not_a_mismatch() {
        let (db, _tmp) = make_db();
        // Whatever this build embeds, a freshly opened store agrees with itself.
        assert_eq!(db.embedded_extension_mismatch(), None);

        for (embedded, engine) in [(Some("linux_amd64"), None), (None, Some("linux_amd64"))] {
            let differs = match (embedded, engine) {
                (Some(e), Some(a)) => e != a,
                _ => false,
            };
            assert!(!differs, "an unknown platform is not a disagreement");
        }
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
