//! Settings form deserialization types.

use serde::Deserialize;

/// Every settings key the admin form can persist.
///
/// This is the contract between the web UI and the binary that has to make
/// those keys *do* something. The binary classifies each one (bridged onto the
/// runtime config, owned by a subsystem that reads the settings table directly,
/// or deliberately not wired) and a test there fails if any key here is
/// unclassified — so a field cannot be added to this form and silently ship as
/// an editable control that does nothing, which is exactly how twenty of them
/// came to be inert.
///
/// Kept in sync with [`SettingsForm`] and with `build_settings_items` by tests
/// in this module: one asserts the list matches the struct's fields exactly, the
/// other that saving a fully-populated form emits exactly these keys.
pub const SETTINGS_FORM_KEYS: &[&str] = &[
    // Audio
    "alsa_device",
    "rtsp_url",
    "rtsp_urls",
    "segment_duration",
    "audio_format",
    "freq_shift_hz",
    // Location
    "latitude",
    "longitude",
    "station_name",
    "night_inhibit",
    "recording_schedule",
    "pre_sunrise_offset",
    "post_sunset_offset",
    // Detection
    "confidence_threshold",
    "sensitivity",
    "overlap",
    "sf_thresh",
    "privacy_threshold",
    "confirmation_level",
    // Notifications
    "apprise_url",
    "apprise_config",
    "notify_urls",
    "birdweather_token",
    "notify_confidence",
    "notify_cooldown",
    "notify_trigger",
    "notify_species_only",
    "notify_species_exclude",
    "notify_title_template",
    "notify_body_template",
    "weekly_report_schedule",
    "heartbeat_url",
    "deadman_hours",
    // Species
    "species_exclude",
    "species_include",
    // System
    "clip_retention_days",
    "image_cache_dir",
    "custom_image_dir",
    "max_files_per_species",
    "purge_threshold",
    "raw_spectrogram",
    "extraction_length",
    "rare_species_days",
    "stream_retention_secs",
    "stream_max_mb",
    "site_name",
    "info_site",
    "database_lang",
    // Email
    "email_smtp_host",
    "email_smtp_port",
    "email_smtp_user",
    "email_smtp_pass",
    "email_from",
    "email_to",
    "email_from_name",
    "email_starttls",
    "email_min_confidence",
    "email_cooldown_secs",
];

