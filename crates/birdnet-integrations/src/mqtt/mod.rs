//! MQTT 3.1.1 integration for broadcasting bird detection events.
//!
//! Publishes bird detection events to an MQTT broker using a pure-Rust,
//! stateless MQTT 3.1.1 publisher.  No external MQTT library is required;
//! the wire protocol is implemented directly over `std::net::TcpStream`.
//!
//! ## Typical use
//!
//! ```rust,no_run
//! use birdnet_integrations::mqtt::{MqttClient, MqttConfig, DetectionPayload};
//!
//! let config = MqttConfig {
//!     host: "mqtt.local".to_string(),
//!     topic_prefix: "garden".to_string(),
//!     ..MqttConfig::default()
//! };
//! let client = MqttClient::new(config);
//!
//! let payload = DetectionPayload {
//!     timestamp: "2026-03-23T06:30:00".to_string(),
//!     scientific_name: "Turdus merula".to_string(),
//!     common_name: "Eurasian Blackbird".to_string(),
//!     confidence: 0.87,
//!     confidence_pct: 87,
//!     file_name: None,
//!     rtsp_id: None,
//! };
//! client.publish_detection(&payload).unwrap();
//! ```
//!
//! ## Home Assistant auto-discovery
//!
//! Configure the sensor in Home Assistant `configuration.yaml`:
//!
//! ```yaml
//! mqtt:
//!   sensor:
//!     - name: "Last Bird Detection"
//!       state_topic: "birdnet/detection/#"
//!       value_template: "{{ value_json.common_name }}"
//! ```

pub mod discovery;
pub mod publisher;
pub mod types;

pub use discovery::{HaDiscovery, HaDiscoveryConfig};
pub use publisher::{
    KEEPALIVE_SECS, PRESENCE_OFFLINE, PRESENCE_ONLINE, PresenceSession, publish, publish_with,
};
pub use types::{ConnAckError, DetectionPayload, MqttConfig, MqttError, QosLevel, TlsConfig, Will};

/// High-level MQTT client for publishing bird detection events.
///
/// Wraps the stateless [`publish`] function with detection-specific
/// topic construction and JSON serialisation.
#[derive(Debug, Clone)]
pub struct MqttClient {
    config: MqttConfig,
}

impl MqttClient {
    /// Create a new client with the given configuration.
    #[must_use]
    pub const fn new(config: MqttConfig) -> Self {
        Self { config }
    }

    /// Publish a bird detection event to the broker.
    ///
    /// Topic: `{prefix}/detection/{species_safe}` where `species_safe`
    /// replaces spaces with underscores.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if the connection or publish fails.
    pub fn publish_detection(&self, payload: &DetectionPayload) -> Result<(), MqttError> {
        let species_safe = payload.common_name.replace(' ', "_");
        let topic = self.config.detection_topic(&species_safe);
        let json =
            serde_json::to_string(payload).map_err(|e| MqttError::Serialise(e.to_string()))?;
        publish(&self.config, &topic, json.as_bytes())
    }

    // `publish_status` and `publish_daily_stats` used to live here. Both had
    // zero callers for the life of the project, which is why the two Home
    // Assistant entities they fed were permanently `unknown`.
    //
    // They are not restored, they are replaced. Both topics are now owned by
    // the presence session, which publishes them *retained* and at `QoS` 1 on
    // a connection that carries a last will. A stateless `publish_status`
    // beside that would be a second writer to `{prefix}/status` with a
    // different retain flag — the one arrangement in which Home Assistant
    // shows a live station as offline — so leaving it available was a trap
    // rather than a convenience.

    /// Return a reference to the underlying configuration.
    #[must_use]
    pub const fn config(&self) -> &MqttConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_topic_construction() {
        let config = MqttConfig {
            topic_prefix: "garden".to_string(),
            ..MqttConfig::default()
        };
        let client = MqttClient::new(config);
        assert_eq!(
            client.config().detection_topic("Eurasian_Blackbird"),
            "garden/detection/Eurasian_Blackbird"
        );
        assert_eq!(client.config().status_topic(), "garden/status");
    }

    #[test]
    fn detection_payload_topic_spaces() {
        let config = MqttConfig::default();
        let client = MqttClient::new(config);
        let payload = DetectionPayload {
            timestamp: "2026-03-23T06:30:00".to_string(),
            scientific_name: "Turdus merula".to_string(),
            common_name: "Eurasian Blackbird".to_string(),
            confidence: 0.87,
            confidence_pct: 87,
            file_name: None,
            rtsp_id: None,
        };
        // Verify topic uses underscores (not spaces)
        let species_safe = payload.common_name.replace(' ', "_");
        let expected_topic = client.config().detection_topic(&species_safe);
        assert_eq!(expected_topic, "birdnet/detection/Eurasian_Blackbird");
    }

    #[test]
    fn publish_to_offline_broker_returns_error() {
        // Connect to a port that should not have an MQTT broker
        let config = MqttConfig {
            host: "127.0.0.1".to_string(),
            port: 19_999,
            timeout_ms: 200,
            ..MqttConfig::default()
        };
        let client = MqttClient::new(config);
        let payload = DetectionPayload {
            timestamp: "2026-03-23T06:30:00".to_string(),
            scientific_name: "Turdus merula".to_string(),
            common_name: "Eurasian Blackbird".to_string(),
            confidence: 0.87,
            confidence_pct: 87,
            file_name: None,
            rtsp_id: None,
        };
        let result = client.publish_detection(&payload);
        assert!(matches!(result, Err(MqttError::Connection(_))));
    }
}
