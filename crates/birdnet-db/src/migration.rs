//! Database schema migration framework.
//!
//! Uses a `schema_version` table to track applied migrations.
//! Migrations are defined as SQL strings and applied in order.

use rusqlite::Connection;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

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
    Migration {
        version: 19,
        description: "Create outbound_queue for store-and-forward BirdWeather uploads",
        // A field station on flaky Wi-Fi/LTE loses every BirdWeather upload
        // that fails after its in-flight retries — real data loss, since the
        // community-science record is append-only and accepts late posts (the
        // payload carries its own timestamp). Failed uploads are parked here
        // and replayed by a background drainer once the network returns.
        //
        // Deliberately generic (`kind` column) so future channels can opt in,
        // but MQTT and Apprise/email stay fire-and-forget BY DESIGN: they are
        // live telemetry / look-now alerts, and replaying them hours later is
        // worse than dropping them. The local database remains ground truth.
        //
        // `next_attempt_at` is unix seconds (monotonic enough for a queue and
        // cheap to index); `attempts` counts replay attempts by the drainer,
        // not the original in-flight retries.
        up_sql: "CREATE TABLE IF NOT EXISTS outbound_queue (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            kind            TEXT NOT NULL,
            payload         TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            attempts        INTEGER NOT NULL DEFAULT 0,
            next_attempt_at INTEGER NOT NULL DEFAULT 0,
            last_error      TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_outbound_queue_due
            ON outbound_queue (kind, next_attempt_at);",
    },
    Migration {
        version: 20,
        description: "Add Duration_Secs to detections for the saved clip's length",
        // The Recordings Clips browser wants to show how long each saved clip
        // is. The extractor already knows the extracted clip's length (its
        // sample count ÷ sample rate); persist it so the grid renders a real
        // duration instead of omitting the column. Nullable like
        // `correlation_id` / `Source`: historical and BirdNET-Pi-imported rows,
        // and the quarantine-approve path (which re-inserts without
        // re-extracting), have no clip length to record and stay NULL — never a
        // faked value. Not indexed: nothing filters or sorts by duration.
        up_sql: "ALTER TABLE detections ADD COLUMN Duration_Secs REAL;",
    },
    Migration {
        version: 21,
        description: "Track background maintenance runs so schedules survive restarts",
        // Scheduled maintenance (integrity check, session prune, per-species
        // recording cap, backup + VACUUM) used to be driven by tokio intervals
        // measured from process start. That silently disabled every job on any
        // station restarting more often than the job's period: a settings
        // change ("applies on restart"), an update, a power cut, or a watchdog
        // bounce reset the clock, so a station rebooting daily never ran the
        // weekly backup + VACUUM — not once, ever. Backups are the only input
        // to `resilience::check_and_recover`, so that turned recoverable
        // corruption into total data loss on exactly the unattended
        // deployments the schedule exists to protect.
        //
        // Recording each job's last completion here makes the schedule
        // wall-clock based and restart-durable: on boot the loop reads what is
        // actually overdue and runs it. Keyed by a stable job name; the value
        // is Unix seconds, so it is comparable across reboots and clock zones
        // (and never a locale-formatted string).
        //
        // A dedicated table, not `settings`: this is internal scheduler state,
        // not an operator-editable preference, and it must not surface in the
        // admin settings form or be clobbered by a settings import.
        up_sql: "CREATE TABLE IF NOT EXISTS maintenance_runs (
            job TEXT PRIMARY KEY,
            last_run_unix INTEGER NOT NULL
        );",
    },
    Migration {
        version: 22,
        description: "Record when a detection's clip was reclaimed, without losing its name",
        // Retention has to reclaim audio eventually — the per-species cap and
        // the disk-full purge both delete clip files. The question is what
        // happens to the row that pointed at one.
        //
        // Clearing `File_Name` was the obvious answer and the wrong one: the
        // filename is *evidence*. It carries the capture timestamp and source
        // the clip was cut from, it is how a detection is matched back to an
        // archived copy or an offline analysis, and a researcher re-examining a
        // season of data should still be able to see that a detection had audio
        // and what it was called. Retention must reclaim disk, never provenance.
        //
        // So the name stays and this column records *when* the audio went. That
        // is strictly more information than before — the row now distinguishes
        // "never had a clip" (NULL name) from "had one, reclaimed on this date"
        // — and it gives the reader queries a precise way to exclude clips that
        // can no longer be played, so the browser stops offering a dead play
        // button and the retention pass stops re-selecting rows it already
        // handled.
        //
        // Nullable and unindexed: NULL means "audio still present", which is
        // the overwhelming majority, and nothing filters on the timestamp
        // itself — only on its presence, alongside `File_Name` predicates that
        // already scan the same rows.
        up_sql: "ALTER TABLE detections ADD COLUMN Clip_Pruned_At INTEGER;",
    },
    Migration {
        version: 23,
        description: "Make the detections UNIQUE key NULL-insensitive so a re-import cannot duplicate",
        // Every duplicate-suppression path in this project — the detection
        // pipeline's writes, and above all the BirdNET-Pi importer's
        // `INSERT OR IGNORE` — rests entirely on `idx_detections_unique`.
        // `File_Name` is part of that key and is nullable, and SQLite considers
        // NULLs distinct in a UNIQUE index. A row with no filename therefore
        // conflicts with nothing, and `INSERT OR IGNORE` ignores nothing.
        //
        // For an importing user that is the worst shape a bug can take. The
        // CSV/TSV path yields NULL for an empty `File_Name` field, for `\\N`,
        // for the literal `NULL`, and for any row that simply has fewer than
        // twelve columns — so re-importing the same export doubled those rows,
        // silently, and reported "imported N, skipped 0" as success. Anyone who
        // re-ran an import after a failure (the only recovery this offers, as
        // batches commit as they go) doubled their history and had every
        // dashboard, rate and analytic quietly computed over it. Data you
        // cannot trust is worse than data you do not have.
        //
        // `chunk_offset_secs` took `NOT NULL DEFAULT` for exactly this reason
        // (migration 11). `File_Name` cannot: NULL is *meaningful* there —
        // migration 22 made it the difference between "never had a clip" and
        // "had one, reclaimed on this date", and `locks.rs` filters on
        // `IS NOT NULL`. So the index absorbs the NULL instead of the column,
        // via `COALESCE`, leaving every existing semantic and query untouched.
        //
        // The DELETE first is a repair, not just a precondition for the index:
        // databases that already took a double import carry the duplicates
        // now, and creating the index over them would fail. It keeps the
        // earliest row of each group (`MIN(rowid)`), which is the
        // first-imported one. Nothing references detections by rowid — other
        // tables key on (Date, Time, Sci_Name) — so collapsing them orphans
        // nothing.
        up_sql: "DELETE FROM detections
                   WHERE rowid NOT IN (
                     SELECT MIN(rowid) FROM detections
                      GROUP BY Date, Time, Sci_Name, COALESCE(File_Name, ''), chunk_offset_secs
                   );
                 DROP INDEX IF EXISTS idx_detections_unique;
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_detections_unique
                     ON detections(Date, Time, Sci_Name, COALESCE(File_Name, ''), chunk_offset_secs);",
    },
    Migration {
        version: 24,
        description: "Fold chunk_offset_secs into Date/Time so a chunk's timestamp is when it was heard",
        // Every chunk of one recording used to be stamped with the *file's*
        // start time. A 15-second segment is five 3-second chunks, so five rows
        // landed on a single instant, differing only in `chunk_offset_secs` —
        // a column the detections API does not even return. One continuous song
        // therefore read as five duplicate detections in the UI, and every
        // time-bucketed analytic (sessionisation, gap analysis, the dawn-chorus
        // curve) saw five simultaneous detections that never happened.
        //
        // BirdNET-Pi has always added the offset, in its `Detection`
        // constructor: `file_date + timedelta(seconds=self.start)`. So this
        // table has been holding two conventions at once — imported BirdNET-Pi
        // rows with chunk-accurate times, and natively recorded rows without.
        // The pipeline now adds the offset at inference (matching BirdNET-Pi's
        // placement); this brings the history that is already on disk onto the
        // same convention, which is the only way the two stop disagreeing.
        //
        // Safe to target by `chunk_offset_secs > 0` alone. The column is
        // `NOT NULL DEFAULT 0.0` (migration 11), and everything that is already
        // correctly stamped carries 0: BirdNET-Pi imports (the importer does
        // not write the column at all), the quarantine → approve path, and the
        // first chunk of every recording, whose offset is genuinely zero.
        //
        // The `datetime(...) IS NOT NULL` guard is not defensive padding.
        // `Date`/`Time` are free-form `TEXT NOT NULL` — the column type forbids
        // NULL, not nonsense — and a station's history can hold values that name
        // no point in time (a NULL `Date` arrives from the importer as ""). For
        // those rows `datetime()` yields NULL, and without the guard this
        // statement would write NULL into a NOT NULL column and abort the whole
        // migration. They are left exactly as they are: unplaceable in, and
        // unplaceable out.
        //
        // Both SET expressions read the pre-update row (SQLite evaluates an
        // UPDATE's right-hand sides against the old values), so `Time` using
        // `Date` is correct rather than order-dependent. The offset truncates to
        // whole seconds because `Time` has no sub-second resolution to put them
        // in — the same truncation the pipeline applies.
        //
        // No unique-key collision is reachable: two chunks of one file shift by
        // different amounts, and a shifted native row keeps its non-zero
        // `chunk_offset_secs`, so it can never land on an offset-0 imported row.
        up_sql: "UPDATE detections
                    SET Date = strftime('%Y-%m-%d',
                            datetime(Date || ' ' || Time,
                                     '+' || CAST(chunk_offset_secs AS INTEGER) || ' seconds')),
                        Time = strftime('%H:%M:%S',
                            datetime(Date || ' ' || Time,
                                     '+' || CAST(chunk_offset_secs AS INTEGER) || ' seconds'))
                  WHERE chunk_offset_secs > 0
                    AND datetime(Date || ' ' || Time) IS NOT NULL;",
    },
    Migration {
        version: 25,
        description: "Record where imported detections came from, so a merged history stays attributable",
        // Until now an import was indistinguishable from a recording. The
        // BirdNET-Pi importer copies every row through verbatim and the
        // destination has no column that says otherwise, so after importing
        // another station's history there is no query that separates the two.
        //
        // That is fine when the two stations are the same station. It is not
        // fine otherwise, and nothing checked: the validator's four checks are
        // table-readable, non-empty, date-format and confidence-range, none of
        // which involves `Lat`/`Lon` or a timezone. So a merged database could
        // silently contain detections from two sites and two clocks, and every
        // location- and hour-dependent analytic — solar overlays, the dawn
        // chorus, sessionisation, "first of year" — would read it as one.
        //
        // For a research station that is the difference between a dataset and a
        // dataset you have to throw away, because the damage is not detectable
        // after the fact. This is the column that makes it detectable, and it
        // has to exist before the import that needs it.
        //
        // `import_batch_id IS NULL` means "this station recorded it", which is
        // true of every row that already exists and every row recorded from here
        // on. Nothing is rewritten and no existing query changes meaning.
        up_sql: "CREATE TABLE IF NOT EXISTS import_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            imported_at TEXT NOT NULL DEFAULT (datetime('now')),
            source_kind TEXT NOT NULL,
            source_label TEXT,
            source_path TEXT,
            source_lat REAL,
            source_lon REAL,
            station_lat REAL,
            station_lon REAL,
            distance_km REAL,
            source_utc_offset_secs INTEGER,
            applied_shift_secs INTEGER NOT NULL DEFAULT 0,
            row_count INTEGER NOT NULL DEFAULT 0,
            notes TEXT
        );
        ALTER TABLE detections ADD COLUMN import_batch_id INTEGER REFERENCES import_batches(id);
        CREATE INDEX IF NOT EXISTS idx_detections_import_batch
            ON detections(import_batch_id);",
    },
    Migration {
        version: 26,
        description: "Carry the reviewer's verdict on the detection, so curation can reach the analytics",
        // `detection_reviews` (migration 13) has stored confirmed/rejected
        // verdicts since it landed, and exactly one surface ever read them: the
        // quality dashboard's own "Review verdict trend" panel. Every other
        // analytic — species counts, the life list, the heat map, the dawn
        // chorus, phenology, every behavioural and time-series query — counted
        // rejected detections exactly as it counted confirmed ones.
        //
        // So an operator could spend a season rejecting false positives and
        // every chart would look exactly as it did before. The only way to make
        // a rejection *mean* anything was to delete the detection instead, which
        // discards the evidence — the opposite of what a reviewable record is
        // for.
        //
        // For a research station this is the gap between "a log of what a model
        // reported" and "a dataset whose numbers a reviewer stands behind".
        //
        // The verdict is denormalised onto the detection rather than joined at
        // query time for two reasons: it makes the exclusion a single indexable
        // predicate that reads identically in SQLite and DuckDB, and it rides
        // the existing column-copy sync into the OLAP store for free — a join
        // would have needed a second table mirrored and kept in step.
        // `detection_reviews` remains the record of *who said what and when*;
        // this column is the current verdict, maintained beside it.
        //
        // Backfilled from the existing table so verdicts already recorded take
        // effect immediately rather than only for reviews made from now on.
        //
        // `detections_analytic` is where the verdict becomes real on the SQLite
        // side, mirroring what `detections_ts` does in DuckDB. The split between
        // the view and the raw table is deliberate and is the whole design:
        //
        // * **Aggregates** read the view. A count, a heat map, a phenology curve
        //   or a species total is a claim about what was *there*, and a reviewer
        //   who rejected a detection has said it was not.
        // * **Record-level surfaces** — the Today list, detection detail,
        //   recordings, the review queue itself — keep reading the raw table. A
        //   reviewer has to be able to see a rejected detection in order to
        //   listen to it again and change their mind, and a verdict that hid its
        //   own evidence would be a trap.
        //
        // `IS NOT 'rejected'` rather than `<> 'rejected'`: SQLite's `<>` against
        // NULL yields NULL, which `WHERE` treats as false, so the plain
        // comparison would exclude every *unreviewed* detection — almost all of
        // them — and empty the dashboards on any station with a review backlog.
        // `IS NOT` is SQLite's null-safe inequality and keeps NULL (unreviewed)
        // in the view. The DuckDB view uses `IS DISTINCT FROM` for the same
        // reason.
        up_sql: "ALTER TABLE detections ADD COLUMN review_verdict TEXT;
        UPDATE detections
           SET review_verdict = (
                 SELECT r.status FROM detection_reviews r
                  WHERE r.date = detections.Date
                    AND r.time = detections.Time
                    AND r.sci_name = detections.Sci_Name)
         WHERE EXISTS (
                 SELECT 1 FROM detection_reviews r
                  WHERE r.date = detections.Date
                    AND r.time = detections.Time
                    AND r.sci_name = detections.Sci_Name);
        CREATE INDEX IF NOT EXISTS idx_detections_review_verdict
            ON detections(review_verdict);
        CREATE VIEW IF NOT EXISTS detections_analytic AS
            SELECT * FROM detections WHERE review_verdict IS NOT 'rejected';",
    },
    Migration {
        version: 27,
        description: "Record how long the station actually listened, so counts can be normalised",
        // A detection count is not an abundance. It is a count of detections
        // divided by nothing, and the denominator moves: a solar recording
        // window lengthens by six hours between December and June, a week of
        // downtime removes seven days of listening, a failed microphone halves
        // the channels. Every one of those changes the count without changing a
        // single bird.
        //
        // Comparing raw counts across seasons or across years — which is the
        // whole point of running a station for years — therefore measures the
        // station as much as the birds. The correction is elementary and
        // standard (detections per unit listening effort); what was missing was
        // anywhere to put the effort.
        //
        // `birdnet-behavioral` has shipped `effort_corrected_abundance_sql`
        // since the phenology module landed, joining a `recordings` table that
        // existed only in that module's own tests. This is the real one.
        //
        // Per (date, source) rather than per day: a station with three
        // microphones where one dies has not lost a third of its listening if
        // the other two cover the same airspace, and only the operator can say
        // which. Storing the breakdown keeps that decision available instead of
        // baking one interpretation into the schema.
        up_sql: "CREATE TABLE IF NOT EXISTS recording_effort (
            date TEXT NOT NULL,
            source TEXT NOT NULL,
            seconds REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (date, source)
        );
        CREATE INDEX IF NOT EXISTS idx_recording_effort_date
            ON recording_effort(date);",
    },
    Migration {
        version: 28,
        description: "Remember whether a maintenance job passed, not just when it ran",
        // `maintenance_runs` (migration 21) recorded *when* each job last
        // completed and nothing about how it went, so the only way to learn
        // whether the database was sound was to check it again.
        //
        // The health badge did exactly that. It sits in `layout.html`, on every
        // page, with `hx-trigger="load, every 30s"`, and it ran a full
        // `PRAGMA quick_check` each time. That pragma reads every page of the
        // database file. Measured on a three-year station (2.76 M detections,
        // 1.29 GB) on NVMe it costs 1.5-1.9 s warm; the enclosing partial took
        // 3.8 s. A Raspberry Pi reading the same file from an SD card at
        // ~45 MB/s is looking at ~30 s — longer than the refresh interval, so
        // the checks would overlap, and every open browser tab adds another
        // full read of the database twice a minute, forever, competing with the
        // detection write path for the same card.
        //
        // The daily integrity check already runs; it simply threw its answer
        // away. Storing it turns the badge into a read of one row.
        //
        // `ok` is nullable on purpose: NULL means "this job has no pass/fail to
        // report" — either it predates this column or it is a job like the
        // session prune that cannot fail meaningfully. A never-run integrity
        // check has no row at all, which is a third state the badge must not
        // confuse with a failure.
        up_sql: "ALTER TABLE maintenance_runs ADD COLUMN ok INTEGER;",
    },
    Migration {
        version: 29,
        description: "Cover the whole-history aggregates the species screens run on every load",
        // The species list, the life list and the per-species hour histogram
        // each aggregate the *entire* detection history, uncached, on every
        // page load. That is fine for a season and not fine for the multi-year
        // station this project is for: the work grows linearly with how long
        // the station has been useful.
        //
        // Measured on a seeded three-year station — 2 755 374 detections,
        // 1.43 GB, warm page cache, x86_64 NVMe (a Raspberry Pi reading an SD
        // card is several times worse across the board):
        //
        //   query                                 before    after
        //   species list (GROUP BY Com,Sci)        4.96 s    1.31 s
        //   life-list firsts (MIN per Sci_Name)    4.12 s    0.58 s
        //   per-species hour histogram             4.82 s    1.15 s
        //
        // The existing indexes are single-column (`Com_Name`, `Sci_Name`), so
        // every one of these plans scanned an index and then went back to the
        // table for the other columns. These two are chosen to be *covering*
        // for those aggregates — `review_verdict` is in each because every one
        // of them reads `detections_analytic`, whose WHERE clause needs it, and
        // a covering index that omits it stops covering.
        //
        // Cost, measured on the same database rather than estimated:
        //   * +130.6 MB, 9.0 % of the file. A third index (Com,Sci,Conf,verdict)
        //     would take the species list to 0.31 s but cost 18.6 % in total,
        //     which is the wrong trade on an SD card for a further ~1 s.
        //   * Inserts 0.20 ms -> 0.27 ms per committed row (4 922 -> 3 666
        //     rows/s). A station producing a few detections a second is three
        //     orders of magnitude below that, so the write path does not care.
        //   * ~7 s to build on this hardware, once, during the migration.
        up_sql: "CREATE INDEX IF NOT EXISTS idx_detections_species_hour_cover
            ON detections(Com_Name, Time, Sci_Name, Confidence, review_verdict);
        CREATE INDEX IF NOT EXISTS idx_detections_sci_first_cover
            ON detections(Sci_Name, Date, Time, review_verdict);",
    },
    Migration {
        version: 30,
        description: "Maintain the species totals on write, so reading them stops costing the whole history",
        // Migration 29 made the species aggregates cheaper. It did not make them
        // *bounded*: they still read every detection ever recorded, so the cost
        // of opening the species list grows with how long the station has been
        // worth running. At ten years it is back where it started.
        //
        // `species_summary` is those aggregates kept up to date on write. It is
        // grouped by (Com_Name, Sci_Name, hour-of-day), which is the coarsest
        // grouping that still answers all of:
        //
        //   * the species list        -- SUM over a species' 24 hour buckets
        //   * the per-species hour histogram -- that species' 24 rows, directly
        //   * average confidence      -- confidence_sum / detections
        //
        // A station with 200 species holds at most 4 800 rows here, so every one
        // of those reads is a scan of a few thousand rows instead of millions,
        // and stays that way in year ten.
        //
        // ## Why triggers and not a maintenance call
        //
        // `detections` is written from four crates and at least eight call
        // sites: the capture pipeline, the BirdNET-Pi importer, the CSV
        // reimporter, quarantine release, relabelling, verdict apply/undo,
        // single-row admin delete, and the store reset. A summary maintained by
        // calling a Rust function would need every one of those to remember,
        // and the ninth — written next year by someone who has never read this
        // comment — would drift silently, which is worse than being slow.
        //
        // A trigger cannot be forgotten. It fires on the table, so every path
        // that reaches the table is covered by construction, including paths
        // that do not exist yet.
        //
        // ## What the triggers are maintaining
        //
        // The summary is a pure function of five things about a detection:
        // Com_Name, Sci_Name, SUBSTR(Time,1,2), Confidence, and whether the
        // review verdict is 'rejected'. Count and sum are exactly reversible, so
        // insert adds, delete subtracts, and update withdraws the old row's
        // contribution and admits the new one. Nothing here needs a recompute.
        //
        // MIN/MAX are deliberately *not* stored. They are not reversible: a
        // delete of the earliest detection cannot be undone without rescanning
        // the species. The life list's first-seen query stays on migration 29's
        // covering index (0.58 s at 2.76 M rows, the cheapest of the three)
        // rather than buy a second maintenance rule that could drift.
        //
        // ## The UPDATE guard
        //
        // The update trigger's WHEN clause names exactly that dependency set, so
        // an update that touches none of it does no work at all. This is not a
        // micro-optimisation: `maintenance.rs` sets `Clip_Pruned_At` in bulk and
        // the lock/unlock handlers set `is_locked`, and an unguarded trigger
        // would turn each of those rows into a withdraw plus an admit of the
        // same bucket -- two index writes to reach the number it started from.
        //
        // ## Ordering note for whoever adds migration 31
        //
        // These triggers exist from here on, so a later migration that rewrites
        // `detections` in bulk will fire them and the summary will follow along.
        // That is the intent. A migration that rebuilds the table by
        // create-copy-drop-rename must drop the summary triggers first and
        // re-run the backfill after, or it will double-count the copy.
        //
        // `INSERT OR REPLACE` on `detections` would also drift, because
        // `recursive_triggers` is off by default and the implied delete would
        // not fire the delete trigger. There is none today -- every importer
        // uses `INSERT OR IGNORE`, whose ignored rows correctly fire nothing --
        // and `species_summary_is_maintained_by_every_write_path` fails if one
        // appears.
        up_sql: "CREATE TABLE IF NOT EXISTS species_summary (
            Com_Name       TEXT    NOT NULL,
            Sci_Name       TEXT    NOT NULL,
            hour           TEXT    NOT NULL,
            detections     INTEGER NOT NULL,
            confidence_sum REAL    NOT NULL,
            PRIMARY KEY (Com_Name, Sci_Name, hour)
        ) WITHOUT ROWID;

        DELETE FROM species_summary;
        INSERT INTO species_summary (Com_Name, Sci_Name, hour, detections, confidence_sum)
            SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2), COUNT(*), SUM(Confidence)
              FROM detections
             WHERE review_verdict IS NOT 'rejected'
             GROUP BY Com_Name, Sci_Name, SUBSTR(Time, 1, 2);

        DROP TRIGGER IF EXISTS species_summary_ai;
        CREATE TRIGGER species_summary_ai AFTER INSERT ON detections
        WHEN NEW.review_verdict IS NOT 'rejected'
        BEGIN
            INSERT INTO species_summary (Com_Name, Sci_Name, hour, detections, confidence_sum)
            VALUES (NEW.Com_Name, NEW.Sci_Name, SUBSTR(NEW.Time, 1, 2), 1, NEW.Confidence)
            ON CONFLICT(Com_Name, Sci_Name, hour) DO UPDATE SET
                detections     = species_summary.detections + 1,
                confidence_sum = species_summary.confidence_sum + NEW.Confidence;
        END;

        DROP TRIGGER IF EXISTS species_summary_ad;
        CREATE TRIGGER species_summary_ad AFTER DELETE ON detections
        WHEN OLD.review_verdict IS NOT 'rejected'
        BEGIN
            UPDATE species_summary
               SET detections     = detections - 1,
                   confidence_sum = confidence_sum - OLD.Confidence
             WHERE Com_Name = OLD.Com_Name
               AND Sci_Name = OLD.Sci_Name
               AND hour     = SUBSTR(OLD.Time, 1, 2);
            DELETE FROM species_summary
             WHERE Com_Name = OLD.Com_Name
               AND Sci_Name = OLD.Sci_Name
               AND hour     = SUBSTR(OLD.Time, 1, 2)
               AND detections <= 0;
        END;

        DROP TRIGGER IF EXISTS species_summary_au;
        CREATE TRIGGER species_summary_au AFTER UPDATE ON detections
        WHEN OLD.Com_Name   IS NOT NEW.Com_Name
          OR OLD.Sci_Name   IS NOT NEW.Sci_Name
          OR OLD.Time       IS NOT NEW.Time
          OR OLD.Confidence IS NOT NEW.Confidence
          OR (OLD.review_verdict IS 'rejected') IS NOT (NEW.review_verdict IS 'rejected')
        BEGIN
            UPDATE species_summary
               SET detections     = detections - 1,
                   confidence_sum = confidence_sum - OLD.Confidence
             WHERE OLD.review_verdict IS NOT 'rejected'
               AND Com_Name = OLD.Com_Name
               AND Sci_Name = OLD.Sci_Name
               AND hour     = SUBSTR(OLD.Time, 1, 2);

            DELETE FROM species_summary
             WHERE OLD.review_verdict IS NOT 'rejected'
               AND Com_Name = OLD.Com_Name
               AND Sci_Name = OLD.Sci_Name
               AND hour     = SUBSTR(OLD.Time, 1, 2)
               AND detections <= 0;

            INSERT INTO species_summary (Com_Name, Sci_Name, hour, detections, confidence_sum)
            SELECT NEW.Com_Name, NEW.Sci_Name, SUBSTR(NEW.Time, 1, 2), 1, NEW.Confidence
             WHERE NEW.review_verdict IS NOT 'rejected'
            ON CONFLICT(Com_Name, Sci_Name, hour) DO UPDATE SET
                detections     = species_summary.detections + 1,
                confidence_sum = species_summary.confidence_sum + NEW.Confidence;
        END;",
    },
    Migration {
        version: 31,
        description: "Record what the microphones sound like, so a failing one is visible before the season is",
        // ## The failure this exists to make visible
        //
        // Everything else this station measures is about the birds. Nothing
        // measures the *station*. Over a year in a sealed enclosure the most
        // likely silent failure is not the software: it is a microphone that
        // stops hearing — water in the capsule, a spider's web across the port,
        // a connector working loose in a thermal cycle, a preamp drifting.
        //
        // Every one of those presents identically: fewer detections. Which is
        // also what autumn looks like. The detection deadman only fires when a
        // station goes *silent*; a microphone at half sensitivity keeps
        // detecting the loud, close birds and quietly stops hearing everything
        // else, and no gauge in this project can tell that from a quiet season.
        //
        // The measurement that separates them is the station's own noise floor.
        // Ambient background does not go away when the birds do; if the floor
        // drops 20 dB and stays down, the microphone is deaf, not the wood.
        //
        // ## Shape
        //
        // One row per (date, hour, source). A station with three sources
        // accumulates 72 rows a day — 26 000 a year, a rounding error next to
        // the detections — and the hour bucket is the finest grain any of this
        // is read at.
        //
        // `samples` plus the three `*_sum` columns are kept rather than
        // pre-averaged means, for the same reason `species_summary` keeps a
        // `confidence_sum`: a sum and a count can absorb another observation
        // without revisiting the ones already folded in, and a mean cannot.
        //
        // `noise_floor_min_dbfs` is kept beside the mean because the two answer
        // different questions. The mean tracks the hour's typical background;
        // the minimum is the quietest the station heard, which is the value a
        // dying microphone drags down first and hardest.
        up_sql: "CREATE TABLE IF NOT EXISTS audio_levels (
            date                 TEXT    NOT NULL,
            hour                 INTEGER NOT NULL,
            source               TEXT    NOT NULL,
            samples              INTEGER NOT NULL,
            noise_floor_sum_dbfs REAL    NOT NULL,
            noise_floor_min_dbfs REAL    NOT NULL,
            snr_sum_db           REAL    NOT NULL,
            flatness_sum         REAL    NOT NULL,
            rain_samples         INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, hour, source)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS idx_audio_levels_date ON audio_levels(date);",
    },
    Migration {
        version: 32,
        description: "Give every detection a monotonic instant beside its local wall clock",
        // ## What this table has never been able to say
        //
        // `Date` and `Time` are local wall clock with no offset recorded — the
        // shape BirdNET-Pi wrote and this fork kept, deliberately, because
        // compatibility with a decade of existing databases is worth real
        // money. The cost is that the column pair is not a point in time:
        //
        //   * one local hour repeats every autumn, so two detections an hour
        //     apart carry identical `Date`/`Time` and `ORDER BY Date, Time` is
        //     wrong inside it;
        //   * one local hour never happens every spring, so any elapsed-time
        //     arithmetic across it over-reads by an hour;
        //   * `julianday(b) - julianday(a)` across either transition is off by
        //     an hour in one direction or the other, which is how a
        //     sessionisation gap threshold of 30 minutes sees a *negative*
        //     55-minute gap and splits a session that never broke;
        //   * an imported history from another station has to be shifted onto
        //     this station's clock at import, permanently, because there is
        //     nowhere to record that it was on a different one.
        //
        // Every one of those is the same missing column, so this adds it rather
        // than fixing them one at a time. `Date`/`Time` are untouched and stay
        // the display and grouping key; `detected_at_utc` is what ordering,
        // gaps and durations move onto.
        //
        // ## Why the backfill is trustworthy, and where it is not
        //
        // SQLite's `'utc'` modifier converts a local timestamp using the host's
        // tz database **for the date given**, not for today. Verified on this
        // machine before relying on it: Europe/Berlin yields +1 h for a January
        // timestamp and +2 h for a July one; Australia/Sydney yields +11 h and
        // +10 h respectively. So history recorded under a different offset is
        // converted with the offset that was actually in force, which is a
        // materially better answer than stamping everything with today's.
        //
        // Two dates it cannot get right, because the information is not there:
        //
        //   * **The repeated hour.** Local 02:30 on a Berlin fall-back day is
        //     two real instants; `strftime` returns the later one (01:30Z on
        //     2026-10-25, the CET reading). Both passes therefore backfill to
        //     the same instant. There is nothing in the row to do better with.
        //   * **The hour that never happened.** Local 02:30 on a Berlin
        //     spring-forward day does not exist; `strftime` collapses it onto
        //     00:30Z, the same instant as local 01:30, rather than returning
        //     NULL. Imported histories can contain such times.
        //
        // Both are recorded here so the next reader does not have to rediscover
        // them, and neither is a reason to skip the backfill: an instant that is
        // an hour out for two hours a year is strictly better than no instant
        // at all for every hour of every year.
        //
        // ## Unplaceable rows, and the guard this does *not* need
        //
        // `Date`/`Time` are free-form `TEXT NOT NULL` — the column type forbids
        // NULL, not nonsense — so a station's history can hold values naming no
        // point in time (a NULL source `Date` arrives from the importer as "").
        // Those rows keep a NULL instant and stay exactly as unplaceable as they
        // were.
        //
        // Migration 24 needed an explicit `datetime(...) IS NOT NULL` guard for
        // that, and this deliberately does not, because the situations differ:
        // 24 wrote its result back into a `NOT NULL` column, so an unparseable
        // row would have aborted the whole migration. `detected_at_utc` is
        // nullable, and `strftime` already yields NULL for input it cannot
        // parse — checked directly rather than assumed: `''`, `' '` and
        // `'not-a-date 25:99:99'` all return NULL. A guard here would change no
        // row's outcome, and a guard that looks protective without being
        // protective is worse than none: the next reader budgets for it.
        //
        // The trigger is the `species_summary` lesson applied again: this table
        // is written from four crates and at least eight call sites, and the
        // ninth — written next year by someone who has never read this comment —
        // would leave the column NULL and silently fall out of every ordering
        // that uses it. A trigger fires on the table, so it covers paths that do
        // not exist yet. It is guarded on `IS NULL`, so the write paths that set
        // the value explicitly (which is all of them today, and which can do
        // better than this for a live detection — see
        // `sqlite::queries::detections::write`) pay one `WHEN` evaluation and
        // nothing else.
        up_sql: "ALTER TABLE detections ADD COLUMN detected_at_utc INTEGER;

        UPDATE detections
           SET detected_at_utc = CAST(strftime('%s', Date || ' ' || Time, 'utc') AS INTEGER);

        CREATE INDEX IF NOT EXISTS idx_detections_utc
            ON detections(detected_at_utc DESC);

        DROP TRIGGER IF EXISTS detections_stamp_utc;
        CREATE TRIGGER detections_stamp_utc AFTER INSERT ON detections
        WHEN NEW.detected_at_utc IS NULL
        BEGIN
            UPDATE detections
               SET detected_at_utc =
                     CAST(strftime('%s', NEW.Date || ' ' || NEW.Time, 'utc') AS INTEGER)
             WHERE rowid = NEW.rowid;
        END;",
    },
    Migration {
        version: 33,
        description: "Retire three indexes nothing reads, and make two of the rest partial",
        // ## What was measured
        //
        // A synthetic three-year station — 3 285 000 rows at 3 000 detections a
        // day, 180 species, this schema and every index on it, `ANALYZE` run —
        // comes to **1.83 GB, of which 73.7 % is index rather than data**.
        // Migrations 29 and 30 benchmarked the indexes they added and were right
        // to add them. This is the tail nobody re-measured afterwards.
        //
        // ## The three that are dropped
        //
        // Each was checked by asking what production SQL mentions the column in
        // a `WHERE`, `ORDER BY` or `GROUP BY`, and then by asking SQLite which
        // index it actually picks:
        //
        //   * `idx_detections_chunk_offset` (29.6 MB) — **no production query
        //     names `chunk_offset_secs` at all.** The column's real job is as
        //     part of the composite unique key (migration 24), which is a
        //     different index.
        //   * `idx_detections_correlation_id` (29.6 MB) — the only query on
        //     `correlation_id` in the tree is inside a unit test, whose own
        //     comment says the index "lets a *future* endpoint pull by
        //     correlation_id efficiently". That endpoint has not arrived in nine
        //     migrations. When it does, this is one `CREATE INDEX`.
        //   * `idx_detections_source` (46.1 MB) — there is no `WHERE Source`
        //     anywhere. The two queries that mention the column are
        //     `todays_source_activity` (`WHERE Date = ?1 GROUP BY Source`) and a
        //     tiebreak in the nearest-detection lookup, and `EXPLAIN QUERY PLAN`
        //     shows both choosing `idx_detections_date` instead. It has never
        //     been read.
        //
        // ## The one that was costing 268 ms a minute, forever
        //
        // `locked_file_names` — `WHERE is_locked = 1 AND File_Name IS NOT NULL`
        // — is re-read by the disk manager **every 60 seconds** so that locking
        // a clip in `/admin/recordings` takes effect without a restart. That is
        // the right design. But `is_locked` is 0 for essentially every row, so
        // `ANALYZE` tells the planner the column has one or two distinct values
        // and a seek buys nothing, and the query plans as `SCAN detections`.
        //
        // Measured on the three-year database with forty clips locked:
        //
        //   * shipped index: **267.6 ms**, `SCAN detections`
        //   * partial covering index: **0.16 ms**, `SEARCH … USING COVERING
        //     INDEX idx_detections_locked`
        //   * index size: **4.1 kB**, down from 29.6 MB
        //
        // A full scan of the whole history, 1 440 times a day, holding the
        // connection the detection writer also needs. A partial index is the
        // shape this query always wanted: SQLite uses one when the query's
        // `WHERE` implies the index's, and there is nothing in it but the locked
        // rows. `File_Name` is carried so the index covers the query outright.
        //
        // `idx_detections_import_batch` gets the same treatment for the same
        // reason: `import_batch_id` is NULL for every locally recorded row, and
        // its one query is `WHERE import_batch_id IS NOT NULL`. On a station
        // that has never imported anything the index becomes empty rather than
        // 29.6 MB of NULLs.
        //
        // ## Cost
        //
        // Index work only — no row is rewritten, and an index is derived data,
        // so this is the recoverable kind of migration: re-running the `CREATE`s
        // rebuilds it exactly. 3.8 s on the three-year fixture. Afterwards the
        // file is **1.67 GB, 164.6 MB (9.0 %) smaller**, and five fewer B-trees
        // are touched on every insert.
        up_sql: "DROP INDEX IF EXISTS idx_detections_chunk_offset;
        DROP INDEX IF EXISTS idx_detections_correlation_id;
        DROP INDEX IF EXISTS idx_detections_source;

        DROP INDEX IF EXISTS idx_detections_locked;
        CREATE INDEX IF NOT EXISTS idx_detections_locked
            ON detections(is_locked, File_Name) WHERE is_locked = 1;

        DROP INDEX IF EXISTS idx_detections_import_batch;
        CREATE INDEX IF NOT EXISTS idx_detections_import_batch
            ON detections(import_batch_id) WHERE import_batch_id IS NOT NULL;

        ANALYZE;",
    },
    Migration {
        version: 34,
        description: "Let an operator keep an imported history without merging it into the analytics",
        // ## The gap this closes
        //
        // Migration 25 gave every imported detection an `import_batch_id`, and
        // `provenance.rs` warns before an import that merging two sites damages
        // a dataset in a way that "is not detectable after the fact". The column
        // was then read by exactly nothing: no analytic filtered on it, so the
        // life list, first-of-year, species richness, phenology, the heat map,
        // co-occurrence and the dawn chorus all read the union of two sites as
        // one station. Removing the import is now possible; keeping it *and*
        // keeping the analytics honest was not.
        //
        // ## Why the view rather than the queries
        //
        // `detections_analytic` is already the one place a reviewer's verdict
        // becomes real for every SQLite-side analytic — that is what migration 26
        // established. Provenance belongs in the same choke point for the same
        // reason: forty query sites cannot be kept in step by hand, and the one
        // that is forgotten is the one that quietly publishes another site's
        // records as this station's.
        //
        // ## The cost, measured
        //
        // A settings lookup inside a view over three million rows looks alarming
        // and is not: SQLite recognises the subquery as invariant and evaluates
        // it once per statement, which `EXPLAIN QUERY PLAN` reports as
        // `SCALAR SUBQUERY`. On the three-year fixture (3 285 000 rows), five
        // runs of the History calendar aggregate:
        //
        //     old view (rejected filter only)   median 942.5 ms  (923.5 – 1004.5)
        //     new view, setting off             median 964.2 ms  (935.5 – 1042.5)
        //     new view, setting absent          median 980.8 ms  (948.1 – 1083.1)
        //
        // The ranges overlap, so the overhead is at or below run-to-run variance
        // rather than measurably zero — which is the honest way to put it. With
        // the setting *on* the same query is faster, because it reads fewer rows.
        //
        // `import_batch_id IS NULL` is written first so a station that has never
        // imported anything short-circuits before the subquery is considered at
        // all.
        //
        // ## The covering indexes have to carry the new column
        //
        // Migration 29 made the three whole-history species aggregates — the
        // species list, the life-list firsts and the per-species hour histogram
        // — index-only, taking them from ~4.9 s to ~1.3 s on a three-year
        // station. Adding `import_batch_id` to the view's `WHERE` puts a column
        // in the plan that those indexes do not carry, and an index-only scan
        // becomes a scan plus a table lookup per row. Caught by migration 29's
        // own gate, `the_whole_history_species_aggregates_are_index_only`, which
        // is exactly what it was written for:
        //
        //     the species list aggregate must be index-only; plan was:
        //     SCAN detections USING INDEX idx_detections_species_hour_cover | …
        //
        // So both covering indexes gain the column. Measured on the three-year
        // fixture: `idx_detections_species_hour_cover` 197.6 -> 201.0 MB and
        // `idx_detections_sci_first_cover` 155.0 -> 158.2 MB — **+6.6 MB in
        // total**, because `import_batch_id` is NULL for every locally recorded
        // row and SQLite spends no payload bytes on a NULL. All three plans go
        // back to `COVERING INDEX`. 11.7 s to rebuild both, once.
        //
        // ## Default
        //
        // Absent means included. Merging two sites is a legitimate thing to want
        // — only the operator knows whether these are one site with a moved GPS
        // fix or two a county apart — so an upgrade changes no number on any
        // existing station. The DuckDB copy carries the same rule; see
        // `birdnet_behavioral::queries::detections_ts_view_sql`.
        up_sql: "DROP VIEW IF EXISTS detections_analytic;
        CREATE VIEW detections_analytic AS
            SELECT * FROM detections
             WHERE review_verdict IS NOT 'rejected'
               AND (import_batch_id IS NULL
                    OR NOT EXISTS (SELECT 1 FROM settings
                                    WHERE key = 'analytics_exclude_imports'
                                      AND value = 'true'));

        DROP INDEX IF EXISTS idx_detections_species_hour_cover;
        CREATE INDEX IF NOT EXISTS idx_detections_species_hour_cover
            ON detections(Com_Name, Time, Sci_Name, Confidence, review_verdict, import_batch_id);

        DROP INDEX IF EXISTS idx_detections_sci_first_cover;
        CREATE INDEX IF NOT EXISTS idx_detections_sci_first_cover
            ON detections(Sci_Name, Date, Time, review_verdict, import_batch_id);

        ANALYZE;",
    },
    Migration {
        version: 35,
        description: "Let an alert-rule webhook authenticate, instead of only working against open endpoints",
        // An alert rule could fire a webhook at any URL, but with no way to
        // carry a credential. That restricted the feature to endpoints that
        // authenticate by URL alone — a Slack or Discord webhook, or a
        // home-LAN service with no auth at all. Anything with an API key
        // (Home Assistant's `/api/webhook` is the common one, along with every
        // hosted automation service) could not be targeted, and the operator's
        // only recourse was to run their own unauthenticated relay.
        //
        // Three columns rather than one, because the three schemes put the
        // credential in different places and folding them into a single
        // "header line" field would mean parsing operator input to find out
        // which one they meant:
        //
        //   kind = ''       no authentication (the existing behaviour)
        //   kind = 'bearer' Authorization: Bearer <value>
        //   kind = 'basic'  Authorization: Basic base64(<value>), value = user:password
        //   kind = 'header' <name>: <value>
        //
        // `''` as the default is what makes this a no-op upgrade: every
        // existing rule keeps sending exactly the request it sent before.
        //
        // The value is a secret at rest in the operator's own database, which
        // is the same place the BirdWeather token and the session secret
        // already live. What it must not do is escape from there: the rules
        // export redacts it, the admin table never renders it, and the
        // dispatch error path logs the rule name rather than the request.
        up_sql: "ALTER TABLE alert_rules
                     ADD COLUMN action_webhook_auth_kind TEXT NOT NULL DEFAULT '';
                 ALTER TABLE alert_rules
                     ADD COLUMN action_webhook_auth_value TEXT;
                 ALTER TABLE alert_rules
                     ADD COLUMN action_webhook_header_name TEXT;",
    },
    Migration {
        version: 36,
        description: "Widen the quarantine reason CHECK, which was silently discarding new reasons",
        // ## How this was found
        //
        // The taxon-aware daylight filter added a fourth quarantine reason,
        // `implausible_hour`. Its end-to-end test recorded no detection *and*
        // no quarantine row, and `insert_quarantine` had returned `Ok(())`.
        //
        // The table was created (migration 10) with
        // `CHECK(reason IN ('below_sf_thresh','low_confidence','manual'))`, and
        // `insert_quarantine` uses `INSERT OR IGNORE` — which is there to
        // absorb the `UNIQUE(date, time, sci_name)` collision when the same
        // detection is offered twice. `OR IGNORE` does not distinguish between
        // constraints: it swallowed the CHECK violation exactly as it swallows
        // a duplicate, and reported success. Every detection quarantined for
        // the new reason would have been dropped on the floor, with no error
        // anywhere and no row to find afterwards.
        //
        // ## Why the table is rebuilt
        //
        // SQLite cannot alter a CHECK constraint in place; the constraint is
        // part of the stored `CREATE TABLE` text, so the documented procedure
        // is rebuild-and-rename.
        //
        // `PRAGMA foreign_keys` is not touched: nothing references
        // `quarantine` (it is a staging table read only by the review UI), so
        // the rename cannot orphan a child row. The three indexes are
        // recreated because `DROP TABLE` takes them with it.
        //
        // The copy names its columns rather than `SELECT *` so a column added
        // to one side and not the other fails loudly here instead of silently
        // shifting every value one place along.
        up_sql: "CREATE TABLE quarantine_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT NOT NULL,
            time TEXT NOT NULL,
            sci_name TEXT NOT NULL,
            com_name TEXT NOT NULL,
            confidence REAL NOT NULL,
            sf_probability REAL,
            reason TEXT NOT NULL CHECK(reason IN
                ('below_sf_thresh','low_confidence','implausible_hour','manual')),
            reviewed INTEGER NOT NULL DEFAULT 0,
            approved INTEGER NOT NULL DEFAULT 0,
            file_name TEXT,
            lat REAL,
            lon REAL,
            week INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(date, time, sci_name)
        );

        INSERT INTO quarantine_new
            (id, date, time, sci_name, com_name, confidence, sf_probability,
             reason, reviewed, approved, file_name, lat, lon, week, created_at)
        SELECT id, date, time, sci_name, com_name, confidence, sf_probability,
               reason, reviewed, approved, file_name, lat, lon, week, created_at
          FROM quarantine;

        DROP TABLE quarantine;
        ALTER TABLE quarantine_new RENAME TO quarantine;

        CREATE INDEX IF NOT EXISTS idx_quarantine_reviewed ON quarantine(reviewed);
        CREATE INDEX IF NOT EXISTS idx_quarantine_date ON quarantine(date);
        CREATE INDEX IF NOT EXISTS idx_quarantine_sci_name ON quarantine(sci_name);",
    },
    Migration {
        version: 37,
        description: "Record third-octave band levels, so the station measures its soundscape",
        // ## What this stores that `audio_levels` does not
        //
        // `audio_levels` keeps one broadband noise floor and SNR per source per
        // hour, which is enough to notice a microphone going deaf and not
        // enough to say what changed. This keeps a level per **band**, so the
        // shape of the change is visible: the top bands alone (a failing
        // capsule), one band alone (an oscillating preamp), everything under
        // 200 Hz (wind, or a mount resonating).
        //
        // ## Why the band is a column and not a row per band
        //
        // A row per (date, hour, source, band) is 30 rows an hour per source
        // rather than one — about 260 000 rows a year for a single-microphone
        // station. That is still small, and it is the shape that survives the
        // band set changing: a station running at 22.05 kHz measures 27 bands
        // and one at 48 kHz measures 30, so a fixed 30-column table would carry
        // three columns that are NULL for some stations and not others, and a
        // 32 kHz station added later would need a migration to widen it.
        //
        // Accumulated the same way `audio_levels` is — running sums plus a
        // count, folded by `INSERT ... ON CONFLICT DO UPDATE` — so an hour is
        // one row that each observation updates in place, and a restart mid-
        // hour loses nothing.
        //
        // `mean_power_sum` rather than `mean_db_sum`: decibels are logarithmic
        // and the mean of the interval must be an energy mean (see
        // `birdnet_core::audio::soundlevel::BandLevel::mean_db`). Summing the
        // dB values and dividing would answer a different question, ~43 dB
        // away from this one for a band with a transient in it.
        up_sql: "CREATE TABLE IF NOT EXISTS sound_levels (
            date            TEXT    NOT NULL,
            hour            INTEGER NOT NULL,
            source          TEXT    NOT NULL,
            band_hz         REAL    NOT NULL,
            samples         INTEGER NOT NULL,
            mean_power_sum  REAL    NOT NULL,
            min_db          REAL    NOT NULL,
            max_db          REAL    NOT NULL,
            PRIMARY KEY (date, hour, source, band_hz)
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS idx_sound_levels_date ON sound_levels(date);

        CREATE TABLE IF NOT EXISTS sound_level_broadband (
            date            TEXT    NOT NULL,
            hour            INTEGER NOT NULL,
            source          TEXT    NOT NULL,
            samples         INTEGER NOT NULL,
            a_power_sum     REAL    NOT NULL,
            z_power_sum     REAL    NOT NULL,
            calibration_db  REAL    NOT NULL DEFAULT 0.0,
            PRIMARY KEY (date, hour, source)
        ) WITHOUT ROWID;",
    },
    Migration {
        version: 38,
        description: "Remember which species the station has confirmed present, for dynamic thresholds",
        // A species confirmed present at a site gets an easier threshold for a
        // while (`birdnet_core::detection::dynamic_threshold`). Held in memory
        // by the event processor; persisted here so a restart does not forget
        // what the site contains — a station that reboots nightly for a backup
        // would otherwise never accumulate anything.
        //
        // `expires_at_ms` is an absolute epoch, not a duration: the lease is
        // "until", and storing a remaining-time would make a restart's clock
        // skew extend every lease it loaded.
        //
        // No CHECK on `level`. The vocabulary is the Rust type's, and a CHECK
        // here would be a second copy of it that could disagree — which is how
        // the quarantine `reason` constraint came to be silently dropping rows
        // (migration 36).
        up_sql: "CREATE TABLE IF NOT EXISTS dynamic_thresholds (
            sci_name          TEXT    PRIMARY KEY,
            level             INTEGER NOT NULL,
            confirmations     INTEGER NOT NULL,
            expires_at_ms     INTEGER NOT NULL,
            first_learned_ms  INTEGER NOT NULL,
            last_confirmed_ms INTEGER NOT NULL
        ) WITHOUT ROWID;

        CREATE INDEX IF NOT EXISTS idx_dynamic_thresholds_expiry
            ON dynamic_thresholds(expires_at_ms);",
    },
    Migration {
        version: 39,
        description: "Give each audio source a real filter chain instead of three fixed switches",
        // `pipeline_high_pass` and `pipeline_dc_removal` are a high-pass at a
        // fixed 120 Hz and one at a fixed 5 Hz. That is a compromise chosen for
        // a garden, and it is wrong in a different direction at most sites: a
        // station beside a motorway needs a steeper cut than one section, and a
        // station with mains hum needs a *notch*, which no high-pass can
        // provide without also removing everything below it.
        //
        // `eq_chain` holds the specification parsed by
        // `birdnet_core::audio::eq::EqChain` — `kind:freq[:q[:gain[:passes]]]`,
        // stages separated by `;`. Empty means "use the two boolean columns",
        // so this migration changes nothing on its own and every existing
        // station keeps hearing exactly what it heard before. The columns stay
        // rather than being dropped: they are the fallback, and a chain an
        // operator later clears returns to them.
        //
        // Text rather than JSON. The value is short, an operator edits it by
        // hand in the admin form, and a parse error has to name the offending
        // stage — which a JSON blob makes harder, not easier.
        //
        // `pipeline_agc` is untouched. It is a dynamic-range process, not a
        // filter, and it has no place in a chain of biquads.
        up_sql: "ALTER TABLE audio_sources
                     ADD COLUMN eq_chain TEXT NOT NULL DEFAULT '';",
    },
];

