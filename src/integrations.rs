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
        .map(parse_species_list)
        .unwrap_or_default();

    // Dual-filter: exclude list from config file OR CLI --notify-species-exclude.
    let species_notify_exclude = {
        let from_config = config
            .and_then(|c| c.get("APPRISE_WATCHLIST_EXCLUDE"))
            .map(parse_species_list)
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

#[cfg(test)]
mod tests {
    //! Unit tests for the integration-client construction helpers.
    //!
    //! These exist because the carryover from PR #49 noted that
    //! `src/integrations.rs` was at 0 % unit coverage. Every helper here
    //! is a pure config-to-handle mapping: extract a value from CLI or
    //! config, decide whether to construct, and return the handle (or
    //! None). The tests fix the precedence between CLI flag, config
    //! value, and "neither configured" so a future refactor can't
    //! silently break the "CLI wins" rule that the production
    //! deployment story depends on.
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn default_cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    fn config_with(entries: &[(&str, &str)]) -> birdnet_core::config::Config {
        let content = entries
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        birdnet_core::config::Config::parse(&content).unwrap()
    }

    fn test_state() -> birdnet_web::state::AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        birdnet_web::state::AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    // ── create_apprise_client ──────────────────────────────────────────

    #[test]
    fn apprise_is_none_when_neither_url_nor_config_file_set() {
        let cli = default_cli();
        assert!(create_apprise_client(&cli, None).is_none());
    }

    #[test]
    fn apprise_built_from_cli_url() {
        let mut cli = default_cli();
        cli.apprise_url = Some("http://localhost:8000".to_owned());
        assert!(create_apprise_client(&cli, None).is_some());
    }

    #[test]
    fn apprise_built_from_config_url() {
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_URL", "http://broker:8000")]);
        assert!(create_apprise_client(&cli, Some(&cfg)).is_some());
    }

    #[test]
    fn apprise_built_in_cli_only_mode_when_config_file_set() {
        // CLI-only mode: no HTTP URL, only an Apprise config file. The
        // helper still returns a handle; notifications fan out via the
        // `apprise` CLI rather than the HTTP API.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut cli = default_cli();
        cli.apprise_config = Some(tmp.path().to_path_buf());
        assert!(create_apprise_client(&cli, None).is_some());
    }

    // ── create_birdweather_client ──────────────────────────────────────

    #[test]
    fn birdweather_is_none_without_token() {
        let cli = default_cli();
        assert!(create_birdweather_client(&cli, None).is_none());
    }

    #[test]
    fn birdweather_built_from_cli_token() {
        let mut cli = default_cli();
        cli.birdweather_token = Some("station-abc".to_owned());
        cli.latitude = Some(42.36);
        cli.longitude = Some(-71.06);
        let client = create_birdweather_client(&cli, None).expect("client built");
        assert_eq!(client.coordinates(), (42.36, -71.06));
    }

    #[test]
    fn birdweather_built_from_config_token() {
        let cli = default_cli();
        let cfg = config_with(&[
            ("BIRDWEATHER_TOKEN", "config-station"),
            ("LATITUDE", "40.0"),
            ("LONGITUDE", "-74.0"),
        ]);
        let client = create_birdweather_client(&cli, Some(&cfg)).expect("client built");
        assert_eq!(client.coordinates(), (40.0, -74.0));
    }

    #[test]
    fn birdweather_defaults_coordinates_to_zero_when_unset() {
        // Token present, coordinates absent → station coordinates
        // default to (0, 0). BirdWeather treats that as an
        // intentionally-anonymous station rather than rejecting the
        // submission.
        let mut cli = default_cli();
        cli.birdweather_token = Some("anonymous".to_owned());
        let client = create_birdweather_client(&cli, None).expect("client built");
        assert_eq!(client.coordinates(), (0.0, 0.0));
    }

    // ── create_heartbeat_client ────────────────────────────────────────

    #[test]
    fn heartbeat_is_none_without_url() {
        let cli = default_cli();
        assert!(create_heartbeat_client(&cli, None).is_none());
    }

    #[test]
    fn heartbeat_built_from_cli_url() {
        let mut cli = default_cli();
        cli.heartbeat_url = Some("https://heartbeat.example/ping".to_owned());
        assert!(create_heartbeat_client(&cli, None).is_some());
    }

    #[test]
    fn heartbeat_built_from_config_url() {
        let cli = default_cli();
        let cfg = config_with(&[("HEARTBEAT_URL", "https://hb.example/ping")]);
        assert!(create_heartbeat_client(&cli, Some(&cfg)).is_some());
    }

    // ── create_notification_filter ─────────────────────────────────────

    #[test]
    fn filter_uses_each_detection_trigger_by_default() {
        use birdnet_integrations::notification::TriggerMode;
        let cli = default_cli();
        let f = create_notification_filter(&cli);
        assert_eq!(f.trigger, TriggerMode::EachDetection);
        // No species filter → allow everything.
        assert!(f.species_filter.is_allowed("Pica pica"));
        assert!(f.species_filter.is_allowed("Corvus corax"));
    }

    #[test]
    fn filter_parses_new_species_trigger() {
        use birdnet_integrations::notification::TriggerMode;
        let mut cli = default_cli();
        cli.notify_trigger = "new-species".to_owned();
        let f = create_notification_filter(&cli);
        assert_eq!(f.trigger, TriggerMode::NewSpecies);
    }

    #[test]
    fn filter_parses_new_species_daily_trigger() {
        use birdnet_integrations::notification::TriggerMode;
        let mut cli = default_cli();
        cli.notify_trigger = "new-species-daily".to_owned();
        let f = create_notification_filter(&cli);
        assert_eq!(f.trigger, TriggerMode::NewSpeciesDaily);
    }

    #[test]
    fn filter_honours_species_exclude_list() {
        let mut cli = default_cli();
        cli.notify_species_exclude = Some("Pica pica, Corvus corax".to_owned());
        let f = create_notification_filter(&cli);
        assert!(!f.species_filter.is_allowed("Pica pica"));
        assert!(!f.species_filter.is_allowed("Corvus corax"));
        assert!(f.species_filter.is_allowed("Turdus merula"));
    }

    #[test]
    fn filter_honours_species_only_list() {
        let mut cli = default_cli();
        cli.notify_species_only = Some("Turdus merula,Erithacus rubecula".to_owned());
        let f = create_notification_filter(&cli);
        assert!(f.species_filter.is_allowed("Turdus merula"));
        assert!(f.species_filter.is_allowed("Erithacus rubecula"));
        assert!(!f.species_filter.is_allowed("Pica pica"));
    }

    // ── create_notification_template ───────────────────────────────────

    #[test]
    fn template_default_when_neither_title_nor_body_supplied() {
        let cli = default_cli();
        let t = create_notification_template(&cli, None);
        // Default template produced by NotificationTemplate::default().
        let ctx = birdnet_integrations::notification::NotificationContext {
            sci_name: "Pica pica".to_owned(),
            com_name: "Eurasian Magpie".to_owned(),
            confidence: 0.9,
            confidence_pct: 90,
            date: "2026-05-19".to_owned(),
            time: "09:00:00".to_owned(),
            week: 20,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };
        let (title, body) = t.render(&ctx);
        assert!(!title.is_empty(), "default title must not be empty");
        assert!(!body.is_empty(), "default body must not be empty");
    }

    #[test]
    fn template_uses_cli_title_and_body() {
        let mut cli = default_cli();
        cli.notify_title_template = Some("Title $comname".to_owned());
        cli.notify_body_template = Some("Body $sciname".to_owned());
        let t = create_notification_template(&cli, None);
        let ctx = birdnet_integrations::notification::NotificationContext {
            sci_name: "Pica pica".to_owned(),
            com_name: "Eurasian Magpie".to_owned(),
            confidence: 0.9,
            confidence_pct: 90,
            date: String::new(),
            time: String::new(),
            week: 0,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };
        let (title, body) = t.render(&ctx);
        assert!(title.contains("Eurasian Magpie"));
        assert!(body.contains("Pica pica"));
    }

    #[test]
    fn template_falls_back_to_config_when_cli_unset() {
        let cli = default_cli();
        let cfg = config_with(&[
            ("APPRISE_TITLE_TEMPLATE", "Config-$comname"),
            ("APPRISE_BODY_TEMPLATE", "Body-$confidencepct"),
        ]);
        let t = create_notification_template(&cli, Some(&cfg));
        let ctx = birdnet_integrations::notification::NotificationContext {
            sci_name: "X".to_owned(),
            com_name: "Y".to_owned(),
            confidence: 0.5,
            confidence_pct: 50,
            date: String::new(),
            time: String::new(),
            week: 0,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };
        let (title, body) = t.render(&ctx);
        assert!(title.contains("Config-Y"));
        assert!(body.contains("50"));
    }

    #[test]
    fn template_only_title_uses_default_body_with_full_substitutions() {
        let mut cli = default_cli();
        cli.notify_title_template = Some("only-title".to_owned());
        let t = create_notification_template(&cli, None);
        let ctx = birdnet_integrations::notification::NotificationContext {
            sci_name: "Pica pica".to_owned(),
            com_name: "Eurasian Magpie".to_owned(),
            confidence: 0.9,
            confidence_pct: 90,
            date: "2026-05-19".to_owned(),
            time: "09:00:00".to_owned(),
            week: 20,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };
        let (title, body) = t.render(&ctx);
        assert_eq!(title, "only-title");
        // The hand-rolled default body uses $comname / $sciname /
        // $confidencepct / $time / $date placeholders.
        assert!(body.contains("Eurasian Magpie"));
        assert!(body.contains("Pica pica"));
        assert!(body.contains("90"));
    }

    #[test]
    fn template_only_body_uses_default_title() {
        let mut cli = default_cli();
        cli.notify_body_template = Some("only-body".to_owned());
        let t = create_notification_template(&cli, None);
        let ctx = birdnet_integrations::notification::NotificationContext {
            sci_name: "P. pica".to_owned(),
            com_name: "Eurasian Magpie".to_owned(),
            confidence: 0.9,
            confidence_pct: 90,
            date: String::new(),
            time: String::new(),
            week: 0,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };
        let (title, body) = t.render(&ctx);
        assert_eq!(body, "only-body");
        // The hand-rolled default title is "Bird Detection: $comname".
        assert!(title.contains("Eurasian Magpie"));
    }

    // ── create_email_notifier ──────────────────────────────────────────

    #[test]
    fn email_notifier_is_none_when_no_smtp_host_configured() {
        // Empty settings DB → no smtp host → no notifier.
        let state = test_state();
        assert!(create_email_notifier(&state).is_none());
    }

    #[test]
    fn email_notifier_built_from_settings_table() {
        // Seed the settings rows the helper reads, then prove it
        // constructs a notifier. We don't actually send mail; we only
        // pin that the configuration path builds the handle.
        use birdnet_db::settings::{SettingsCategory, ensure_settings_table, set};
        let state = test_state();
        state.with_db(|conn| {
            ensure_settings_table(conn).unwrap();
            set(
                conn,
                "email_smtp_host",
                "smtp.example.com",
                SettingsCategory::Notifications,
            )
            .unwrap();
            set(
                conn,
                "email_smtp_port",
                "587",
                SettingsCategory::Notifications,
            )
            .unwrap();
            set(
                conn,
                "email_from",
                "birds@example.com",
                SettingsCategory::Notifications,
            )
            .unwrap();
            set(
                conn,
                "email_to",
                "operator@example.com",
                SettingsCategory::Notifications,
            )
            .unwrap();
        });
        let notifier = create_email_notifier(&state);
        assert!(
            notifier.is_some(),
            "email notifier should construct when smtp host is set"
        );
    }

    // ── create_mqtt_client / get_mqtt_client_ref ───────────────────────

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

    // ── create_auth_config ─────────────────────────────────────────────

    #[test]
    fn auth_none_when_no_password_configured() {
        let cli = default_cli();
        // Config absent.
        assert!(create_auth_config(None).is_none());
        // Config present but no CADDY_PWD.
        let cfg = config_with(&[("SOMETHING", "irrelevant")]);
        assert!(create_auth_config(Some(&cfg)).is_none());
        // Defeats the borrow checker complaining about unused `cli`.
        let _ = cli;
    }

    #[test]
    fn auth_built_with_default_username_when_only_password_set() {
        let cfg = config_with(&[("CADDY_PWD", "hunter2")]);
        let auth = create_auth_config(Some(&cfg));
        assert!(auth.is_some(), "auth should be built when password set");
    }

    #[test]
    fn auth_built_with_custom_username() {
        let cfg = config_with(&[("CADDY_PWD", "hunter2"), ("CADDY_USER", "operator")]);
        let auth = create_auth_config(Some(&cfg));
        assert!(auth.is_some());
    }
}
