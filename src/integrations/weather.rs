//! Weather poll-loop bootstrap (O-23 follow-up, `G-26`).
//!
//! Default-off per the maintainer's privacy posture: a fresh install does
//! NOT phone home until the operator sets `BNB_WEATHER_ENABLED=1`. When
//! enabled and station coordinates resolve, this spawns a background
//! tokio task that calls [`birdnet_integrations::weather::Client::fetch_hourly`]
//! every `POLL_INTERVAL`, upserts the rows into the `weather` SQLite table,
//! and prunes rows older than 30 days.
//!
//! Which upstream it asks comes from `WEATHER_PROVIDER` (`open-meteo`, the
//! default; `met-no`; `wunderground`). A name nobody implements does not start
//! the poll: see [`birdnet_integrations::weather::Provider::resolve`] for why
//! that is a refusal rather than a fall back.
//!
//! Coordinate resolution order:
//!
//! 1. `BNB_STATION_LAT` / `BNB_STATION_LON` env (locale-tolerant decimals
//!    so a `,`-separator operator works the same as a `.`-separator one).
//! 2. `LATITUDE` / `LONGITUDE` from the loaded config file.
//!
//! Open-Meteo timestamps are normalised at write time from the upstream
//! `YYYY-MM-DDTHH:MM` shape to `YYYY-MM-DDTHH:MM:00Z`, matching the
//! day-strip and dawn-chorus range bounds in `routes::pages::today` and
//! `routes::pages::dawn_chorus` so range queries find the rows.

use std::time::Duration;

use birdnet_core::config::Config;
use birdnet_db::weather::{WeatherRow, WeatherStore};
use birdnet_integrations::weather as upstream;
use birdnet_web::state::AppState;

/// Retention window for cached weather rows.
const RETENTION_DAYS: u32 = 30;

/// Spawn the background Open-Meteo poll loop when both `BNB_WEATHER_ENABLED=1`
/// and station coordinates resolve.
///
/// Returns the spawned task's [`tokio::task::JoinHandle`] when active,
/// `None` when disabled (the operator hasn't opted in), when coordinates
/// can't be resolved, or when the HTTP client cannot be initialised.
pub fn spawn_weather_poll(
    config: Option<&Config>,
    state: AppState,
) -> Option<tokio::task::JoinHandle<()>> {
    if !upstream::is_enabled() {
        tracing::debug!("weather poll disabled (set BNB_WEATHER_ENABLED=1 to opt in)");
        return None;
    }

    let lat = resolve_lat(config)?;
    let lon = resolve_lon(config)?;

    let provider = match resolve_provider(config) {
        Ok(p) => p,
        Err(e) => {
            // Named rather than swallowed: the operator asked for a specific
            // upstream and is not getting it, and a station quietly serving a
            // gridded forecast where a personal weather station was configured
            // looks exactly like one that is working.
            tracing::warn!(error = %e, "weather provider not usable; poll disabled");
            return None;
        }
    };

    // An explicit base URL overrides the provider's own host — that is how a
    // self-hosted Open-Meteo is reached, and how these are tested.
    let base = std::env::var("BNB_WEATHER_BASE_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let client = match upstream::Client::new_for(provider.clone(), base.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "weather client init failed; poll disabled");
            return None;
        }
    };

    tracing::info!(
        lat,
        lon,
        provider = provider.key(),
        base = %base.unwrap_or_else(|| provider.default_base_url().to_owned()),
        interval_secs = upstream::POLL_INTERVAL.as_secs(),
        "weather poll loop enabled ({})",
        provider.label()
    );

    Some(tokio::spawn(async move {
        poll_loop(client, lat, lon, state, upstream::POLL_INTERVAL).await;
    }))
}

/// The poll loop body. Public-crate so it can be exercised with a stub
/// `AppState` if a future test wants to.
async fn poll_loop(
    client: upstream::Client,
    lat: f64,
    lon: f64,
    state: AppState,
    interval: Duration,
) {
    loop {
        match client.fetch_hourly(lat, lon).await {
            Ok(rows) if !rows.is_empty() => {
                let normalised: Vec<WeatherRow> = rows.into_iter().map(normalise_row).collect();
                let row_count = normalised.len();
                let state2 = state.clone();
                let res = tokio::task::spawn_blocking(move || -> (usize, usize) {
                    state2.with_db(|conn| {
                        let mut written = 0;
                        for row in &normalised {
                            if conn.upsert(row).is_ok() {
                                written += 1;
                            }
                        }
                        // Date-relative, so it takes the same clock guard the
                        // other retention jobs do (RC-17).
                        let pruned = if crate::maintenance::clock_is_safe_for_retention() {
                            conn.prune_older_than_days(RETENTION_DAYS).unwrap_or(0)
                        } else {
                            0
                        };
                        (written, pruned)
                    })
                })
                .await;
                match res {
                    Ok((written, pruned)) => tracing::debug!(
                        written,
                        pruned,
                        fetched = row_count,
                        "weather rows refreshed"
                    ),
                    Err(e) => tracing::warn!(error = %e, "weather DB write task failed"),
                }
            }
            Ok(_) => tracing::debug!("weather fetch returned no rows"),
            Err(e) => tracing::warn!(error = %e, "weather fetch failed (will retry)"),
        }
        tokio::time::sleep(interval).await;
    }
}

/// Open-Meteo emits ISO-8601 truncated to the minute (no seconds, no
/// `Z`). The day-strip / dawn-chorus renderers query with
/// `YYYY-MM-DDTHH:MM:SSZ` bounds, so widen the upstream string at write
/// time to keep the range queries finding rows. Anything that already
/// carries seconds passes through.
fn normalise_row(mut row: WeatherRow) -> WeatherRow {
    row.at = normalise_at(&row.at);
    row
}

