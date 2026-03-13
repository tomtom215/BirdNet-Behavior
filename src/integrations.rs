//! Integration client construction helpers.
//!
//! Creates `Apprise` and `BirdWeather` clients from CLI flags and/or config
//! file values.  Returns `None` when the integration is not configured.

use std::sync::Arc;

use crate::cli::Cli;

/// Type alias for the shared Apprise client handle.
pub type AppriseHandle = Arc<tokio::sync::Mutex<birdnet_integrations::apprise::Client>>;

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