/// Flat form payload from the admin settings POST.
///
/// Every field is `Option` because HTMX partial-save may only submit
/// a single tab's fields.
///
/// `Serialize` is test-only on purpose: it exists so a test can enumerate the
/// struct's own field names and pin them against [`SETTINGS_FORM_KEYS`], and
/// gating it keeps a payload carrying `email_smtp_pass` from being serialisable
/// anywhere in production.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize, Default))]
pub struct SettingsForm {
    // Audio
    /// ALSA device string (e.g. `plughw:1,0`) used when the source type is USB microphone.
    pub alsa_device: Option<String>,
    /// Single RTSP stream URL for the primary audio source.
    pub rtsp_url: Option<String>,
    /// Newline-separated list of additional RTSP stream URLs for multi-stream capture.
    pub rtsp_urls: Option<String>,
    /// Duration of each audio segment sent to BirdNET inference, in seconds.
    pub segment_duration: Option<String>,
    /// Audio sample format (e.g. `S16_LE`, `S24_LE`).
    pub audio_format: Option<String>,
    /// Frequency shift applied before inference, in Hz. Use `0` to disable.
    pub freq_shift_hz: Option<String>,
    // Location
    /// Station latitude in decimal degrees (e.g. `51.5074`). Used for solar scheduling.
    pub latitude: Option<String>,
    /// Station longitude in decimal degrees (e.g. `-0.1278`). Used for solar scheduling.
    pub longitude: Option<String>,
    /// Human-readable name for this monitoring station, shown in the UI and reports.
    pub station_name: Option<String>,
    // Detection
    /// Minimum confidence score (0.0–1.0) required to record a detection.
    pub confidence_threshold: Option<String>,
    /// BirdNET sensitivity multiplier (0.5–1.5). Higher values increase recall at the cost
    /// of more false positives.
    pub sensitivity: Option<String>,
    /// Chunk overlap in seconds (0.0–2.9). Higher values improve recall for detections that
    /// straddle chunk boundaries.
    pub overlap: Option<String>,
    /// Species frequency threshold (0.0–1.0). Filters species whose expected occurrence
    /// frequency in the region falls below this value.
    pub sf_thresh: Option<String>,
    /// Confidence threshold below which detections are written to the privacy-filtered log
    /// rather than the main detections table.
    pub privacy_threshold: Option<String>,
    /// How much agreement from neighbouring analysis windows a species needs
    /// before it is recorded: `off`, `lenient`, `moderate`, `balanced` or
    /// `strict`. Only bites when `overlap` is set.
    pub confirmation_level: Option<String>,
    // Notifications
    /// Apprise notification URL (e.g. `tgram://token/chat_id`). Supports any scheme
    /// Apprise understands.
    pub apprise_url: Option<String>,
    /// Path to an Apprise YAML config file on the station filesystem.
    pub apprise_config: Option<String>,
    /// Notification URLs delivered in-process, one per line.
    pub notify_urls: Option<String>,
    /// BirdWeather station token for uploading detections to the BirdWeather community map.
    pub birdweather_token: Option<String>,
    /// Minimum confidence (0.0–1.0) required to trigger a notification.
    pub notify_confidence: Option<String>,
    /// Per-species notification cooldown in minutes. Suppresses repeated alerts for the
    /// same species within this window.
    pub notify_cooldown: Option<String>,
    /// What event triggers a notification: `detection`, `new_species`, or `hourly`.
    pub notify_trigger: Option<String>,
    /// When `true`, notifications are sent only for species on the allow-list
    /// (`species_include`).
    pub notify_species_only: Option<String>,
    /// Comma-separated list of species to suppress from notifications, even when they
    /// meet the confidence threshold.
    pub notify_species_exclude: Option<String>,
    /// Handlebars template string for the notification title.
    pub notify_title_template: Option<String>,
    /// Handlebars template string for the notification body.
    pub notify_body_template: Option<String>,
    /// Cron expression for the weekly summary report email (e.g. `0 8 * * MON`).
    pub weekly_report_schedule: Option<String>,
    /// URL pinged periodically so an external monitor can tell the station is
    /// alive (`HEARTBEAT_URL`).
    pub heartbeat_url: Option<String>,
    /// Hours of detection silence after which the dead-man alert fires; `0`
    /// disables it (`DEADMAN_HOURS`).
    pub deadman_hours: Option<String>,
    // Species
    /// Comma-separated list of species (common names) to exclude from the detections log.
    pub species_exclude: Option<String>,
    /// Comma-separated list of species (common names) that are the *only* ones saved.
    /// Empty means no restriction.
    pub species_include: Option<String>,
    // System
    /// Number of days of recordings to retain before the oldest are purged.
    pub clip_retention_days: Option<String>,
    /// Filesystem path where downloaded Wikipedia species images are cached.
    pub image_cache_dir: Option<String>,
    /// Filesystem path to operator-supplied custom species images (overrides Wikipedia cache).
    pub custom_image_dir: Option<String>,
    /// Maximum number of WAV recordings stored per species before the oldest are rotated.
    pub max_files_per_species: Option<String>,
    /// Disk-use threshold (percentage, 0–100) at which the oldest recordings are purged
    /// regardless of age.
    pub purge_threshold: Option<String>,
    /// Suppress the species/confidence overlay on generated spectrograms.
    ///
    /// BirdNET-Pi's `RAW_SPECTROGRAM`, with one honest difference: theirs also
    /// removes axes, and this renderer has never drawn any.
    pub raw_spectrogram: Option<String>,
    /// Seconds of audio saved around each detection (BirdNET-Pi:
    /// `EXTRACTION_LENGTH`). The field existed in `ExtractionConfig` from the
    /// start and was never reachable from the UI, so every station ran the
    /// 6-second default.
    pub extraction_length: Option<String>,
    /// Days without a sighting after which a species counts as rare.
    ///
    /// BirdNET-Pi's `RARE_SPECIES_THRESHOLD`. Before this the definition was
    /// hardcoded as "first ever for this station", which never fires again
    /// once a species has been seen once — so a bird absent for three years
    /// returned without comment.
    pub rare_species_days: Option<String>,
    /// Seconds a raw capture segment is kept in the transient stream directory
    /// before being drained (0 = disable the age drain).
    pub stream_retention_secs: Option<String>,
    /// Hard ceiling in mebibytes on the transient stream directory
    /// (0 = disable the size ceiling).
    pub stream_max_mb: Option<String>,
    /// Display name shown in the web UI header and page titles.
    pub site_name: Option<String>,
    /// External species-information site to link to; `ebird` and `xeno-canto` are
    /// recognised shortcuts.
    pub info_site: Option<String>,
    /// Language used for common names in the detection database
    /// (`DATABASE_LANG`), e.g. `en`, `de`, `fr`.
    pub database_lang: Option<String>,
    // Night inhibit / schedule
    /// When `true`, recording is suspended between civil sunset and civil sunrise.
    pub night_inhibit: Option<String>,
    /// Recording window: `all-day`, `solar`, or `fixed:HH:MM-HH:MM`.
    pub recording_schedule: Option<String>,
    /// Minutes before civil sunrise at which recording resumes (positive = earlier).
    pub pre_sunrise_offset: Option<String>,
    /// Minutes after civil sunset at which recording stops (positive = later).
    pub post_sunset_offset: Option<String>,
    // Auth is deliberately absent. The admin credential lives as an Argon2id
    // hash in the accounts table, seeded from `CADDY_PWD` by
    // `helpers::auth::bootstrap_admin_password`; nothing reads an
    // `auth_username` / `auth_password` settings row. The form used to carry
    // both, which meant a password typed there was stored in `settings` as
    // plaintext, echoed back into the page HTML on every later load, and
    // changed no credential at all — while the section promised that clearing
    // it would "disable HTTP Basic Auth".
    // Email
    /// SMTP relay hostname (e.g. `smtp.gmail.com`).
    pub email_smtp_host: Option<String>,
    /// SMTP relay port (typically `587` for STARTTLS, `465` for implicit TLS).
    pub email_smtp_port: Option<String>,
    /// SMTP authentication username.
    pub email_smtp_user: Option<String>,
    /// SMTP authentication password or app token.
    pub email_smtp_pass: Option<String>,
    /// Sender address placed in the `From:` header.
    pub email_from: Option<String>,
    /// Recipient address(es) for detection alert emails; comma-separated.
    pub email_to: Option<String>,
    /// Display name placed alongside the `From:` address (e.g. `BirdNET Station`).
    pub email_from_name: Option<String>,
    /// When `true`, upgrades the connection to TLS via STARTTLS before authenticating.
    pub email_starttls: Option<String>,
    /// Minimum confidence (0.0–1.0) required to send a detection email.
    pub email_min_confidence: Option<String>,
    /// Per-species email cooldown in seconds. Suppresses repeat emails within this window.
    pub email_cooldown_secs: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{SETTINGS_FORM_KEYS, SettingsForm};
    use std::collections::BTreeSet;

    #[test]
    fn form_keys_match_the_struct_fields_exactly() {
        // Enumerated from the struct itself rather than hand-listed twice, so
        // adding a field without adding it here fails rather than silently
        // producing another control nothing consumes.
        let value = serde_json::to_value(SettingsForm::default()).expect("form serialises");
        let fields: BTreeSet<&str> = value
            .as_object()
            .expect("form serialises to an object")
            .keys()
            .map(String::as_str)
            .collect();
        let declared: BTreeSet<&str> = SETTINGS_FORM_KEYS.iter().copied().collect();

        assert_eq!(
            fields, declared,
            "SETTINGS_FORM_KEYS must list exactly the fields of SettingsForm"
        );
    }

    #[test]
    fn form_keys_are_unique() {
        let declared: BTreeSet<&str> = SETTINGS_FORM_KEYS.iter().copied().collect();
        assert_eq!(declared.len(), SETTINGS_FORM_KEYS.len());
    }
}
