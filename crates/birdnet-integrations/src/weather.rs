//! Weather client for the O-23 signal-context overlay, over one of three
//! upstreams (`G-26`).
//!
//! Off by default. The poll loop only starts when both:
//!
//! 1. `BNB_WEATHER_ENABLED=1` is set in the environment, AND
//! 2. The caller supplies station coordinates (otherwise there's
//!    nothing to fetch for).
//!
//! # Which upstream, and why there are three
//!
//! [`Provider::OpenMeteo`] stays the default: free, no API key, terms that
//! allow non-commercial use, and self-hostable — an operator uneasy about the
//! third-party fetch can run their own and point `BNB_WEATHER_BASE_URL` at it.
//!
//! [`Provider::MetNorway`] is the Norwegian Meteorological Institute's
//! Locationforecast. Also keyless, and a better model over Europe. Its terms
//! require a User-Agent that identifies the application and gives a contact
//! address; [`Client::new_for`] sets one, and a generic agent is refused with
//! `403` by the service.
//!
//! [`Provider::Wunderground`] is the one that is not a forecast at all. An
//! operator running a **personal weather station** has an anemometer ten
//! metres from the microphone, and ground truth at the microphone is a far
//! better covariate for bird activity than any gridded model. It needs the
//! station's own id and an API key.
//!
//! An enum rather than a trait. The set is closed — these three and whatever
//! is added deliberately — and `birdnet-integrations` constructs no runtime and
//! carries no `async-trait`, so a trait here would either be
//! dyn-incompatible (`async fn` in traits) or need a boxed-future dance for no
//! gain. Exhaustive `match` is also what makes a fourth provider *fail to
//! compile* until every site handles it, which is the property that matters.
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

/// MET Norway's Locationforecast host.
pub const MET_NORWAY_BASE_URL: &str = "https://api.met.no";

/// The Weather Company's personal-weather-station host.
pub const WUNDERGROUND_BASE_URL: &str = "https://api.weather.com";

/// Where the station's weather comes from.
///
/// See the module docs for why this is an enum and not a trait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    /// Open-Meteo's gridded forecast. The default, and self-hostable.
    OpenMeteo,
    /// MET Norway's Locationforecast. Keyless; requires an identifying
    /// User-Agent.
    MetNorway,
    /// A personal weather station's own current observations.
    Wunderground {
        /// The station's Weather Underground id, e.g. `KMAHANOV10`.
        station_id: String,
        /// The API key the station's owner holds.
        api_key: String,
    },
}

impl Provider {
    /// Every provider's configuration name, in the order the docs list them.
    pub const NAMES: &'static [&'static str] = &["open-meteo", "met-no", "wunderground"];

    /// The name an operator writes in the config.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::OpenMeteo => "open-meteo",
            Self::MetNorway => "met-no",
            Self::Wunderground { .. } => "wunderground",
        }
    }

    /// The name to show a person.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::OpenMeteo => "Open-Meteo",
            Self::MetNorway => "MET Norway",
            Self::Wunderground { .. } => "Weather Underground (personal station)",
        }
    }

    /// This provider's default host.
    #[must_use]
    pub const fn default_base_url(&self) -> &'static str {
        match self {
            Self::OpenMeteo => DEFAULT_BASE_URL,
            Self::MetNorway => MET_NORWAY_BASE_URL,
            Self::Wunderground { .. } => WUNDERGROUND_BASE_URL,
        }
    }

    /// Resolve a provider from its configured name and credentials.
    ///
    /// An unknown name is an error rather than a silent fall back to the
    /// default: a station configured for a personal weather station and
    /// quietly served a gridded forecast instead is worse than one that
    /// refuses to start the poll and says why.
    ///
    /// # Errors
    ///
    /// [`WeatherError::Config`] for an unknown name, or for `wunderground`
    /// without both a station id and an API key — the two things it cannot
    /// work without, and the failure would otherwise be a `401` every half
    /// hour in a log nobody reads.
    pub fn resolve(
        name: &str,
        station_id: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Self, WeatherError> {
        let trimmed = name.trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "" | "open-meteo" | "openmeteo" => Ok(Self::OpenMeteo),
            "met-no" | "metno" | "yr" | "yr.no" => Ok(Self::MetNorway),
            "wunderground" | "wu" | "pws" => {
                let station_id = station_id.map(str::trim).filter(|s| !s.is_empty());
                let api_key = api_key.map(str::trim).filter(|s| !s.is_empty());
                match (station_id, api_key) {
                    (Some(station_id), Some(api_key)) => Ok(Self::Wunderground {
                        station_id: station_id.to_owned(),
                        api_key: api_key.to_owned(),
                    }),
                    _ => Err(WeatherError::Config(
                        "the wunderground provider needs both WEATHER_STATION_ID and \
                         WEATHER_API_KEY (the key may be mounted as a file with \
                         BIRDNET_WEATHER_API_KEY_FILE)"
                            .into(),
                    )),
                }
            }
            other => Err(WeatherError::Config(format!(
                "unknown weather provider {other:?}; expected one of {}",
                Self::NAMES.join(", ")
            ))),
        }
    }
}

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
    /// The upstream returned a non-success response.
    Api(String),
    /// The provider cannot be used as configured.
    Config(String),
}

