//! The start-to-start comparison the binary makes before it serves (UP-3).

use std::path::Path;

use birdnet_web::boot_journal::{Anomaly, BootRecord, journal_path, record};
use birdnet_web::data_volume::{MountShape, mount_shape};
use birdnet_web::state::AppState;

/// Record this start against the last one and return what changed that
/// should not have. Logged at `error` per anomaly; a journal that cannot be
/// written is a warning, and the station starts either way.
///
/// `db_present` is whether the database file existed *before* this start
/// opened it — `open_or_create` makes the path exist, so the caller reads it
/// first.
pub fn record_boot(
    config_path: &Path,
    db_path: &Path,
    db_present: bool,
    state: &AppState,
) -> Vec<Anomaly> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let path = journal_path(config_path, data_dir);
    let detections = if db_present {
        state.with_db(|conn| birdnet_db::sqlite::detection_count(conn).unwrap_or(0))
    } else {
        0
    };
    let separate_mount = match mount_shape(data_dir) {
        MountShape::SeparateMount { .. } => Some(true),
        MountShape::NotAMount => Some(false),
        MountShape::Unknown => None,
    };
    let now = BootRecord {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        db_path: db_path.display().to_string(),
        db_present,
        detections,
        separate_mount,
        booted_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let (anomalies, written) = record(&path, &now);
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "boot journal could not be written; the next start cannot compare itself to this one");
    }
    for anomaly in &anomalies {
        tracing::error!(kind = anomaly.key(), "boot journal: {}", anomaly.describe());
    }
    if anomalies.is_empty() {
        tracing::info!(path = %path.display(), detections, "boot journal: this start matches the last");
    }
    anomalies
}
