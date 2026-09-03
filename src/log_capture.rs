//! A `tracing` layer that feeds the admin log viewer and an on-disk error log.
//!
//! # What was wrong
//!
//! `GET /admin/system/logs` streams [`LogBroadcaster`], and
//! `routes/admin/logs.rs`'s own module doc said its lines "are captured by a
//! custom `tracing` layer". No such layer existed anywhere in the workspace.
//! The page replayed an empty backlog and then emitted keep-alives for ever,
//! on every station, since the feature was written. In Docker, where the
//! operator has no `journalctl`, that page is the whole story.
//!
//! # Why the layer lives in the binary
//!
//! `LogBroadcaster` belongs to `birdnet-web` because the SSE route reads it.
//! *Which* subscriber layers get installed is an application decision, the
//! same one that owns the tokio runtime, so the `Layer` implementation is here
//! and `birdnet-web` stays free of `tracing-subscriber`.
//!
//! # Two sinks, deliberately different
//!
//! The broadcaster is in-memory and carries everything the filter admits: it
//! is a live view, and it is gone at reboot. `errors.jsonl` takes only ERROR
//! and WARN, and its whole purpose is to *survive* the reboot — a station on a
//! default Raspberry Pi OS has a volatile journal, so every watchdog bounce
//! and power cut erases the evidence of what caused it. Capping it is
//! therefore not an optimisation but the thing that makes it safe to have on
//! an SD card at all.
//!
//! # Reentrancy
//!
//! Nothing in here may log. A `tracing::warn!` raised while handling an event
//! re-enters [`Layer::on_event`] and deadlocks on the file mutex. Failures are
//! swallowed on purpose; the counter in [`ErrorLog::dropped`] is what an
//! operator can see instead.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use birdnet_web::routes::admin::logs::{LogBroadcaster, LogLine};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Largest `errors.jsonl` allowed before it is restarted.
///
/// A quiet station writes nothing here, so this bound is only reached by one
/// that is genuinely failing — and a station failing in a loop must not be
/// able to fill the card the recordings live on. At ~250 bytes a line this is
/// roughly four thousand lines, which is far more context than any bug report
/// needs.
const ERROR_LOG_MAX_BYTES: u64 = 1_000_000;

/// The file name, beside the database.
pub const ERROR_LOG_NAME: &str = "errors.jsonl";

/// Where the error log lives for a given database path.
///
/// Beside the database, because that is the directory the station already
/// knows is writable and already backs up.
#[must_use]
pub fn error_log_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(ERROR_LOG_NAME)
}

/// The append-only ERROR/WARN sink.
#[derive(Debug)]
struct ErrorLog {
    /// Open handle, reopened with truncation when the cap is reached.
    file: Mutex<Option<File>>,
    /// Bytes in the file, tracked rather than stat-ed per write.
    written: AtomicU64,
    /// Lines that could not be written. Never logged — see the module doc.
    dropped: AtomicU64,
    /// Path, kept for the reopen after truncation.
    path: PathBuf,
}

impl ErrorLog {
    /// Open (or create) the log, continuing an existing file.
    fn open(path: PathBuf) -> Self {
        let existing = std::fs::metadata(&path).map_or(0, |m| m.len());
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Self {
            file: Mutex::new(file),
            written: AtomicU64::new(existing),
            dropped: AtomicU64::new(0),
            path,
        }
    }

