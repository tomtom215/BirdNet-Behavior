//! Overlay admin-UI settings on top of the file-based configuration.
//!
//! The admin settings form (`/admin/settings`) persists key/value pairs to the
//! SQLite `settings` table, but the rest of the binary reads its configuration
//! from `/etc/birdnet/birdnet.conf` (parsed into [`Config`]) plus CLI flags.
//! Without a bridge the settings form is write-only: changing the confidence
//! threshold or the audio device in the UI has no effect on the running
//! station.
//!
//! This module is that bridge. At startup we read the settings table and layer
//! every known, non-empty value on top of the file config as a runtime
//! override — the database wins. The mapping is explicit (UI key → config key)
//! rather than blanket, because the two namespaces differ (the UI uses
//! lowercase `confidence_threshold`; the runtime reads BirdNET-Pi's uppercase
//! `CONFIDENCE`) and because only a curated, side-effect-free subset of
//! station/detection settings is wired here. Notification/integration and auth
//! settings are applied by their own subsystems so their activation can be
//! reasoned about (and messaged) on its own.
//!
//! Changes take effect on the next restart, matching the "Changes apply on next
//! restart" notice the settings page already shows.

use birdnet_core::config::Config;
use birdnet_db::settings::{ensure_settings_table, list};
use birdnet_web::state::AppState;

/// Explicit map from an admin-UI settings key to the config key the runtime
/// reads. Only settings with a verified runtime consumer and no external
/// side-effect are listed; anything absent is still stored but, as before, has
/// no effect on the running daemon.
const SETTING_TO_CONFIG_KEY: &[(&str, &str)] = &[
    // Detection tuning (consumed in `crate::daemon`).
    ("confidence_threshold", "CONFIDENCE"),
    ("sensitivity", "SENSITIVITY"),
    ("overlap", "OVERLAP"),
    ("sf_thresh", "SF_THRESH"),
    ("privacy_threshold", "PRIVACY_THRESHOLD"),
    // Audio capture (consumed in `crate::capture`).
    ("alsa_device", "ALSA_CARD"),
    ("alsa_devices", "ALSA_CARDS"),
    ("rtsp_url", "RTSP_URL"),
    ("audio_format", "AUDIOFMT"),
    // Station / location.
    ("latitude", "LATITUDE"),
    ("longitude", "LONGITUDE"),
    ("station_name", "STATION_NAME"),
    ("site_name", "SITENAME"),
    ("info_site", "INFO_SITE"),
    // System / disk management.
    ("image_cache_dir", "IMAGE_CACHE_DIR"),
    ("max_files_per_species", "MAX_FILES_SPECIES"),
    ("purge_threshold", "DISK_PURGE_THRESHOLD"),
];

/// Resolve the runtime config key a given admin-UI setting maps to, if any.
fn config_key_for(setting_key: &str) -> Option<&'static str> {
    SETTING_TO_CONFIG_KEY
        .iter()
        .find(|(ui, _)| *ui == setting_key)
        .map(|(_, cfg)| *cfg)
}

/// Layer the given settings rows on top of the file config as overrides.
///
/// Pure (no I/O) so the mapping and merge precedence are unit-testable without
/// a database. Empty values are skipped so clearing a field falls back to the
/// file config rather than blanking a key. When the file config is absent but
/// overrides exist, a fresh [`Config`] is created to carry them.
fn apply_setting_overrides<'a, I>(config: Option<Config>, settings: I) -> Option<Config>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut merged = config;
    let mut applied = 0_usize;

    for (key, value) in settings {
        if value.trim().is_empty() {
            continue;
        }
        let Some(config_key) = config_key_for(key) else {
            continue;
        };
        merged
            .get_or_insert_with(Config::empty)
            .set(config_key, value);
        applied += 1;
    }

    if applied > 0 {
        tracing::info!(
            count = applied,
            "applied admin settings as runtime config overrides (database wins over the config file)"
        );
    }

    merged
}

/// Read the admin settings table and overlay its known values on the file
/// config, returning the merged configuration the rest of startup should use.
///
/// The database value wins over the config file. This is what makes the
/// settings form take effect: the confidence threshold, audio device and other
/// station/detection settings configured in the web UI are applied here before
/// the capture and detection subsystems are built.
#[must_use]
pub fn overlay_db_settings(config: Option<Config>, state: &AppState) -> Option<Config> {
    let rows = state.with_db(|conn| {
        // The table may not exist yet on a brand-new database; treat that (and
        // any read error) as "no overrides" rather than failing startup.
        ensure_settings_table(conn).ok();
        list(conn, None).unwrap_or_default()
    });

    let pairs: Vec<(String, String)> = rows.into_iter().map(|s| (s.key, s.value)).collect();
    apply_setting_overrides(config, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_detection_keys() {
        assert_eq!(config_key_for("confidence_threshold"), Some("CONFIDENCE"));
        assert_eq!(config_key_for("sensitivity"), Some("SENSITIVITY"));
        assert_eq!(config_key_for("alsa_device"), Some("ALSA_CARD"));
        assert_eq!(config_key_for("alsa_devices"), Some("ALSA_CARDS"));
    }

    #[test]
    fn unknown_key_maps_to_nothing() {
        assert_eq!(config_key_for("totally_unknown_setting"), None);
        // Auth and notification keys are intentionally not bridged here.
        assert_eq!(config_key_for("auth_password"), None);
        assert_eq!(config_key_for("apprise_url"), None);
    }

    #[test]
    fn override_wins_over_file_config() {
        let file = Config::parse("CONFIDENCE=0.25\nSENSITIVITY=1.0").unwrap();
        let merged =
            apply_setting_overrides(Some(file), [("confidence_threshold", "0.8")]).unwrap();
        assert_eq!(merged.get("CONFIDENCE"), Some("0.8"));
        // Untouched file values are preserved.
        assert_eq!(merged.get("SENSITIVITY"), Some("1.0"));
    }

    #[test]
    fn empty_value_does_not_override() {
        let file = Config::parse("ALSA_CARD=plughw:1,0").unwrap();
        let merged = apply_setting_overrides(Some(file), [("alsa_device", "  ")]).unwrap();
        // Blank UI field falls back to the file config rather than blanking it.
        assert_eq!(merged.get("ALSA_CARD"), Some("plughw:1,0"));
    }

    #[test]
    fn unknown_keys_are_ignored_in_merge() {
        let merged = apply_setting_overrides(
            None,
            [
                ("totally_unknown_setting", "x"),
                ("confidence_threshold", "0.7"),
            ],
        )
        .expect("a known override should produce a config");
        assert_eq!(merged.get("CONFIDENCE"), Some("0.7"));
        assert_eq!(merged.get("totally_unknown_setting"), None);
    }

    #[test]
    fn creates_config_when_file_absent_but_overrides_present() {
        let merged = apply_setting_overrides(None, [("confidence_threshold", "0.9")])
            .expect("overrides with no file config should still produce a config");
        assert_eq!(merged.get("CONFIDENCE"), Some("0.9"));
    }

    #[test]
    fn no_overrides_preserves_none() {
        assert!(apply_setting_overrides(None, []).is_none());
    }

    #[test]
    fn no_known_overrides_preserves_original_config() {
        let file = Config::parse("CONFIDENCE=0.42").unwrap();
        let merged = apply_setting_overrides(Some(file), [("unknown", "v")]).unwrap();
        assert_eq!(merged.get("CONFIDENCE"), Some("0.42"));
    }
}
