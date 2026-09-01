//! External service integrations for BirdNET-Pi.
//!
//! Provides clients for `BirdWeather`, Apprise notifications,
//! species image caching (Flickr/Wikipedia), SMTP email alerts,
//! heartbeat monitoring, notification templates, weekly reports, and
//! a lightweight MQTT publisher for IoT/Home Assistant integration, and
//! in-process delivery to Discord/Slack/Telegram/ntfy/Gotify/Pushover.

pub mod apprise;
pub mod auto_update;
pub mod birdweather;
pub mod dispatch;
pub mod email;
pub mod heartbeat;
pub mod mqtt;
pub mod notification;
/// Shared retry backoff with jitter, used by the HTTP integration clients.
mod retry;
pub mod species_images;
pub mod weather;
pub mod webhook;
pub mod weekly_report;
