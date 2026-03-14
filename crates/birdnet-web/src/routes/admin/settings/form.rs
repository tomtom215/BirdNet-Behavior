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
    pub segment_duration: Option<String>,
    // Location
    pub latitude: Option<String>,
    pub longitude: Option<String>,
    pub station_name: Option<String>,
    // Detection
    pub confidence_threshold: Option<String>,
    pub sensitivity: Option<String>,
    pub overlap: Option<String>,
    // Notifications
    pub apprise_url: Option<String>,
    pub birdweather_token: Option<String>,
    pub notify_confidence: Option<String>,
    pub notify_cooldown: Option<String>,
    // Species
    pub species_exclude: Option<String>,
    pub species_include: Option<String>,
    // System
    pub recording_days: Option<String>,
    pub image_cache_dir: Option<String>,
    // Night inhibit / schedule
    pub night_inhibit: Option<String>,
    pub pre_sunrise_offset: Option<String>,
    pub post_sunset_offset: Option<String>,
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
