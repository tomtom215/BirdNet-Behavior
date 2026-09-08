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
    /// RFC 3339 timestamp, the station's local wall clock with its offset.
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
    /// The soundscape this detection was heard in, from
    /// [`Client::post_soundscape`], with where in it the detection sits.
    /// `None` when the clip could not be uploaded: the detection still goes,
    /// as it always did, and is simply unverifiable there. `#[serde(default)]`
    /// so a payload parked before this field existed still replays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soundscape: Option<SoundscapeRef>,
    /// The classifier, in `BirdWeather`'s vocabulary — `2p4` for the V2.4
    /// model, which is the one value both reference projects send. `None`
    /// for a model the API has no name for; the field is optional there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
}

/// Where a detection sits in an uploaded soundscape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, serde::Deserialize)]
pub struct SoundscapeRef {
    /// The id `BirdWeather` assigned the soundscape.
    pub id: u64,
    /// Seconds into the soundscape the detection starts.
    pub start_secs: f32,
    /// Seconds into the soundscape the detection ends.
    pub end_secs: f32,
}

/// `BirdWeather`'s name for a classifier, from the model file's name.
///
/// BirdNET-Pi sends `2p4` for `BirdNET_GLOBAL_6K_V2.4_Model_FP16` and
/// `alpha` for its older bundled model; birdnet-go sends `2p4` regardless.
/// Only the value both agree on is sent, and only for the model it names;
/// anything else is left out rather than mislabelled.
#[must_use]
pub fn algorithm_for_model(model_name: &str) -> Option<&'static str> {
    model_name.contains("V2.4").then_some("2p4")
}

/// Response from `BirdWeather` API.
#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    /// Whether the request succeeded.
    pub success: bool,
    /// Optional error message.
    pub message: Option<String>,
}

/// A soundscape to upload: the clip's bytes and when it starts.
#[derive(Debug, Clone)]
pub struct SoundscapePost {
    /// RFC 3339 timestamp of the clip's first sample, local with offset.
    pub timestamp: String,
    /// The audio file, whole.
    pub audio: Vec<u8>,
    /// Its extension — `wav`, `flac`, `mp3`, `ogg` — which the API is told
    /// as `?type=`, and its MIME type for `Content-Type`.
    pub extension: String,
    /// The `Content-Type` for `audio`.
    pub content_type: String,
}

/// What `BirdWeather` answers a soundscape upload with.
#[derive(Debug, Deserialize)]
pub struct SoundscapeResponse {
    /// Whether the upload succeeded.
    pub success: bool,
    /// The stored soundscape.
    pub soundscape: Option<StoredSoundscape>,
    /// Optional error message.
    pub message: Option<String>,
}

/// The soundscape as `BirdWeather` stored it.
#[derive(Debug, Deserialize)]
pub struct StoredSoundscape {
    /// The id every detection heard in it is posted with.
    pub id: u64,
}

/// The JSON `BirdWeather` expects for a detection: the six fields every post
/// carried, plus the soundscape reference and algorithm when known.
fn detection_body(detection: &DetectionPost) -> serde_json::Value {
    let mut body = serde_json::json!({
        "timestamp": detection.timestamp,
        "lat": detection.lat,
        "lon": detection.lon,
        "commonName": detection.common_name,
        "scientificName": detection.scientific_name,
        "confidence": detection.confidence,
    });
    if let Some(s) = detection.soundscape {
        body["soundscapeId"] = serde_json::json!(s.id);
        body["soundscapeStartTime"] = serde_json::json!(s.start_secs);
        body["soundscapeEndTime"] = serde_json::json!(s.end_secs);
    }
    if let Some(a) = &detection.algorithm {
        body["algorithm"] = serde_json::json!(a);
    }
    body
}

