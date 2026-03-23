//! External service integrations for BirdNET-Pi.
//!
//! Provides clients for `BirdWeather`, Apprise notifications,
//! species image caching (Flickr/Wikipedia), SMTP email alerts,
//! heartbeat monitoring, notification templates, weekly reports, and
//! a lightweight MQTT publisher for IoT/Home Assistant integration.

pub mod apprise;
pub mod auto_update;
pub mod birdweather;
pub mod email;
pub mod heartbeat;
pub mod mqtt;
pub mod notification;
pub mod species_images;
pub mod weekly_report;
