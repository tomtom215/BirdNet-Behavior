//! What each BirdNET-Pi `birdnet.conf` key becomes here.
//!
//! Every row is derived from how BirdNET-Pi's own code uses the key
//! (`scripts/install_config.sh`, `config.php`, `advanced.php`, `utils/*.py`,
//! the `*.sh` helpers at Nachtzuster/BirdNET-Pi `88985a3`), not from its name.
//! Where the name misleads, the row says so: `APPRISE_ONLY_NOTIFY_SPECIES_NAMES`
//! is the **exclusion** list and `_2` the inclusion list, so a rename would
//! invert both.
//!
//! The migration guide's mapping table is generated from [`RULES`] by
//! [`mapping_table_markdown`] and a test fails when the two differ, so the
//! documentation cannot say something the importer does not do.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::{
    Assignment, ConfEntry, ConfFile, ConfigLine, Decision, Plan, Row, Verdict, hide, is_secret_key,
};

/// How a BirdNET-Pi value is turned into this station's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convert {
    /// The value as written, trimmed. A blank value means "not set" and is left.
    Verbatim,
    /// As written, unless it is a shell variable (`$HOSTNAME`), which only
    /// BirdNET-Pi's shell scripts would have expanded.
    NotShellVariable,
    /// A latitude or longitude. BirdNET-Pi's installer writes `0.0000` when it
    /// cannot find one, so a position of exactly 0, 0 (or -1, -1) means
    /// "unknown" and is not copied.
    Coordinate,
    /// `1` or `0` into `true` or `false`.
    Switch,
    /// `1` into the day the weekly report goes (`monday`, BirdNET-Pi's own
    /// day), `0` into `disabled`.
    WeeklyReport,
    /// `purge` or `keep`, in any case.
    FullDisk,
    /// BirdNET-Pi's audio format if this station can write it.
    AudioFormat,
    /// `ALLABOUTBIRDS` or `EBIRD` into the lower-case names used here.
    InfoSite,
    /// A percentage of the species list into a rank: `max(10, 60 x p)`
    /// (`utils/analysis.py: filter_humans`). Also sets `privacy_threshold` to 0,
    /// the rule that is a confidence here and a rank there.
    PrivacyRank,
    /// A file on this device: a web address is not one.
    LocalFile,
    /// A message template, with the HTML entities BirdNET-Pi's form wrote.
    HtmlUnescape,
    /// A comma-separated list of common names, normalised.
    NameList,
    /// The three notification switches into one list of rules.
    TriggerSwitches,
}

/// Where a BirdNET-Pi key goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Becomes a setting of this station (the settings table, written through
    /// the settings form's own path).
    Setting {
        /// The settings key.
        key: &'static str,
        /// How the value is converted.
        convert: Convert,
    },
    /// This station reads the key from `birdnet.conf` only; the line to put
    /// there is given.
    ConfigFile {
        /// The key this station reads.
        key: &'static str,
        /// How the value is converted.
        convert: Convert,
    },
    /// Not carried over, and what to do instead. The reason is the row's text.
    Note,
    /// Not carried over; nothing replaces it.
    Nothing,
}

/// One BirdNET-Pi key and what happens to it.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// The key in `birdnet.conf`.
    pub pi_key: &'static str,
    /// What BirdNET-Pi does with it, from its code.
    pub pi_meaning: &'static str,
    /// Where it goes.
    pub target: Target,
    /// Whether the value is a credential, or an address that holds one.
    pub secret: bool,
    /// What differs, what is lost, or why there is no equivalent. Always
    /// truthful for every value the key can hold; value-specific text is added
    /// by the importer when it sees the value.
    pub caveat: &'static str,
}

const fn set(
    pi_key: &'static str,
    pi_meaning: &'static str,
    key: &'static str,
    convert: Convert,
    caveat: &'static str,
) -> Rule {
    Rule {
        pi_key,
        pi_meaning,
        target: Target::Setting { key, convert },
        secret: false,
        caveat,
    }
}

const fn secret(rule: Rule) -> Rule {
    Rule {
        secret: true,
        ..rule
    }
}

const fn file(
    pi_key: &'static str,
    pi_meaning: &'static str,
    key: &'static str,
    convert: Convert,
    caveat: &'static str,
) -> Rule {
    Rule {
        pi_key,
        pi_meaning,
        target: Target::ConfigFile { key, convert },
        secret: false,
        caveat,
    }
}

const fn note(pi_key: &'static str, pi_meaning: &'static str, caveat: &'static str) -> Rule {
    Rule {
        pi_key,
        pi_meaning,
        target: Target::Note,
        secret: false,
        caveat,
    }
}

const fn nothing(pi_key: &'static str, pi_meaning: &'static str, caveat: &'static str) -> Rule {
    Rule {
        pi_key,
        pi_meaning,
        target: Target::Nothing,
        secret: false,
        caveat,
    }
}

