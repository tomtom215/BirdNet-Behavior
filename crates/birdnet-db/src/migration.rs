//! Database schema migration framework.
//!
//! Uses a `schema_version` table to track applied migrations.
//! Migrations are defined as SQL strings and applied in order.

use rusqlite::Connection;
use std::fmt;

/// Migration errors.
#[derive(Debug)]
pub enum MigrationError {
    /// `SQLite` error during migration.
    Sqlite(rusqlite::Error),
    /// Migration logic error.
    Logic(String),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "migration sqlite error: {e}"),
            Self::Logic(msg) => write!(f, "migration error: {msg}"),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Logic(_) => None,
        }
    }
}

impl From<rusqlite::Error> for MigrationError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// A single database migration.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Migration version number (must be sequential starting from 1).
    pub version: u32,
    /// Human-readable description.
    pub description: &'static str,
    /// SQL to apply the migration.
    pub up_sql: &'static str,
}

/// All known migrations, in order.
///
/// Add new migrations to the end of this list. Never modify existing migrations.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Create detections table",
        up_sql: "CREATE TABLE IF NOT EXISTS detections (
            Date TEXT NOT NULL,
            Time TEXT NOT NULL,
            Sci_Name TEXT NOT NULL,
            Com_Name TEXT NOT NULL,
            Confidence REAL NOT NULL,
            Lat REAL,
            Lon REAL,
            Cutoff REAL,
            Week INTEGER,
            Sens REAL,
            Overlap REAL,
            File_Name TEXT
        );",
    },
    Migration {
        version: 2,
        description: "Add indexes for common queries",
        up_sql: "CREATE INDEX IF NOT EXISTS idx_detections_date ON detections(Date);
                 CREATE INDEX IF NOT EXISTS idx_detections_species ON detections(Com_Name);
                 CREATE INDEX IF NOT EXISTS idx_detections_sci_name ON detections(Sci_Name);
                 CREATE INDEX IF NOT EXISTS idx_detections_confidence ON detections(Confidence);",
    },
    Migration {
        version: 3,
        description: "Add date-time composite index for time-range queries",
        up_sql: "CREATE INDEX IF NOT EXISTS idx_detections_datetime ON detections(Date, Time);",
    },
    Migration {
        version: 4,
        description: "Add notification_log table",
        up_sql: "CREATE TABLE IF NOT EXISTS notification_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sent_at TEXT NOT NULL DEFAULT (datetime('now')),
            channel TEXT NOT NULL,
            species_com_name TEXT,
            species_sci_name TEXT,
            confidence REAL,
            detection_date TEXT,
            detection_time TEXT,
            status TEXT NOT NULL CHECK(status IN ('sent','failed','skipped')),
            message TEXT,
            error TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_notification_log_sent_at
            ON notification_log(sent_at DESC);
        CREATE INDEX IF NOT EXISTS idx_notification_log_channel
            ON notification_log(channel);",
    },
    Migration {
        version: 5,
        description: "Deduplicate detections and add unique constraint",
        up_sql: "DELETE FROM detections WHERE rowid NOT IN (
                     SELECT MIN(rowid) FROM detections
                     GROUP BY Date, Time, Sci_Name
                 );
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_detections_unique
                     ON detections(Date, Time, Sci_Name);",
    },
    Migration {
        version: 6,
        description: "Create species confidence thresholds table",
        up_sql: "CREATE TABLE IF NOT EXISTS species_thresholds (
            sci_name TEXT PRIMARY KEY,
            confidence_threshold REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    },
    Migration {
        version: 7,
        description: "Add is_locked column to detections for purge protection",
        up_sql: "ALTER TABLE detections ADD COLUMN is_locked INTEGER NOT NULL DEFAULT 0;
                 CREATE INDEX IF NOT EXISTS idx_detections_locked ON detections(is_locked);",
    },
    Migration {
        version: 8,
        description: "Create image_blacklist table for blocking inappropriate species images",
        up_sql: "CREATE TABLE IF NOT EXISTS image_blacklist (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sci_name TEXT NOT NULL,
            url TEXT NOT NULL,
            reason TEXT,
            blacklisted_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(sci_name, url)
        );
        CREATE INDEX IF NOT EXISTS idx_image_blacklist_sci_name ON image_blacklist(sci_name);",
    },
    Migration {
        version: 9,
        description: "Create alert_rules table for conditional detection-triggered actions",
        up_sql: "CREATE TABLE IF NOT EXISTS alert_rules (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            species_pattern TEXT,
            confidence_min REAL NOT NULL DEFAULT 0.0,
            confidence_max REAL NOT NULL DEFAULT 1.0,
            hour_start INTEGER,
            hour_end INTEGER,
            days_of_week TEXT,
            action_type TEXT NOT NULL CHECK(action_type IN ('webhook','log','suppress')),
            action_webhook_url TEXT,
            action_webhook_method TEXT NOT NULL DEFAULT 'POST',
            action_webhook_body TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_alert_rules_enabled ON alert_rules(enabled);",
    },
    Migration {
        version: 10,
        description: "Create quarantine table for rare/uncertain detections pending manual review",
        up_sql: "CREATE TABLE IF NOT EXISTS quarantine (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT NOT NULL,
            time TEXT NOT NULL,
            sci_name TEXT NOT NULL,
            com_name TEXT NOT NULL,
            confidence REAL NOT NULL,
            sf_probability REAL,
            reason TEXT NOT NULL CHECK(reason IN ('below_sf_thresh','low_confidence','manual')),
            reviewed INTEGER NOT NULL DEFAULT 0,
            approved INTEGER NOT NULL DEFAULT 0,
            file_name TEXT,
            lat REAL,
            lon REAL,
            week INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(date, time, sci_name)
        );
        CREATE INDEX IF NOT EXISTS idx_quarantine_reviewed ON quarantine(reviewed);
        CREATE INDEX IF NOT EXISTS idx_quarantine_date ON quarantine(date);
        CREATE INDEX IF NOT EXISTS idx_quarantine_sci_name ON quarantine(sci_name);",
    },
    Migration {
        version: 11,
        description: "Add chunk_offset_secs and relax detections UNIQUE so chunks per file keep all hits",
        // Migration 5 introduced UNIQUE(Date, Time, Sci_Name). It was meant
        // to deduplicate identical detections but had the unintended effect
        // of collapsing every chunk of one recording into a single row: a
        // Eurasian Magpie that calls in chunks 0–4.5 s, 4.5–9 s, 9–13.5 s,
        // … was logged exactly once because all chunks share the same
        // `Time` parsed from the filename. The station then only saw the
        // FIRST chunk's confidence (usually the lowest) and lost every
        // later — often stronger — detection of the same species in the
        // same file.
        //
        // Fix: add a `chunk_offset_secs REAL NOT NULL DEFAULT 0.0` column
        // that records the start time of the chunk within its source file,
        // and include it in the unique key alongside the existing fields.
        // The NOT NULL + DEFAULT means SQLite's NULL-is-distinct UNIQUE
        // semantics don't accidentally let through duplicates from code
        // paths that haven't been updated to populate the column (e.g.
        // the quarantine → approve path, which keeps writing with offset
        // 0). Historical rows imported from BirdNET-Pi also collapse to
        // offset 0, which matches their semantics (one detection per
        // (date, time, species) before chunking existed).
        up_sql: "ALTER TABLE detections ADD COLUMN chunk_offset_secs REAL NOT NULL DEFAULT 0.0;
                 DROP INDEX IF EXISTS idx_detections_unique;
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_detections_unique
                     ON detections(Date, Time, Sci_Name, File_Name, chunk_offset_secs);
                 CREATE INDEX IF NOT EXISTS idx_detections_chunk_offset
                     ON detections(chunk_offset_secs);",
    },
    Migration {
        version: 12,
        description: "Add correlation_id to detections for log-to-row traceability",
        // PR #49 propagated a `correlation_id` through the
        // decode→infer→notify→DB log path so an operator could grep
        // for "the one file that did X" across the stream. That id
        // never reached the database row though, which broke the
        // round trip — an admin looking at a suspicious detection in
        // the web UI couldn't pivot back to the log slice that
        // produced it. This migration carries the id to durable
        // storage.
        //
        // Nullable on purpose:
        //  - rows that pre-date this column have no id to backfill
        //    (history is fine without it);
        //  - the quarantine-approve path and the BirdNET-Pi importer
        //    don't have a daemon-generated id either, so they keep
        //    writing NULL until we wire them up;
        //  - omitting the column on insert is harmless going forward.
        //
        // No UNIQUE / no NOT NULL — we don't index on it because the
        // operator-facing lookup pattern is "find the rows for one
        // file" not "find one row by id", and a UNIQUE constraint
        // would force the importer to invent fake ids.
        up_sql: "ALTER TABLE detections ADD COLUMN correlation_id TEXT;
                 CREATE INDEX IF NOT EXISTS idx_detections_correlation_id
                     ON detections(correlation_id);",
    },
    Migration {
        version: 13,
        description: "Create detection_reviews table for manual confirm/reject triage of detections",
        // A reviewer verdict on an individual detection, identified by the
        // same (date, time, sci_name) triple the rest of the UI keys on. This
        // is an *annotation*, not a data move: a 'rejected' verdict flags a
        // likely-misidentified detection for the operator without deleting the
        // row (unlike quarantine, which gates rows *out* of `detections`
        // before they are ever admitted). UNIQUE(date, time, sci_name) makes
        // the verdict idempotent — re-reviewing the same detection updates the
        // existing verdict via INSERT … ON CONFLICT rather than piling up rows.
        up_sql: "CREATE TABLE IF NOT EXISTS detection_reviews (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT NOT NULL,
            time TEXT NOT NULL,
            sci_name TEXT NOT NULL,
            com_name TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('confirmed','rejected')),
            notes TEXT,
            reviewed_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(date, time, sci_name)
        );
        CREATE INDEX IF NOT EXISTS idx_detection_reviews_status
            ON detection_reviews(status);
        CREATE INDEX IF NOT EXISTS idx_detection_reviews_reviewed_at
            ON detection_reviews(reviewed_at DESC);",
    },
    Migration {
        version: 14,
        description: "Create users, sessions, and audit_log tables for accounts (O-15)",
        // O-15 adds the accounts surface that O-14's cookie sessions
        // build on. Day-zero shape: a single seeded `admin` row so
        // existing single-admin deployments see no behavioural change.
        // `pwd_argon2` is the canonical column name — current installs
        // store the password in the CADDY_PWD env var (not the DB), so
        // the seed row writes an empty hash and the auth-middleware
        // path still reads CADDY_PWD until the wire is flipped. See the
        // TODO(O-15-followup) markers in `accounts.rs` and the auth
        // module for the credential-store migration.
        //
        // Adapted from the O-15 accounts proposal's 009_accounts.sql for this
        // chain:
        //  - Migration version is 14, not 009 (the package was authored
        //    against an earlier numbering scheme — the chain in main has
        //    grown to 13 since).
        //  - The seed step that read from `settings WHERE key =
        //    'admin_password_hash'` was a no-op against this fork (the
        //    settings table doesn't carry that key); collapsed to one
        //    unconditional INSERT … WHERE NOT EXISTS.
        up_sql: "CREATE TABLE IF NOT EXISTS users (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT NOT NULL UNIQUE,
            pwd_argon2    TEXT NOT NULL,
            role          TEXT NOT NULL CHECK (role IN ('admin','viewer')),
            label         TEXT,
            created_at    TEXT NOT NULL DEFAULT (datetime('now')),
            disabled_at   TEXT
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id           TEXT PRIMARY KEY,
            user_id      INTEGER NOT NULL
                         REFERENCES users(id) ON DELETE CASCADE,
            issued_at    TEXT NOT NULL DEFAULT (datetime('now')),
            last_seen    TEXT NOT NULL DEFAULT (datetime('now')),
            expires_at   TEXT NOT NULL,
            user_agent   TEXT,
            ip_hash      TEXT
        );
        CREATE INDEX IF NOT EXISTS sessions_user_expires
            ON sessions (user_id, expires_at);
        CREATE INDEX IF NOT EXISTS sessions_expires
            ON sessions (expires_at);
        CREATE TABLE IF NOT EXISTS audit_log (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            at           TEXT NOT NULL DEFAULT (datetime('now')),
            user_id      INTEGER REFERENCES users(id),
            action       TEXT NOT NULL,
            target       TEXT,
            metadata     TEXT
        );
        CREATE INDEX IF NOT EXISTS audit_log_at
            ON audit_log (at DESC);
        CREATE INDEX IF NOT EXISTS audit_log_action
            ON audit_log (action, at DESC);
        INSERT INTO users (username, pwd_argon2, role, label)
        SELECT 'admin', '', 'admin', 'Administrator'
        WHERE NOT EXISTS (SELECT 1 FROM users WHERE username = 'admin');",
    },
    Migration {
        version: 15,
        description: "Create audio_sources table for first-class CRUD (O-13)",
        // O-13 replaces the audio.rs stub with a real entity model. The
        // table carries one row per microphone or RTSP stream; the audio
        // daemon continues to read `state.audio_source()` (a single
        // string) until a follow-up PR teaches it to consume the table
        // directly. See TODO(O-13-followup) in `routes::admin::audio`
        // for the daemon-side change spelled out.
        //
        // The seed step pulls from `settings.audio_source` when present
        // so anyone with a configured single-string source lands on a
        // populated page after upgrade. In this fork the source is set
        // via the `with_audio_source` builder from CLI/env rather than
        // the settings table, so the SELECT is typically a no-op — the
        // table just starts empty and the operator adds rows via /admin/audio.
        //
        // Adapted from the O-13 audio-sources proposal's 008_audio_sources.sql.
        // Renumbered to 15 (the chain has grown past 008 since the
        // package was authored).
        up_sql: "CREATE TABLE IF NOT EXISTS audio_sources (
            id            TEXT PRIMARY KEY,
            kind          TEXT NOT NULL
                          CHECK (kind IN ('usb-alsa','pipewire','rtsp')),
            device_id     TEXT NOT NULL,
            label         TEXT,
            sample_rate   INTEGER NOT NULL DEFAULT 48000
                          CHECK (sample_rate IN (8000, 16000, 22050, 32000, 44100, 48000)),
            channels      TEXT    NOT NULL DEFAULT 'mono'
                          CHECK (channels IN ('mono','left','right','stereo')),
            bit_depth     INTEGER NOT NULL DEFAULT 24
                          CHECK (bit_depth IN (16, 24)),
            gain_db       REAL    NOT NULL DEFAULT 0.0
                          CHECK (gain_db BETWEEN -24.0 AND 36.0),
            rtsp_transport TEXT   NOT NULL DEFAULT 'auto'
                          CHECK (rtsp_transport IN ('auto','tcp','udp')),
            schedule_quiet_start  TEXT,
            schedule_quiet_end    TEXT,
            pipeline_high_pass        INTEGER NOT NULL DEFAULT 1,
            pipeline_dc_removal       INTEGER NOT NULL DEFAULT 1,
            pipeline_agc              INTEGER NOT NULL DEFAULT 0,
            pipeline_rtsp_keepalive   INTEGER NOT NULL DEFAULT 1,
            disabled_at   TEXT,
            created_at    TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS audio_sources_kind_active
            ON audio_sources (kind, disabled_at);
        -- `settings` is created lazily at runtime by `settings::ensure_settings_table`,
        -- not by the migration chain, so a fresh DB does not yet have it when the
        -- seed below runs. Materialise it here (idempotent) so the SELECT … FROM
        -- settings parses against an empty table on a fresh install and against
        -- the runtime-populated table on an upgrade.
        CREATE TABLE IF NOT EXISTS settings (
            key        TEXT PRIMARY KEY NOT NULL,
            value      TEXT NOT NULL,
            category   TEXT NOT NULL DEFAULT 'general',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        INSERT INTO audio_sources (id, kind, device_id, label)
        SELECT 'src_seed_1',
               CASE
                 WHEN value LIKE 'rtsp://%' THEN 'rtsp'
                 WHEN value LIKE 'alsa_%'   THEN 'pipewire'
                 ELSE 'usb-alsa'
               END,
               value,
               NULL
          FROM settings
         WHERE key = 'audio_source'
           AND value IS NOT NULL
           AND value <> ''
           AND NOT EXISTS (SELECT 1 FROM audio_sources LIMIT 1);",
    },
    Migration {
        version: 16,
        description: "Create weather table for signal-context overlays (O-23)",
        // O-23 stores one row per 30-min slot of cached Open-Meteo data so
        // the day-strip / dawn-chorus overlays can paint without ever
        // network-fetching from a request handler. The poll job is
        // off-by-default — `BNB_WEATHER_ENABLED=1` opts in.
        //
        // The original DIFF folder named this `010_weather.sql`; renumbered
        // here to fit the actual chain position.
        up_sql: "CREATE TABLE IF NOT EXISTS weather (
            at            TEXT PRIMARY KEY,
            temp_c        REAL,
            precip_mm     REAL,
            wind_kt       REAL,
            wind_dir_deg  INTEGER,
            pressure_hpa  REAL,
            cloud_pct     INTEGER,
            code          INTEGER,
            fetched_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS weather_at ON weather (at DESC);",
    },
    Migration {
        version: 17,
        description: "Add (Date, Com_Name) composite index for per-species range analytics",
        // The heaviest analytics queries scan a date range and aggregate by
        // species: species_sparklines (streamgraph / phenology / diversity),
        // the co-occurrence self-joins on distinct (Date, Com_Name), and the
        // phenology year scan. A leading-Date composite covers the range filter
        // and the species grouping in one index, so these become index-range
        // scans instead of full-table scans — the biggest single win for
        // page-to-page navigation on a Raspberry Pi. The existing single-column
        // idx_detections_date stays for pure date lookups.
        up_sql: "CREATE INDEX IF NOT EXISTS idx_detections_date_species
                     ON detections(Date, Com_Name);",
    },
    Migration {
        version: 18,
        description: "Add Source column to tag detections by audio stream",
        // Multi-stream stations run several RTSP mics/cameras at once; the same
        // bird heard by two streams is recorded as two detections (the unique
        // key includes File_Name, which carries the stream id). To make that
        // attributable — and to enable an optional, opt-in cross-stream collapse
        // later — every new detection is tagged with its source label: the RTSP
        // stream id (e.g. `cam1`) or `local` for the on-board mic. Nullable, so
        // historical / imported BirdNET-Pi rows (unknown source) stay NULL and
        // nothing is rewritten. Indexed for per-source filtering and grouping.
        up_sql: "ALTER TABLE detections ADD COLUMN Source TEXT;
                 CREATE INDEX IF NOT EXISTS idx_detections_source ON detections(Source);",
    },
];

