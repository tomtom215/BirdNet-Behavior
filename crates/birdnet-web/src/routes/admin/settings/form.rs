//! Settings form deserialization types.

use serde::Deserialize;

/// Flat form payload from the admin settings POST.
///
/// Every field is `Option` because HTMX partial-save may only submit
/// a single tab's fields.
#[derive(Debug, Deserialize)]
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
    /// Channel selection: `mono`, `left`, `right`, or `stereo`.
    pub audio_channels: Option<String>,
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
    // Notifications
    /// Apprise notification URL (e.g. `tgram://token/chat_id`). Supports any scheme
    /// Apprise understands.
    pub apprise_url: Option<String>,
    /// Path to an Apprise YAML config file on the station filesystem.
    pub apprise_config: Option<String>,
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
    /// Controls whether species images are attached to notifications (`true`/`false`).
    pub notify_image: Option<String>,
    /// Cron expression for the weekly summary report email (e.g. `0 8 * * MON`).
    pub weekly_report_schedule: Option<String>,
    // Species
    /// Comma-separated list of species (common names) to exclude from the detections log.
    pub species_exclude: Option<String>,
    /// Comma-separated list of species (common names) that are the *only* ones saved.
    /// Empty means no restriction.
    pub species_include: Option<String>,
    // System
    /// Number of days of recordings to retain before the oldest are purged.
    pub recording_days: Option<String>,
    /// Filesystem path where downloaded Wikipedia species images are cached.
    pub image_cache_dir: Option<String>,
    /// Filesystem path to operator-supplied custom species images (overrides Wikipedia cache).
    pub custom_image_dir: Option<String>,
    /// Maximum number of WAV recordings stored per species before the oldest are rotated.
    pub max_files_per_species: Option<String>,
    /// Disk-use threshold (percentage, 0–100) at which the oldest recordings are purged
    /// regardless of age.
    pub purge_threshold: Option<String>,
    /// Display name shown in the web UI header and page titles.
    pub site_name: Option<String>,
    /// External species-information site to link to; `ebird` and `xeno-canto` are
    /// recognised shortcuts.
    pub info_site: Option<String>,
    // Night inhibit / schedule
    /// When `true`, recording is suspended between civil sunset and civil sunrise.
    pub night_inhibit: Option<String>,
    /// Minutes before civil sunrise at which recording resumes (positive = earlier).
    pub pre_sunrise_offset: Option<String>,
    /// Minutes after civil sunset at which recording stops (positive = later).
    pub post_sunset_offset: Option<String>,
    // Auth
    /// Admin username for the web UI login (used by the O-15 accounts wire).
    pub auth_username: Option<String>,
    /// Admin password (stored as Argon2id hash; plain-text only during initial set).
    pub auth_password: Option<String>,
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