/// A migration that rewrites rows that already exist, rather than only changing
/// the schema around them.
///
/// The distinction is worth a type because the two are not equally recoverable.
/// A schema change is additive: older code ignores the new column and the data
/// it was computed from is still there. A rewrite destroys its own input — after
/// migration 24 there is nothing on disk that says what a detection's timestamp
/// used to be, so "undo it" is not a query anyone can write.
///
/// Membership here buys a migration two things: a complete copy of the database
/// taken immediately before it runs, and a dry-run an operator can look at
/// first.
struct HistoryRewrite {
    /// The migration version this describes.
    version: u32,
    /// A `SELECT` returning `(label, value)` text pairs describing what the
    /// migration would do, evaluated against the database *before* it runs.
    preview_sql: &'static str,
}

/// Every migration that rewrites existing rows. Add to this when writing one.
const HISTORY_REWRITES: &[HistoryRewrite] = &[HistoryRewrite {
    version: 24,
    // Deliberately mirrors migration 24's own WHERE clauses rather than
    // approximating them: a preview that counts a different set of rows than
    // the migration moves is worse than no preview, because it is believed.
    preview_sql: "SELECT 'detections whose timestamp moves' AS label,
                         CAST(COUNT(*) AS TEXT) AS value
                    FROM detections
                   WHERE chunk_offset_secs > 0
                     AND datetime(Date || ' ' || Time) IS NOT NULL
                  UNION ALL
                  SELECT 'left alone (already chunk-accurate, offset 0)',
                         CAST(COUNT(*) AS TEXT)
                    FROM detections WHERE chunk_offset_secs <= 0
                  UNION ALL
                  SELECT 'left alone (Date/Time name no point in time)',
                         CAST(COUNT(*) AS TEXT)
                    FROM detections
                   WHERE chunk_offset_secs > 0
                     AND datetime(Date || ' ' || Time) IS NULL
                  UNION ALL
                  SELECT 'largest shift, seconds',
                         COALESCE(CAST(MAX(CAST(chunk_offset_secs AS INTEGER)) AS TEXT), '0')
                    FROM detections
                   WHERE chunk_offset_secs > 0
                     AND datetime(Date || ' ' || Time) IS NOT NULL
                  UNION ALL
                  SELECT 'of those, rows that roll onto the next day',
                         CAST(COUNT(*) AS TEXT)
                    FROM detections
                   WHERE chunk_offset_secs > 0
                     AND datetime(Date || ' ' || Time) IS NOT NULL
                     AND date(datetime(Date || ' ' || Time,
                              '+' || CAST(chunk_offset_secs AS INTEGER) || ' seconds')) <> Date
                  UNION ALL
                  SELECT 'earliest affected detection',
                         COALESCE(MIN(Date || ' ' || Time), '(none)')
                    FROM detections
                   WHERE chunk_offset_secs > 0
                     AND datetime(Date || ' ' || Time) IS NOT NULL
                  UNION ALL
                  SELECT 'latest affected detection',
                         COALESCE(MAX(Date || ' ' || Time), '(none)')
                    FROM detections
                   WHERE chunk_offset_secs > 0
                     AND datetime(Date || ' ' || Time) IS NOT NULL",
}];