/// The keys BirdNET-Pi writes into `birdnet.conf`, as of `88985a3`, and the
/// ones its older releases wrote.
pub const RULES: &[Rule] = &[
    // ---- station -------------------------------------------------------
    set(
        "SITE_NAME",
        "title in the web interface's banner (`common.php`)",
        "site_name",
        Convert::NotShellVariable,
        "BirdNET-Pi's default is the shell variable `$HOSTNAME`, which is not copied as text.",
    ),
    set(
        "LATITUDE",
        "position for the occurrence filter and the weather of a detection",
        "latitude",
        Convert::Coordinate,
        "A position of 0, 0 (what BirdNET-Pi writes when it cannot find one) is not copied: \
         a station without a position runs without the occurrence filter here, where \
         BirdNET-Pi asked the model about the Gulf of Guinea.",
    ),
    set(
        "LONGITUDE",
        "position for the occurrence filter and the weather of a detection",
        "longitude",
        Convert::Coordinate,
        "See `LATITUDE`.",
    ),
    // ---- model ---------------------------------------------------------
    note(
        "MODEL",
        "which classifier runs: BirdNET V1, V2.4 FP16 (the default), Perch v2 or a BirdNET-Go classifier (`utils/models.py`)",
        "The classifier is chosen on the Classifier page, not copied: a new install here runs \
         BirdNET V2.4, the same network BirdNET-Pi runs by default (32-bit floats where \
         BirdNET-Pi uses 16-bit).",
    ),
    set(
        "SF_THRESH",
        "how likely a species must be here, this week, to be accepted (occurrence filter threshold)",
        "sf_thresh",
        Convert::Verbatim,
        "The same number with the same meaning, but see `DATA_MODEL_VERSION`.",
    ),
    note(
        "DATA_MODEL_VERSION",
        "which occurrence model: 1 (the default) or 2 (`utils/models.py: get_meta_model`)",
        "Version 1 is not offered here (it is not hosted as ONNX): this station uses version 2 \
         with V2.4, and asks it about a week of its own 48-week year, not the ISO week. \
         Measured with BirdNET-Pi's own range models: the species lists differ on most days of \
         the year (the calendar alone, on 219 to 341 days a year, by a mean of 5 to 33 species; \
         version 1 against version 2 on every day, by a mean of 17 to 141). On the 120 clips \
         tested, BirdNET-Pi's own stored rows were the same under all three \
         (version 1 with the ISO week, version 2 with the ISO week, version 2 with the \
         48-week year), and a bird that falls between the lists was not among them \
         (see \"Same audio, same detections\").",
    ),
    // ---- integrations --------------------------------------------------
    secret(set(
        "BIRDWEATHER_ID",
        "station ID for uploading detections and soundscapes to BirdWeather (`reporting.py`)",
        "birdweather_token",
        Convert::Verbatim,
        "Uploads resume, with this station's position, as before. Without this the uploads \
         stop and nothing says so.",
    )),
    secret(set(
        "HEARTBEAT_URL",
        "address pinged after every analysis (`reporting.py`)",
        "heartbeat_url",
        Convert::Verbatim,
        "Pinged on a schedule here rather than after every analysis. Such addresses carry \
         the monitor's key, so it is hidden in the preview.",
    )),
    secret(file(
        "FLICKR_API_KEY",
        "key for the Flickr image provider",
        "FLICKR_API_KEY",
        Convert::Verbatim,
        "Read from `birdnet.conf` only: there is no field for it in the browser.",
    )),
    file(
        "FLICKR_FILTER_EMAIL",
        "show only this Flickr user's photographs",
        "FLICKR_FILTER_EMAIL",
        Convert::Verbatim,
        "Read from `birdnet.conf` only.",
    ),
    file(
        "IMAGE_PROVIDER",
        "where species pictures come from: `WIKIPEDIA`, `FLICKR`, or blank for none",
        "IMAGE_PROVIDER",
        Convert::Verbatim,
        "Read from `birdnet.conf` only, in either case. Blank means no pictures in BirdNET-Pi \
         and Wikipedia here.",
    ),
    set(
        "INFO_SITE",
        "which site a species name links to: `ALLABOUTBIRDS` or `EBIRD` (`common.php`)",
        "info_site",
        Convert::InfoSite,
        "Lower-case here. This station's own default is eBird, so it is set explicitly.",
    ),
    set(
        "DATABASE_LANG",
        "language of the common names stored with a detection",
        "database_lang",
        Convert::Verbatim,
        "Names are translated when shown here, not when stored; the history you import keeps \
         the names it was recorded with.",
    ),
    // ---- sign-in -------------------------------------------------------
    secret(note(
        "CADDY_PWD",
        "password of the `birdnet` user for BirdNET-Pi's web interface (Caddy basic auth, `common.php`)",
        "Not copied, so no import ever writes a password. This station's administrator is \
         `admin`, not `birdnet`; set its password under Access. (If you put this file in place \
         as this station's own `birdnet.conf`, `CADDY_PWD` becomes the password of `admin` the \
         next time the station starts.)",
    )),
    secret(nothing(
        "ICE_PWD",
        "password between BirdNET-Pi's streaming programs (Icecast)",
        "This station streams from inside one program; there is nothing to authenticate.",
    )),
    note(
        "BIRDNETPI_URL",
        "public address of the site; used by Caddy and as the base of a notification's links (`notifications.py`)",
        "Read from the environment as `BNB_PUBLIC_URL`, not from a setting. Notification links \
         are built from it; without it they use `http://<hostname>.local`.",
    ),
    // ---- recording -----------------------------------------------------
    set(
        "REC_CARD",
        "sound card for the recorder: `arecord -D`, or `default` (`birdnet_recording.sh`)",
        "alsa_device",
        Convert::Verbatim,
        "`default` is the system's default input, as in BirdNET-Pi. Clear the field to let \
         this station find a USB microphone by itself.",
    ),
    secret(set(
        "RTSP_STREAM",
        "comma-separated network audio streams that replace the sound card (`birdnet_recording.sh`)",
        "rtsp_urls",
        Convert::Verbatim,
        "Comma-separated here too. An address with a user name and password is hidden in the \
         preview.",
    )),
    note(
        "RTSP_STREAM_TO_LIVESTREAM",
        "position of the stream the live-audio page plays (0 = the first)",
        "Streams are not numbered here: choose the default live source again (`livestream_source`).",
    ),
    note(
        "CHANNELS",
        "channel count the recorder asks the sound card for (`arecord -c`)",
        "Per audio source here: the station reads the device's channels and reduces stereo to \
         mono by a rule chosen on the Audio page (`--channel-report` records and compares).",
    ),
    set(
        "RECORDING_LENGTH",
        "length in seconds of each recording BirdNET-Pi analyses (3 to 60)",
        "segment_duration",
        Convert::Verbatim,
        "",
    ),
    set(
        "EXTRACTION_LENGTH",
        "seconds of audio saved around a detection; blank = 6",
        "extraction_length",
        Convert::Verbatim,
        "",
    ),
    set(
        "AUDIOFMT",
        "format of the saved clip, any `sox` format (`reporting.py: extract_detection`)",
        "audio_format",
        Convert::AudioFormat,
        "BirdNET-Pi's default is mp3; a new install here saves WAV. This station writes wav, \
         mp3, flac and ogg; mp3, flac and ogg need ffmpeg or sox, and fall back to WAV with a \
         log line without one.",
    ),
    // ---- detection -----------------------------------------------------
    set(
        "CONFIDENCE",
        "lowest confidence stored as a detection (`analysis.py`)",
        "confidence_threshold",
        Convert::Verbatim,
        "",
    ),
    set(
        "SENSITIVITY",
        "sigmoid sensitivity of the classifier, 0.5 to 1.5",
        "sensitivity",
        Convert::Verbatim,
        "The same number, with the same meaning, on V2.4.",
    ),
    set(
        "OVERLAP",
        "seconds two analysis windows overlap, 0 to 2.9",
        "overlap",
        Convert::Verbatim,
        "",
    ),
    set(
        "PRIVACY_THRESHOLD",
        "percent of the species list within which a human sound blanks a window (`analysis.py: filter_humans`)",
        "privacy_rank",
        Convert::PrivacyRank,
        "A percentage there, a rank here: `privacy_rank` is `max(10, 60 x value)`, the same \
         rule. `privacy_threshold`, a confidence here, is set to 0 so the value is not read \
         twice, in two scales.",
    ),
    // ---- disk ----------------------------------------------------------
    set(
        "FULL_DISK",
        "what to do at the purge threshold: `purge` the oldest day's clips or `keep` everything and stop the services (`disk_check.sh`)",
        "full_disk_action",
        Convert::FullDisk,
        "`keep` here stops recording and deletes nothing when the disk is full, and records \
         again by itself once there is room. `purge` removes the oldest clips of the \
         most-recorded species first, never below 5 clips of a species, never a locked clip.",
    ),
    set(
        "PURGE_THRESHOLD",
        "disk percentage at which `disk_check.sh` acts; 95 if unset",
        "purge_threshold",
        Convert::Verbatim,
        "Called `DISK_PURGE_THRESHOLD` in this station's `birdnet.conf`.",
    ),
    set(
        "MAX_FILES_SPECIES",
        "clips kept per species; 0 keeps all (`disk_species_clean.sh`)",
        "max_files_per_species",
        Convert::Verbatim,
        "A different rule: BirdNET-Pi keeps the N most **confident** clips of a species, plus \
         everything from the last 8 days and every protected one. This station keeps the N \
         **newest** clips of a species, and never a locked one. A BirdNET-Pi \"protected\" \
         clip is a locked one here, but the move does not carry the protection across: lock the clips you want to keep once they are imported.",
    ),
    // ---- notifications -------------------------------------------------
    set(
        "APPRISE_NOTIFICATION_TITLE",
        "title of a notification (`notifications.py`)",
        "notify_title_template",
        Convert::HtmlUnescape,
        "The same `$comname`, `$sciname`, `$reason` and the rest are understood. `$confidence` \
         prints two decimals here, BirdNET-Pi's raw number there.",
    ),
    set(
        "APPRISE_NOTIFICATION_BODY",
        "body of a notification, in releases before September 2025 (now `body.txt`)",
        "notify_body_template",
        Convert::HtmlUnescape,
        "Later releases keep the body in `body.txt`, which the import reads too.",
    ),
    set(
        "APPRISE_NOTIFY_EACH_DETECTION",
        "send a notification for every detection (`notifications.py`)",
        "notify_trigger",
        Convert::TriggerSwitches,
        "The three switches are one setting here, a list of rules. When more than one rule \
         matches a detection, one message is sent that names every reason; BirdNET-Pi sends \
         one message per rule. All three off means no notifications at all, as there.",
    ),
    set(
        "APPRISE_NOTIFY_NEW_SPECIES",
        "send a notification for a species seen five times or fewer in the last 7 days, this detection included",
        "notify_trigger",
        Convert::TriggerSwitches,
        "See `APPRISE_NOTIFY_EACH_DETECTION`. BirdNET-Pi counts the last 8 calendar days \
         (`Date >= today - 7`); this station counts 7.",
    ),
    set(
        "APPRISE_NOTIFY_NEW_SPECIES_EACH_DAY",
        "send a notification for the first detection of a species each day",
        "notify_trigger",
        Convert::TriggerSwitches,
        "See `APPRISE_NOTIFY_EACH_DETECTION`.",
    ),
    set(
        "APPRISE_WEEKLY_REPORT",
        "send the weekly report, Mondays at 01:00 (cron, `weekly_report.sh`)",
        "weekly_report_schedule",
        Convert::WeeklyReport,
        "A weekday here, not an hour. 1 becomes `monday`, 0 `disabled`.",
    ),
    set(
        "APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES",
        "seconds before the same species may notify again; 0 = no limit",
        "notify_cooldown",
        Convert::Verbatim,
        "BirdNET-Pi's default is no limit; this station's is 300 seconds, so the value is \
         always set explicitly.",
    ),
    set(
        "APPRISE_ONLY_NOTIFY_SPECIES_NAMES",
        "species that are **never** notified (an exclusion list, despite the name; `notifications.py: should_notify`)",
        "notify_species_exclude",
        Convert::NameList,
        "Common names, comma-separated, as in BirdNET-Pi.",
    ),
    set(
        "APPRISE_ONLY_NOTIFY_SPECIES_NAMES_2",
        "species that are the **only** ones notified (an inclusion list; `notifications.py: should_notify`)",
        "notify_species_only",
        Convert::NameList,
        "Common names, comma-separated, as in BirdNET-Pi.",
    ),
    // ---- display -------------------------------------------------------
    set(
        "RAW_SPECTROGRAM",
        "draw spectrograms without axes and labels (`spectrogram.sh`)",
        "raw_spectrogram",
        Convert::Switch,
        "This renderer draws no axes in either mode; the setting hides the species and \
         confidence text.",
    ),
    set(
        "RARE_SPECIES_THRESHOLD",
        "days without a sighting after which a species is called rare (`overview.php`)",
        "rare_species_days",
        Convert::Verbatim,
        "",
    ),
    nothing(
        "COLOR_SCHEME",
        "light or dark page colours for everyone (`common.php`)",
        "Chosen per browser here (the Display menu), so each person has their own.",
    ),
    set(
        "CUSTOM_IMAGE",
        "the picture shown on the Overview page, a path or a web address (`overview.php`)",
        "dashboard_image",
        Convert::LocalFile,
        "BirdNET-Pi also accepts a web address (it reads the picture with `file_get_contents`); \
         this station reads a file on this device, so a web address is not carried over.",
    ),
    set(
        "CUSTOM_IMAGE_TITLE",
        "the heading over that picture",
        "dashboard_image_title",
        Convert::Verbatim,
        "",
    ),
    nothing(
        "FREQSHIFT_TOOL",
        "how a clip is pitch-shifted when you ask for it on the Recordings page: `sox` or `ffmpeg`",
        "BirdNET-Pi makes a shifted copy of a clip on demand. `freq_shift_hz` here rewrites \
         every **new** clip as it is saved, which is not the same, so it is not set.",
    ),
    nothing("FREQSHIFT_HI", "upper frequency for the `ffmpeg` shift", "See `FREQSHIFT_TOOL`."),
    nothing("FREQSHIFT_LO", "lower frequency for the `ffmpeg` shift", "See `FREQSHIFT_TOOL`."),
    nothing("FREQSHIFT_PITCH", "pitch change for the `sox` shift, in hundredths of a semitone", "See `FREQSHIFT_TOOL`."),
    nothing(
        "FREQSHIFT_RECONNECT_DELAY",
        "milliseconds the live page waits when it switches the live stream's shift",
        "See `FREQSHIFT_TOOL`.",
    ),
    nothing(
        "ACTIVATE_FREQSHIFT_IN_LIVESTREAM",
        "pitch-shift the live audio",
        "See `FREQSHIFT_TOOL`.",
    ),
    // ---- updates, services, internals ------------------------------------
    note(
        "AUTOMATIC_UPDATE",
        "update BirdNET-Pi from git every Sunday at 03:00 (`update_birdnet.sh`)",
        "This station does not update itself from the network. Updates are applied by you, \
         from the update page.",
    ),
    nothing(
        "SILENCE_UPDATE_INDICATOR",
        "hide the number of commits behind",
        "There is no such indicator.",
    ),
    nothing(
        "LogLevel_BirdnetRecordingService",
        "`ffmpeg` log level of BirdNET-Pi's recording service",
        "BirdNET-Pi's three log levels are for the programs it runs; this station is one \
         program with one log level (`LOG_LEVEL`).",
    ),
    nothing(
        "LogLevel_LiveAudioStreamService",
        "`ffmpeg` log level of BirdNET-Pi's live-stream service",
        "See `LogLevel_BirdnetRecordingService`.",
    ),
    nothing(
        "LogLevel_SpectrogramViewerService",
        "`sox` log level of BirdNET-Pi's spectrogram service",
        "See `LogLevel_BirdnetRecordingService`.",
    ),
    nothing("BIRDNET_USER", "the Unix user BirdNET-Pi runs as", "Internal to BirdNET-Pi."),
    nothing("RECS_DIR", "where BirdNET-Pi records and finds its audio", "This station's recordings directory is its own. Give the old folder to the recordings step of the move to bring the clips across."),
    nothing("PROCESSED", "where analysed recordings wait to be deleted", "Internal to BirdNET-Pi."),
    nothing("EXTRACTED", "where BirdNET-Pi keeps its clips (`By_Date/...`)", "Not read by this station. Give the old install's folder to the recordings step of the move to bring the clips across."),
    nothing("IDFILE", "BirdNET-Pi's list of species identified so far", "Internal to BirdNET-Pi."),
    nothing("LAST_RUN", "debugging aid", "Internal to BirdNET-Pi."),
    nothing("THIS_RUN", "debugging aid", "Internal to BirdNET-Pi."),
    // Written by releases of BirdNET-Pi before 2025.
    secret(nothing("PUSHED_APP_KEY", "key for the Pushed.co service, dropped in 2022", "Pushed.co is not supported; use a notification address.")),
    secret(nothing("PUSHED_APP_SECRET", "secret for the Pushed.co service, dropped in 2022", "See `PUSHED_APP_KEY`.")),
    secret(nothing("DB_PWD", "password of a database BirdNET-Pi no longer uses", "This station uses SQLite.")),
    secret(nothing("DB_ROOT_PWD", "root password of a database BirdNET-Pi no longer uses", "See `DB_PWD`.")),
    nothing("DO_RECORDING", "switch for BirdNET-Pi's recorder", "Internal to BirdNET-Pi."),
    nothing("DO_EXTRACTIONS", "switch for BirdNET-Pi's clip extraction", "Internal to BirdNET-Pi."),
    nothing("BIRDNETLOG_URL", "address of BirdNET-Pi's log viewer", "There is no separate log viewer."),
    nothing("WEBTERMINAL_URL", "address of BirdNET-Pi's web terminal", "This station has no web terminal."),
    nothing("EXTRACTIONLOG_URL", "address of BirdNET-Pi's extraction log", "There is no separate log viewer."),
    nothing("EXTRACTIONS_URL", "address of BirdNET-Pi's extractions", "Clips are served by this station itself."),
    nothing("VENV", "BirdNET-Pi's Python environment", "Internal to BirdNET-Pi."),
    nothing("INSTALL_DATE", "when BirdNET-Pi was installed", "Internal to BirdNET-Pi."),
    nothing("ANALYZED", "where BirdNET-Pi keeps analysed files", "Internal to BirdNET-Pi."),
    nothing("TIMESTAMP_FORMAT", "date format in an old release's file names", "Internal to BirdNET-Pi."),
];

