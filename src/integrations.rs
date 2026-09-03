//! Integration client construction helpers.
//!
//! Creates `Apprise`, `BirdWeather`, email, heartbeat, MQTT, and
//! notification clients from CLI flags and/or config file values, each
//! returning `None` when the integration is not configured. One submodule
//! per integration keeps each builder — and its `CLI`-vs-config precedence
//! tests — focused; this module is a thin facade that re-exports the public
//! surface so callers use `integrations::create_*` unchanged.

mod acoustic_health;
mod apprise;
mod birdweather;
mod deadman;
mod effort;
mod email;
mod heartbeat;
mod mqtt;
mod notification;
mod station_health;
mod store_forward;
mod weather;

#[cfg(test)]
mod test_support;

pub use acoustic_health::spawn_acoustic_health;
pub use apprise::{AppriseHandle, create_apprise_client};
pub use birdweather::create_birdweather_client;
pub use deadman::{DEFAULT_DEADMAN_HOURS, spawn_detection_deadman};
pub use effort::spawn_effort_recorder;
pub use email::{EmailHandle, create_email_notifier};
pub use heartbeat::{create_heartbeat_client, spawn_heartbeat};
pub use mqtt::{MqttHandle, create_mqtt_client, get_mqtt_client_ref, publish_ha_discovery};
pub use notification::{create_notification_filter, create_notification_template};
pub use station_health::spawn_station_health;
pub use store_forward::spawn_birdweather_drainer;
pub use store_forward::unix_now_secs;
pub use weather::spawn_weather_poll;