/// One pending history-rewriting migration, and what it would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPreview {
    /// The migration version.
    pub version: u32,
    /// The migration's description.
    pub description: String,
    /// `(label, value)` pairs describing the impact, in report order.
    pub rows: Vec<(String, String)>,
}

/// Describe what the pending history-rewriting migrations would do, without
/// applying anything.
///
/// Only migrations listed in `HISTORY_REWRITES` appear: a schema-only
/// migration has nothing to preview, because it moves no data.
///
/// `HISTORY_REWRITES` is deliberately *not* an intra-doc link: it is private,
/// and linking a private item from a `pub` item's docs is
/// `rustdoc::private_intra_doc_links`, which CI denies. Leave the brackets off.
///
/// # Errors
///
/// Returns `MigrationError` if the schema version cannot be read. A preview
/// query that fails because the tables it reads do not exist yet is *not* an
/// error — a database too young to hold the rows a migration would rewrite has
/// nothing to report — and yields an entry with no rows.
pub fn preview_pending(conn: &Connection) -> Result<Vec<MigrationPreview>, MigrationError> {
    let current = current_version(conn)?;
    let mut out = Vec::new();

    for rewrite in HISTORY_REWRITES {
        if rewrite.version <= current {
            continue;
        }
        let description = MIGRATIONS
            .iter()
            .find(|m| m.version == rewrite.version)
            .map_or("(unknown migration)", |m| m.description)
            .to_owned();

        let rows = match collect_preview(conn, rewrite.preview_sql) {
            Ok(rows) => rows,
            // A fresh database has no `detections` table yet, so the preview
            // has nothing to look at. That is an empty report, not a failure.
            Err(ref e) if preview_target_missing(e) => Vec::new(),
            Err(e) => return Err(MigrationError::Sqlite(e)),
        };
        out.push(MigrationPreview {
            version: rewrite.version,
            description,
            rows,
        });
    }
    Ok(out)
}

