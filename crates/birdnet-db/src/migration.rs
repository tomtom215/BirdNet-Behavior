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
}