/// The keys whose values are credentials, derived from [`RULES`].
#[cfg(test)]
pub(crate) fn secret_keys() -> impl Iterator<Item = &'static str> {
    RULES.iter().filter(|r| r.secret).map(|r| r.pi_key)
}

impl Convert {
    /// Settings this conversion also writes, whatever the value.
    const fn also(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::PrivacyRank => &[("privacy_threshold", "0")],
            _ => &[],
        }
    }
}

/// What a conversion made of a value.
enum Conv {
    /// Use this.
    Set {
        value: String,
        /// Whether it differs from what was written.
        converted: bool,
        /// A sentence about it, when there is one.
        note: String,
    },
    /// Nothing to import, and why.
    Leave(String),
    /// A value this station cannot use, and why.
    Refuse(String),
}

fn same(value: &str) -> Conv {
    Conv::Set {
        value: value.to_owned(),
        converted: false,
        note: String::new(),
    }
}

fn changed(value: String, note: impl Into<String>) -> Conv {
    Conv::Set {
        value,
        converted: true,
        note: note.into(),
    }
}

/// Convert one value. `raw` is trimmed already.
fn convert(how: Convert, raw: &str, conf: &ConfFile) -> Conv {
    if raw.is_empty()
        && !matches!(
            how,
            Convert::TriggerSwitches | Convert::WeeklyReport | Convert::Switch
        )
    {
        return Conv::Leave("blank in the file, so there is nothing to carry over".to_owned());
    }
    match how {
        Convert::Verbatim => same(raw),
        Convert::LocalFile => {
            let lower = raw.to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                Conv::Refuse(
                    "a web address: this station shows a picture from a file on this device, \
                     so save the picture there and enter its path"
                        .to_owned(),
                )
            } else {
                same(raw)
            }
        }
        Convert::NotShellVariable => {
            if raw.contains('$') {
                Conv::Leave(format!(
                    "{raw} is a shell variable that only BirdNET-Pi's scripts would have expanded"
                ))
            } else {
                same(raw)
            }
        }
        Convert::Coordinate => coordinate(raw, conf),
        Convert::Switch => match raw {
            "1" => changed("true".to_owned(), "1 is on"),
            "0" | "" => changed("false".to_owned(), "0 is off"),
            other => Conv::Refuse(format!("expected 0 or 1, found {other}")),
        },
        Convert::WeeklyReport => match raw {
            "1" => changed(
                "monday".to_owned(),
                "BirdNET-Pi sends it Mondays at 01:00; here the day is chosen, not the hour",
            ),
            "0" | "" => changed("disabled".to_owned(), "0 is off"),
            other => Conv::Refuse(format!("expected 0 or 1, found {other}")),
        },
        Convert::FullDisk => {
            let lower = raw.to_ascii_lowercase();
            match lower.as_str() {
                "purge" => Conv::Set {
                    value: lower.clone(),
                    converted: lower != raw,
                    note: String::new(),
                },
                "keep" => Conv::Set {
                    value: lower.clone(),
                    converted: lower != raw,
                    note: "when the disk is full this station stops recording and deletes nothing"
                        .to_owned(),
                },
                other => Conv::Refuse(format!("expected purge or keep, found {other}")),
            }
        }
        Convert::AudioFormat => {
            let lower = raw.to_ascii_lowercase();
            if matches!(lower.as_str(), "wav" | "mp3" | "flac" | "ogg") {
                if lower == raw { same(raw) } else { changed(lower, "lower case") }
            } else {
                Conv::Refuse(format!(
                    "this station writes wav, mp3, flac and ogg; {raw} is not one of them, \
                     so clips stay in the format already set"
                ))
            }
        }
        Convert::InfoSite => match raw.to_ascii_uppercase().as_str() {
            "ALLABOUTBIRDS" => changed("allaboutbirds".to_owned(), "lower case"),
            "EBIRD" => changed("ebird".to_owned(), "lower case"),
            _ => Conv::Refuse(format!("expected ALLABOUTBIRDS or EBIRD, found {raw}")),
        },
        Convert::PrivacyRank => match raw.parse::<f64>() {
            Ok(p) if p.is_finite() && p >= 0.0 => {
                // `int(6000 * p / 100.0)`, then `max(10, ...)`.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let rank = ((6000.0 * p / 100.0).floor() as u64).max(10);
                changed(
                    rank.to_string(),
                    format!(
                        "BirdNET-Pi blanks a window when a human sound ranks in the top \
                         max(10, 60 x {raw}) = {rank} classes; here the same rule is a rank"
                    ),
                )
            }
            _ => Conv::Refuse(format!("expected a number from 0, found {raw}")),
        },
        Convert::HtmlUnescape => {
            let text = unescape_html(raw);
            if text == raw {
                same(raw)
            } else {
                changed(text, "HTML entities written by BirdNET-Pi's form are decoded")
            }
        }
        Convert::NameList => {
            let names: Vec<&str> = raw
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect();
            if names.is_empty() {
                Conv::Leave("blank in the file, so there is nothing to carry over".to_owned())
            } else {
                let joined = names.join(", ");
                if joined == raw { same(raw) } else { changed(joined, "tidied") }
            }
        }
        Convert::TriggerSwitches => {
            let on = |key: &str| conf.get(key).map(str::trim) == Some("1");
            let rules = [
                (on("APPRISE_NOTIFY_EACH_DETECTION"), "each"),
                (on("APPRISE_NOTIFY_NEW_SPECIES"), "new-species"),
                (on("APPRISE_NOTIFY_NEW_SPECIES_EACH_DAY"), "new-species-daily"),
            ];
            let list: Vec<&str> = rules.iter().filter(|(a, _)| *a).map(|(_, n)| *n).collect();
            if list.is_empty() {
                changed(
                    "off".to_owned(),
                    "all three switches are 0: BirdNET-Pi sends no notification, and neither \
                     will this station",
                )
            } else {
                changed(list.join(","), "")
            }
        }
    }
}