    /// Append one line, restarting the file if it has grown past the cap.
    fn append(&self, line: &str) {
        let Ok(mut guard) = self.file.lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        // Past the cap: start again rather than rotating. A second file would
        // double the worst-case footprint, and the newest failures are the
        // ones a bug report needs — an operator reads this after something
        // went wrong, not before.
        if self.written.load(Ordering::Relaxed) >= ERROR_LOG_MAX_BYTES {
            *guard = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)
                .ok();
            self.written.store(0, Ordering::Relaxed);
        }
        let Some(file) = guard.as_mut() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if file.write_all(line.as_bytes()).is_ok() {
            self.written.fetch_add(line.len() as u64, Ordering::Relaxed);
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Collects an event's `message` and its other fields.
#[derive(Default)]
struct Fields {
    /// The format-string body, i.e. what a human reads.
    message: String,
    /// Everything else, rendered `key=value`.
    extra: String,
}

impl Fields {
    /// The message with its structured fields appended.
    ///
    /// Fields are not dropped: `tracing::warn!(error = %e, "publish failed")`
    /// carries the entire diagnosis in `error`, and a viewer showing only
    /// "publish failed" would be worse than the journal it replaces.
    fn render(self) -> String {
        if self.extra.is_empty() {
            self.message
        } else if self.message.is_empty() {
            self.extra
        } else {
            format!("{} {}", self.message, self.extra)
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.push(field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(
                self.extra,
                "{}{}={value:?}",
                if self.extra.is_empty() { "" } else { " " },
                field.name()
            );
        }
    }
}

impl Fields {
    /// Append one `key=value` pair without the `Debug` quoting.
    fn push(&mut self, key: &str, value: &str) {
        let _ = write!(
            self.extra,
            "{}{key}={value}",
            if self.extra.is_empty() { "" } else { " " }
        );
    }
}

/// The layer itself.
#[derive(Debug, Clone)]
pub struct LogCapture {
    /// The live view behind `GET /admin/system/logs`.
    broadcaster: LogBroadcaster,
    /// The reboot-surviving ERROR/WARN sink, when one is configured.
    errors: Option<Arc<ErrorLog>>,
}

impl LogCapture {
    /// Build a layer feeding `broadcaster`, and `path` when given.
    ///
    /// `None` leaves only the in-memory view, which is what the test paths and
    /// any subcommand that is not the server want.
    #[must_use]
    pub fn new(broadcaster: LogBroadcaster, path: Option<PathBuf>) -> Self {
        Self {
            broadcaster,
            errors: path.map(|p| Arc::new(ErrorLog::open(p))),
        }
    }
}

/// Milliseconds since the Unix epoch, or 0 on a clock before it.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl<S: Subscriber> Layer<S> for LogCapture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);

        // URL credentials are stripped here rather than at each call site.
        // `errors.jsonl` travels in the support bundle, and an RTSP URL with
        // `user:pass@` in its authority is the shape that gets posted to a
        // public forum by an operator who was told the bundle was redacted.
        let message = crate::support::redact_url_credentials(&fields.render());

        let line = LogLine {
            level: meta.level().to_string(),
            message,
            target: meta.target().to_owned(),
            timestamp_ms: now_ms(),
        };

        if let Some(errors) = &self.errors
            && matches!(*meta.level(), Level::ERROR | Level::WARN)
        {
            errors.append(&format!(
                "{}\n",
                serde_json::json!({
                    "ts_ms": line.timestamp_ms,
                    "level": line.level,
                    "target": line.target,
                    "message": line.message,
                })
            ));
        }

        self.broadcaster.publish(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt as _;

    /// Run `f` with only a `LogCapture` installed, and return what it captured.
    fn capture<F: FnOnce()>(path: Option<PathBuf>, f: F) -> Vec<LogLine> {
        let broadcaster = LogBroadcaster::new();
        let layer = LogCapture::new(broadcaster.clone(), path);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, f);
        broadcaster.recent(100)
    }

    #[test]
    fn an_event_reaches_the_broadcaster_the_admin_page_streams() {
        // The whole defect: no `Layer` implementation existed, so this
        // returned an empty vec on every station for the life of the feature.
        let lines = capture(None, || {
            tracing::warn!("the microphone stopped");
        });
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].level, "WARN");
        assert_eq!(lines[0].message, "the microphone stopped");
        assert!(
            lines[0].target.starts_with("birdnet_behavior"),
            "target: {}",
            lines[0].target
        );
        assert!(lines[0].timestamp_ms > 0);
    }

    #[test]
    fn structured_fields_travel_with_the_message() {
        // `tracing::warn!(error = %e, "publish failed")` carries the whole
        // diagnosis in `error`. A viewer showing only "publish failed" would
        // be worse than the journal it stands in for.
        let lines = capture(None, || {
            tracing::error!(source = "RTSP_1", attempts = 3, "capture gave up");
        });
        assert_eq!(lines.len(), 1);
        let m = &lines[0].message;
        assert!(m.contains("capture gave up"), "{m}");
        assert!(m.contains("source=RTSP_1"), "{m}");
        assert!(m.contains("attempts=3"), "{m}");
    }

    #[test]
    fn url_credentials_never_reach_the_viewer_or_the_file() {
        // `errors.jsonl` travels in the support bundle.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(ERROR_LOG_NAME);
        let lines = capture(Some(path.clone()), || {
            tracing::error!("cannot reach rtsp://admin:hunter2@cam.local/stream");
        });
        assert!(
            !lines[0].message.contains("hunter2"),
            "{}",
            lines[0].message
        );
        let on_disk = std::fs::read_to_string(&path).expect("error log");
        assert!(!on_disk.contains("hunter2"), "{on_disk}");
        assert!(
            on_disk.contains("cam.local"),
            "the host still helps: {on_disk}"
        );
    }

