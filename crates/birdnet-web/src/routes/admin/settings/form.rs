//! Settings form deserialization types.

use serde::Deserialize;

/// Flat form payload from the admin settings POST.
///
/// Every field is `Option` because HTMX partial-save may only submit
/// a single tab's fields.
#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    // Audio
    pub alsa_device: Option<String>,
    pub rtsp_url: Option<String>,
    pub rtsp_urls: Option<String>,
    pub segment_duration: Option<String>,
    pub audio_channels: Option<String>,
    pub audio_format: Option<String>,
    pub freq_shift_hz: Option<String>,
    // Location
    pub latitude: Option<String>,
    pub longitude: Option<String>,
    pub station_name: Option<String>,
    // Detection
    pub confidence_threshold: Option<String>,
    pub sensitivity: Option<String>,
    pub overlap: Option<String>,
    pub sf_thresh: Option<String>,
    pub privacy_threshold: Option<String>,
    // Notifications
    pub apprise_url: Option<String>,
    pub apprise_config: Option<String>,
    pub birdweather_token: Option<String>,
    pub notify_confidence: Option<String>,
    pub notify_cooldown: Option<String>,
    pub notify_trigger: Option<String>,
    pub notify_species_only: Option<String>,
    pub notify_species_exclude: Option<String>,
    pub notify_title_template: Option<String>,
    pub notify_body_template: Option<String>,
    pub notify_image: Option<String>,
    pub weekly_report_schedule: Option<String>,
    // Species
    pub species_exclude: Option<String>,
    pub species_include: Option<String>,
    // System
    pub recording_days: Option<String>,
    pub image_cache_dir: Option<String>,
    pub custom_image_dir: Option<String>,
    pub max_files_per_species: Option<String>,
    pub purge_threshold: Option<String>,
    pub site_name: Option<String>,
    pub info_site: Option<String>,
    // Night inhibit / schedule
    pub night_inhibit: Option<String>,
    pub pre_sunrise_offset: Option<String>,
    pub post_sunset_offset: Option<String>,
    // Auth
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    // Email
    pub email_smtp_host: Option<String>,
    pub email_smtp_port: Option<String>,
    pub email_smtp_user: Option<String>,
    pub email_smtp_pass: Option<String>,
    pub email_from: Option<String>,
    pub email_to: Option<String>,
    pub email_from_name: Option<String>,
    pub email_starttls: Option<String>,
    pub email_min_confidence: Option<String>,
    pub email_cooldown_secs: Option<String>,
}