fn coordinate(raw: &str, conf: &ConfFile) -> Conv {
    let Ok(this) = raw.parse::<f64>() else {
        return Conv::Refuse(format!("expected a number, found {raw}"));
    };
    let other = |key: &str| conf.get(key).and_then(|v| v.trim().parse::<f64>().ok());
    let (lat, lon) = (other("LATITUDE"), other("LONGITUDE"));
    let unknown = matches!((lat, lon), (Some(a), Some(b))
        if (a == 0.0 && b == 0.0) || (a == -1.0 && b == -1.0));
    if unknown {
        Conv::Leave(
            "a position of 0, 0 is BirdNET-Pi's \"unknown\"; it is not copied".to_owned(),
        )
    } else {
        let _ = this;
        same(raw)
    }
}

/// The HTML entities BirdNET-Pi's own `html.unescape` would undo, for the ones
/// its settings form can write.
fn unescape_html(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&amp;", "&")
}

/// Decide one entry.
pub(crate) fn decide(
    entry: &ConfEntry,
    rule: Option<&Rule>,
    conf: &ConfFile,
    current: &HashMap<String, String>,
    check: &dyn Fn(&str, &str) -> Option<String>,
) -> Decision {
    let secret = is_secret_key(&entry.key);
    let raw = entry.value.trim();
    let shown = if secret { hide(raw) } else { raw.to_owned() };
    let mut row = Row {
        pi_key: entry.key.clone(),
        shown,
        verdict: Verdict::NotCarried,
        becomes: Vec::new(),
        was: None,
        why: String::new(),
    };
    let mut assignments = Vec::new();
    let mut config_lines = Vec::new();

    let Some(rule) = rule else {
        row.why = "not a key BirdNET-Pi writes, or one this version does not know".to_owned();
        return Decision { row, assignments, config_lines };
    };

    match rule.target {
        Target::Note => {
            row.verdict = Verdict::Attention;
            row.why = note_for(rule, raw);
            if row.why.is_empty() {
                row.verdict = Verdict::Left;
            }
        }
        Target::Nothing => {
            row.verdict = Verdict::NotCarried;
            row.why = rule.caveat.to_owned();
        }
        Target::Setting { key, convert: how } => {
            match convert(how, raw, conf) {
                Conv::Leave(why) => {
                    row.verdict = Verdict::Left;
                    row.why = why;
                }
                Conv::Refuse(why) => {
                    row.verdict = Verdict::Refused;
                    row.why = why;
                }
                Conv::Set { value, converted, note } => {
                    if let Some(problem) = check(key, &value) {
                        row.verdict = Verdict::Refused;
                        row.why = problem;
                    } else {
                        row.verdict = if converted { Verdict::Converted } else { Verdict::Imported };
                        row.why = join_sentences(&[&note, rule.caveat_for_row()]);
                        let shown_value = if secret { hide(&value) } else { value.clone() };
                        row.becomes.push((key.to_owned(), shown_value));
                        row.was = current.get(key).map(|v| {
                            if secret { hide(v) } else { v.clone() }
                        });
                        assignments.push(Assignment { setting: key, value, secret });
                        for (extra, v) in how.also() {
                            row.becomes.push(((*extra).to_owned(), (*v).to_owned()));
                            assignments.push(Assignment {
                                setting: extra,
                                value: (*v).to_owned(),
                                secret: false,
                            });
                        }
                        if rule.pi_key == "MAX_FILES_SPECIES" && value_is_positive(raw) {
                            row.verdict = Verdict::Attention;
                        }
                        if rule.pi_key == "FULL_DISK"
                            && raw.eq_ignore_ascii_case("keep")
                            && conf.get("MAX_FILES_SPECIES").is_some_and(value_is_positive)
                        {
                            row.why.push_str(
                                " MAX_FILES_SPECIES is also set, and that cap still removes \
                                 old clips on this station as it did on BirdNET-Pi.",
                            );
                            row.verdict = Verdict::Attention;
                        }
                    }
                }
            }
        }
        Target::ConfigFile { key, convert: how } => match convert(how, raw, conf) {
            Conv::Leave(why) => {
                row.verdict = Verdict::Left;
                row.why = why;
            }
            Conv::Refuse(why) => {
                row.verdict = Verdict::Refused;
                row.why = why;
            }
            Conv::Set { value, .. } => {
                row.verdict = Verdict::ConfigFile;
                row.why = format!(
                    "Put {key}={} in this station's birdnet.conf. {}",
                    if secret { "(the value above)" } else { value.as_str() },
                    rule.caveat
                );
                row.becomes.push((key.to_owned(), if secret { hide(&value) } else { value.clone() }));
                config_lines.push(ConfigLine { key, value, secret });
            }
        },
    }

    // Value-specific words for the keys whose meaning depends on the value.
    match (rule.pi_key, raw) {
        ("MODEL", "BirdNET_GLOBAL_6K_V2.4_Model_FP16") => {
            row.verdict = Verdict::Left;
            row.why = "BirdNET V2.4, which a new install here runs too (32-bit floats where \
                       BirdNET-Pi uses 16-bit)."
                .to_owned();
        }
        ("MODEL", "BirdNET_6K_GLOBAL_MODEL") => {
            row.why = "BirdNET V1: this station does not offer it. It runs V2.4, a newer \
                       network with many more species; detections will differ."
                .to_owned();
        }
        ("MODEL", "Perch_v2") => {
            row.why = "Perch v2 can be chosen on the Classifier page; nothing is changed by \
                       this import."
                .to_owned();
        }
        ("DATA_MODEL_VERSION", "2") => {
            row.verdict = Verdict::Left;
            row.why = "occurrence model 2, which this station uses with V2.4 as well.".to_owned();
        }
        _ => {}
    }

    Decision { row, assignments, config_lines }
}

