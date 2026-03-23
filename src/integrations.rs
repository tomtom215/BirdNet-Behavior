//! Integration client construction helpers.
//!
//! Creates `Apprise`, `BirdWeather`, heartbeat, and notification filter
//! clients from CLI flags and/or config file values.
//! Returns `None` when the integration is not configured.

use std::sync::Arc;

use crate::cli::Cli;

/// Type alias for the shared Apprise client handle.
pub type AppriseHandle = Arc<tokio::sync::Mutex<birdnet_integrations::apprise::Client>>;

/// Type alias for the shared email notifier handle.
pub type EmailHandle = Arc<birdnet_integrations::email::EmailNotifier>;

/// Type alias for the heartbeat client handle.
pub type HeartbeatHandle = Arc<birdnet_integrations::heartbeat::HeartbeatClient>;

/// Create an Apprise notification client from CLI flags and/or config file values.
///
/// Returns `None` if neither an Apprise URL nor config file is configured.
pub fn create_apprise_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<AppriseHandle> {
    let apprise_url = cli
        .apprise_url
        .clone()
        .or_else(|| config?.get("APPRISE_URL").map(String::from));

    let apprise_config_file = cli.apprise_config.clone().or_else(|| {
        config?
            .get("APPRISE_CONFIG_FILE")
            .map(std::path::PathBuf::from)
    });

    // Need at least one of: URL or config file.
    if apprise_url.is_none() && apprise_config_file.is_none() {
        return None;
    }

    // Use the URL if present, or a placeholder for CLI-only mode.
    let url = apprise_url.unwrap_or_default();

    let min_confidence = if (cli.notify_confidence - 0.8).abs() > f32::EPSILON {
        cli.notify_confidence
    } else {
        config
            .and_then(|c| c.get_parsed::<f32>("APPRISE_MIN_CONFIDENCE").ok())
            .unwrap_or(cli.notify_confidence)
    };

    let cooldown_secs = config
        .and_then(|c| c.get_parsed::<u64>("APPRISE_COOLDOWN").ok())
        .unwrap_or(300);

    // Helper to split a comma-separated config value into a Vec<String>.
    let parse_species_list = |list: &str| -> Vec<String> {
        list.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let species_watchlist = config
        .and_then(|c| c.get("APPRISE_WATCHLIST"))
        .map(|list| parse_species_list(&list))
        .unwrap_or_default();

    // Dual-filter: exclude list from config file OR CLI --notify-species-exclude.
    let species_notify_exclude = {
        let from_config = config
            .and_then(|c| c.get("APPRISE_WATCHLIST_EXCLUDE"))
            .map(|list| parse_species_list(&list))
            .unwrap_or_default();
        let from_cli = cli
            .notify_species_exclude
            .as_deref()
            .map(parse_species_list)
            .unwrap_or_default();
        // Merge both sources; dedup not strictly necessary but keeps it clean.
        let mut merged = from_config;
        for s in from_cli {
            if !merged.contains(&s) {
                merged.push(s);
            }
        }
        merged
    };

    let notify_config = birdnet_integrations::apprise::NotifyConfig {
        min_confidence,
        species_watchlist,
        species_notify_exclude,
        cooldown: std::time::Duration::from_secs(cooldown_secs),
        per_species_cooldown: std::collections::HashMap::new(),
    };

    let client_result = if url.is_empty() {
        // CLI-only mode: no HTTP server configured.
        #[allow(clippy::redundant_clone)] // else branch also borrows apprise_config_file
        let cfg_path = apprise_config_file
            .clone()
            .expect("config file required when no URL");
        tracing::info!(
            path = %cfg_path.display(),
            "Apprise CLI-only notifications enabled"
        );
        birdnet_integrations::apprise::Client::new_cli_only(cfg_path, notify_config)
    } else {
        birdnet_integrations::apprise::Client::new(&url, notify_config).map(|c| {
            if let Some(ref cfg_path) = apprise_config_file {
                tracing::info!(
                    url = %url,
                    path = %cfg_path.display(),
                    min_confidence = %min_confidence,
                    cooldown_secs,
                    "Apprise notifications enabled (HTTP + CLI config)"
                );
                c.with_config_file(cfg_path.clone())
            } else {
                tracing::info!(
                    url = %url,
                    min_confidence = %min_confidence,
                    cooldown_secs,
                    "Apprise notifications enabled"
                );
                c
            }
        })
    };

    match client_result {
        Ok(client) => Some(Arc::new(tokio::sync::Mutex::new(client))),
        Err(e) => {
            tracing::error!(error = %e, "failed to create Apprise client");
            None
        }
    }
}

/// Create a `BirdWeather` client from CLI flags and/or config file values.
///
/// Returns `None` if no station token is configured.
pub fn create_birdweather_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_integrations::birdweather::Client> {
    let token = cli
        .birdweather_token
        .clone()
        .or_else(|| config?.get("BIRDWEATHER_TOKEN").map(String::from))?;

    let lat = cli
        .latitude
        .or_else(|| config?.get_parsed::<f64>("LATITUDE").ok())
        .unwrap_or(0.0);

    let lon = cli
        .longitude
        .or_else(|| config?.get_parsed::<f64>("LONGITUDE").ok())
        .unwrap_or(0.0);

    match birdnet_integrations::birdweather::Client::new(&token, lat, lon) {
        Ok(client) => {
            tracing::info!(lat, lon, "BirdWeather uploads enabled");
            Some(client)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to create BirdWeather client");
            None
        }
    }
}

/// Create a heartbeat client from CLI flags and/or config file values.
///
/// Returns `None` if no heartbeat URL is configured.
pub fn create_heartbeat_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<HeartbeatHandle> {
    let url = cli
        .heartbeat_url
        .clone()
        .or_else(|| config?.get("HEARTBEAT_URL").map(String::from))?;

    match birdnet_integrations::heartbeat::HeartbeatClient::new(&url) {
        Ok(client) => {
            tracing::info!(url = %url, "heartbeat monitoring enabled");
            Some(Arc::new(client))
        }
        Err(e) => {
            tracing::warn!(error = %e, "heartbeat client not created");
            None
        }
    }
}

/// Create a notification filter from CLI flags.
pub fn create_notification_filter(
    cli: &Cli,
) -> birdnet_integrations::notification::NotificationFilter {
    use birdnet_integrations::notification::{NotificationFilter, SpeciesFilter, TriggerMode};

    let trigger = TriggerMode::parse(&cli.notify_trigger);
    let species_filter = SpeciesFilter::new(
        cli.notify_species_exclude.as_deref(),
        cli.notify_species_only.as_deref(),
    );

    tracing::info!(
        trigger = %trigger,
        "notification filter configured"
    );

    NotificationFilter {
        trigger,
        species_filter,
    }
}

/// Create a notification template from CLI flags and/or config.
pub fn create_notification_template(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_integrations::notification::NotificationTemplate {
    use birdnet_integrations::notification::NotificationTemplate;

    let title = cli
        .notify_title_template
        .clone()
        .or_else(|| config?.get("APPRISE_TITLE_TEMPLATE").map(String::from));

    let body = cli
        .notify_body_template
        .clone()
        .or_else(|| config?.get("APPRISE_BODY_TEMPLATE").map(String::from));

    match (title, body) {
        (Some(t), Some(b)) => {
            tracing::debug!("custom notification template configured");
            NotificationTemplate::new(t, b)
        }
        (Some(t), None) => NotificationTemplate::new(
            t,
            "$comname ($sciname) detected ($confidencepct% confidence) at $time on $date"
                .to_string(),
        ),
        (None, Some(b)) => NotificationTemplate::new("Bird Detection: $comname".to_string(), b),
        (None, None) => NotificationTemplate::default(),
    }
}

/// Create an email notifier from settings stored in the `SQLite` database.
///
/// Returns `None` if no SMTP host is configured or construction fails.
pub fn create_email_notifier(state: &birdnet_web::state::AppState) -> Option<EmailHandle> {
    use birdnet_db::settings::{ensure_settings_table, get_or};
    use birdnet_integrations::email::{EmailConfig, EmailNotifier};

    // Helper: unwrap a settings Result to String, falling back to the default.
    fn s(r: Result<String, birdnet_db::settings::SettingsError>, default: &str) -> String {
        r.unwrap_or_else(|_| default.to_string())
    }

    let smtp_host: String = state
        .with_db(|conn| {
            ensure_settings_table(conn).ok();
            Ok::<String, birdnet_db::settings::SettingsError>(s(
                get_or(conn, "email_smtp_host", ""),
                "",
            ))
        })
        .unwrap_or_default();
    if smtp_host.is_empty() {
        return None;
    }

    let cfg = state
        .with_db(|conn| {
            let smtp_port = s(get_or(conn, "email_smtp_port", "587"), "587")
                .parse::<u16>()
                .unwrap_or(587);
            let use_starttls = s(get_or(conn, "email_starttls", "true"), "true") != "false";
            let min_confidence = s(get_or(conn, "email_min_confidence", "0.80"), "0.80")
                .parse::<f64>()
                .unwrap_or(0.80);
            let cooldown_secs = s(get_or(conn, "email_cooldown_secs", "300"), "300")
                .parse::<u64>()
                .unwrap_or(300);
            let from_name_str = s(get_or(conn, "email_from_name", ""), "");
            Ok::<EmailConfig, birdnet_db::settings::SettingsError>(EmailConfig {
                smtp_host: smtp_host.clone(),
                smtp_port,
                username: s(get_or(conn, "email_smtp_user", ""), ""),
                password: s(get_or(conn, "email_smtp_pass", ""), ""),
                from_address: s(get_or(conn, "email_from", ""), ""),
                to_address: s(get_or(conn, "email_to", ""), ""),
                from_name: if from_name_str.is_empty() {
                    None
                } else {
                    Some(from_name_str)
                },
                use_starttls,
                min_confidence,
                cooldown_secs,
            })
        })
        .unwrap_or_else(|_| EmailConfig {
            smtp_host: smtp_host.clone(),
            smtp_port: 587,
            username: String::new(),
            password: String::new(),
            from_address: String::new(),
            to_address: String::new(),
            from_name: None,
            use_starttls: true,
            min_confidence: 0.80,
            cooldown_secs: 300,
        });

    match EmailNotifier::new(cfg) {
        Ok(notifier) => {
            tracing::info!(smtp_host = %smtp_host, "email alerts enabled");
            Some(Arc::new(notifier))
        }
        Err(e) => {
            tracing::warn!(error = %e, "email notifier not started (check SMTP settings)");
            None
        }
    }
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

/// Create an HTTP Basic Auth config from the config file.
///
/// Looks for `CADDY_PWD` (password) and defaults username to "birdnet"
/// to match BirdNET-Pi's Caddy setup.
pub fn create_auth_config(
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_web::auth::AuthConfig> {
    let password = config?.get("CADDY_PWD")?;
    let username = config
        .and_then(|c| c.get("CADDY_USER"))
        .unwrap_or("birdnet");

    let auth = birdnet_web::auth::AuthConfig::new(username, password)?;
    tracing::info!(username = %username, "basic auth enabled");
    Some(auth)
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