impl std::fmt::Display for WeatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "http error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Api(m) => write!(f, "api error: {m}"),
            Self::Config(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for WeatherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Decode(e) => Some(e),
            Self::Api(_) | Self::Config(_) => None,
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

/// Async client for whichever upstream the station is configured for.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    provider: Provider,
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

    /// Build a client for `provider`, at `base_url` or the provider's default.
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Http`] if the underlying HTTP client cannot
    /// be constructed.
    pub fn new_for(provider: Provider, base_url: Option<String>) -> Result<Self, WeatherError> {
        // MET Norway's terms require a User-Agent that identifies the
        // application and gives a contact address; a generic one is answered
        // with 403. The same string is sent to every provider — there is no
        // reason to be less identifiable to the others.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!(
                "BirdNet-Behavior/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/tomtom215/BirdNet-Behavior)"
            ))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.unwrap_or_else(|| provider.default_base_url().to_owned()),
            provider,
        })
    }

    /// Which upstream this client asks.
    #[must_use]
    pub const fn provider(&self) -> &Provider {
        &self.provider
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
            provider: Provider::OpenMeteo,
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
        match &self.provider {
            Provider::OpenMeteo => self.fetch_open_meteo(lat, lon).await,
            Provider::MetNorway => self.fetch_met_norway(lat, lon).await,
            Provider::Wunderground {
                station_id,
                api_key,
            } => self.fetch_wunderground(station_id, api_key).await,
        }
    }

    async fn fetch_open_meteo(&self, lat: f64, lon: f64) -> Result<Vec<WeatherRow>, WeatherError> {
        let url = format!(
            "{base}/v1/forecast?latitude={lat}&longitude={lon}\
             &hourly=temperature_2m,precipitation,wind_speed_10m,wind_direction_10m,\
             pressure_msl,cloud_cover,weather_code\
             &past_days=1&forecast_days=2&timezone=UTC",
            base = self.base_url
        );
        let body: ForecastResponse = self.get_json(&url, "open-meteo").await?;
        Ok(body.into_rows())
    }

    async fn fetch_met_norway(&self, lat: f64, lon: f64) -> Result<Vec<WeatherRow>, WeatherError> {
        // MET Norway asks that coordinates be truncated to four decimals — any
        // more is a distinct cache key for a position no forecast resolves.
        let url = format!(
            "{base}/weatherapi/locationforecast/2.0/compact?lat={lat:.4}&lon={lon:.4}",
            base = self.base_url
        );
        let body: MetNorwayResponse = self.get_json(&url, "met.no").await?;
        Ok(body.into_rows())
    }

    async fn fetch_wunderground(
        &self,
        station_id: &str,
        api_key: &str,
    ) -> Result<Vec<WeatherRow>, WeatherError> {
        // `units=m` asks for metric: °C, km/h, hPa, mm. Everything downstream
        // assumes it.
        let url = format!(
            "{base}/v2/pws/observations/current?stationId={station_id}\
             &format=json&units=m&apiKey={api_key}",
            base = self.base_url
        );
        let body: WundergroundResponse = self.get_json(&url, "wunderground").await?;
        Ok(body.into_rows())
    }

    /// Fetch and decode, turning a non-success status into a named API error.
    ///
    /// The URL is deliberately **not** in the error: the Wunderground one
    /// carries the station's API key in its query string, and an error that
    /// logs the URL puts that key in the journal every half hour.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        who: &str,
    ) -> Result<T, WeatherError> {
        let res = self.http.get(url).send().await?;
        if !res.status().is_success() {
            return Err(WeatherError::Api(format!(
                "{who} returned {}",
                res.status()
            )));
        }
        Ok(res.json().await?)
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
        // model. See [`MS_TO_KT`], shared with the MET Norway decoder.
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

/// MET Norway's Locationforecast 2.0 `compact` response.
///
/// Shape and units read off a live response (`api.met.no`, 2026-09-11,
/// `lat=52.52&lon=13.405`), not from memory: `properties.meta.units` states
/// `celsius`, `m/s`, `degrees`, `hPa`, `%` and `mm`, which is what the
/// conversions below assume.
#[derive(Debug, Deserialize)]
struct MetNorwayResponse {
    #[serde(default)]
    properties: Option<MetNorwayProperties>,
}

#[derive(Debug, Deserialize)]
struct MetNorwayProperties {
    #[serde(default)]
    timeseries: Vec<MetNorwayEntry>,
}

#[derive(Debug, Deserialize)]
struct MetNorwayEntry {
    time: String,
    data: MetNorwayData,
}

#[derive(Debug, Deserialize)]
struct MetNorwayData {
    #[serde(default)]
    instant: Option<MetNorwayInstant>,
    /// Present only while the forecast is still hourly. The live response has
    /// it on the first 64 of 91 entries and then drops to six-hour steps, so a
    /// missing block is ordinary rather than an error.
    #[serde(default)]
    next_1_hours: Option<MetNorwayPeriod>,
}

#[derive(Debug, Deserialize)]
struct MetNorwayInstant {
    #[serde(default)]
    details: Option<MetNorwayDetails>,
}

#[derive(Debug, Deserialize)]
struct MetNorwayDetails {
    #[serde(default)]
    air_temperature: Option<f32>,
    #[serde(default)]
    wind_speed: Option<f32>,
    #[serde(default)]
    wind_from_direction: Option<f32>,
    #[serde(default)]
    air_pressure_at_sea_level: Option<f32>,
    #[serde(default)]
    cloud_area_fraction: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct MetNorwayPeriod {
    #[serde(default)]
    details: Option<MetNorwayPeriodDetails>,
}

#[derive(Debug, Deserialize)]
struct MetNorwayPeriodDetails {
    #[serde(default)]
    precipitation_amount: Option<f32>,
}

impl MetNorwayResponse {
    fn into_rows(self) -> Vec<WeatherRow> {
        let Some(properties) = self.properties else {
            return Vec::new();
        };
        properties
            .timeseries
            .into_iter()
            .map(|entry| {
                let details = entry.data.instant.and_then(|i| i.details);
                let precip = entry
                    .data
                    .next_1_hours
                    .and_then(|p| p.details)
                    .and_then(|d| d.precipitation_amount);
                WeatherRow {
                    at: entry.time,
                    temp_c: details.as_ref().and_then(|d| d.air_temperature),
                    precip_mm: precip,
                    wind_kt: details
                        .as_ref()
                        .and_then(|d| d.wind_speed)
                        .map(|v| v * MS_TO_KT),
                    // Degrees arrive as a float; the column is whole degrees.
                    wind_dir_deg: details
                        .as_ref()
                        .and_then(|d| d.wind_from_direction)
                        .map(round_to_degrees),
                    pressure_hpa: details.as_ref().and_then(|d| d.air_pressure_at_sea_level),
                    cloud_pct: details
                        .as_ref()
                        .and_then(|d| d.cloud_area_fraction)
                        .map(round_to_degrees),
                    // MET Norway describes the sky with a `symbol_code` string
                    // ("partlycloudy_day"), not a WMO number. Mapping one to
                    // the other would be a table of guesses; the column stays
                    // empty rather than carrying an invented code.
                    code: None,
                }
            })
            .collect()
    }
}

/// A personal weather station's current observation, from
/// `GET /v2/pws/observations/current`.
///
/// **Written against the published response shape, not a live call**: the
/// endpoint needs a station owner's API key, which this repository does not
/// have, so unlike the MET Norway decoder above this one is *not* verified
/// against real bytes. The test below pins the documented sample. Treat a
/// field that never arrives as the first thing to check if a station reports
/// nulls.
#[derive(Debug, Deserialize)]
struct WundergroundResponse {
    #[serde(default)]
    observations: Vec<WundergroundObservation>,
}

#[derive(Debug, Deserialize)]
struct WundergroundObservation {
    #[serde(default)]
    #[serde(rename = "obsTimeUtc")]
    obs_time_utc: Option<String>,
    #[serde(default)]
    winddir: Option<i32>,
    #[serde(default)]
    metric: Option<WundergroundMetric>,
}

#[derive(Debug, Deserialize)]
struct WundergroundMetric {
    #[serde(default)]
    temp: Option<f32>,
    /// km/h under `units=m`.
    #[serde(rename = "windSpeed")]
    #[serde(default)]
    wind_speed: Option<f32>,
    #[serde(default)]
    pressure: Option<f32>,
    /// mm per hour. **Not** `precipTotal`, which accumulates since local
    /// midnight — storing that in a column documented as "precipitation
    /// accumulated during this hour" would read as a downpour by evening on a
    /// day with one morning shower.
    #[serde(rename = "precipRate")]
    #[serde(default)]
    precip_rate: Option<f32>,
}

impl WundergroundResponse {
    fn into_rows(self) -> Vec<WeatherRow> {
        self.observations
            .into_iter()
            .filter_map(|obs| {
                // Without a timestamp there is no row: `at` is the table's key.
                let at = obs.obs_time_utc?;
                let metric = obs.metric;
                Some(WeatherRow {
                    at,
                    temp_c: metric.as_ref().and_then(|m| m.temp),
                    precip_mm: metric.as_ref().and_then(|m| m.precip_rate),
                    wind_kt: metric
                        .as_ref()
                        .and_then(|m| m.wind_speed)
                        .map(|v| v * KMH_TO_KT),
                    wind_dir_deg: obs.winddir,
                    pressure_hpa: metric.as_ref().and_then(|m| m.pressure),
                    // A personal station measures the air, not the sky.
                    cloud_pct: None,
                    code: None,
                })
            })
            .collect()
    }
}

/// 1 m/s in knots.
const MS_TO_KT: f32 = 1.943_844_5;

/// 1 km/h in knots.
const KMH_TO_KT: f32 = 0.539_956_8;

/// A float bearing or percentage as the whole number the column stores.
#[expect(
    clippy::cast_possible_truncation,
    reason = "bearings are 0-360 and percentages 0-100; both fit i32 with room to spare"
)]
const fn round_to_degrees(v: f32) -> i32 {
    v.round() as i32
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

// ── three upstreams, one row shape (G-26) ───────────────────────────────
//
// These gates were written against the single-provider client and observed
// failing: `Provider` did not exist, `Client::new_for` did not exist, and
// `fetch_hourly` had no match to dispatch on, so each one stops at a
// compile error naming what is missing.
//
// The MET Norway fixture is a **real response**, captured from
// `api.met.no/weatherapi/locationforecast/2.0/compact?lat=52.52&lon=13.405`
// on 2026-09-11 and trimmed to three of its 91 entries — two hourly and one
// from the six-hourly tail. The Wunderground sample is the documented shape;
// that endpoint needs a station owner's key, which is why its decoder carries
// a note saying it is unverified against real bytes.
#[cfg(test)]
mod provider_tests {
    use super::*;

    /// Verbatim bytes from a live MET Norway response.
    const MET_NORWAY: &str = include_str!("testdata/met_norway_compact.json");

    #[test]
    fn a_provider_name_resolves_to_the_provider_it_names() {
        for (name, expected) in [
            ("open-meteo", Provider::OpenMeteo),
            ("openmeteo", Provider::OpenMeteo),
            ("OPEN-METEO", Provider::OpenMeteo),
            // Unset is the default, which is what keeps an existing station
            // on Open-Meteo without touching its config.
            ("", Provider::OpenMeteo),
            ("  met-no  ", Provider::MetNorway),
            ("yr.no", Provider::MetNorway),
            ("MetNo", Provider::MetNorway),
        ] {
            assert_eq!(
                Provider::resolve(name, None, None).expect("resolves"),
                expected,
                "for {name:?}"
            );
        }

        let wu = Provider::resolve("wunderground", Some(" KMAHANOV10 "), Some(" abc123 "))
            .expect("resolves with credentials");
        assert_eq!(
            wu,
            Provider::Wunderground {
                station_id: "KMAHANOV10".to_owned(),
                api_key: "abc123".to_owned(),
            },
            "both halves are trimmed"
        );
    }

    /// The counterpart: a name nobody implements is refused rather than
    /// quietly served by the default. A station configured for its own
    /// anemometer and given a gridded forecast instead would look like it
    /// was working.
    #[test]
    fn an_unknown_provider_is_refused_and_names_the_ones_that_exist() {
        let err = Provider::resolve("accuweather", None, None)
            .expect_err("an unimplemented provider must not fall back");
        assert!(matches!(err, WeatherError::Config(_)), "{err:?}");
        let msg = err.to_string();
        for name in Provider::NAMES {
            assert!(msg.contains(name), "the refusal must list {name}: {msg}");
        }
    }

    /// Wunderground without its two credentials is refused at resolve time,
    /// not discovered as a 401 every half hour.
    #[test]
    fn wunderground_without_both_credentials_is_refused_up_front() {
        for (station, key) in [
            (None, None),
            (Some("KMAHANOV10"), None),
            (None, Some("abc123")),
            (Some("   "), Some("abc123")),
            (Some("KMAHANOV10"), Some("")),
        ] {
            match Provider::resolve("wunderground", station, key) {
                Err(WeatherError::Config(_)) => {}
                other => panic!(
                    "station {station:?} with key {key:?} must be refused as a \
                     configuration error, got {other:?}"
                ),
            }
        }
    }

    /// The real MET Norway response decodes to the values it states, in the
    /// units `properties.meta.units` declares.
    #[test]
    fn met_norway_rows_carry_the_values_the_response_states() {
        let resp: MetNorwayResponse = serde_json::from_str(MET_NORWAY).expect("real response");
        let rows = resp.into_rows();
        assert_eq!(rows.len(), 3, "one row per timeseries entry");

        let first = &rows[0];
        assert_eq!(
            first.at, "2026-09-11T08:00:00Z",
            "already second-precision Z"
        );
        assert!((first.temp_c.unwrap() - 15.2).abs() < 1e-4);
        assert!((first.pressure_hpa.unwrap() - 1020.8).abs() < 1e-3);
        // 1.6 m/s in knots.
        assert!(
            (first.wind_kt.unwrap() - 3.110_151_2).abs() < 1e-4,
            "wind_kt = {:?}",
            first.wind_kt
        );
        // Bearings and cloud fractions arrive as floats and are stored whole.
        assert_eq!(first.wind_dir_deg, Some(170), "169.6 rounds to 170");
        assert_eq!(first.cloud_pct, Some(97), "96.9 rounds to 97");
        assert_eq!(first.precip_mm, Some(0.0), "from next_1_hours");
        assert_eq!(
            first.code, None,
            "met.no describes the sky with a symbol string, not a WMO code; \
             inventing one would be a guess in a column that reads as fact"
        );
    }

    /// The six-hourly tail has no `next_1_hours` block at all. That is
    /// ordinary — the live response drops to six-hour steps after 64 of its 91
    /// entries — so the row must still exist, with no precipitation rather
    /// than no row.
    #[test]
    fn a_met_norway_entry_without_an_hourly_block_still_yields_its_instant() {
        let resp: MetNorwayResponse = serde_json::from_str(MET_NORWAY).expect("real response");
        let rows = resp.into_rows();
        let tail = rows.last().expect("the six-hourly entry");
        assert_eq!(tail.at, "2026-09-14T00:00:00Z");
        assert!(
            tail.temp_c.is_some(),
            "the instant is still there: {tail:?}"
        );
        assert_eq!(tail.precip_mm, None, "and no hourly precipitation to read");
    }

    /// An empty or malformed envelope is no rows, not a panic.
    #[test]
    fn a_met_norway_response_without_properties_yields_no_rows() {
        let resp: MetNorwayResponse = serde_json::from_str("{}").expect("decodes");
        assert!(resp.into_rows().is_empty());
    }

    /// The documented Wunderground shape, decoded.
    #[test]
    fn a_wunderground_observation_converts_to_one_row() {
        let json = r#"{"observations":[{
            "stationID":"KMAHANOV10",
            "obsTimeUtc":"2026-09-11T08:53:14Z",
            "humidity":60,
            "winddir":186,
            "metric":{"temp":2,"windSpeed":8,"pressure":1012.16,
                      "precipRate":1.5,"precipTotal":9.9,"dewpt":-5}
        }]}"#;
        let resp: WundergroundResponse = serde_json::from_str(json).expect("documented shape");
        let rows = resp.into_rows();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.at, "2026-09-11T08:53:14Z");
        assert!((row.temp_c.unwrap() - 2.0).abs() < 1e-4);
        assert!((row.pressure_hpa.unwrap() - 1012.16).abs() < 1e-2);
        assert_eq!(row.wind_dir_deg, Some(186));
        // 8 km/h in knots.
        assert!(
            (row.wind_kt.unwrap() - 4.319_654_4).abs() < 1e-4,
            "wind_kt = {:?} — km/h, not m/s",
            row.wind_kt
        );
        assert_eq!(
            row.precip_mm,
            Some(1.5),
            "the hourly *rate*, not precipTotal (9.9) which accumulates since \
             local midnight and would read as a downpour by evening"
        );
        assert_eq!(
            row.cloud_pct, None,
            "a personal station measures the air, not the sky"
        );
    }

    /// Without a timestamp there is no row: `at` is the weather table's key,
    /// and a row keyed on nothing would overwrite whatever sorts first.
    #[test]
    fn a_wunderground_observation_without_a_timestamp_is_dropped() {
        let json = r#"{"observations":[
            {"metric":{"temp":2}},
            {"obsTimeUtc":"2026-09-11T08:53:14Z","metric":{"temp":3}}
        ]}"#;
        let resp: WundergroundResponse = serde_json::from_str(json).expect("decodes");
        let rows = resp.into_rows();
        assert_eq!(rows.len(), 1, "only the one that can be keyed: {rows:?}");
        assert!((rows[0].temp_c.unwrap() - 3.0).abs() < 1e-4);
    }

    #[test]
    fn a_wunderground_response_with_no_observations_yields_no_rows() {
        let resp: WundergroundResponse =
            serde_json::from_str(r#"{"observations":[]}"#).expect("decodes");
        assert!(resp.into_rows().is_empty());
    }

    /// Every provider has a distinct name, label and default host, and the
    /// `NAMES` list is the set `resolve` accepts. A name in the docs that
    /// `resolve` rejects is the drift this catches.
    #[test]
    fn every_listed_name_resolves_and_round_trips() {
        assert_eq!(Provider::NAMES.len(), 3);
        for name in Provider::NAMES {
            let provider = Provider::resolve(name, Some("id"), Some("key"))
                .unwrap_or_else(|e| panic!("NAMES lists {name}, which does not resolve: {e}"));
            assert_eq!(
                provider.key(),
                *name,
                "key() must round-trip the listed name"
            );
            assert!(!provider.label().is_empty());
            assert!(
                provider.default_base_url().starts_with("https://"),
                "{name} has no default host"
            );
        }
        // And the three are actually different upstreams, not one aliased.
        let hosts: std::collections::BTreeSet<&str> = Provider::NAMES
            .iter()
            .map(|n| {
                Provider::resolve(n, Some("id"), Some("key"))
                    .expect("resolves")
                    .default_base_url()
            })
            .collect();
        assert_eq!(hosts.len(), 3, "each provider has its own host: {hosts:?}");
    }

    /// The provider decides **which endpoint is actually requested**.
    ///
    /// Nothing else here proves that: every other gate exercises a decoder or a
    /// struct field, and a `fetch_hourly` whose `match` sent every provider to
    /// the same place would pass all of them. So this stands up a listener,
    /// points the client at it, and reads the request line off the socket.
    ///
    /// Observed failing with `Provider::MetNorway` wired to `fetch_open_meteo`:
    /// the path came back as `/v1/forecast`.
    #[tokio::test]
    async fn each_provider_asks_its_own_endpoint() {
        for (provider, expected_path, expect_in_query) in [
            (Provider::OpenMeteo, "/v1/forecast", vec!["latitude=52.52"]),
            (
                Provider::MetNorway,
                "/weatherapi/locationforecast/2.0/compact",
                vec!["lat=52.5200", "lon=13.4050"],
            ),
            (
                Provider::Wunderground {
                    station_id: "KMAHANOV10".to_owned(),
                    api_key: "secret-key".to_owned(),
                },
                "/v2/pws/observations/current",
                vec!["stationId=KMAHANOV10", "units=m"],
            ),
        ] {
            let (addr, request) = spawn_one_shot_server().await;
            let client =
                Client::new_for(provider.clone(), Some(format!("http://{addr}"))).expect("client");
            // The body is empty JSON, so decoding yields no rows; the request
            // line is what this gate is about.
            let _ = client.fetch_hourly(52.52, 13.405).await;

            let line = request.await.expect("the server saw a request");
            let target = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
            assert_eq!(
                path,
                expected_path,
                "{} must ask {expected_path}, asked {path}",
                provider.key()
            );
            for needle in expect_in_query {
                assert!(
                    query.contains(needle),
                    "{}'s query is missing {needle}: {query}",
                    provider.key()
                );
            }
        }
    }

    /// A listener that accepts one request, hands back its request line, and
    /// answers with an empty JSON body.
    async fn spawn_one_shot_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Content-Length: 2\r\n\r\n{}",
                )
                .await;
            let _ = socket.shutdown().await;
            request.lines().next().unwrap_or_default().to_owned()
        });
        (addr, handle)
    }

    /// A client built for a provider keeps it, and takes the provider's own
    /// host unless one is given.
    #[test]
    fn a_client_built_for_a_provider_points_at_that_providers_host() {
        let met = Client::new_for(Provider::MetNorway, None).expect("client");
        assert_eq!(met.provider(), &Provider::MetNorway);
        assert_eq!(met.base_url, MET_NORWAY_BASE_URL);

        let self_hosted = Client::new_for(
            Provider::OpenMeteo,
            Some("http://localhost:8080".to_owned()),
        )
        .expect("client");
        assert_eq!(self_hosted.base_url, "http://localhost:8080");
        assert_eq!(self_hosted.provider(), &Provider::OpenMeteo);
    }
}