/// Whether a failed preview query failed only because the table it reads does
/// not exist yet.
///
/// The one benign failure: a database too young to hold the rows a migration
/// would rewrite has nothing to report. Every other failure has to propagate —
/// "nothing to change" is the single answer an operator acts on by upgrading
/// without looking further, so a preview that could not run must never produce
/// it.
///
/// Deliberately narrow. `rusqlite` splits statement-preparation failures across
/// two variants depending on whether `SQLite` could attribute the error to a
/// token: a missing *table* arrives as `SqliteFailure`, while a missing
/// *column* on a table that does exist arrives as `SqlInputError` and is not
/// benign. Split out from the `match` arm so both sides of that line are
/// testable without having to provoke each error shape from live `SQLite`.
fn preview_target_missing(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("no such table"))
}

/// Run one preview query and collect its `(label, value)` rows.
fn collect_preview(conn: &Connection, sql: &str) -> Result<Vec<(String, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Environment variable that waives the pre-migration backup.
///
/// The backup needs as much free space as the database itself, and a station
/// whose disk is too full for it would otherwise be unable to start at all.
/// Setting this is the operator saying they accept an unrecoverable rewrite.
const SKIP_BACKUP_ENV: &str = "BIRDNET_SKIP_MIGRATION_BACKUP";

/// Whether the operator has waived the pre-migration backup.
///
/// Unset, empty, and `"0"` all mean "take the backup": an empty value is what a
/// shell leaves behind after `FOO=` or an unexpanded `${FOO}`, and `0` is what
/// someone writes when they mean off. Anything else is consent.
///
/// Split out from the `var_os` call so the predicate is testable without
/// mutating process-global environment state from a test, which would race with
/// every other test in this binary.
fn backup_waived(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty() && v != "0")
}

