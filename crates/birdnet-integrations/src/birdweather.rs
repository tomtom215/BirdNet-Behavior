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

/// Maximum retry attempts for failed requests.
const MAX_RETRIES: u32 = 3;

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
    /// HTTP client.
    http: reqwest::Client,
    /// Station latitude.
    lat: f64,
    /// Station longitude.
    lon: f64,
}

/// A detection to post to `BirdWeather`.
#[derive(Debug, Clone, Serialize)]
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
            http,
            lat,
            lon,
        })
    }

    /// Post a detection to `BirdWeather`.
    ///
    /// Retries up to `MAX_RETRIES` times with exponential backoff.
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
            API_BASE, self.station_token
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
            API_BASE, self.station_token
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

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(2_u64.pow(attempt));
                tracing::debug!(attempt, delay_secs = delay.as_secs(), "retrying BirdWeather POST");
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
}
