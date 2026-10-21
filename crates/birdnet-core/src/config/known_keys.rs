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
    "BIRDWEATHER_LOCATION_ACCURACY",
    "BIRDWEATHER_THRESHOLD",
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
    "EMBED_ORIGINS",
    "EXTRACTION_LENGTH",
    "FLICKR_API_KEY",
    "FLICKR_FILTER_EMAIL",
    "FREQ_SHIFT",
    "FULL_DISK",
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
    "MODEL_2_ID",
    "MODEL_2_LABELS",
    "MODEL_2_PATH",
    "MODEL_2_SAMPLE_RATE",
    "MODEL_2_THRESHOLD",
    "MODEL_3_ID",
    "MODEL_3_LABELS",
    "MODEL_3_PATH",
    "MODEL_3_SAMPLE_RATE",
    "MODEL_3_THRESHOLD",
    "MODEL_DIR",
    "MODEL_ID",
    "MODEL_PATH",
    "MODEL_ROUTES",
    "MODEL_SAMPLE_RATE",
    "MODEL_THRESHOLD",
    "MQTT_CA_FILE",
    "MQTT_CLIENT_CERT",
    "MQTT_CLIENT_KEY",
    "MQTT_HA_DISCOVERY",
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
    "NOISE_REMEMBER_SECS",
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
    "PRIVACY_RANK",
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

/// Keys the installer writes into `birdnet.conf` that nothing here reads.
///
/// They cannot be in [`KNOWN_CONFIG_KEYS`], whose drift gate requires a
/// reader, and are nonetheless expected in the file.
///
/// * `BIRDNET_LISTEN` — written by `installer/lib/62-config-file.sh` and read
///   back by the installer on re-run, to keep the listen address.
/// * `CADDY_USER` — written by the same file; the sign-in form reads it from
///   the environment, and the file keeps the operator's record of it.
///
/// Each drew an unknown-key warning on every start of a fresh install and
/// kept `--doctor` from ever exiting 0.
pub const INSTALLER_KEYS: &[&str] = &["BIRDNET_LISTEN", "CADDY_USER"];

/// What a key BirdNET-Pi writes into `birdnet.conf` is here.
///
/// A `birdnet.conf` carried over from BirdNET-Pi has sixty default keys and
/// this station reads twenty of them under the same name. For the rest an
/// operator was told only "not a setting this station reads", which is true and
/// sends them nowhere. [`birdnet_pi_equivalent`] says where each goes, or that
/// it goes nowhere and why. The full table, with how each value changes, is
/// `birdnet_migrate::conf::RULES`; a test in the binary's crate holds the two
/// to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiEquivalent {
    /// The same thing under another key of this station's `birdnet.conf`.
    Renamed {
        /// This station's key.
        ours: &'static str,
        /// What else is different, or empty.
        note: &'static str,
    },
    /// Not a `birdnet.conf` key here; a setting in the browser, which importing
    /// the file (Station, Data) or `--import-birdnet-pi-conf` carries across.
    Setting {
        /// What to look for on the Settings page.
        name: &'static str,
        /// What else is different, or empty.
        note: &'static str,
    },
    /// Nothing here does this, or it needs more than renaming.
    None {
        /// Why, and what to do instead.
        reason: &'static str,
    },
}

