//! The start-to-start comparison the binary makes before it serves (UP-3).

use std::collections::BTreeMap;
use std::path::Path;

use birdnet_core::audio::capture::alsa::{CardRef, PROC_ASOUND, card_id_in, parse_card_ref};
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
    let devices: Vec<String> = if db_present {
        state.with_db(|conn| {
            use birdnet_db::audio_sources::{AudioSourceStore, SourceKind};
            conn.list()
                .map(|rows| {
                    rows.into_iter()
                        .filter(|s| s.kind == SourceKind::UsbAlsa)
                        .map(|s| s.device_id)
                        .collect()
                })
                .unwrap_or_default()
        })
    } else {
        Vec::new()
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
        alsa_cards: alsa_cards_behind(&devices, Path::new(PROC_ASOUND)),
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

/// The card id the kernel reports behind each index-form ALSA device string
/// (AU-1); id-form devices, unparseable strings and empty indexes are left
/// out, so the journal only compares what an index can lie about.
fn alsa_cards_behind(devices: &[String], proc_asound: &Path) -> BTreeMap<String, String> {
    devices
        .iter()
        .filter_map(|device| match parse_card_ref(device) {
            Some(CardRef::Index(idx)) => {
                card_id_in(proc_asound, &idx).map(|id| (device.clone(), id))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::alsa_cards_behind;

    #[test]
    fn only_index_form_devices_with_a_card_behind_them_are_recorded() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("card1")).unwrap();
        std::fs::write(root.path().join("card1/id"), "PRO\n").unwrap();
        let devices = [
            "plughw:1,0".to_owned(),
            "plughw:CARD=PRO,DEV=0".to_owned(),
            "plughw:3,0".to_owned(),
            "default".to_owned(),
        ];
        let map = alsa_cards_behind(&devices, root.path());
        assert_eq!(map.len(), 1, "{map:?}");
        assert_eq!(map.get("plughw:1,0").map(String::as_str), Some("PRO"));
    }
}
