//! Integration client construction helpers.
//!
//! Creates `Apprise` and `BirdWeather` clients from CLI flags and/or config
//! file values.  Returns `None` when the integration is not configured.

use std::sync::Arc;

use crate::cli::Cli;

/// Type alias for the shared Apprise client handle.
pub type AppriseHandle = Arc<tokio::sync::Mutex<birdnet_integrations::apprise::Client>>;

/// Type alias for the shared email notifier handle.
pub type EmailHandle = Arc<birdnet_integrations::email::EmailNotifier>;

/// Create an Apprise notification client from CLI flags and/or config file values.
///
/// Returns `None` if no Apprise URL is configured.
pub fn create_apprise_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<AppriseHandle> {
    let apprise_url = cli
        .apprise_url
        .clone()
        .or_else(|| config?.get("APPRISE_URL").map(String::from));

    let url = apprise_url?;

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

    let species_watchlist = config
        .and_then(|c| c.get("APPRISE_WATCHLIST"))
        .map(|list| {
            list.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let notify_config = birdnet_integrations::apprise::NotifyConfig {
        min_confidence,
        species_watchlist,
        cooldown: std::time::Duration::from_secs(cooldown_secs),
    };

    match birdnet_integrations::apprise::Client::new(&url, notify_config) {
        Ok(client) => {
            tracing::info!(
                url = %url,
                min_confidence = %min_confidence,
                cooldown_secs,
                "Apprise notifications enabled"
            );
            Some(Arc::new(tokio::sync::Mutex::new(client)))
        }
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

/// Create an email notifier from settings stored in the SQLite database.
///
/// Returns `None` if no SMTP host is configured or construction fails.
pub fn create_email_notifier(
    state: &birdnet_web::state::AppState,
) -> Option<EmailHandle> {
    use birdnet_db::settings::{ensure_settings_table, get_or};
    use birdnet_integrations::email::{EmailConfig, EmailNotifier};

    // Helper: unwrap a settings Result to String, falling back to the default.
    fn s(r: Result<String, birdnet_db::settings::SettingsError>, default: &str) -> String {
        r.unwrap_or_else(|_| default.to_string())
    }

    let smtp_host: String = state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        Ok::<String, birdnet_db::settings::SettingsError>(
            s(get_or(conn, "email_smtp_host", ""), "")
        )
    }).unwrap_or_default();
    if smtp_host.is_empty() {
        return None;
    }

    let cfg = state.with_db(|conn| {
        let smtp_port = s(get_or(conn, "email_smtp_port", "587"), "587")
            .parse::<u16>().unwrap_or(587);
        let use_starttls = s(get_or(conn, "email_starttls", "true"), "true") != "false";
        let min_confidence = s(get_or(conn, "email_min_confidence", "0.80"), "0.80")
            .parse::<f64>().unwrap_or(0.80);
        let cooldown_secs = s(get_or(conn, "email_cooldown_secs", "300"), "300")
            .parse::<u64>().unwrap_or(300);
        let from_name_str = s(get_or(conn, "email_from_name", ""), "");
        Ok::<EmailConfig, birdnet_db::settings::SettingsError>(EmailConfig {
            smtp_host: smtp_host.clone(),
            smtp_port,
            username: s(get_or(conn, "email_smtp_user", ""), ""),
            password: s(get_or(conn, "email_smtp_pass", ""), ""),
            from_address: s(get_or(conn, "email_from", ""), ""),
            to_address: s(get_or(conn, "email_to", ""), ""),
            from_name: if from_name_str.is_empty() { None } else { Some(from_name_str) },
            use_starttls,
            min_confidence,
            cooldown_secs,
        })
    }).unwrap_or_else(|_| EmailConfig {
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
