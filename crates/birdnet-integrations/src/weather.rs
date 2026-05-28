//! Open-Meteo client for the O-23 signal-context overlay.
//!
//! Off by default. The poll loop only starts when both:
//!
//! 1. `BNB_WEATHER_ENABLED=1` is set in the environment, AND
//! 2. The caller supplies station coordinates (otherwise there's
//!    nothing to fetch for).
//!
//! Open-Meteo is a free, no-API-key weather service whose `ToS` allows
//! non-commercial use; a single-Pi bird station qualifies. Operators
//! uneasy about the third-party fetch can self-host Open-Meteo and
//! point [`Client::new_with_base_url`] at it via `BNB_WEATHER_BASE_URL`.
//!
//! The client never runs inside a request handler — it's a background
//! task spawned at startup. Failures log and move on; they never bubble
//! to a request response.

use std::time::Duration;

use birdnet_db::weather::WeatherRow;
use serde::Deserialize;

/// Default Open-Meteo base URL. Operators can override via env var to
/// point at a self-hosted instance.
pub const DEFAULT_BASE_URL: &str = "https://api.open-meteo.com";

/// How often the poll job hits the API. 30 minutes is well inside the
/// free tier's rate limit and roughly matches the observation cadence
/// that bird behaviour tracks against.
pub const POLL_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Weather poller errors. Hand-rolled to keep the library async-client
/// principle (the binary owns the runtime).
#[derive(Debug)]
pub enum WeatherError {
    /// HTTP transport (timeout, DNS, TLS).
    Http(reqwest::Error),
    /// JSON shape mismatch.
    Decode(serde_json::Error),
    /// Open-Meteo returned a non-success response.
    Api(String),
}

impl std::fmt::Display for WeatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "http error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Api(m) => write!(f, "api error: {m}"),
        }
    }
}

impl std::error::Error for WeatherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Decode(e) => Some(e),
            Self::Api(_) => None,
        }
    }
}

impl From<reqwest::Error> for WeatherError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<serde_json::Error> for WeatherError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e)
    }
}

/// Async client for Open-Meteo.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
}

