//! `BirdWeather` upload client construction.

use crate::cli::Cli;

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

    // Optional API-base override: `BIRDNET_BIRDWEATHER_URL` env, then the
    // `BIRDWEATHER_URL` config key. For research programmes that route
    // observations to a self-hosted ingest (sensitive-species data kept
    // under the programme's own governance) and for the end-to-end test
    // suite. Logged whenever active so a misdirected station is visible in
    // the first journal lines, not after a season of silent uploads.
    let base_override = std::env::var("BIRDNET_BIRDWEATHER_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            config?
                .get("BIRDWEATHER_URL")
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(String::from)
        });

    match birdnet_integrations::birdweather::Client::new(&token, lat, lon) {
        Ok(client) => {
            let client = if let Some(url) = base_override {
                let client = client.with_base_url(&url);
                tracing::info!(
                    endpoint = %client.base_url(),
                    "BirdWeather uploads enabled (custom endpoint)"
                );
                client
            } else {
                tracing::info!(lat, lon, "BirdWeather uploads enabled");
                client
            };
            Some(client)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to create BirdWeather client");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::create_birdweather_client;
    use crate::integrations::test_support::{config_with, default_cli};

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
}