/// Percent-encode a query value: everything but unreserved characters.
fn urlencode(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            // Infallible: writing to a String never errors.
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
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

        let body = detection_body(detection);

        self.post_with_retry(&url, &body).await
    }

    /// Upload a soundscape and return the id `BirdWeather` gave it.
    ///
    /// The shape both reference projects use: the audio bytes as the body,
    /// `?timestamp=` (and `&type=`) on the URL, the id in
    /// `{"success":true,"soundscape":{"id":…}}`. One attempt — a clip is a
    /// megabyte on a metered uplink, and the detection post that follows
    /// goes with or without it.
    ///
    /// # Errors
    ///
    /// Returns `BirdWeatherError` on network failure, a non-2xx answer, or a
    /// 2xx whose body carries no soundscape id.
    pub async fn post_soundscape(
        &self,
        soundscape: SoundscapePost,
    ) -> Result<u64, BirdWeatherError> {
        let url = format!(
            "{}/stations/{}/soundscapes?timestamp={}&type={}",
            self.base_url,
            self.station_token,
            urlencode(&soundscape.timestamp),
            urlencode(&soundscape.extension),
        );
        let resp = self
            .http
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, soundscape.content_type)
            .body(soundscape.audio)
            .send()
            .await
            .map_err(|e| BirdWeatherError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BirdWeatherError::Api(format!("{status}: {text}")));
        }
        let parsed = resp
            .json::<SoundscapeResponse>()
            .await
            .map_err(|e| BirdWeatherError::Api(e.to_string()))?;
        match (parsed.success, parsed.soundscape) {
            (true, Some(stored)) => Ok(stored.id),
            (_, _) => {
                Err(BirdWeatherError::Api(parsed.message.unwrap_or_else(|| {
                    "soundscape accepted without an id".to_owned()
                })))
            }
        }
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

    /// The body carries the soundscape reference and algorithm only when
    /// they are known, and a parked payload from before either existed still
    /// deserializes (the store-and-forward queue replays it verbatim).
    #[test]
    fn the_detection_body_carries_the_soundscape_when_there_is_one() {
        let bare = DetectionPost {
            timestamp: "2026-07-01T12:00:00+01:00".into(),
            common_name: "Eurasian Magpie".into(),
            scientific_name: "Pica pica".into(),
            confidence: 0.9,
            lat: 51.5,
            lon: -0.1,
            soundscape: None,
            algorithm: None,
        };
        let body = detection_body(&bare);
        assert!(body.get("soundscapeId").is_none(), "{body}");
        assert!(body.get("algorithm").is_none(), "{body}");
        assert_eq!(body["timestamp"], "2026-07-01T12:00:00+01:00");

        let with = DetectionPost {
            soundscape: Some(SoundscapeRef {
                id: 4242,
                start_secs: 4.5,
                end_secs: 7.5,
            }),
            algorithm: Some("2p4".into()),
            ..bare
        };
        let body = detection_body(&with);
        assert_eq!(body["soundscapeId"], 4242);
        assert_eq!(body["soundscapeStartTime"], 4.5);
        assert_eq!(body["soundscapeEndTime"], 7.5);
        assert_eq!(body["algorithm"], "2p4");

        let parked = r#"{"timestamp":"2026-07-01T12:00:00Z","common_name":"Eurasian Magpie","scientific_name":"Pica pica","confidence":0.9,"lat":51.5,"lon":-0.1}"#;
        let replayed: DetectionPost = serde_json::from_str(parked).expect("old payload replays");
        assert!(replayed.soundscape.is_none());
        let round_trip = serde_json::to_string(&with).unwrap();
        let back: DetectionPost = serde_json::from_str(&round_trip).unwrap();
        assert_eq!(back.soundscape, with.soundscape);
    }

    #[test]
    fn the_algorithm_is_named_only_for_the_model_both_references_name() {
        assert_eq!(
            algorithm_for_model("BirdNET_GLOBAL_6K_V2.4_Model_FP16"),
            Some("2p4")
        );
        assert_eq!(
            algorithm_for_model("BirdNET+_V3.0-preview3_Global_11K_FP32"),
            None
        );
        assert_eq!(algorithm_for_model("demo-model"), None);
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(
            urlencode("2026-07-01T12:00:00+01:00"),
            "2026-07-01T12%3A00%3A00%2B01%3A00"
        );
        assert_eq!(urlencode("wav"), "wav");
    }

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