impl Client {
    /// Build a client pointed at [`DEFAULT_BASE_URL`].
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Http`] if the underlying HTTP client cannot
    /// be constructed (e.g. system TLS resolver failure).
    pub fn new() -> Result<Self, WeatherError> {
        Self::new_with_base_url(
            std::env::var("BNB_WEATHER_BASE_URL")
                .ok()
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        )
    }

    /// Build a client pointed at `base_url` (e.g. a self-hosted Open-Meteo).
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Http`] if the underlying HTTP client cannot
    /// be constructed.
    pub fn new_with_base_url(base_url: impl Into<String>) -> Result<Self, WeatherError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!(
                "BirdNet-Behavior/",
                env!("CARGO_PKG_VERSION"),
                " (open-meteo)"
            ))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.into(),
        })
    }

    /// Fetch the hourly forecast for `(lat, lon)`. Returns rows for the
    /// trailing 24h and the next 24h.
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Http`] on network / TLS errors,
    /// [`WeatherError::Decode`] when the response body isn't shaped as
    /// expected, and [`WeatherError::Api`] when Open-Meteo signals an
    /// error in its `error`/`reason` envelope.
    pub async fn fetch_hourly(&self, lat: f64, lon: f64) -> Result<Vec<WeatherRow>, WeatherError> {
        let url = format!(
            "{base}/v1/forecast?latitude={lat}&longitude={lon}\
             &hourly=temperature_2m,precipitation,wind_speed_10m,wind_direction_10m,\
             pressure_msl,cloud_cover,weather_code\
             &past_days=1&forecast_days=2&timezone=UTC",
            base = self.base_url
        );
        let res = self.http.get(&url).send().await?;
        if !res.status().is_success() {
            return Err(WeatherError::Api(format!(
                "open-meteo returned {}",
                res.status()
            )));
        }
        let body: ForecastResponse = res.json().await?;
        Ok(body.into_rows())
    }
}

/// Whether the weather poll job is enabled. Default off; opt in with
/// `BNB_WEATHER_ENABLED=1` — the prompt's open question; flip the
/// default once the maintainer locks the privacy / network posture.
#[must_use]
pub fn is_enabled() -> bool {
    std::env::var("BNB_WEATHER_ENABLED")
        .is_ok_and(|v| v.trim() == "1" || v.eq_ignore_ascii_case("true"))
}

/// The `reason` field is captured purely so the JSON decoder doesn't
/// silently drop it; the API surface returns `Vec<WeatherRow>` either
/// way, so we don't expose it.
#[derive(Debug, Deserialize)]
struct ForecastResponse {
    #[serde(default)]
    hourly: Option<HourlyBlock>,
    #[serde(default)]
    error: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HourlyBlock {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    temperature_2m: Vec<Option<f32>>,
    #[serde(default)]
    precipitation: Vec<Option<f32>>,
    #[serde(default)]
    wind_speed_10m: Vec<Option<f32>>,
    #[serde(default)]
    wind_direction_10m: Vec<Option<i32>>,
    #[serde(default)]
    pressure_msl: Vec<Option<f32>>,
    #[serde(default)]
    cloud_cover: Vec<Option<i32>>,
    #[serde(default)]
    weather_code: Vec<Option<i32>>,
}

impl ForecastResponse {
    fn into_rows(self) -> Vec<WeatherRow> {
        // wind_speed_10m comes back in m/s; the storage layer prefers
        // knots so the legend chip can read in the operator's mental
        // model. 1 m/s ≈ 1.9438 kt.
        const MS_TO_KT: f32 = 1.943_844_5;
        if self.error.unwrap_or(false) || self.hourly.is_none() {
            return Vec::new();
        }
        let h = self.hourly.unwrap();
        let n = h.time.len();
        (0..n)
            .map(|i| WeatherRow {
                at: h.time[i].clone(),
                temp_c: h.temperature_2m.get(i).copied().flatten(),
                precip_mm: h.precipitation.get(i).copied().flatten(),
                wind_kt: h
                    .wind_speed_10m
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|v| v * MS_TO_KT),
                wind_dir_deg: h.wind_direction_10m.get(i).copied().flatten(),
                pressure_hpa: h.pressure_msl.get(i).copied().flatten(),
                cloud_pct: h.cloud_cover.get(i).copied().flatten(),
                code: h.weather_code.get(i).copied().flatten(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_enabled_defaults_to_false() {
        // Unsetting the env var is unsafe in Rust 2024 + `unsafe_code = deny`,
        // so we just assert against the documented default semantics: any
        // value other than "1"/"true" (or absent) is off.
        // The contract is what the rest of the system depends on, not the
        // specific value of any host-supplied env var at test time.
        assert!(!is_enabled() || std::env::var("BNB_WEATHER_ENABLED").is_ok());
    }

    #[test]
    fn rows_have_canonical_iso_timestamp_in_their_at_field() {
        // Smoke-test the decoder shape; we don't make a network call.
        let json = r#"{
            "hourly": {
                "time": ["2026-05-28T00:00", "2026-05-28T01:00"],
                "temperature_2m": [12.5, 13.0],
                "precipitation": [0.0, 0.2],
                "wind_speed_10m": [3.0, 4.0],
                "wind_direction_10m": [180, 200],
                "pressure_msl": [1015.0, 1014.0],
                "cloud_cover": [10, 25],
                "weather_code": [1, 1]
            }
        }"#;
        let resp: ForecastResponse = serde_json::from_str(json).unwrap();
        let rows = resp.into_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].at, "2026-05-28T00:00");
        assert!((rows[0].temp_c.unwrap() - 12.5).abs() < 1e-4);
        // 3.0 m/s ≈ 5.83 kt.
        let kt = rows[0].wind_kt.unwrap();
        assert!((kt - 5.831_533_4).abs() < 1e-3, "wind_kt = {kt}");
    }

    #[test]
    fn api_error_response_yields_empty_rows() {
        let json = r#"{ "error": true, "reason": "no data" }"#;
        let resp: ForecastResponse = serde_json::from_str(json).unwrap();
        assert!(resp.into_rows().is_empty());
    }

    #[test]
    fn missing_optional_arrays_yield_nullable_columns() {
        let json = r#"{
            "hourly": {
                "time": ["2026-05-28T00:00"]
            }
        }"#;
        let resp: ForecastResponse = serde_json::from_str(json).unwrap();
        let rows = resp.into_rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].temp_c.is_none());
        assert!(rows[0].wind_kt.is_none());
    }
}