/// Where a backup for `version` should go, avoiding any file already there.
///
/// Never overwrites: a second upgrade attempt must not clobber the copy the
/// first one made, which may be the only surviving pre-migration state.
fn backup_target(db: &Path, version: u32) -> Option<PathBuf> {
    let name = db.file_name()?.to_string_lossy();
    let base = format!("{name}.pre-migration-{version}.backup");
    let dir = db.parent().unwrap_or_else(|| Path::new("."));
    let first = dir.join(&base);
    if !first.exists() {
        return Some(first);
    }
    (1..1000)
        .map(|n| dir.join(format!("{base}.{n}")))
        .find(|p| !p.exists())
}

/// Copy the database immediately before a history-rewriting migration.
///
/// Uses `VACUUM INTO`, not a file copy: it runs against the live connection and
/// produces a single consistent file, where copying the `.db` alone would leave
/// behind whatever is still in the WAL.
///
/// Returns the backup path, or `None` when there is nothing to back up (an
/// in-memory database) or the operator has waived it.
///
/// # Errors
///
/// Returns `MigrationError` if the backup cannot be written. This deliberately
/// fails the migration rather than proceeding: the rewrite destroys its own
/// input, so "could not make it recoverable" has to mean "did not do it".
fn backup_before_rewrite(
    conn: &Connection,
    version: u32,
) -> Result<Option<PathBuf>, MigrationError> {
    if backup_waived(std::env::var_os(SKIP_BACKUP_ENV).as_deref()) {
        tracing::warn!(
            version,
            "{SKIP_BACKUP_ENV} is set — applying a history-rewriting migration with no backup. \
             The previous timestamps will not be recoverable."
        );
        return Ok(None);
    }

    // An in-memory database has no file to copy and no history worth keeping.
    let Some(path) = conn.path().filter(|p| !p.is_empty() && *p != ":memory:") else {
        return Ok(None);
    };
    let db = Path::new(path).to_path_buf();

    let Some(target) = backup_target(&db, version) else {
        return Err(MigrationError::Logic(format!(
            "could not find a free filename for the pre-migration-{version} backup next to \
             {}; move the existing backups aside and retry",
            db.display()
        )));
    };

    conn.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
        .map_err(|e| {
            MigrationError::Logic(format!(
                "migration {version} rewrites existing detections and could not first back the \
                 database up to {}: {e}\n\
                 The backup needs about as much free space as the database itself. Free some \
                 and restart. To upgrade without a backup — accepting that the previous \
                 timestamps cannot be recovered — set {SKIP_BACKUP_ENV}=1.",
                target.display()
            ))
        })?;

    tracing::info!(
        version,
        backup = %target.display(),
        "backed the database up before a history-rewriting migration"
    );
    Ok(Some(target))
}

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