fn value_is_positive(v: &str) -> bool {
    v.trim().parse::<u64>().is_ok_and(|n| n > 0)
}

fn note_for(rule: &Rule, _raw: &str) -> String {
    rule.caveat.to_owned()
}

fn join_sentences(parts: &[&str]) -> String {
    let mut out = String::new();
    for p in parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        let mut chars = p.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
        if !p.ends_with(['.', '!', '?', ')']) {
            out.push('.');
        }
    }
    out
}

impl Rule {
    /// The caveat to show beside a row that was set: all of it, because every
    /// caveat in the table is true of every value the key can hold.
    const fn caveat_for_row(&self) -> &'static str {
        self.caveat
    }
}

/// Behaviour BirdNET-Pi has that this station would change by default, pinned
/// explicitly so a migrated station behaves as the old one did.
///
/// Pushes rows (and assignments) for settings **no BirdNET-Pi key names**:
/// BirdNET-Pi notifies for every stored detection, with no minimum of its own,
/// and this station's notification minimum defaults to 0.80, which would
/// silently stop notifications for detections between the station's confidence
/// and 0.80.
pub(crate) fn implied(
    conf: &ConfFile,
    current: &HashMap<String, String>,
    check: &dyn Fn(&str, &str) -> Option<String>,
    out: &mut Plan,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let notifies = conf.entries.iter().any(|e| {
        e.key.starts_with("APPRISE_NOTIFY_") || e.key == "APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES"
    });
    if notifies {
        let (setting, value) = ("notify_confidence", "0");
        let refused = check(setting, value);
        let mut row = Row {
            pi_key: "(no BirdNET-Pi key)".to_owned(),
            shown: String::new(),
            verdict: Verdict::Converted,
            becomes: vec![(setting.to_owned(), value.to_owned())],
            was: current.get(setting).cloned(),
            why: "BirdNET-Pi notifies for every detection it stores and has no minimum of its \
                  own. This station has a separate notification minimum (0.80 unless set), \
                  which would have stopped notifications for detections between your \
                  confidence and 0.80. Set to 0 so the same detections notify; raise it \
                  later if you want fewer."
                .to_owned(),
        };
        if let Some(problem) = refused {
            row.verdict = Verdict::Refused;
            row.becomes.clear();
            row.why = problem;
        } else {
            out.assignments.push(Assignment {
                setting,
                value: value.to_owned(),
                secret: false,
            });
        }
        rows.push(row);
    }
    rows
}