/// BirdNET-Pi's `birdnet.conf` keys that this station does not read under the
/// same name, and what each is here. Sorted by key.
pub const BIRDNET_PI_KEYS: &[(&str, PiEquivalent)] = &[
    (
        "ACTIVATE_FREQSHIFT_IN_LIVESTREAM",
        PiEquivalent::None {
            reason: "BirdNET-Pi shifts the live audio's pitch for the listener; this station's FREQ_SHIFT rewrites every new clip, which is not the same",
        },
    ),
    (
        "ANALYZED",
        PiEquivalent::None {
            reason: "BirdNET-Pi's folder of analysed files; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES",
        PiEquivalent::Renamed {
            ours: "APPRISE_COOLDOWN",
            note: "also in seconds; this station's default is 300, BirdNET-Pi's is no limit, so set it",
        },
    ),
    (
        "APPRISE_NOTIFICATION_BODY",
        PiEquivalent::Renamed {
            ours: "APPRISE_BODY_TEMPLATE",
            note: "",
        },
    ),
    (
        "APPRISE_NOTIFICATION_TITLE",
        PiEquivalent::Renamed {
            ours: "APPRISE_TITLE_TEMPLATE",
            note: "",
        },
    ),
    (
        "APPRISE_NOTIFY_EACH_DETECTION",
        PiEquivalent::Renamed {
            ours: "APPRISE_TRIGGER",
            note: "one list of rules, for example each,new-species; all three switches at 0 is APPRISE_TRIGGER=off",
        },
    ),
    (
        "APPRISE_NOTIFY_NEW_SPECIES",
        PiEquivalent::Renamed {
            ours: "APPRISE_TRIGGER",
            note: "one list of rules, for example each,new-species; all three switches at 0 is APPRISE_TRIGGER=off",
        },
    ),
    (
        "APPRISE_NOTIFY_NEW_SPECIES_EACH_DAY",
        PiEquivalent::Renamed {
            ours: "APPRISE_TRIGGER",
            note: "one list of rules, for example each,new-species-daily; all three switches at 0 is APPRISE_TRIGGER=off",
        },
    ),
    (
        "APPRISE_ONLY_NOTIFY_SPECIES_NAMES",
        PiEquivalent::Renamed {
            ours: "APPRISE_WATCHLIST_EXCLUDE",
            note: "BirdNET-Pi's key is the EXCLUSION list despite its name",
        },
    ),
    (
        "APPRISE_ONLY_NOTIFY_SPECIES_NAMES_2",
        PiEquivalent::Renamed {
            ours: "APPRISE_WATCHLIST",
            note: "BirdNET-Pi's _2 key is the INCLUSION list",
        },
    ),
    (
        "APPRISE_WEEKLY_REPORT",
        PiEquivalent::Renamed {
            ours: "WEEKLY_REPORT_SCHEDULE",
            note: "a weekday here: 1 is monday, 0 is disabled",
        },
    ),
    (
        "AUTOMATIC_UPDATE",
        PiEquivalent::None {
            reason: "this station does not update itself from the network; updates are applied from the update page",
        },
    ),
    (
        "BIRDNETLOG_URL",
        PiEquivalent::None {
            reason: "BirdNET-Pi's log viewer address; this station has no separate log viewer",
        },
    ),
    (
        "BIRDNETPI_URL",
        PiEquivalent::None {
            reason: "read from the environment as BNB_PUBLIC_URL; notification links are built from it",
        },
    ),
    (
        "BIRDNET_USER",
        PiEquivalent::None {
            reason: "the Unix user BirdNET-Pi runs as; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "BIRDWEATHER_ID",
        PiEquivalent::Renamed {
            ours: "BIRDWEATHER_TOKEN",
            note: "without it BirdWeather uploads stop",
        },
    ),
    (
        "CHANNELS",
        PiEquivalent::None {
            reason: "channels are per audio source here (see --channel-report)",
        },
    ),
    (
        "COLOR_SCHEME",
        PiEquivalent::None {
            reason: "light or dark is chosen per browser here",
        },
    ),
    (
        "CUSTOM_IMAGE",
        PiEquivalent::Setting {
            name: "dashboard_image",
            note: "a path to a file on this device, not a web address",
        },
    ),
    (
        "CUSTOM_IMAGE_TITLE",
        PiEquivalent::Setting {
            name: "dashboard_image_title",
            note: "",
        },
    ),
    (
        "DATA_MODEL_VERSION",
        PiEquivalent::None {
            reason: "this station uses occurrence model 2 with V2.4; version 1 is not offered, so the same SF_THRESH admits a somewhat different list of species",
        },
    ),
    (
        "DB_PWD",
        PiEquivalent::None {
            reason: "a database password; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "DB_ROOT_PWD",
        PiEquivalent::None {
            reason: "a database password; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "DO_EXTRACTIONS",
        PiEquivalent::None {
            reason: "a BirdNET-Pi switch; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "DO_RECORDING",
        PiEquivalent::None {
            reason: "a BirdNET-Pi switch; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "EXTRACTED",
        PiEquivalent::None {
            reason: "where BirdNET-Pi keeps its clips; this station keeps clips in its own recordings folder, and the recordings step of the move brings the old ones across",
        },
    ),
    (
        "EXTRACTIONLOG_URL",
        PiEquivalent::None {
            reason: "BirdNET-Pi's extraction log address; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "EXTRACTIONS_URL",
        PiEquivalent::None {
            reason: "BirdNET-Pi's clip address; clips are served by this station itself",
        },
    ),
    (
        "FREQSHIFT_HI",
        PiEquivalent::None {
            reason: "BirdNET-Pi shifts a clip's pitch on request; this station's FREQ_SHIFT rewrites every new clip, which is not the same",
        },
    ),
    (
        "FREQSHIFT_LO",
        PiEquivalent::None {
            reason: "BirdNET-Pi shifts a clip's pitch on request; this station's FREQ_SHIFT rewrites every new clip, which is not the same",
        },
    ),
    (
        "FREQSHIFT_PITCH",
        PiEquivalent::None {
            reason: "BirdNET-Pi shifts a clip's pitch on request; this station's FREQ_SHIFT rewrites every new clip, which is not the same",
        },
    ),
    (
        "FREQSHIFT_RECONNECT_DELAY",
        PiEquivalent::None {
            reason: "belongs to BirdNET-Pi's live-audio pitch shift, which has no equivalent",
        },
    ),
    (
        "FREQSHIFT_TOOL",
        PiEquivalent::None {
            reason: "BirdNET-Pi shifts a clip's pitch on request; this station's FREQ_SHIFT rewrites every new clip, which is not the same",
        },
    ),
    (
        "ICE_PWD",
        PiEquivalent::None {
            reason: "BirdNET-Pi's streaming programs authenticate to each other; this station streams from one program",
        },
    ),
    (
        "IDFILE",
        PiEquivalent::None {
            reason: "BirdNET-Pi's list of species identified so far; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "INSTALL_DATE",
        PiEquivalent::None {
            reason: "when BirdNET-Pi was installed; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "LAST_RUN",
        PiEquivalent::None {
            reason: "a BirdNET-Pi debugging aid; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "LogLevel_BirdnetRecordingService",
        PiEquivalent::None {
            reason: "BirdNET-Pi's three log levels are for the programs it runs; this station has one, LOG_LEVEL",
        },
    ),
    (
        "LogLevel_LiveAudioStreamService",
        PiEquivalent::None {
            reason: "BirdNET-Pi's three log levels are for the programs it runs; this station has one, LOG_LEVEL",
        },
    ),
    (
        "LogLevel_SpectrogramViewerService",
        PiEquivalent::None {
            reason: "BirdNET-Pi's three log levels are for the programs it runs; this station has one, LOG_LEVEL",
        },
    ),
    (
        "PROCESSED",
        PiEquivalent::None {
            reason: "where BirdNET-Pi parks analysed recordings; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "PURGE_THRESHOLD",
        PiEquivalent::Renamed {
            ours: "DISK_PURGE_THRESHOLD",
            note: "without it the station purges at 95 %",
        },
    ),
    (
        "PUSHED_APP_KEY",
        PiEquivalent::None {
            reason: "Pushed.co is not supported; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "PUSHED_APP_SECRET",
        PiEquivalent::None {
            reason: "Pushed.co is not supported; written by older BirdNET-Pi releases and no longer used by BirdNET-Pi itself",
        },
    ),
    (
        "RARE_SPECIES_THRESHOLD",
        PiEquivalent::Setting {
            name: "rare_species_days",
            note: "days without a sighting",
        },
    ),
    (
        "RAW_SPECTROGRAM",
        PiEquivalent::Setting {
            name: "raw_spectrogram",
            note: "",
        },
    ),
    (
        "REC_CARD",
        PiEquivalent::Renamed {
            ours: "ALSA_CARD",
            note: "without it the microphone is not selected",
        },
    ),
    (
        "RTSP_STREAM",
        PiEquivalent::Renamed {
            ours: "RTSP_URLS",
            note: "comma-separated here too",
        },
    ),
    (
        "RTSP_STREAM_TO_LIVESTREAM",
        PiEquivalent::None {
            reason: "streams are not numbered here; choose the default live source again",
        },
    ),
    (
        "SILENCE_UPDATE_INDICATOR",
        PiEquivalent::None {
            reason: "there is no update indicator",
        },
    ),
    (
        "SITE_NAME",
        PiEquivalent::Renamed {
            ours: "SITENAME",
            note: "",
        },
    ),
    (
        "THIS_RUN",
        PiEquivalent::None {
            reason: "a BirdNET-Pi debugging aid; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "TIMESTAMP_FORMAT",
        PiEquivalent::None {
            reason: "a file-name format of an old BirdNET-Pi release; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "VENV",
        PiEquivalent::None {
            reason: "BirdNET-Pi's Python environment; internal to BirdNET-Pi; nothing here needs it",
        },
    ),
    (
        "WEBTERMINAL_URL",
        PiEquivalent::None {
            reason: "BirdNET-Pi's web terminal address; this station has no web terminal",
        },
    ),
];

/// What BirdNET-Pi's `key` is on this station, if it is one of BirdNET-Pi's
/// keys that this station does not read under that name.
#[must_use]
pub fn birdnet_pi_equivalent(key: &str) -> Option<&'static PiEquivalent> {
    BIRDNET_PI_KEYS
        .binary_search_by_key(&key, |(k, _)| k)
        .ok()
        .map(|i| &BIRDNET_PI_KEYS[i].1)
}

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
    fn the_birdnet_pi_table_is_sorted_and_names_keys_that_exist_here() {
        let keys: Vec<&str> = BIRDNET_PI_KEYS.iter().map(|(k, _)| *k).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(keys, sorted, "BIRDNET_PI_KEYS must be sorted and unique");
        for (pi, eq) in BIRDNET_PI_KEYS {
            assert!(
                !is_known(pi),
                "{pi} is read here under that name, so it needs no entry"
            );
            if let PiEquivalent::Renamed { ours, .. } = eq {
                assert!(is_known(ours), "{pi} points at {ours}, which nothing reads");
            }
        }
    }

    #[test]
    fn a_birdnet_pi_key_is_looked_up_and_one_of_ours_is_not() {
        assert!(matches!(
            birdnet_pi_equivalent("BIRDWEATHER_ID"),
            Some(PiEquivalent::Renamed {
                ours: "BIRDWEATHER_TOKEN",
                ..
            })
        ));
        assert!(birdnet_pi_equivalent("CONFIDENCE").is_none());
        assert!(birdnet_pi_equivalent("NOT_A_KEY").is_none());
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
