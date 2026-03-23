//! Home Assistant MQTT auto-discovery support.
//!
//! Publishes MQTT discovery messages so that Home Assistant automatically
//! creates sensors, binary sensors, and device trackers without manual
//! `configuration.yaml` entries.
//!
//! ## Protocol
//!
//! Home Assistant's MQTT discovery uses a well-known topic prefix
//! (default: `homeassistant`) followed by the component type, a unique
//! identifier, and `config`:
//!
//! ```text
//! homeassistant/<component>/<unique_id>/config
//! ```
//!
//! Publishing an empty payload to the config topic removes the entity.
//!
//! ## Entities registered
//!
//! | Entity | Type | Topic |
//! |--------|------|-------|
//! | Last detected species | `sensor` | `birdnet/detection/#` |
//! | Detection confidence | `sensor` | `birdnet/detection/#` |
//! | Station status | `binary_sensor` | `birdnet/status` |
//! | Total detections today | `sensor` | `birdnet/stats/today` |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use birdnet_integrations::mqtt::{MqttClient, MqttConfig};
//! use birdnet_integrations::mqtt::discovery::{HaDiscovery, HaDiscoveryConfig};
//!
//! let mqtt_config = MqttConfig::default();
//! let client = MqttClient::new(mqtt_config.clone());
//! let discovery = HaDiscovery::new(
//!     mqtt_config,
//!     HaDiscoveryConfig {
//!         station_name: "Garden Station".to_string(),
//!         ..HaDiscoveryConfig::default()
//!     },
//! );
//! discovery.publish_all().unwrap();
//! ```
//!
//! ## Reference
//!
//! <https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery>

use super::publisher::publish;
use super::types::{MqttConfig, MqttError};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Home Assistant discovery configuration.
#[derive(Debug, Clone)]
pub struct HaDiscoveryConfig {
    /// Discovery topic prefix (default: `homeassistant`).
    pub discovery_prefix: String,
    /// Human-readable station name shown in the HA UI.
    pub station_name: String,
    /// Unique device identifier (must be stable across restarts).
    ///
    /// Defaults to `birdnet_behavior`.
    pub device_id: String,
    /// Software version string shown in the HA device info panel.
    pub sw_version: String,
}

impl Default for HaDiscoveryConfig {
    fn default() -> Self {
        Self {
            discovery_prefix: "homeassistant".to_string(),
            station_name: "BirdNet-Behavior".to_string(),
            device_id: "birdnet_behavior".to_string(),
            sw_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// HaDiscovery
// ---------------------------------------------------------------------------

/// Publishes Home Assistant MQTT auto-discovery payloads.
#[derive(Debug, Clone)]
pub struct HaDiscovery {
    mqtt: MqttConfig,
    ha: HaDiscoveryConfig,
}

impl HaDiscovery {
    /// Create a new discovery publisher.
    #[must_use]
    pub const fn new(mqtt: MqttConfig, ha: HaDiscoveryConfig) -> Self {
        Self { mqtt, ha }
    }

    /// Publish all discovery messages to the MQTT broker.
    ///
    /// Call this once at startup (or after reconnect) to register all entities.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if any message fails to publish.
    pub fn publish_all(&self) -> Result<(), MqttError> {
        self.publish_last_species()?;
        self.publish_confidence()?;
        self.publish_station_status()?;
        self.publish_detections_today()?;
        Ok(())
    }

    /// Remove all discovery entries from Home Assistant.
    ///
    /// Sends an empty payload to each config topic, which causes HA to
    /// remove the corresponding entities.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if any message fails to publish.
    pub fn remove_all(&self) -> Result<(), MqttError> {
        for topic in self.all_config_topics() {
            publish(&self.mqtt, &topic, &[])?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Per-entity publishers
    // -----------------------------------------------------------------------

    /// Publish discovery for the "last detected species" sensor.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] on publish failure.
    fn publish_last_species(&self) -> Result<(), MqttError> {
        let unique_id = format!("{}_last_species", self.ha.device_id);
        let state_topic = format!("{}/detection/#", self.mqtt.topic_prefix);
        let payload = self.sensor_payload(
            &unique_id,
            "Last Detected Bird",
            &state_topic,
            "{{ value_json.common_name }}",
            Some("mdi:bird"),
            None,
        );
        let topic = self.config_topic("sensor", &unique_id);
        publish(&self.mqtt, &topic, payload.as_bytes())
    }

    /// Publish discovery for the detection confidence sensor.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] on publish failure.
    fn publish_confidence(&self) -> Result<(), MqttError> {
        let unique_id = format!("{}_confidence", self.ha.device_id);
        let state_topic = format!("{}/detection/#", self.mqtt.topic_prefix);
        let payload = self.sensor_payload(
            &unique_id,
            "Detection Confidence",
            &state_topic,
            "{{ value_json.confidence_pct }}",
            Some("mdi:percent"),
            Some("%"),
        );
        let topic = self.config_topic("sensor", &unique_id);
        publish(&self.mqtt, &topic, payload.as_bytes())
    }

    /// Publish discovery for the station online/offline binary sensor.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] on publish failure.
    fn publish_station_status(&self) -> Result<(), MqttError> {
        let unique_id = format!("{}_status", self.ha.device_id);
        let state_topic = self.mqtt.status_topic();

        // HA binary_sensor: payload_on = "online", payload_off = "offline".
        let payload = format!(
            r#"{{
  "name": "{station} Status",
  "unique_id": "{uid}",
  "state_topic": "{st}",
  "payload_on": "online",
  "payload_off": "offline",
  "device_class": "connectivity",
  "icon": "mdi:radio-tower",
  "device": {device}
}}"#,
            station = esc_json(&self.ha.station_name),
            uid = esc_json(&unique_id),
            st = esc_json(&state_topic),
            device = self.device_block(),
        );
        let topic = self.config_topic("binary_sensor", &unique_id);
        publish(&self.mqtt, &topic, payload.as_bytes())
    }

    /// Publish discovery for the "detections today" count sensor.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] on publish failure.
    fn publish_detections_today(&self) -> Result<(), MqttError> {
        let unique_id = format!("{}_detections_today", self.ha.device_id);
        let state_topic = format!("{}/stats/today", self.mqtt.topic_prefix);
        let payload = self.sensor_payload(
            &unique_id,
            "Detections Today",
            &state_topic,
            "{{ value_json.count }}",
            Some("mdi:counter"),
            None,
        );
        let topic = self.config_topic("sensor", &unique_id);
        publish(&self.mqtt, &topic, payload.as_bytes())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a sensor discovery JSON payload.
    fn sensor_payload(
        &self,
        unique_id: &str,
        name: &str,
        state_topic: &str,
        value_template: &str,
        icon: Option<&str>,
        unit: Option<&str>,
    ) -> String {
        let icon_field = icon
            .map(|i| format!(",\n  \"icon\": \"{i}\""))
            .unwrap_or_default();
        let unit_field = unit
            .map(|u| format!(",\n  \"unit_of_measurement\": \"{u}\""))
            .unwrap_or_default();

        format!(
            r#"{{
  "name": "{name}",
  "unique_id": "{uid}",
  "state_topic": "{st}",
  "value_template": "{vt}"{icon}{unit},
  "device": {device}
}}"#,
            name = esc_json(name),
            uid = esc_json(unique_id),
            st = esc_json(state_topic),
            vt = esc_json(value_template),
            icon = icon_field,
            unit = unit_field,
            device = self.device_block(),
        )
    }

    /// Build the HA device info block (shared by all entities).
    fn device_block(&self) -> String {
        format!(
            r#"{{
    "identifiers": ["{}"],
    "name": "{}",
    "model": "BirdNet-Behavior",
    "manufacturer": "tomtom215",
    "sw_version": "{}"
  }}"#,
            esc_json(&self.ha.device_id),
            esc_json(&self.ha.station_name),
            esc_json(&self.ha.sw_version),
        )
    }

