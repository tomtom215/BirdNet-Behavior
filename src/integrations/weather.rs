//! Open-Meteo poll-loop bootstrap (O-23 follow-up).
//!
//! Default-off per the maintainer's privacy posture: a fresh install does
//! NOT phone home until the operator sets `BNB_WEATHER_ENABLED=1`. When
//! enabled and station coordinates resolve, this spawns a background
//! tokio task that calls [`birdnet_integrations::weather::Client::fetch_hourly`]
//! every `POLL_INTERVAL`, upserts the rows into the `weather` SQLite table,
//! and prunes rows older than 30 days.
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

    let client = match upstream::Client::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "weather client init failed; poll disabled");
            return None;
        }
    };

    let base = std::env::var("BNB_WEATHER_BASE_URL")
        .unwrap_or_else(|_| upstream::DEFAULT_BASE_URL.to_string());
    tracing::info!(
        lat,
        lon,
        base = %base,
        interval_secs = upstream::POLL_INTERVAL.as_secs(),
        "weather poll loop enabled (Open-Meteo)"
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
                        let pruned = conn.prune_older_than_days(RETENTION_DAYS).unwrap_or(0);
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
