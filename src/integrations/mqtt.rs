//! MQTT client construction and Home Assistant auto-discovery.

use std::sync::Arc;

use crate::cli::Cli;

/// Default MQTT port for a plaintext connection.
const MQTT_PORT_PLAIN: u16 = 1883;

/// Default MQTT port for a TLS connection.
const MQTT_PORT_TLS: u16 = 8883;

/// Resolve the broker's TLS settings from flags and config.
///
/// `--mqtt-ca-file` or `MQTT_CA_FILE` implies TLS on its own: configuring a
/// trust anchor and then connecting in plaintext is never what was meant, and
/// the failure would be silent.
fn resolve_tls(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_integrations::mqtt::TlsConfig> {
    let ca_file = cli.mqtt_ca_file.clone().or_else(|| {
        config
            .and_then(|c| c.get("MQTT_CA_FILE"))
            .map(std::path::PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    });
    let enabled = cli.mqtt_tls
        || ca_file.is_some()
        || config
            .and_then(|c| c.get_parsed::<bool>("MQTT_TLS").ok())
            .unwrap_or(false);
    if !enabled {
        return None;
    }
    Some(birdnet_integrations::mqtt::TlsConfig {
        ca_file,
        server_name: cli.mqtt_tls_server_name.clone().or_else(|| {
            config
                .and_then(|c| c.get("MQTT_TLS_SERVER_NAME"))
                .map(String::from)
                .filter(|s| !s.is_empty())
        }),
    })
}

/// Resolve the broker port.
///
/// An operator who turns on TLS and does not also change the port almost
/// certainly meant 8883; leaving 1883 would connect to the broker's plaintext
/// listener and fail the handshake with something unhelpful.
fn resolve_port(cli: &Cli, config: Option<&birdnet_core::config::Config>, tls: bool) -> u16 {
    cli.mqtt_port
        .or_else(|| config.and_then(|c| c.get_parsed::<u16>("MQTT_PORT").ok()))
        .unwrap_or(if tls { MQTT_PORT_TLS } else { MQTT_PORT_PLAIN })
}

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

    let tls = resolve_tls(cli, config);
    let port = resolve_port(cli, config, tls.is_some());

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
        tls,
    };

    tracing::info!(
        host = %host,
        port,
        tls = cfg.tls.is_some(),
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

    let tls = resolve_tls(cli, config);
    let port = resolve_port(cli, config, tls.is_some());

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
            tls,
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

    // ── TLS resolution ──────────────────────────────────────────────────

    #[test]
    fn tls_is_off_unless_something_asks_for_it() {
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        let handle = create_mqtt_client(&cli, None).expect("client built");
        assert!(handle.config().tls.is_none());
        assert_eq!(handle.config().port, 1883);
    }

    #[test]
    fn turning_on_tls_moves_the_default_port_to_8883() {
        // An operator who enables TLS and does not also change the port almost
        // certainly meant 8883. Leaving 1883 connects to the broker's
        // plaintext listener and fails the handshake with something that does
        // not mention the port at all.
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        cli.mqtt_tls = true;
        let handle = create_mqtt_client(&cli, None).expect("client built");
        assert!(handle.config().tls.is_some());
        assert_eq!(handle.config().port, 8883);
    }

    #[test]
    fn an_explicit_port_is_never_overridden() {
        // Counterpart: the default must not become a rule. A broker on 1883
        // with TLS, or on 8884, is the operator's business.
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        cli.mqtt_tls = true;
        cli.mqtt_port = Some(1883);
        assert_eq!(
            create_mqtt_client(&cli, None)
                .expect("client")
                .config()
                .port,
            1883
        );

        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        let cfg = config_with(&[("MQTT_TLS", "true"), ("MQTT_PORT", "8884")]);
        assert_eq!(
            create_mqtt_client(&cli, Some(&cfg))
                .expect("client")
                .config()
                .port,
            8884
        );
    }

    #[test]
    fn a_ca_file_turns_tls_on_by_itself() {
        // Configuring a trust anchor and then connecting in plaintext is never
        // what was meant, and the failure would be silent: the station would
        // publish unencrypted to a broker the operator believed was protected.
        let mut cli = default_cli();
        cli.mqtt_host = Some("mqtt.local".to_owned());
        cli.mqtt_ca_file = Some(std::path::PathBuf::from("/etc/birdnet/mqtt-ca.pem"));
        let handle = create_mqtt_client(&cli, None).expect("client built");
        let tls = handle.config().tls.as_ref().expect("TLS is on");
        assert_eq!(
            tls.ca_file.as_deref(),
            Some(std::path::Path::new("/etc/birdnet/mqtt-ca.pem"))
        );
        assert_eq!(handle.config().port, 8883);
    }

    #[test]
    fn tls_settings_come_through_from_the_config_file_too() {
        let cli = default_cli();
        let cfg = config_with(&[
            ("MQTT_HOST", "mqtt.local"),
            ("MQTT_TLS", "true"),
            ("MQTT_CA_FILE", "/etc/ca.pem"),
            ("MQTT_TLS_SERVER_NAME", "broker.example"),
        ]);
        let handle = create_mqtt_client(&cli, Some(&cfg)).expect("client built");
        let tls = handle.config().tls.as_ref().expect("TLS is on");
        assert_eq!(
            tls.ca_file.as_deref(),
            Some(std::path::Path::new("/etc/ca.pem"))
        );
        assert_eq!(tls.server_name.as_deref(), Some("broker.example"));
    }

    #[test]
    fn a_blank_ca_file_setting_does_not_turn_tls_on() {
        // Every surface that can supply this produces a blank rather than an
        // absent value when the operator declines it: `docker-compose.yml`
        // interpolates `${BIRDNET_MQTT_CA_FILE:-}` whether or not it was set,
        // and `.env.example` ships the key with an empty value.
        let cli = default_cli();
        let cfg = config_with(&[("MQTT_HOST", "mqtt.local"), ("MQTT_CA_FILE", "")]);
        let handle = create_mqtt_client(&cli, Some(&cfg)).expect("client built");
        assert!(handle.config().tls.is_none());
        assert_eq!(handle.config().port, 1883);
    }
}
