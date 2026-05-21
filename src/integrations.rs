//! Integration client construction helpers.
//!
//! Creates `Apprise`, `BirdWeather`, email, heartbeat, MQTT, notification,
//! and HTTP-auth clients from CLI flags and/or config file values, each
//! returning `None` when the integration is not configured. One submodule
//! per integration keeps each builder — and its `CLI`-vs-config precedence
//! tests — focused; this module is a thin facade that re-exports the public
//! surface so callers use `integrations::create_*` unchanged.

mod apprise;
mod auth;
mod birdweather;
mod email;
mod heartbeat;
mod mqtt;
mod notification;

#[cfg(test)]
mod test_support;

pub use apprise::{AppriseHandle, create_apprise_client};
pub use auth::create_auth_config;
pub use birdweather::create_birdweather_client;
pub use email::{EmailHandle, create_email_notifier};
pub use heartbeat::{HeartbeatHandle, create_heartbeat_client};
pub use mqtt::{MqttHandle, create_mqtt_client, get_mqtt_client_ref, publish_ha_discovery};
pub use notification::{create_notification_filter, create_notification_template};
