//! `BirdWeather` API client.
//!
//! Posts soundscapes and detections to `app.birdweather.com`.
//! Includes retry queue with offline buffering for unreliable connections.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// `BirdWeather` API base URL.
const API_BASE: &str = "https://app.birdweather.com/api/v1";

/// Default request timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Total request attempts (initial + retries) before a POST is abandoned.
const MAX_ATTEMPTS: u32 = 3;

/// Tag under which failed uploads are parked in the binary's
/// store-and-forward queue (`outbound_queue` table) for later replay.
pub const QUEUE_KIND: &str = "birdweather";

/// `BirdWeather` client errors.
#[derive(Debug)]
pub enum BirdWeatherError {
    /// HTTP request failed.
    Http(String),
    /// Invalid response from API.
    Api(String),
    /// Station token not configured.
    NoToken,
}

impl fmt::Display for BirdWeatherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "BirdWeather HTTP error: {msg}"),
            Self::Api(msg) => write!(f, "BirdWeather API error: {msg}"),
            Self::NoToken => write!(f, "BirdWeather station token not configured"),
        }
    }
}

impl std::error::Error for BirdWeatherError {}

/// `BirdWeather` API client.
#[derive(Debug, Clone)]
pub struct Client {
    /// Station token (from `BirdWeather` settings).
    station_token: String,
    /// API base (no trailing slash). [`API_BASE`] in production; overridden
    /// via [`Client::with_base_url`] for self-hosted ingests and the
    /// store-and-forward end-to-end test's stub server.
    base_url: String,
    /// HTTP client.
    http: reqwest::Client,
    /// Station latitude.
    lat: f64,
    /// Station longitude.
    lon: f64,
}

/// A detection to post to `BirdWeather`.
///
/// `Deserialize` is required by the store-and-forward queue: a post that
/// fails during a network outage is parked as JSON in the local database
/// and replayed verbatim by the drainer once the uplink returns.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct DetectionPost {
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Common name of the detected species.
    pub common_name: String,
    /// Scientific name.
    pub scientific_name: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// Latitude.
    pub lat: f64,
    /// Longitude.
    pub lon: f64,
}

/// Response from `BirdWeather` API.
#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    /// Whether the request succeeded.
    pub success: bool,
    /// Optional error message.
    pub message: Option<String>,
}

/// Soundscape upload metadata.
#[derive(Debug, Clone, Serialize)]
pub struct SoundscapePost {
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Latitude.
    pub lat: f64,
    /// Longitude.
    pub lon: f64,
}

impl Client {
    /// Create a new `BirdWeather` client.
    ///
    /// # Errors
    ///
    /// Returns `BirdWeatherError::NoToken` if the token is empty.
    pub fn new(station_token: &str, lat: f64, lon: f64) -> Result<Self, BirdWeatherError> {
        if station_token.is_empty() {
            return Err(BirdWeatherError::NoToken);
        }

        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| BirdWeatherError::Http(e.to_string()))?;