/// The mapping table, as the Markdown the migration guide carries.
///
/// One row per [`RULES`] entry, in table order. The guide embeds exactly this
/// text between two marker comments and a test fails when they differ.
#[must_use]
pub fn mapping_table_markdown() -> String {
    let mut out = String::new();
    out.push_str("| BirdNET-Pi key | What it does there | Here | What changes |\n");
    out.push_str("|---|---|---|---|\n");
    for r in RULES {
        let here = match r.target {
            Target::Setting { key, convert } => {
                let mut s = format!("setting `{key}`");
                for (extra, v) in convert.also() {
                    let _ = write!(s, " and `{extra}` = {v}");
                }
                s
            }
            Target::ConfigFile { key, .. } => format!("`{key}` in `birdnet.conf`"),
            Target::Note => "not carried over (see the note)".to_owned(),
            Target::Nothing => "no equivalent".to_owned(),
        };
        let secret = if r.secret { " *(a secret; never shown or logged)*" } else { "" };
        let _ = writeln!(
            out,
            "| `{}`{} | {} | {} | {} |",
            r.pi_key,
            secret,
            md_cell(r.pi_meaning),
            md_cell(&here),
            md_cell(r.caveat),
        );
    }
    out
}

fn md_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_appears_once() {
        let mut seen = std::collections::BTreeSet::new();
        for r in RULES {
            assert!(seen.insert(r.pi_key), "{} is in the table twice", r.pi_key);
        }
    }

    #[test]
    fn credentials_are_marked() {
        let secrets: Vec<&str> = secret_keys().collect();
        for key in [
            "BIRDWEATHER_ID",
            "CADDY_PWD",
            "ICE_PWD",
            "RTSP_STREAM",
            "FLICKR_API_KEY",
            "PUSHED_APP_SECRET",
        ] {
            assert!(secrets.contains(&key), "{key} must be a secret");
        }
    }
}