fn normalise_at(at: &str) -> String {
    // Sample upstream input: "2026-05-28T13:00"
    // Target after widen:    "2026-05-28T13:00:00Z"
    if at.len() == "YYYY-MM-DDTHH:MM".len()
        && at.as_bytes().get(10) == Some(&b'T')
        && at.as_bytes().get(13) == Some(&b':')
    {
        return format!("{at}:00Z");
    }
    at.to_string()
}

/// Which upstream the operator asked for, with its credentials.
///
/// The API key is read from the config the same way every other credential is,
/// which means `BIRDNET_WEATHER_API_KEY_FILE` mounts it from a file — see
/// `birdnet_core::config::secret_file`. Nothing here logs it.
fn resolve_provider(config: Option<&Config>) -> Result<upstream::Provider, upstream::WeatherError> {
    // Each key is read as a literal `config.get("KEY")` rather than through a
    // helper closure. `tests/every_config_key_is_known.rs` finds reads by their
    // shape, and a closure hides the name from it — which is exactly how it
    // reported these three as "known but nothing reads them".
    upstream::Provider::resolve(
        config
            .and_then(|c| c.get("WEATHER_PROVIDER"))
            .unwrap_or_default(),
        config.and_then(|c| c.get("WEATHER_STATION_ID")),
        config.and_then(|c| c.get("WEATHER_API_KEY")),
    )
}

fn resolve_lat(config: Option<&Config>) -> Option<f64> {
    resolve_decimal_env("BNB_STATION_LAT")
        .or_else(|| config.and_then(|cfg| cfg.get_parsed::<f64>("LATITUDE").ok()))
}

fn resolve_lon(config: Option<&Config>) -> Option<f64> {
    resolve_decimal_env("BNB_STATION_LON")
        .or_else(|| config.and_then(|cfg| cfg.get_parsed::<f64>("LONGITUDE").ok()))
}

fn resolve_decimal_env(key: &str) -> Option<f64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| birdnet_core::config::locale::parse_decimal(&raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_widens_minute_precision_input() {
        assert_eq!(normalise_at("2026-05-28T13:00"), "2026-05-28T13:00:00Z");
        assert_eq!(normalise_at("2026-01-02T05:30"), "2026-01-02T05:30:00Z");
    }

    #[test]
    fn normalise_passes_through_second_precision_input() {
        let canonical = "2026-05-28T13:00:00Z";
        assert_eq!(normalise_at(canonical), canonical);
    }

    #[test]
    fn normalise_leaves_unexpected_shapes_alone() {
        // Defensive: don't munge anything that doesn't match the
        // documented upstream format.
        assert_eq!(normalise_at(""), "");
        assert_eq!(normalise_at("garbage"), "garbage");
        assert_eq!(normalise_at("2026-05-28"), "2026-05-28");
    }

    /// The provider the operator configured is the one the poll asks (`G-26`).
    ///
    /// Observed failing against the single-provider bootstrap: `resolve_provider`
    /// did not exist, and the client was built with `Client::new()`, which is
    /// Open-Meteo whatever the config says.
    #[test]
    fn the_configured_provider_is_the_one_resolved() {
        use crate::integrations::test_support::config_with;
        use birdnet_integrations::weather::Provider;

        // No key at all is the default, which is what leaves an existing
        // station on Open-Meteo without touching its config.
        assert_eq!(
            resolve_provider(None).expect("no config resolves"),
            Provider::OpenMeteo
        );
        assert_eq!(
            resolve_provider(Some(&config_with(&[]))).expect("empty config resolves"),
            Provider::OpenMeteo
        );
        assert_eq!(
            resolve_provider(Some(&config_with(&[("WEATHER_PROVIDER", "met-no")])))
                .expect("met-no resolves"),
            Provider::MetNorway
        );
        assert_eq!(
            resolve_provider(Some(&config_with(&[
                ("WEATHER_PROVIDER", "wunderground"),
                ("WEATHER_STATION_ID", "KMAHANOV10"),
                ("WEATHER_API_KEY", "abc123"),
            ])))
            .expect("wunderground resolves with both credentials"),
            Provider::Wunderground {
                station_id: "KMAHANOV10".to_owned(),
                api_key: "abc123".to_owned(),
            }
        );
    }

    /// The counterpart: a misconfigured provider does not silently become the
    /// default. Without this, a `resolve_provider` that returned
    /// `Provider::OpenMeteo` on every error would pass the gate above.
    #[test]
    fn a_misconfigured_provider_is_an_error_rather_than_the_default() {
        use crate::integrations::test_support::config_with;

        for entries in [
            vec![("WEATHER_PROVIDER", "accuweather")],
            // Asked for the personal station, gave it no station.
            vec![
                ("WEATHER_PROVIDER", "wunderground"),
                ("WEATHER_API_KEY", "abc123"),
            ],
            vec![
                ("WEATHER_PROVIDER", "wunderground"),
                ("WEATHER_STATION_ID", "KMAHANOV10"),
            ],
        ] {
            let cfg = config_with(&entries);
            assert!(
                resolve_provider(Some(&cfg)).is_err(),
                "{entries:?} must not resolve to a working provider"
            );
        }
    }

    #[test]
    fn resolve_lat_lon_returns_none_without_inputs() {
        // No env, no config → no coords. The env-var read is what makes
        // this test non-hermetic; we accept "BNB_STATION_LAT is not set
        // in the test process" as the documented precondition.
        if std::env::var("BNB_STATION_LAT").is_err() {
            assert_eq!(resolve_lat(None), None);
        }
        if std::env::var("BNB_STATION_LON").is_err() {
            assert_eq!(resolve_lon(None), None);
        }
    }
}
