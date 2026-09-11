//! The `birdnet.conf` keys this station reads, and the nearest one to a key
//! it does not (LC-7).
//!
//! `Config` is a bag of strings: a typo'd key is parsed, stored and never asked
//! for, and until this list existed nothing could tell an operator that
//! `CONFIDENC=0.90` had changed nothing. The list is the set of keys some
//! reader in the workspace asks for; `tests/every_config_key_is_known.rs`
//! scans the source for those reads and fails when the two drift in either
//! direction, so a new `config.get("NEW_KEY")` cannot ship without an entry
//! here and an entry here cannot outlive its last reader.

/// Every `birdnet.conf` key some reader in the workspace asks for.
///
/// Sorted, so the drift gate can compare it as a set and a reader can find a
/// key. Keys that also exist as `BIRDNET_*` environment variables are listed
/// under the name the *file* uses.
pub const KNOWN_CONFIG_KEYS: &[&str] = &[
    "ALSA_CARD",
    "ALSA_CARDS",
    "ANALYTICS_DB_PATH",
    "APPRISE_BODY_TEMPLATE",
    "APPRISE_CONFIG_FILE",
    "APPRISE_COOLDOWN",
    "APPRISE_MIN_CONFIDENCE",
    "APPRISE_TITLE_TEMPLATE",
    "APPRISE_TRIGGER",
    "APPRISE_URL",
    "APPRISE_WATCHLIST",
    "APPRISE_WATCHLIST_EXCLUDE",
    "AUDIOFMT",
    "AUDIO_FORMAT",
    "BACKUP_SCHEDULE",
    "BIRDNET_DYNAMIC_THRESHOLD",
    "BIRDNET_DYNAMIC_THRESHOLD_HOURS",
    "BIRDNET_DYNAMIC_THRESHOLD_MIN",
    "BIRDNET_DYNAMIC_THRESHOLD_TRIGGER",
    "BIRDWEATHER_TOKEN",
    "BIRDWEATHER_URL",
    "BNB_API_TOKEN",
    "CADDY_PWD",
    "CLIP_PEAK_CEILING_DBFS",
    "CLIP_RETENTION_DAYS",
    "CLIP_TARGET_LUFS",
    "CONFIDENCE",
    "CONFIRMATION_LEVEL",
    "CUSTOM_IMAGE_DIR",
    "DATABASE_LANG",
    "DB_PATH",
    "DEADMAN_HOURS",
    "DISK_PURGE_THRESHOLD",
    "DUPLICATE_INTERVAL_SECS",
    "EBIRD_API_KEY",
    "EBIRD_BACK_DAYS",
    "EBIRD_DIST_KM",
    "EBIRD_REGION",
    "EXTRACTION_LENGTH",
    "FLICKR_API_KEY",
    "FLICKR_FILTER_EMAIL",
    "FREQ_SHIFT",
    "HEARTBEAT_URL",
    "IMAGE_CACHE_DIR",
    "IMAGE_PROVIDER",
    "INFO_SITE",
    "LABELS_DIR",
    "LABELS_PATH",
    "LANG",
    "LATITUDE",
    "LOG_LEVEL",
    "LOG_MODULES",
    "LONGITUDE",
    "MAX_FILES_SPECIES",
    "METADATA_LABELS_PATH",
    "METADATA_MODEL_PATH",
    "MODEL",
    "MODEL_PATH",
    "MQTT_CA_FILE",
    "MQTT_HOST",
    "MQTT_PASSWORD",
    "MQTT_PORT",
    "MQTT_RETAIN",
    "MQTT_TLS",
    "MQTT_TLS_SERVER_NAME",
    "MQTT_TOPIC_PREFIX",
    "MQTT_USERNAME",
    "NIGHT_EXTRA_NOCTURNAL",
    "NIGHT_FILTER",
    "NIGHT_INHIBIT",
    "NIGHT_MARGIN_MINS",
    "NOISE_CLASSES",
    "NOISE_THRESHOLD",
    "NOTIFY_RATE_PER_MINUTE",
    "NOTIFY_URLS",
    "OFFSITE_BACKUP",
    "OFFSITE_KEEP",
    "OFFSITE_PASSPHRASE",
    "OFFSITE_RSYNC_BWLIMIT",
    "OFFSITE_S3_ACCESS_KEY",
    "OFFSITE_S3_ADDRESSING",
    "OFFSITE_S3_BUCKET",
    "OFFSITE_S3_ENDPOINT",
    "OFFSITE_S3_PREFIX",
    "OFFSITE_S3_REGION",
    "OFFSITE_S3_SECRET_KEY",
    "OFFSITE_SFTP_DIR",
    "OFFSITE_SFTP_HOST",
    "OFFSITE_SFTP_HOST_KEY_POLICY",
    "OFFSITE_SFTP_IDENTITY",
    "OFFSITE_SFTP_KNOWN_HOSTS",
    "OFFSITE_SFTP_PORT",
    "OFFSITE_SFTP_USER",
    "OVERLAP",
    "PIPEWIRE_DEVICE",
    "POST_SUNSET_OFFSET",
    "PRE_CAPTURE_SECS",
    "PRE_SUNRISE_OFFSET",
    "PRIVACY_THRESHOLD",
    "PRIVATE_MODE",
    "PUBLIC_ACCESS",
    "PURGE_SPECIES_FLOOR",
    "RAW_AUDIO_KEEP_EVERY",
    "RECORDING_LENGTH",
    "RECORDING_SCHEDULE",
    "RECS_DIR",
    "RTSP_URL",
    "RTSP_URLS",
    "SEGMENT_DURATION",
    "SENSITIVITY",
    "SF_THRESH",
    "SITENAME",
    "SPECIES_ALIASES_PATH",
    "STATION_HEALTH_ALERTS",
    "STATION_NAME",
    "STREAM_MAX_MB",
    "STREAM_RETENTION_SECS",
    "TLS_CERT",
    "TLS_DIR",
    "TLS_KEY",
    "WATCHDOG_BACKOFF_BASE_SECS",
    "WATCHDOG_BACKOFF_CAP_SECS",
    "WATCHDOG_CHECK_SECS",
    "WATCHDOG_DOWN_WARN_AFTER_SECS",
    "WATCHDOG_DOWN_WARN_EVERY_SECS",
    "WATCHDOG_STALL_FLOOR_SECS",
    "WATCHDOG_STALL_SEGMENTS",
    "WEATHER_API_KEY",
    "WEATHER_PROVIDER",
    "WEATHER_STATION_ID",
    "WEEKLY_REPORT_SCHEDULE",
];

