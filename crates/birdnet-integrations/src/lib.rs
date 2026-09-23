//! External service integrations for BirdNET-Pi.
//!
//! Provides clients for `BirdWeather`, Apprise notifications,
//! species image caching (Flickr/Wikipedia), SMTP email alerts,
//! heartbeat monitoring, notification templates, weekly reports,
//! eBird recent-observations corroboration, and
//! a lightweight MQTT publisher for IoT/Home Assistant integration, and
//! in-process delivery to Discord/Slack/Telegram/ntfy/Gotify/Pushover.

pub mod apprise;
pub mod auto_update;
pub mod birdweather;
pub mod dispatch;
pub mod ebird;
pub mod email;
pub mod heartbeat;
pub mod model_catalog;
pub mod mqtt;
pub mod notification;
pub mod offsite;
/// Shared retry backoff with jitter, used by the HTTP integration clients.
mod retry;
pub mod species_images;
pub mod weather;
pub mod webhook;
pub mod weekly_report;

/// A `reqwest::Error` as text, without the URL it carries.
///
/// `reqwest::Error`'s `Display` appends ` for url (<the whole URL>)`, and the
/// BirdWeather token, the heartbeat ping id, the Apprise API key and the
/// Flickr API key all live in that URL. Every error text built from one goes
/// through here, or through `without_url()` directly.
pub(crate) fn http_error_text(e: reqwest::Error) -> String {
    e.without_url().to_string()
}