/// Ensure the `schema_version` tracking table exists.
fn ensure_version_table(conn: &Connection) -> Result<(), MigrationError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    Ok(())
}

/// Get the current schema version (0 if no migrations applied).
///
/// # Errors
///
/// Returns `MigrationError` on query failure.
pub fn current_version(conn: &Connection) -> Result<u32, MigrationError> {
    ensure_version_table(conn)?;
    // Propagate the error rather than swallowing it with `unwrap_or(0)`.
    // `COALESCE(MAX(version), 0)` already returns 0 for the empty table, so the
    // only thing `unwrap_or(0)` ever masked was a *real* error (e.g. a transient
    // `SQLITE_BUSY` past the busy timeout) — which would report version 0 on an
    // already-migrated DB and make `migrate` re-apply migration 1, hitting a
    // PRIMARY KEY conflict and turning a momentary lock into a fatal startup.
    let version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Apply all pending migrations.
///
/// Returns the number of migrations applied.
///
/// # Errors
///
/// Returns `MigrationError` if any migration fails. Each migration's schema
/// changes and its `schema_version` bump commit together in a single
/// transaction, so a crash or error mid-migration rolls back cleanly: the
/// database is always left at the last *fully* applied version, never with a
/// changed schema whose version went unrecorded (which on the next boot would
/// re-run the migration and hard-fail any non-idempotent step such as
/// `ALTER TABLE ADD COLUMN`).
pub fn migrate(conn: &Connection) -> Result<u32, MigrationError> {
    ensure_version_table(conn)?;
    let current = current_version(conn)?;
    let mut applied = 0;

    // Newer-DB-than-binary detection: if the DB carries a schema version this
    // binary doesn't know about, we are running an older binary against a newer
    // schema — typically because the operator downgraded. Migrations are
    // additive, so older code usually still works against newer columns/tables,
    // but it's a real failure mode that produces baffling runtime errors. Warn
    // loudly rather than error so a recovery downgrade remains possible.
    if let Some(max_known) = MIGRATIONS.iter().map(|m| m.version).max()
        && current > max_known
    {
        tracing::warn!(
            db_version = current,
            binary_max_version = max_known,
            "database schema is newer than this binary knows about — likely a downgrade. \
             The application may misbehave against unrecognised columns or tables; \
             upgrade to a binary that supports schema version {current} if available."
        );
    }

    for migration in MIGRATIONS {
        if migration.version <= current {
            continue;
        }

        // Verify sequential ordering
        if migration.version != current + applied + 1 {
            return Err(MigrationError::Logic(format!(
                "expected migration version {}, found {}",
                current + applied + 1,
                migration.version
            )));
        }

        tracing::info!(
            version = migration.version,
            description = migration.description,
            "applying migration"
        );

        // Apply the migration's DDL and record its version atomically. SQLite
        // supports transactional DDL, so a failure (or power loss) mid-migration
        // rolls the whole step back rather than leaving the schema changed but
        // the version unrecorded.
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(migration.up_sql)?;
        tx.execute(
            "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.description],
        )?;
        tx.commit()?;

        applied += 1;
    }

    if applied > 0 {
        tracing::info!(
            applied,
            new_version = current + applied,
            "migrations complete"
        );
    }

    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        conn
    }

    #[test]
    fn fresh_db_starts_at_version_zero() {
        let conn = memory_db();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn migrate_applies_all_migrations() {
        let conn = memory_db();
        let applied = migrate(&conn).unwrap();
        let expected = u32::try_from(MIGRATIONS.len()).unwrap();
        assert_eq!(applied, expected);
        assert_eq!(current_version(&conn).unwrap(), expected);
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = memory_db();
        let first = migrate(&conn).unwrap();
        let second = migrate(&conn).unwrap();
        assert!(first > 0);
        assert_eq!(second, 0);
    }

    #[test]
    fn migrate_succeeds_against_newer_db_version() {
        // Simulate a downgrade: a binary that knows up to schema vN runs
        // against a DB written by a newer binary at vN+5. `migrate` should
        // log a warning but still return 0 (no migrations to apply) — older
        // additive schemas usually still work for the older binary.
        let conn = memory_db();
        migrate(&conn).unwrap();
        let known_max = MIGRATIONS.iter().map(|m| m.version).max().unwrap();
        // Force the version forward as if a newer binary had written it.
        conn.execute(
            "INSERT INTO schema_version (version, description) VALUES (?1, 'future')",
            rusqlite::params![known_max + 5],
        )
        .unwrap();

        let applied = migrate(&conn).expect("downgrade must not error");
        assert_eq!(applied, 0, "no migrations should apply on a newer DB");
        assert_eq!(current_version(&conn).unwrap(), known_max + 5);
    }

    #[test]
    fn detections_table_exists_after_migration() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-03-11', '08:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.87)",
            [],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn indexes_exist_after_migration() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='detections'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // 6 explicit + 1 implicit rowid index
        assert!(
            index_count >= 6,
            "expected at least 6 indexes, got {index_count}"
        );
    }

    #[test]
    fn version_table_tracks_history() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        let rows: Vec<(u32, String)> = conn
            .prepare("SELECT version, description FROM schema_version ORDER BY version")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        assert_eq!(rows.len(), MIGRATIONS.len());
        assert_eq!(rows[0].0, 1);
        assert_eq!(rows[0].1, "Create detections table");
    }
}