    #[test]
    fn only_error_and_warn_are_persisted() {
        // The discrimination. Persisting everything would put ~11 500 lines a
        // day on the SD card the recordings share, which is the failure the
        // journal already has.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(ERROR_LOG_NAME);
        let lines = capture(Some(path.clone()), || {
            tracing::info!("analysed a file");
            tracing::debug!("chunk 3");
            tracing::warn!("disk is filling");
            tracing::error!("database is corrupt");
        });
        assert_eq!(lines.len(), 4, "all four reach the live view");

        let on_disk = std::fs::read_to_string(&path).expect("error log");
        let written: Vec<&str> = on_disk.lines().collect();
        assert_eq!(written.len(), 2, "only ERROR and WARN persist: {on_disk}");
        assert!(written[0].contains("disk is filling"));
        assert!(written[1].contains("database is corrupt"));
        assert!(!on_disk.contains("analysed a file"));
    }

    #[test]
    fn each_persisted_line_is_one_json_object() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(ERROR_LOG_NAME);
        capture(Some(path.clone()), || {
            tracing::error!(detail = "a \"quoted\" value\nwith a newline", "broke");
        });
        let on_disk = std::fs::read_to_string(&path).expect("error log");
        assert_eq!(
            on_disk.lines().count(),
            1,
            "a newline in a field must not split the record: {on_disk}"
        );
        let v: serde_json::Value = serde_json::from_str(on_disk.trim()).expect("valid JSON");
        assert_eq!(v["level"], "ERROR");
        assert!(
            v["message"].as_str().expect("message").contains("broke"),
            "{v}"
        );
    }

    #[test]
    fn the_error_log_is_capped_and_keeps_the_newest() {
        // A station failing in a loop must not fill the card the recordings
        // live on.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(ERROR_LOG_NAME);
        let log = ErrorLog::open(path.clone());
        let filler = format!("{}\n", "x".repeat(1000));
        for _ in 0..(ERROR_LOG_MAX_BYTES / 1001 + 10) {
            log.append(&filler);
        }
        log.append("NEWEST\n");
        let size = std::fs::metadata(&path).expect("metadata").len();
        assert!(
            size < ERROR_LOG_MAX_BYTES,
            "capped at {ERROR_LOG_MAX_BYTES}, got {size}"
        );
        let on_disk = std::fs::read_to_string(&path).expect("error log");
        assert!(on_disk.ends_with("NEWEST\n"), "the newest line survives");
        assert_eq!(log.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_unwritable_path_is_silent_rather_than_recursive() {
        // Reporting this failure with `tracing::warn!` would re-enter
        // `on_event` and deadlock on the file mutex.
        let log = ErrorLog::open(PathBuf::from("/proc/definitely/not/writable/x.jsonl"));
        log.append("line\n");
        assert_eq!(log.dropped.load(Ordering::Relaxed), 1);
    }

    // ── the wiring, which is the part that was actually missing ────────

    /// An `AppState` over an in-memory database, for the wiring gates.
    fn state() -> birdnet_web::state::AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("memory db");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        birdnet_web::state::AppState::from_connection(conn, PathBuf::from(":memory:"))
    }

    #[test]
    fn a_wired_state_replays_what_the_station_logged() {
        // The gate that matters is the wiring, not the layer: a layer writing
        // to one broadcaster while `AppState` holds another passes any test of
        // either half alone and still shows an operator nothing. `recent(50)`
        // is exactly what `GET /admin/system/logs` replays to a connecting
        // client.
        let broadcaster = LogBroadcaster::new();
        let state = state().with_log_broadcaster(broadcaster.clone());

        let subscriber = tracing_subscriber::registry().with(LogCapture::new(broadcaster, None));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(source = "RTSP_1", "the microphone stopped");
        });

        let replayed = state.log_broadcaster().recent(50);
        assert_eq!(replayed.len(), 1, "{replayed:?}");
        assert_eq!(replayed[0].level, "WARN");
        assert!(
            replayed[0].message.contains("the microphone stopped"),
            "{}",
            replayed[0].message
        );
        assert!(replayed[0].message.contains("RTSP_1"), "with its fields");
    }

    #[test]
    fn an_unwired_state_shows_nothing() {
        // The discrimination, and the shipped behaviour: every `AppState`
        // constructor makes its own empty broadcaster, so without
        // `with_log_broadcaster` the layer publishes into one object and the
        // SSE route reads another — indistinguishable from a station that has
        // logged nothing at all.
        let broadcaster = LogBroadcaster::new();
        let state = state(); // deliberately not wired

        let subscriber = tracing_subscriber::registry().with(LogCapture::new(broadcaster, None));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("the microphone stopped");
        });

        assert!(
            state.log_broadcaster().recent(50).is_empty(),
            "this is the bug the gate above describes"
        );
    }

    #[test]
    fn the_error_log_sits_beside_the_database() {
        assert_eq!(
            error_log_path(Path::new("/var/lib/birdnet/birds.db")),
            PathBuf::from("/var/lib/birdnet/errors.jsonl")
        );
    }
}