/// Whether `key` is one this station reads.
#[must_use]
pub fn is_known(key: &str) -> bool {
    KNOWN_CONFIG_KEYS.binary_search(&key).is_ok()
}

/// The known key an unknown one was most likely meant to be, if any is close.
///
/// Close means: the same name with a `BIRDNET_` prefix added or removed
/// (`BIRDNET_LATITUDE` in a file whose key is `LATITUDE`), the same name in
/// another case, or within two edits of a known key (`CONFIDENC`, `LATITUDE`
/// with a transposition). Nothing is suggested for a key that is far from
/// every known one: a wrong guess sends the operator to the wrong setting.
#[must_use]
pub fn did_you_mean(key: &str) -> Option<&'static str> {
    let upper = key.to_ascii_uppercase();
    if let Some(stripped) = upper.strip_prefix("BIRDNET_")
        && let Some(known) = KNOWN_CONFIG_KEYS.iter().find(|k| **k == stripped)
    {
        return Some(known);
    }
    let prefixed = format!("BIRDNET_{upper}");
    if let Some(known) = KNOWN_CONFIG_KEYS.iter().find(|k| **k == prefixed) {
        return Some(known);
    }
    KNOWN_CONFIG_KEYS
        .iter()
        .map(|k| (edit_distance(&upper, k), *k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
}

/// Levenshtein distance between two ASCII-ish strings, by chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_unique_so_binary_search_is_sound() {
        let mut sorted = KNOWN_CONFIG_KEYS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted, KNOWN_CONFIG_KEYS,
            "KNOWN_CONFIG_KEYS must be sorted and unique"
        );
        assert!(is_known("CONFIDENCE"));
        assert!(!is_known("CONFIDENC"));
    }

    #[test]
    fn a_near_miss_names_the_key_it_was_meant_to_be() {
        assert_eq!(did_you_mean("CONFIDENC"), Some("CONFIDENCE"));
        assert_eq!(did_you_mean("LATTITUDE"), Some("LATITUDE"));
        assert_eq!(did_you_mean("confidence"), Some("CONFIDENCE"));
        assert_eq!(did_you_mean("BIRDNET_LATITUDE"), Some("LATITUDE"));
        assert_eq!(
            did_you_mean("DYNAMIC_THRESHOLD"),
            Some("BIRDNET_DYNAMIC_THRESHOLD")
        );
    }

    #[test]
    fn a_key_far_from_everything_gets_no_guess() {
        assert_eq!(did_you_mean("FROBNICATE_LEVEL"), None);
        assert_eq!(did_you_mean("X"), None);
    }

    #[test]
    fn edit_distance_is_levenshtein() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("abc", "abx"), 1);
        assert_eq!(edit_distance("abc", "ab"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }
}
