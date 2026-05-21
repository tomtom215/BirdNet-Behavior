//! MQTT client construction and Home Assistant auto-discovery.

use std::sync::Arc;

use crate::cli::Cli;

/// Type alias for the shared MQTT client handle.
pub type MqttHandle = Arc<birdnet_integrations::mqtt::MqttClient>;

/// Create an MQTT client from CLI flags and/or config file values.
///
/// Returns `None` if no MQTT broker host is configured.
pub fn create_mqtt_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<MqttHandle> {
    let host = cli
        .mqtt_host
        .clone()
        .or_else(|| config?.get("MQTT_HOST").map(String::from))?;

    let port = config
        .and_then(|c| c.get_parsed::<u16>("MQTT_PORT").ok())
        .unwrap_or(cli.mqtt_port);

    let username = cli
        .mqtt_username
        .clone()
        .or_else(|| config?.get("MQTT_USERNAME").map(String::from));

    let password = cli
        .mqtt_password
        .clone()
        .or_else(|| config?.get("MQTT_PASSWORD").map(String::from));

    let topic_prefix = config
        .and_then(|c| c.get("MQTT_TOPIC_PREFIX"))
        .map_or_else(|| cli.mqtt_topic_prefix.clone(), String::from);

    let retain = cli.mqtt_retain
        || config
            .and_then(|c| c.get_parsed::<bool>("MQTT_RETAIN").ok())
            .unwrap_or(false);

    let cfg = birdnet_integrations::mqtt::MqttConfig {
        host: host.clone(),
        port,
        client_id: cli.mqtt_client_id.clone(),
        username,
        password,
        topic_prefix,
        qos: birdnet_integrations::mqtt::QosLevel::AtMostOnce,
        retain,
        timeout_ms: 5_000,
    };

    tracing::info!(
        host = %host,
        port,
        topic_prefix = %cfg.topic_prefix,
        "MQTT integration enabled"
    );

    Some(Arc::new(birdnet_integrations::mqtt::MqttClient::new(cfg)))
}

/// Return a cloned `MqttClient` when MQTT is configured (used by HA discovery).
///
/// This is a lightweight helper used at startup only.
pub fn get_mqtt_client_ref(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_integrations::mqtt::MqttClient> {
    let host = cli
        .mqtt_host
        .clone()
        .or_else(|| config?.get("MQTT_HOST").map(String::from))?;

    let port = config
        .and_then(|c| c.get_parsed::<u16>("MQTT_PORT").ok())
        .unwrap_or(cli.mqtt_port);

    let username = cli
        .mqtt_username
        .clone()
        .or_else(|| config?.get("MQTT_USERNAME").map(String::from));

    let password = cli
        .mqtt_password
        .clone()
        .or_else(|| config?.get("MQTT_PASSWORD").map(String::from));

    let topic_prefix = config
        .and_then(|c| c.get("MQTT_TOPIC_PREFIX"))
        .map_or_else(|| cli.mqtt_topic_prefix.clone(), String::from);

    let retain = cli.mqtt_retain
        || config
            .and_then(|c| c.get_parsed::<bool>("MQTT_RETAIN").ok())
            .unwrap_or(false);

    Some(birdnet_integrations::mqtt::MqttClient::new(
        birdnet_integrations::mqtt::MqttConfig {
            host,
            port,
            client_id: cli.mqtt_client_id.clone(),
            username,
            password,
            topic_prefix,
            qos: birdnet_integrations::mqtt::QosLevel::AtMostOnce,
            retain,
            timeout_ms: 5_000,
        },
    ))
}

/// Publish Home Assistant MQTT auto-discovery messages if enabled.
///
/// Reads the station name from CLI / config and publishes four entities:
/// last-species sensor, confidence sensor, connectivity binary sensor,
/// and detections-today sensor.  Failures are logged as warnings (non-fatal).
pub fn publish_ha_discovery(
    client: &birdnet_integrations::mqtt::MqttClient,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) {
    if !cli.mqtt_ha_discovery {
        return;
    }

    let station_name = cli
        .site_name
        .clone()
        .or_else(|| config?.get("STATION_NAME").map(String::from))
        .unwrap_or_else(|| "BirdNet-Behavior".to_string());

    let discovery = birdnet_integrations::mqtt::HaDiscovery::new(
        client.config().clone(),
        birdnet_integrations::mqtt::HaDiscoveryConfig {
            station_name: station_name.clone(),
            ..birdnet_integrations::mqtt::HaDiscoveryConfig::default()
        },
    );

    match discovery.publish_all() {
        Ok(()) => tracing::info!(
            station = %station_name,
            "Home Assistant MQTT auto-discovery published"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "Home Assistant MQTT auto-discovery failed (broker may be offline)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{create_mqtt_client, get_mqtt_client_ref};
    use crate::integrations::test_support::{config_with, default_cli};

    #[test]
    fn mqtt_none_when_no_host_configured() {
        let cli = default_cli();
        assert!(create_mqtt_client(&cli, None).is_none());
        assert!(get_mqtt_client_ref(&cli, None).is_none());
    }

    #[test]
    fn mqtt_built_from_cli_host() {
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        assert!(create_mqtt_client(&cli, None).is_some());
        assert!(get_mqtt_client_ref(&cli, None).is_some());
    }

    #[test]
    fn mqtt_built_from_config_host() {
        let cli = default_cli();
        let cfg = config_with(&[("MQTT_HOST", "mqtt.config")]);
        assert!(create_mqtt_client(&cli, Some(&cfg)).is_some());
    }

    #[test]
    fn mqtt_picks_config_port_over_cli_when_present() {
        // CLI default is 1883; config overrides.
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        let cfg = config_with(&[("MQTT_PORT", "8883")]);
        let handle = create_mqtt_client(&cli, Some(&cfg)).expect("client built");
        assert_eq!(handle.config().port, 8883);
    }

    #[test]
    fn mqtt_uses_config_topic_prefix_when_set() {
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        let cfg = config_with(&[("MQTT_TOPIC_PREFIX", "stations/garden")]);
        let handle = create_mqtt_client(&cli, Some(&cfg)).expect("client built");
        assert_eq!(handle.config().topic_prefix, "stations/garden");
    }

    #[test]
    fn mqtt_retain_set_when_cli_flag_or_config_truthy() {
        // CLI flag wins.
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        cli.mqtt_retain = true;
        let handle = create_mqtt_client(&cli, None).expect("client built");
        assert!(handle.config().retain);

        // Config-only path: CLI flag false but config sets it true.
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        let cfg = config_with(&[("MQTT_RETAIN", "true")]);
        let handle = create_mqtt_client(&cli, Some(&cfg)).expect("client built");
        assert!(handle.config().retain);
    }
}