/// Whether the database carries a schema version this binary does not know.
///
/// Strictly newer: a database at exactly the highest version this binary knows
/// is the normal fully-migrated state, not a downgrade.
///
/// Split out from the branch it guards for the same reason as
/// `any_migrations_applied` below: that branch only emits a log line, so
/// nothing a test can assert on changes when `>` slips to `>=` or `==`. As a
/// predicate the boundary is directly observable.
const fn schema_is_newer_than_binary(db_version: u32, max_known: u32) -> bool {
    db_version > max_known
}

/// Whether `migrate` actually applied anything, and so has something to report.
///
/// Split out for the same reason as `schema_is_newer_than_binary`: it guards a
/// log-only branch, so the boundary is only observable as a predicate.
const fn any_migrations_applied(applied: u32) -> bool {
    applied > 0
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
        && schema_is_newer_than_binary(current, max_known)
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

        // A migration that rewrites existing rows gets a copy of the database
        // first. Outside the transaction below, and necessarily so: `VACUUM
        // INTO` cannot run inside one. That ordering is also the correct one —
        // the backup must capture the state before the rewrite, and a rollback
        // of the transaction simply leaves an unused copy behind.
        if HISTORY_REWRITES
            .iter()
            .any(|r| r.version == migration.version)
        {
            backup_before_rewrite(conn, migration.version)?;
        }

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

    if any_migrations_applied(applied) {
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

    /// The UNIQUE key must not be defeated by a NULL `File_Name`.
    ///
    /// This is the invariant every duplicate-suppression path depends on,
    /// including the BirdNET-Pi importer's `INSERT OR IGNORE`.
    #[test]
    fn the_detections_unique_key_ignores_a_null_file_name() {
        let conn = memory_db();
        migrate(&conn).unwrap();
        let insert = "INSERT OR IGNORE INTO detections
            (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
            VALUES ('2026-01-01','06:00:00','Turdus merula','Blackbird',0.9,NULL)";
        conn.execute(insert, []).unwrap();
        conn.execute(insert, []).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            rows, 1,
            "two identical clip-less detections must collapse to one"
        );
    }

    /// Migration 23 repairs databases that already carry duplicates.
    ///
    /// Anyone who re-ran an import before the fix has them now, so the
    /// migration has to collapse what is already there — not merely stop new
    /// ones. Applies every migration *before* 23 against a table built without
    /// the fixed index, seeds the duplicates that were previously possible,
    /// then applies the repair.
    ///
    /// Pinned to version 23 by number rather than "the last migration": as
    /// written against `MIGRATIONS.len() - 1` this test silently changed
    /// meaning the moment a 24th was added, seeding its duplicates into a table
    /// that already had the fixed index.
    #[test]
    fn migration_23_collapses_pre_existing_duplicates() {
        let conn = memory_db();
        ensure_version_table(&conn).unwrap();
        for m in MIGRATIONS.iter().filter(|m| m.version < 23) {
            conn.execute_batch(m.up_sql).unwrap();
        }
        // The pre-fix index let these coexist: identical but for NULL names.
        let insert = "INSERT OR IGNORE INTO detections
            (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
            VALUES ('2026-01-01','06:00:00','Turdus merula','Blackbird',0.9,NULL)";
        conn.execute(insert, []).unwrap();
        conn.execute(insert, []).unwrap();
        conn.execute(insert, []).unwrap();
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            before, 3,
            "precondition: the old index permitted duplicates"
        );

        let m23 = MIGRATIONS
            .iter()
            .find(|m| m.version == 23)
            .expect("migration 23 exists");
        conn.execute_batch(m23.up_sql).unwrap();

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            after, 1,
            "the repair must collapse duplicates already on disk"
        );
    }

    /// A file-backed database migrated as far as `up_to`, holding one segment's
    /// five chunks all stamped with the file's start second.
    fn file_db_at_version(dir: &Path, up_to: u32) -> (PathBuf, Connection) {
        let path = dir.join("birds.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        ensure_version_table(&conn).unwrap();
        for m in MIGRATIONS.iter().filter(|m| m.version <= up_to) {
            conn.execute_batch(m.up_sql).unwrap();
            conn.execute(
                "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
                rusqlite::params![m.version, m.description],
            )
            .unwrap();
        }
        for offset in [0.0, 3.0, 6.0, 9.0, 12.0] {
            conn.execute(
                "INSERT INTO detections
                    (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
                 VALUES ('2026-03-11','08:30:00','Turdus merula','Blackbird',0.9,'seg.wav',?1)",
                rusqlite::params![offset],
            )
            .unwrap();
        }
        (path, conn)
    }

    /// Migration 24 destroys its own input, so it must leave something to go
    /// back to.
    ///
    /// After it runs there is nothing on disk that records what a detection's
    /// timestamp used to be — the offset is folded in and the old value is
    /// gone. "Undo it" is not a query anyone can write, which makes the copy
    /// taken beforehand the only recovery there is. This asserts the copy
    /// exists *and* that it still holds the un-shifted timestamps, because a
    /// backup taken after the rewrite would pass a mere existence check while
    /// being worthless.
    #[test]
    fn migration_24_leaves_a_restorable_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (path, conn) = file_db_at_version(dir.path(), 23);

        migrate(&conn).unwrap();

        // The live database has moved on.
        let shifted: Vec<String> = conn
            .prepare("SELECT Time FROM detections ORDER BY chunk_offset_secs")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            shifted,
            vec!["08:30:00", "08:30:03", "08:30:06", "08:30:09", "08:30:12"],
            "migration 24 should have folded the offsets in"
        );

        let backup = path.with_file_name("birds.db.pre-migration-24.backup");
        assert!(
            backup.exists(),
            "a history-rewriting migration must leave a backup at {}",
            backup.display()
        );

        // And the backup must predate the rewrite.
        let restored = Connection::open(&backup).unwrap();
        let original: Vec<String> = restored
            .prepare("SELECT Time FROM detections ORDER BY chunk_offset_secs")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            original,
            vec!["08:30:00"; 5],
            "the backup must hold the timestamps as they were before migration 24"
        );
        assert_eq!(
            current_version(&restored).unwrap(),
            23,
            "the backup must be at the pre-migration schema version"
        );
    }

    /// A second upgrade attempt must not clobber the first attempt's copy.
    #[test]
    fn a_second_backup_does_not_overwrite_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let (path, conn) = file_db_at_version(dir.path(), 23);
        std::fs::write(
            path.with_file_name("birds.db.pre-migration-24.backup"),
            b"first",
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(
            std::fs::read(path.with_file_name("birds.db.pre-migration-24.backup")).unwrap(),
            b"first",
            "an existing backup must survive untouched"
        );
        assert!(
            path.with_file_name("birds.db.pre-migration-24.backup.1")
                .exists(),
            "the new backup must go alongside, not on top"
        );
    }

    /// The dry-run must describe the rewrite without performing any part of it.
    #[test]
    fn migration_24_preview_reports_the_shift_without_applying_it() {
        let dir = tempfile::tempdir().unwrap();
        let (_path, conn) = file_db_at_version(dir.path(), 23);
        // A row that runs past midnight, and one that names no point in time.
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('2026-03-11','23:59:55','Parus major','Great Tit',0.8,'late.wav',9.0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('','','Corvus corax','Raven',0.6,'bad.wav',6.0)",
            [],
        )
        .unwrap();

        let previews = preview_pending(&conn).unwrap();
        let p = previews
            .iter()
            .find(|p| p.version == 24)
            .expect("migration 24 is pending and previewable");
        let get = |label: &str| {
            p.rows.iter().find(|(l, _)| l == label).map_or_else(
                || panic!("preview has no {label:?} row: {:?}", p.rows),
                |(_, v)| v.clone(),
            )
        };

        // Four offset>0 chunks from the segment, plus the near-midnight row.
        assert_eq!(get("detections whose timestamp moves"), "5");
        assert_eq!(get("left alone (already chunk-accurate, offset 0)"), "1");
        assert_eq!(get("left alone (Date/Time name no point in time)"), "1");
        assert_eq!(get("largest shift, seconds"), "12");
        assert_eq!(get("of those, rows that roll onto the next day"), "1");

        // Nothing may have moved, and no backup may have been taken.
        let times: Vec<String> = conn
            .prepare("SELECT Time FROM detections WHERE File_Name = 'seg.wav'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            times,
            vec!["08:30:00"; 5],
            "a preview must not apply the migration"
        );
        assert_eq!(current_version(&conn).unwrap(), 23);

        // The report must name the migration it is previewing. Nothing else in
        // this test would notice if it named a different one.
        let expected = MIGRATIONS
            .iter()
            .find(|m| m.version == 24)
            .expect("migration 24 exists")
            .description;
        assert_eq!(p.description, expected);
    }

    /// Once applied, the migration is no longer pending and drops out of the
    /// preview — otherwise the report would keep offering to move rows that
    /// have already moved.
    #[test]
    fn an_applied_rewrite_no_longer_appears_in_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (_path, conn) = file_db_at_version(dir.path(), 23);
        assert!(
            preview_pending(&conn)
                .unwrap()
                .iter()
                .any(|p| p.version == 24)
        );
        migrate(&conn).unwrap();
        assert!(preview_pending(&conn).unwrap().is_empty());
    }

    /// A fresh database has no `detections` table for the preview to read. That
    /// is an empty report, not a startup failure.
    #[test]
    fn previewing_a_fresh_database_is_not_an_error() {
        let conn = memory_db();
        let previews = preview_pending(&conn).unwrap();
        let p = previews
            .iter()
            .find(|p| p.version == 24)
            .expect("24 is pending on a v0 database");
        assert!(p.rows.is_empty(), "{:?}", p.rows);
    }

    /// A database is only "newer than this binary" strictly above the highest
    /// version the binary knows.
    ///
    /// Equality is the normal fully-migrated state — the common case on every
    /// boot — so warning there would cry downgrade at a healthy station. Both
    /// sides of the boundary are asserted; the branch this guards only logs, so
    /// nothing else in the suite would notice it moving.
    #[test]
    fn only_a_strictly_higher_schema_version_means_a_downgrade() {
        let max_known = MIGRATIONS
            .iter()
            .map(|m| m.version)
            .max()
            .expect("there is at least one migration");

        assert!(
            !schema_is_newer_than_binary(max_known, max_known),
            "a fully-migrated database is not a downgrade"
        );
        assert!(
            !schema_is_newer_than_binary(max_known - 1, max_known),
            "a database with migrations still pending is not a downgrade"
        );
        assert!(
            schema_is_newer_than_binary(max_known + 1, max_known),
            "a database past this binary's knowledge is a downgrade"
        );
    }

    /// "Migrations complete" is only true when something was applied.
    ///
    /// Guards a log-only branch, so this predicate is the only place the
    /// zero/non-zero boundary is observable.
    #[test]
    fn nothing_applied_means_nothing_to_report() {
        assert!(!any_migrations_applied(0));
        assert!(any_migrations_applied(1));
        assert!(any_migrations_applied(24));
    }

    /// The error type must name which failure it is and keep the cause
    /// reachable.
    ///
    /// `Display` is what an operator reads when a migration aborts, and
    /// `source()` is what the logging layer walks to get the underlying
    /// `SQLite` message. Neither had an assertion, so an empty `Display` and a
    /// severed `source()` were both invisible.
    #[test]
    fn migration_errors_render_and_keep_their_cause() {
        let sqlite = MigrationError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("no such table: detections".to_owned()),
        ));
        let rendered = sqlite.to_string();
        assert!(
            rendered.starts_with("migration sqlite error: "),
            "{rendered}"
        );
        assert!(rendered.contains("no such table: detections"), "{rendered}");
        assert!(
            std::error::Error::source(&sqlite).is_some(),
            "the underlying rusqlite error must stay reachable"
        );

        let logic = MigrationError::Logic("no free backup filename".to_owned());
        assert_eq!(
            logic.to_string(),
            "migration error: no free backup filename"
        );
        assert!(
            std::error::Error::source(&logic).is_none(),
            "a logic error has no underlying cause to expose"
        );
    }

    /// Only a missing *table* is the benign preview failure.
    ///
    /// The discrimination this makes is the whole point: widen it and a preview
    /// that failed for any reason reports "nothing to change", which is the one
    /// answer an operator acts on by upgrading without looking further. Both
    /// sides are asserted so a blanket `true` cannot pass for a classifier.
    #[test]
    fn only_a_missing_table_is_a_benign_preview_failure() {
        let sqlite_failure = |msg: &str| {
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(msg.to_owned()))
        };

        assert!(
            preview_target_missing(&sqlite_failure("no such table: detections")),
            "a database too young to have the table has nothing to preview"
        );

        // Everything else is a real failure, including the shapes live SQLite
        // actually produces here: a locked database, and a `detections` that
        // exists but lacks the column the preview reads (which rusqlite reports
        // as `SqlInputError`, not `SqliteFailure`).
        assert!(!preview_target_missing(&sqlite_failure(
            "database is locked"
        )));
        assert!(!preview_target_missing(&sqlite_failure("disk I/O error")));
        assert!(!preview_target_missing(&rusqlite::Error::SqlInputError {
            error: rusqlite::ffi::Error::new(1),
            msg: "no such column: chunk_offset_secs".to_owned(),
            sql: "SELECT chunk_offset_secs FROM detections".to_owned(),
            offset: 7,
        }));
        assert!(!preview_target_missing(&rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            None
        )));
    }

    /// A preview that cannot run must say so, not report an empty change set.
    ///
    /// The counterpart to `previewing_a_fresh_database_is_not_an_error`: only
    /// the missing-table case is benign. Widening that arm to swallow every
    /// `SQLite` error would turn a broken preview into "nothing to change",
    /// which is the one answer an operator acts on by upgrading anyway.
    #[test]
    fn a_preview_that_fails_for_another_reason_is_not_reported_as_empty() {
        let conn = memory_db();
        // `detections` exists but lacks the column the preview reads, so the
        // query fails with "no such column", not "no such table".
        conn.execute_batch("CREATE TABLE detections (Date TEXT, Time TEXT);")
            .unwrap();

        let err = preview_pending(&conn).expect_err(
            "a preview query that cannot run must propagate, not report an empty change set",
        );
        assert!(matches!(err, MigrationError::Sqlite(_)), "{err:?}");
    }

    /// The backup escape hatch: unset, empty and `"0"` all keep the backup.
    ///
    /// Tested through the pure predicate rather than by setting the real
    /// environment variable — unit tests share one process environment and run
    /// in parallel, so a test that set it could suppress the backup under an
    /// unrelated test running concurrently.
    #[test]
    fn only_a_meaningful_value_waives_the_pre_migration_backup() {
        assert!(!backup_waived(None), "unset must keep the backup");
        assert!(
            !backup_waived(Some(OsStr::new(""))),
            "an empty value (`FOO=`, an unexpanded `${{FOO}}`) must keep the backup"
        );
        assert!(
            !backup_waived(Some(OsStr::new("0"))),
            "`0` means off, and must keep the backup"
        );
        assert!(
            backup_waived(Some(OsStr::new("1"))),
            "an explicit value is the operator waiving the backup"
        );
        assert!(
            backup_waived(Some(OsStr::new("00"))),
            "only the exact string `0` is the off switch"
        );
    }

    /// Migration 24 folds the chunk offset into the timestamp of history that
    /// is already on disk.
    ///
    /// Five chunks of one 15-second segment went in stamped with the file's
    /// start second; they must come out at the five seconds they were actually
    /// heard. The rows that must *not* move are the point of the other
    /// assertions: an imported BirdNET-Pi row already carries a chunk-accurate
    /// time and offset 0, and a row whose `Date`/`Time` name no point in time
    /// cannot be shifted at all.
    #[test]
    fn migration_24_folds_the_chunk_offset_into_the_timestamp() {
        let conn = memory_db();
        ensure_version_table(&conn).unwrap();
        for m in MIGRATIONS.iter().filter(|m| m.version < 24) {
            conn.execute_batch(m.up_sql).unwrap();
        }

        // Five chunks of one segment, all stamped with the file's start time.
        for offset in [0.0, 3.0, 6.0, 9.0, 12.0] {
            conn.execute(
                "INSERT INTO detections
                    (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
                 VALUES ('2026-03-11','08:30:00','Turdus merula','Blackbird',0.9,'seg.wav',?1)",
                rusqlite::params![offset],
            )
            .unwrap();
        }
        // A segment that runs across midnight: its last chunk is tomorrow.
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('2026-03-11','23:59:55','Parus major','Great Tit',0.8,'late.wav',9.0)",
            [],
        )
        .unwrap();
        // An imported BirdNET-Pi row: already correct, offset 0, must not move.
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('2026-03-11','07:00:03','Erithacus rubecula','Robin',0.7,'pi.wav',0.0)",
            [],
        )
        .unwrap();
        // A row that names no point in time. Shifting it would write NULL into
        // a NOT NULL column and take the whole migration down.
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('','','Corvus corax','Raven',0.6,'bad.wav',6.0)",
            [],
        )
        .unwrap();

        let m24 = MIGRATIONS
            .iter()
            .find(|m| m.version == 24)
            .expect("migration 24 exists");
        conn.execute_batch(m24.up_sql).unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT Date, Time FROM detections
                  WHERE File_Name = 'seg.wav' ORDER BY chunk_offset_secs",
            )
            .unwrap();
        let times: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            times,
            vec![
                ("2026-03-11".to_string(), "08:30:00".to_string()),
                ("2026-03-11".to_string(), "08:30:03".to_string()),
                ("2026-03-11".to_string(), "08:30:06".to_string()),
                ("2026-03-11".to_string(), "08:30:09".to_string()),
                ("2026-03-11".to_string(), "08:30:12".to_string()),
            ],
            "the five chunks must land on the five seconds they were heard"
        );

        let late: (String, String) = conn
            .query_row(
                "SELECT Date, Time FROM detections WHERE File_Name = 'late.wav'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            late,
            ("2026-03-12".to_string(), "00:00:04".to_string()),
            "a chunk past midnight belongs to the next day"
        );

        let imported: (String, String) = conn
            .query_row(
                "SELECT Date, Time FROM detections WHERE File_Name = 'pi.wav'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            imported,
            ("2026-03-11".to_string(), "07:00:03".to_string()),
            "an already-correct imported row must not be shifted a second time"
        );

        let bad: (String, String) = conn
            .query_row(
                "SELECT Date, Time FROM detections WHERE File_Name = 'bad.wav'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            bad,
            (String::new(), String::new()),
            "an unplaceable row stays exactly as it was"
        );
    }

    /// A second boot must not shift every timestamp again.
    ///
    /// This repair is the one migration in the set that is *not* idempotent —
    /// re-running its UPDATE would add the offset a second time and walk every
    /// chunk row forward through the day. Nothing but the version bookkeeping
    /// stops that, so the bookkeeping is what gets asserted.
    #[test]
    fn migration_24_does_not_re_apply_on_a_later_boot() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        // A row written by the fixed pipeline: already stamped at the second it
        // was heard, and still carrying the offset it came from.
        conn.execute(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
             VALUES ('2026-03-11','08:30:09','Turdus merula','Blackbird',0.9,'seg.wav',9.0)",
            [],
        )
        .unwrap();

        let applied = migrate(&conn).unwrap();
        assert_eq!(applied, 0, "an up-to-date database applies nothing");

        let after: (String, String) = conn
            .query_row(
                "SELECT Date, Time FROM detections WHERE File_Name = 'seg.wav'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            after,
            ("2026-03-11".to_string(), "08:30:09".to_string()),
            "a row already on the new convention must not be shifted again"
        );
    }

    #[test]
    fn migration_35_leaves_existing_webhook_rules_unauthenticated() {
        // The no-op-upgrade guarantee. A station upgrading with live alert
        // rules must keep sending exactly the request it sent before: the new
        // columns default to "no credential", not to an empty one, because an
        // empty `Authorization` header is rejected by some servers and counted
        // as a failed login by others.
        let conn = memory_db();
        for m in MIGRATIONS.iter().filter(|m| m.version < 35) {
            conn.execute_batch(m.up_sql).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO alert_rules
                 (name, enabled, species_pattern, confidence_min, confidence_max,
                  action_type, action_webhook_url, action_webhook_method)
             VALUES ('legacy', 1, NULL, 0.0, 1.0, 'webhook', 'https://x/y', 'POST');",
        )
        .unwrap();

        let m35 = MIGRATIONS
            .iter()
            .find(|m| m.version == 35)
            .expect("migration 35 exists");
        conn.execute_batch(m35.up_sql).unwrap();

        let (kind, value, header): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT action_webhook_auth_kind, action_webhook_auth_value,
                        action_webhook_header_name
                   FROM alert_rules WHERE name = 'legacy'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "", "a pre-existing rule must carry no auth scheme");
        assert_eq!(value, None);
        assert_eq!(header, None);

        // And the row still loads as an unauthenticated webhook.
        let rule = crate::alert_rules::list_rules(&conn)
            .unwrap()
            .into_iter()
            .find(|r| r.name == "legacy")
            .expect("the rule survived the migration");
        assert!(matches!(
            rule.action,
            crate::alert_rules::AlertAction::Webhook { auth: None, .. }
        ));
    }

    #[test]
    fn migration_39_leaves_an_existing_source_on_its_old_filter_path() {
        // The no-op-upgrade guarantee. A station that upgrades mid-season must
        // keep hearing exactly what it heard yesterday: the same two fixed
        // high-passes it had before the chain existed. An empty `eq_chain` is
        // what selects that fallback, so the default has to be `''` and not,
        // say, a chain that reproduces the two switches — the latter would
        // look equivalent and would not be, because the boolean columns are
        // still editable and would then be silently overridden.
        let conn = memory_db();
        for m in MIGRATIONS.iter().filter(|m| m.version < 39) {
            conn.execute_batch(m.up_sql).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO audio_sources (id, kind, device_id, pipeline_high_pass)
             VALUES ('legacy_mic', 'usb-alsa', 'plughw:1,0', 1);",
        )
        .unwrap();

        let m39 = MIGRATIONS
            .iter()
            .find(|m| m.version == 39)
            .expect("migration 39 exists");
        conn.execute_batch(m39.up_sql).unwrap();

        let chain: String = conn
            .query_row(
                "SELECT eq_chain FROM audio_sources WHERE id = 'legacy_mic'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chain, "", "a pre-existing source must carry no chain");

        // And the row still loads through the store, with its switches intact.
        let source = crate::audio_sources::AudioSourceStore::get(&conn, "legacy_mic")
            .unwrap()
            .expect("the source survived the migration");
        assert_eq!(source.eq_chain, "");
        assert!(source.pipeline.high_pass);
    }

    #[test]
    fn every_quarantine_reason_is_accepted_by_the_schema() {
        // The `quarantine.reason` column carries a CHECK constraint listing
        // the reasons by name, and `insert_quarantine` writes with
        // `INSERT OR IGNORE` — which it needs, to absorb the
        // `UNIQUE(date, time, sci_name)` collision when a detection is offered
        // twice. `OR IGNORE` does not distinguish between constraints, so a
        // reason the CHECK does not list is discarded exactly as a duplicate
        // is, `Ok(())` is returned, and the row is gone with no error anywhere.
        //
        // That is not hypothetical: it is what `implausible_hour` did before
        // migration 36, and the only symptom was an end-to-end test finding
        // neither a detection nor a quarantine row.
        let conn = memory_db();
        migrate(&conn).unwrap();

        for (i, reason) in crate::sqlite::ALL_QUARANTINE_REASONS.iter().enumerate() {
            let record = crate::sqlite::QuarantineRecord {
                // Distinct times: the table is UNIQUE on (date, time, sci_name)
                // and a collision here would look exactly like the failure
                // this test is for.
                date: "2026-01-15",
                time: &format!("02:{i:02}:00"),
                sci_name: "Cyanistes caeruleus",
                com_name: "Eurasian Blue Tit",
                confidence: 0.95,
                sf_probability: None,
                reason: reason.clone(),
                file_name: None,
                lat: None,
                lon: None,
                week: Some(3),
            };
            crate::sqlite::insert_quarantine(&conn, &record).unwrap();

            let stored: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM quarantine WHERE reason = ?1",
                    rusqlite::params![reason.as_str()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                stored,
                1,
                "the schema silently discarded a quarantine with reason {:?} — add it to \
                 the CHECK constraint in a migration",
                reason.as_str()
            );
        }
    }

    #[test]
    fn the_schema_still_rejects_a_reason_nobody_defined() {
        // Counterpart: widening the CHECK to accept anything would satisfy the
        // gate above, and the column would stop being a closed set — which is
        // what makes `from_db_str`'s fallback to `Manual` safe to rely on.
        let conn = memory_db();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO quarantine
                (date, time, sci_name, com_name, confidence, reason)
             VALUES ('2026-01-15','02:30:00','X','X',0.9,'not_a_reason')",
            [],
        )
        .unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM quarantine", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "the reason column accepts arbitrary strings");
    }

    #[test]
    fn migration_36_keeps_the_quarantine_rows_it_rebuilds_around() {
        // A table rebuild that lost the operator's review queue would be a
        // silent data loss on upgrade, and the queue is the only record of
        // detections the station chose not to admit.
        let conn = memory_db();
        for m in MIGRATIONS.iter().filter(|m| m.version < 36) {
            conn.execute_batch(m.up_sql).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO quarantine
                 (date, time, sci_name, com_name, confidence, reason, reviewed, approved, week)
             VALUES ('2026-01-15','02:30:00','Strix aluco','Tawny Owl',0.91,'low_confidence',1,1,3),
                    ('2026-01-15','03:00:00','Turdus merula','Blackbird',0.42,'below_sf_thresh',0,0,3);",
        )
        .unwrap();

        let m36 = MIGRATIONS
            .iter()
            .find(|m| m.version == 36)
            .expect("migration 36 exists");
        conn.execute_batch(m36.up_sql).unwrap();

        let rows: Vec<(String, String, i64, i64)> = conn
            .prepare("SELECT sci_name, reason, reviewed, approved FROM quarantine ORDER BY time")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "Strix aluco".to_string(),
                    "low_confidence".to_string(),
                    1,
                    1
                ),
                (
                    "Turdus merula".to_string(),
                    "below_sf_thresh".to_string(),
                    0,
                    0
                ),
            ],
            "the rebuild lost or reordered the review queue"
        );

        // And the indexes came back with it — `DROP TABLE` takes them.
        let indexes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index' AND tbl_name = 'quarantine'
                    AND name LIKE 'idx_quarantine_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 3, "the rebuild dropped the quarantine indexes");
    }
}