    /// Build the full config topic for a given component and unique ID.
    fn config_topic(&self, component: &str, unique_id: &str) -> String {
        format!(
            "{}/{}/{}/config",
            self.ha.discovery_prefix, component, unique_id
        )
    }

    /// Return all config topics (used for bulk removal).
    fn all_config_topics(&self) -> Vec<String> {
        [
            ("sensor", format!("{}_last_species", self.ha.device_id)),
            ("sensor", format!("{}_confidence", self.ha.device_id)),
            ("binary_sensor", format!("{}_status", self.ha.device_id)),
            ("sensor", format!("{}_detections_today", self.ha.device_id)),
        ]
        .iter()
        .map(|(comp, uid)| self.config_topic(comp, uid))
        .collect()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal JSON string escaping (double-quotes and backslashes only).
///
/// This is sufficient for the controlled strings used in discovery payloads.
fn esc_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn discovery() -> HaDiscovery {
        HaDiscovery::new(MqttConfig::default(), HaDiscoveryConfig::default())
    }

    #[test]
    fn config_topic_format() {
        let d = discovery();
        assert_eq!(
            d.config_topic("sensor", "birdnet_behavior_confidence"),
            "homeassistant/sensor/birdnet_behavior_confidence/config"
        );
    }

    #[test]
    fn all_config_topics_returns_four() {
        let d = discovery();
        assert_eq!(d.all_config_topics().len(), 4);
    }

    #[test]
    fn sensor_payload_contains_name() {
        let d = discovery();
        let payload = d.sensor_payload(
            "uid_test",
            "Test Sensor",
            "test/topic",
            "{{ value }}",
            None,
            None,
        );
        assert!(payload.contains("Test Sensor"));
        assert!(payload.contains("uid_test"));
        assert!(payload.contains("test/topic"));
    }

    #[test]
    fn esc_json_escapes_quotes() {
        assert_eq!(esc_json(r#"say "hello""#), r#"say \"hello\""#);
        assert_eq!(esc_json(r"a\b"), r"a\\b");
        assert_eq!(esc_json("clean"), "clean");
    }

    #[test]
    fn device_block_contains_station_name() {
        let ha = HaDiscoveryConfig {
            station_name: "My Garden".to_string(),
            ..HaDiscoveryConfig::default()
        };
        let d = HaDiscovery::new(MqttConfig::default(), ha);
        assert!(d.device_block().contains("My Garden"));
    }

    #[test]
    fn publish_to_offline_broker_returns_error() {
        let mqtt = MqttConfig {
            host: "127.0.0.1".to_string(),
            port: 19_998,
            timeout_ms: 200,
            ..MqttConfig::default()
        };
        let d = HaDiscovery::new(mqtt, HaDiscoveryConfig::default());
        assert!(d.publish_all().is_err());
    }
}
