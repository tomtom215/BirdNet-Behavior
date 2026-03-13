//! External service integrations for BirdNET-Pi.
//!
//! Provides clients for `BirdWeather`, Apprise notifications,
//! species image caching (Flickr/Wikipedia), and SMTP email alerts.

pub mod apprise;
pub mod birdweather;
pub mod email;
pub mod species_images;