        Ok(Self {
            station_token: station_token.to_string(),
            base_url: API_BASE.to_owned(),
            http,
            lat,
            lon,
        })
    }

    /// Redirect this client at a different API base.
    ///
    /// Two audiences: researchers running a **self-hosted ingest** (rare /
    /// endangered-species programmes that must keep observation data under
    /// their own governance rather than a public community map), and the
    /// end-to-end test suite, which points the real binary's drainer at a
    /// local stub to prove the replay -> deliver -> dequeue loop. A
    /// trailing slash is tolerated; an empty override keeps the default so
    /// a blank env var cannot produce `"/stations/..."` relative URLs.
    #[must_use]
    pub fn with_base_url(mut self, base_url: &str) -> Self {
        let trimmed = base_url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            trimmed.clone_into(&mut self.base_url);
        }
        self
    }

    /// The API base requests are sent to (no trailing slash).
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Post a detection to `BirdWeather`.
    ///
    /// Makes up to `MAX_ATTEMPTS` attempts (initial + retries) with exponential backoff.
    ///
    /// # Errors
    ///
    /// Returns `BirdWeatherError` on network or API failure.
    pub async fn post_detection(
        &self,
        detection: &DetectionPost,
    ) -> Result<ApiResponse, BirdWeatherError> {
        let url = format!(
            "{}/stations/{}/detections",
            self.base_url, self.station_token
        );

        let body = serde_json::json!({
            "timestamp": detection.timestamp,
            "lat": detection.lat,
            "lon": detection.lon,
            "commonName": detection.common_name,
            "scientificName": detection.scientific_name,
            "confidence": detection.confidence,
        });

        self.post_with_retry(&url, &body).await
    }

    /// Post a soundscape to `BirdWeather`.
    ///
    /// # Errors
    ///
    /// Returns `BirdWeatherError` on network or API failure.
    pub async fn post_soundscape(
        &self,
        soundscape: &SoundscapePost,
    ) -> Result<ApiResponse, BirdWeatherError> {
        let url = format!(
            "{}/stations/{}/soundscapes",
            self.base_url, self.station_token
        );

        let body = serde_json::json!({
            "timestamp": soundscape.timestamp,
            "lat": soundscape.lat,
            "lon": soundscape.lon,
        });

        self.post_with_retry(&url, &body).await
    }

    /// Get the station token.
    pub fn token(&self) -> &str {
        &self.station_token
    }

    /// Get station coordinates.
    pub const fn coordinates(&self) -> (f64, f64) {
        (self.lat, self.lon)
    }

    /// POST with exponential backoff retry.
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<ApiResponse, BirdWeatherError> {
        let mut last_error = BirdWeatherError::Http("no attempts made".into());

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                // Jittered, capped exponential backoff so concurrent retries —
                // and many stations hitting the same endpoint — don't
                // synchronise into a thundering herd.
                let delay = crate::retry::backoff_delay(attempt, crate::retry::jitter_frac());
                tracing::debug!(
                    attempt,
                    delay_secs = delay.as_secs(),
                    "retrying BirdWeather POST"
                );
                tokio::time::sleep(delay).await;
            }

            match self.http.post(url).json(body).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return resp
                            .json::<ApiResponse>()
                            .await
                            .map_err(|e| BirdWeatherError::Api(e.to_string()));
                    }
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    last_error = BirdWeatherError::Api(format!("{status}: {text}"));
                    // A 4xx (other than 429) is a deterministic client error — a
                    // bad station token or malformed payload won't succeed on
                    // retry, so fail fast instead of burning the backoff budget
                    // (and adding load at fleet scale). Retry only 429 and 5xx.
                    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        return Err(last_error);
                    }
                }
                Err(e) => {
                    last_error = BirdWeatherError::Http(e.to_string());
                }
            }
        }

        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_token_returns_error() {
        let result = Client::new("", 42.0, -71.0);
        assert!(matches!(result, Err(BirdWeatherError::NoToken)));
    }

    #[test]
    fn client_stores_coordinates() {
        let client = Client::new("test-token", 42.36, -71.06).unwrap();
        assert_eq!(client.coordinates(), (42.36, -71.06));
        assert_eq!(client.token(), "test-token");
    }

    #[test]
    fn base_url_defaults_to_public_api() {
        let client = Client::new("t", 0.0, 0.0).unwrap();
        assert_eq!(client.base_url(), API_BASE);
    }

    #[test]
    fn with_base_url_overrides_and_normalises() {
        let client = Client::new("t", 0.0, 0.0)
            .unwrap()
            .with_base_url("http://127.0.0.1:9000/api/v1/");
        // Trailing slash trimmed so the joined URL has exactly one separator.
        assert_eq!(client.base_url(), "http://127.0.0.1:9000/api/v1");
    }

    #[test]
    fn with_base_url_ignores_blank_override() {
        // A blank env var must keep the default, never produce relative URLs.
        let client = Client::new("t", 0.0, 0.0).unwrap().with_base_url("   ");
        assert_eq!(client.base_url(), API_BASE);
    }
}
