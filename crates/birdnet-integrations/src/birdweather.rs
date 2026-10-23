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
    /// The API answered with a non-success HTTP status.
    Status {
        /// The HTTP status code.
        status: u16,
        /// `"<status>: <body>"`, as displayed.
        detail: String,
    },
    /// Station token not configured.
    NoToken,
    /// The station has no coordinates `BirdWeather` could be given.
    Location(LocationProblem),
}

impl BirdWeatherError {
    /// Whether replaying the identical request can never succeed, so the
    /// store-and-forward queue should drop it rather than park it (INT13).
    ///
    /// Only statuses that refuse the *body*: 400, 413, 415, 422. Not 401, 403
    /// or 404 — the station token is in the URL, so those are what a wrong
    /// token looks like, and a queued upload must survive the operator fixing
    /// it. Not 429 or 5xx, which are the outages the queue exists for.
    #[must_use]
    pub const fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::Status {
                status: 400 | 413 | 415 | 422,
                ..
            }
        )
    }

    fn status(status: reqwest::StatusCode, text: &str) -> Self {
        Self::Status {
            status: status.as_u16(),
            detail: format!("{status}: {text}"),
        }
    }
}

impl fmt::Display for BirdWeatherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "BirdWeather HTTP error: {msg}"),
            Self::Api(msg) | Self::Status { detail: msg, .. } => {
                write!(f, "BirdWeather API error: {msg}")
            }
            Self::NoToken => write!(f, "BirdWeather station token not configured"),
            Self::Location(problem) => write!(f, "BirdWeather station location: {problem}"),
        }
    }
}

impl std::error::Error for BirdWeatherError {}

/// Why a station's coordinates cannot be sent to `BirdWeather`.
///
/// Every detection is posted with the station's latitude and longitude, and
/// `BirdWeather` plots them on a public map. A station that has not set its
/// location used to be posted at 0°, 0° — "Null Island", in the Gulf of
/// Guinea — and so was every detection it ever uploaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocationProblem {
    /// Latitude or longitude is not set, or is not a number.
    Unset,
    /// A coordinate lies outside ±90° latitude / ±180° longitude, or is not
    /// finite.
    OutOfRange {
        /// The latitude as given.
        lat: f64,
        /// The longitude as given.
        lon: f64,
    },
    /// Exactly 0°, 0°. Taken to mean "never set" rather than a station in the
    /// open sea: the point is ocean, hundreds of kilometres from the nearest
    /// land, so no station sits on it by design. (A buoy there would report
    /// coordinates that are not exactly zero.)
    NullIsland,
}

impl fmt::Display for LocationProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unset => write!(f, "latitude or longitude is not set"),
            Self::OutOfRange { lat, lon } => write!(
                f,
                "latitude {lat} / longitude {lon} is outside -90..90 / -180..180"
            ),
            Self::NullIsland => write!(
                f,
                "latitude and longitude are both 0, which is the unset default \
                 and a point in the Atlantic, not a station"
            ),
        }
    }
}

/// The coordinates to give `BirdWeather`, or why there are none.
///
/// `None` for either coordinate is [`LocationProblem::Unset`]. The single
/// definition of "a location we will publish", shared by the client, the
/// startup check, `--doctor` and the replay of parked uploads.
///
/// # Errors
///
/// A [`LocationProblem`] naming what is wrong.
pub fn check_location(lat: Option<f64>, lon: Option<f64>) -> Result<(f64, f64), LocationProblem> {
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return Err(LocationProblem::Unset);
    };
    if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
        return Err(LocationProblem::OutOfRange { lat, lon });
    }
    if lat == 0.0 && lon == 0.0 {
        return Err(LocationProblem::NullIsland);
    }
    Ok((lat, lon))
}

/// The widest blur radius [`Client::with_location_accuracy`] accepts, in metres.
///
/// A radius of 50 km already hides which town a station is in; above
/// that it hides which country, and a number that large is a typo (kilometres
/// typed into a metres field).
pub const MAX_LOCATION_ACCURACY_M: f64 = 50_000.0;

/// The mean Earth radius, metres. The destination below is computed on a sphere
/// of this radius, so a point `d` metres out is `d` metres out by haversine too.
/// (birdnet-go divides by 111 000 m per degree, 0.2 % short, so its "within N
/// metres" is N + 0.2 %: the haversine test below put a 5 km blur 9 m out.)
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// How many bytes a station's blur seed has.
pub const BLUR_SEED_LEN: usize = 32;

/// A fresh blur seed from the operating system's generator, or `None` if the
/// generator failed.
///
/// A failure is never papered over with zero bytes: a seed of zeros derives a
/// fixed, public, *known* offset, and a draw of `u = 0` is a distance of 0, the
/// exact position. The caller that gets `None` posts nothing
/// ([`Client::post_location`]).
#[must_use]
pub fn random_blur_seed() -> Option<[u8; BLUR_SEED_LEN]> {
    use ring::rand::{SecureRandom as _, SystemRandom};
    let mut seed = [0_u8; BLUR_SEED_LEN];
    SystemRandom::new().fill(&mut seed).ok()?;
    Some(seed)
}

/// Two uniform numbers in `[0, 1)` that depend on the station's seed and the
/// blur radius, and on nothing else.
///
/// `HMAC-SHA256(seed, label || radius)`: the same seed and radius give the same
/// pair on every call, every post and every restart, which is the point (see
/// [`fuzz_location`]). The radius is part of the message so that changing it
/// draws an *independent* offset: with the same pair and a different radius the
/// posted point would slide along one line through the true position, and two
/// observations (the old radius and the new) would fix it exactly.
#[must_use]
pub fn blur_draws(seed: &[u8; BLUR_SEED_LEN], radius_m: f64) -> (f64, f64) {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, seed);
    let mut message = b"birdnet-behavior/birdweather-blur/v1".to_vec();
    message.extend_from_slice(&radius_m.to_bits().to_le_bytes());
    let tag = ring::hmac::sign(&key, &message);
    let tag = tag.as_ref();
    let unit = |from: usize| {
        let mut word = [0_u8; 8];
        word.copy_from_slice(&tag[from..from + 8]);
        let n = u64::from_le_bytes(word) >> 11;
        #[allow(clippy::cast_precision_loss)]
        {
            n as f64 / (1_u64 << 53) as f64
        }
    };
    (unit(0), unit(8))
}

/// A position blurred by up to `radius_m` metres, from two uniform draws in
/// `[0, 1)`.
///
/// The offset is uniform over a **disc** of that radius (the square-root of the
/// first draw sets the distance, so points are not bunched at the centre; the
/// second sets the bearing), measured along the Earth's surface, so a metre is a
/// metre at 60° north too. birdnet-go offsets each axis independently by up to
/// ±radius, which puts a point up to 1.41 × the radius away at the equator and
/// less than the radius east-west near the poles; the disc is what "within N
/// metres" says. The result is rounded to five decimal places (about 1 m) and
/// kept on the globe: latitude clamped to ±90°, longitude wrapped. (birdnet-go
/// floors to four, about 11 m, which moves a point by up to that much *on top
/// of* the radius and makes a 5 m radius a lie.)
///
/// **What the blur hides, and what it does not.** This function is a pure
/// mapping from two draws to a point; [`Client::post_location`] feeds it the
/// *same* draws for every post (derived from a per-station seed by
/// [`blur_draws`]), so the posted position is one fixed point, off by a fixed
/// offset. A fresh draw per post, as birdnet-go does, can be averaged: the mean
/// of N posts is within about `radius / sqrt(N)` of the true position, so a
/// year of posts gives the address back. A fixed offset cannot be averaged.
/// What it does **not** do is hide the position from anyone who learns the
/// offset (the seed file, a second source that places the station within the
/// radius), because subtracting the offset is exact. Without it, all a viewer
/// knows is that the station is within the radius of the posted point.
/// Changing the radius, or losing the seed, draws a new and independent
/// offset, which hands an observer a second sample of the disc. About 1 % of
/// stations land within a tenth of the radius of their true position: the
/// offset is uniform over the disc, and a disc has little area near its centre.
///
#[must_use]
pub fn fuzz_location(lat: f64, lon: f64, radius_m: f64, u: f64, v: f64) -> (f64, f64) {
    if !(radius_m.is_finite() && radius_m > 0.0) {
        return (lat, lon);
    }
    let distance = radius_m * u.clamp(0.0, 1.0).sqrt();
    let bearing = v.clamp(0.0, 1.0) * std::f64::consts::TAU;
    // The spherical destination point: exact at any latitude, including the
    // one where a degree of longitude is no distance at all.
    let (phi1, lambda1) = (lat.to_radians(), lon.to_radians());
    let delta = distance / EARTH_RADIUS_M;
    let phi2 = (phi1.sin() * delta.cos() + phi1.cos() * delta.sin() * bearing.cos()).asin();
    let lambda2 = lambda1
        + (bearing.sin() * delta.sin() * phi1.cos()).atan2(delta.cos() - phi1.sin() * phi2.sin());
    let round = |x: f64| (x * 100_000.0).round() / 100_000.0;
    let new_lat = round(phi2.to_degrees()).clamp(-90.0, 90.0);
    let mut new_lon = round(lambda2.to_degrees());
    // `atan2` can leave the longitude a turn or more out; bring it back.
    new_lon = (new_lon + 180.0).rem_euclid(360.0) - 180.0;
    (new_lat, new_lon)
}

/// `BirdWeather` API client.
#[derive(Clone)]
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
    /// Blur posted positions by up to this many metres; `0.0` posts the exact
    /// position. See [`fuzz_location`].
    location_accuracy_m: f64,
    /// The station's secret blur seed. With a radius set and no seed, nothing
    /// is posted ([`Client::post_location`]); the position is never exact by
    /// default.
    blur_seed: Option<[u8; BLUR_SEED_LEN]>,
    /// Upload only detections at or above this confidence; `None` uploads every
    /// detection the station records. See [`Client::with_min_confidence`].
    min_confidence: Option<f32>,
}

impl std::fmt::Debug for Client {
    /// Hand-written so the station token cannot reach a log through `{:?}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("station_token", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("lat", &self.lat)
            .field("lon", &self.lon)
            .field("location_accuracy_m", &self.location_accuracy_m)
            .field("blur_seed", &self.blur_seed.map(|_| "<redacted>"))
            .field("min_confidence", &self.min_confidence)
            .finish_non_exhaustive()
    }
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
    // Two spellings name the V2.4 network: BirdNET's own file name
    // (`BirdNET_GLOBAL_6K_V2.4_Model_FP16`) and the catalogue id the installers
    // write (`birdnet-v24`, the default since the V2.4 default landed). Only the
    // first was recognised, so a station on the default model posted no
    // algorithm at all.
    let name = model_name.to_ascii_lowercase();
    (name.contains("v2.4") || name.contains("birdnet-v24") || name.contains("birdnet_v24"))
        .then_some("2p4")
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
    /// Returns `BirdWeatherError::NoToken` if the token is empty, and
    /// `BirdWeatherError::Location` if the coordinates are not ones
    /// [`check_location`] accepts: a client holding a made-up position would
    /// publish every detection at it.
    pub fn new(station_token: &str, lat: f64, lon: f64) -> Result<Self, BirdWeatherError> {
        // Trimmed: a token copied from the BirdWeather page tends to bring a
        // newline with it, and the token is a path segment, where the stray
        // character turns into a different station that does not exist.
        let station_token = station_token.trim();
        if station_token.is_empty() {
            return Err(BirdWeatherError::NoToken);
        }
        let (lat, lon) =
            check_location(Some(lat), Some(lon)).map_err(BirdWeatherError::Location)?;

        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| BirdWeatherError::Http(crate::http_error_text(e)))?;

        Ok(Self {
            station_token: station_token.to_string(),
            base_url: API_BASE.to_owned(),
            http,
            lat,
            lon,
            location_accuracy_m: 0.0,
            blur_seed: None,
            min_confidence: None,
        })
    }

    /// Blur the position posted with each detection by up to `metres`.
    ///
    /// tphakala/birdnet-go's `birdweather.locationaccuracy`. The position goes
    /// on a **public map**, and a station in a garden is a station at an address.
    /// Anything that is not a positive, finite number of metres leaves the
    /// position exact, which is also the default: a station that has not asked
    /// for this posts what it always did. **A caller that has been given a
    /// radius must therefore validate it first** and refuse to build the client
    /// on a bad one, as `create_birdweather_client` does: a privacy setting
    /// that fails to parse must not become "no privacy". A radius above
    /// [`MAX_LOCATION_ACCURACY_M`] is capped to it.
    ///
    /// A positive radius draws a random per-process blur seed here if the
    /// client has none yet; [`Client::with_blur_seed`] replaces it with the
    /// station's persistent one. If the system generator fails the client
    /// holds no seed and [`Client::post_location`] returns `None`.
    #[must_use]
    pub fn with_location_accuracy(mut self, metres: f64) -> Self {
        self.location_accuracy_m = if metres.is_finite() && metres > 0.0 {
            metres.min(MAX_LOCATION_ACCURACY_M)
        } else {
            0.0
        };
        if self.location_accuracy_m > 0.0 && self.blur_seed.is_none() {
            self.blur_seed = random_blur_seed();
        }
        self
    }

    /// Use `seed` for the blur offset, so it is the same on every post and, if
    /// the caller keeps the seed, on every restart.
    #[must_use]
    pub const fn with_blur_seed(mut self, seed: [u8; BLUR_SEED_LEN]) -> Self {
        self.blur_seed = Some(seed);
        self
    }

    /// The blur radius in metres; `0.0` means the position is posted exactly.
    #[must_use]
    pub const fn location_accuracy(&self) -> f64 {
        self.location_accuracy_m
    }

    /// Upload only detections at or above `confidence`: a bar for the public
    /// map that is higher than the station's own.
    ///
    /// tphakala/birdnet-go's `birdweather.threshold`. A station keeps a 0.7
    /// detection for its own log and reviews it; BirdWeather's community map is
    /// where a wrong 0.7 reads as fact. Setting this *below* the station's
    /// threshold changes nothing, since a detection the station dropped is never
    /// offered. A non-finite value or one outside `0..=1` is ignored (no extra
    /// bar) rather than clamped: `75` typed for 75 % would otherwise clamp to 1
    /// and silently stop every upload.
    #[must_use]
    pub fn with_min_confidence(mut self, confidence: f32) -> Self {
        self.min_confidence =
            (confidence.is_finite() && (0.0..=1.0).contains(&confidence) && confidence > 0.0)
                .then_some(confidence);
        self
    }

    /// The upload bar, if one was set.
    #[must_use]
    pub const fn min_confidence(&self) -> Option<f32> {
        self.min_confidence
    }

    /// Whether a detection with this `confidence` is to be uploaded.
    #[must_use]
    pub fn wants(&self, confidence: f32) -> bool {
        self.min_confidence.is_none_or(|floor| confidence >= floor)
    }

    /// The position to post for the next detection: the station's own, or, if a
    /// radius was set, the station's position moved by its **one fixed offset**
    /// (see [`fuzz_location`] for what that does and does not hide).
    ///
    /// `None` means *post nothing*: a radius is set but the client has no seed
    /// (the system generator failed when it was configured). The alternative,
    /// the exact position, is the one answer a blur setting must never give.
    #[must_use]
    pub fn post_location(&self) -> Option<(f64, f64)> {
        if self.location_accuracy_m <= 0.0 {
            return Some((self.lat, self.lon));
        }
        let seed = self.blur_seed.as_ref()?;
        let (u, v) = blur_draws(seed, self.location_accuracy_m);
        Some(fuzz_location(
            self.lat,
            self.lon,
            self.location_accuracy_m,
            u,
            v,
        ))
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
        let url = self.station_url("detections");

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
            "{}?timestamp={}&type={}",
            self.station_url("soundscapes"),
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
            .map_err(|e| BirdWeatherError::Http(crate::http_error_text(e)))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(BirdWeatherError::status(status, &text));
        }
        let parsed = resp
            .json::<SoundscapeResponse>()
            .await
            .map_err(|e| BirdWeatherError::Api(crate::http_error_text(e)))?;
        match (parsed.success, parsed.soundscape) {
            (true, Some(stored)) => Ok(stored.id),
            (_, _) => {
                Err(BirdWeatherError::Api(parsed.message.unwrap_or_else(|| {
                    "soundscape accepted without an id".to_owned()
                })))
            }
        }
    }

    /// `{base}/stations/{token}/{tail}`, with the token encoded as the one
    /// path segment it is — a `/` or `?` in it cannot reach another path.
    fn station_url(&self, tail: &str) -> String {
        format!(
            "{}/stations/{}/{tail}",
            self.base_url,
            urlencode(&self.station_token)
        )
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
                            .map_err(|e| BirdWeatherError::Api(crate::http_error_text(e)));
                    }
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    last_error = BirdWeatherError::status(status, &text);
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
                    last_error = BirdWeatherError::Http(crate::http_error_text(e));
                }
            }
        }

        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INT13: only a refusal of the body is permanent. The rest — a wrong
    /// token, a rate limit, an outage — must stay queued.
    #[test]
    fn only_a_refused_body_is_permanent() {
        let err =
            |code: u16| BirdWeatherError::status(reqwest::StatusCode::from_u16(code).unwrap(), "");
        for code in [400, 413, 415, 422] {
            assert!(err(code).is_permanent(), "{code}");
        }
        for code in [401, 403, 404, 408, 429, 500, 502, 503] {
            assert!(!err(code).is_permanent(), "{code}");
        }
        assert!(!BirdWeatherError::Http("timed out".into()).is_permanent());
        assert!(!BirdWeatherError::Api("bad json".into()).is_permanent());
        assert_eq!(
            err(422).to_string(),
            "BirdWeather API error: 422 Unprocessable Entity: ",
            "the displayed text is unchanged"
        );
    }

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

    /// The name the installers give the V2.4 file is the catalogue id, which has
    /// no dot in it: the default install must be labelled like any other V2.4.
    #[test]
    fn the_installed_v24_files_name_is_recognised_too() {
        for name in ["birdnet-v24", "BirdNET_v24_fp32", "birdnet_V2.4_custom"] {
            assert_eq!(algorithm_for_model(name), Some("2p4"), "{name}");
        }
        // The counterparts: the other catalogue ids and near misses are not it.
        for name in ["birdnet-v3", "perch-v2", "model-v2", "demo-v240"] {
            assert_eq!(algorithm_for_model(name), None, "{name}");
        }
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
    fn a_pasted_token_loses_its_surrounding_whitespace() {
        // Copied from the BirdWeather page with the newline after it, the
        // token used to go into the request path as-is, and every upload
        // was refused for good.
        let client = Client::new("  abc123XYZ\n", 42.36, -71.06).unwrap();
        assert_eq!(client.token(), "abc123XYZ");
        assert_eq!(
            client.station_url("detections"),
            "https://app.birdweather.com/api/v1/stations/abc123XYZ/detections"
        );
    }

    #[test]
    fn a_blank_token_is_no_token() {
        assert!(matches!(
            Client::new(" \t\n", 42.36, -71.06),
            Err(BirdWeatherError::NoToken)
        ));
    }

    /// The token is one path segment. A `/` or `?` in it must not become
    /// another path or a query on the same host.
    #[test]
    fn the_token_cannot_change_the_path() {
        let client = Client::new("a/../b?c=d", 42.36, -71.06).unwrap();
        assert_eq!(
            client.station_url("detections"),
            "https://app.birdweather.com/api/v1/stations/a%2F..%2Fb%3Fc%3Dd/detections"
        );
    }

    #[test]
    fn empty_token_returns_error() {
        let result = Client::new("", 42.0, -71.0);
        assert!(matches!(result, Err(BirdWeatherError::NoToken)));
    }

    // ── the location a station may be published at ──────────────────────

    #[test]
    fn a_station_with_no_location_is_not_published_at_null_island() {
        // The defect: a station with a token and no LATITUDE/LONGITUDE got a
        // client at 0, 0 and posted every detection there, onto a public map.
        assert!(matches!(
            Client::new("tok", 0.0, 0.0),
            Err(BirdWeatherError::Location(LocationProblem::NullIsland))
        ));
        assert_eq!(check_location(None, None), Err(LocationProblem::Unset));
        assert_eq!(
            check_location(Some(51.5), None),
            Err(LocationProblem::Unset)
        );
        assert_eq!(
            check_location(None, Some(-0.1)),
            Err(LocationProblem::Unset)
        );
        // `-0.0` is zero: a station typed as "-0.0, 0" is no more placed.
        assert_eq!(
            check_location(Some(-0.0), Some(0.0)),
            Err(LocationProblem::NullIsland)
        );
    }

    #[test]
    fn coordinates_outside_the_globe_or_not_numbers_are_refused() {
        for (lat, lon) in [
            (91.0, 10.0),
            (-90.5, 10.0),
            (10.0, 180.5),
            (10.0, -181.0),
            (f64::NAN, 10.0),
            (10.0, f64::INFINITY),
            (200.0, 0.0), // a longitude typed into the latitude box
        ] {
            assert!(
                matches!(
                    check_location(Some(lat), Some(lon)),
                    Err(LocationProblem::OutOfRange { .. })
                ),
                "({lat}, {lon}) was accepted"
            );
            assert!(Client::new("tok", lat, lon).is_err(), "({lat}, {lon})");
        }
    }

    #[test]
    fn real_places_are_accepted_including_the_ones_that_touch_zero() {
        // Counterpart: the gate refuses 0, 0 and nothing near it. The equator
        // and the prime meridian are real places to keep a station — Quito
        // sits almost on one, Greenwich on the other — and so are the limits.
        for (lat, lon) in [
            (51.48, -0.13),
            (-0.18, -78.47),
            (0.0, 25.0),
            (51.4769, 0.0),
            (90.0, 180.0),
            (-90.0, -180.0),
            (0.000_1, 0.000_1),
        ] {
            assert_eq!(
                check_location(Some(lat), Some(lon)),
                Ok((lat, lon)),
                "({lat}, {lon})"
            );
            assert!(Client::new("tok", lat, lon).is_ok(), "({lat}, {lon})");
        }
    }

    #[test]
    fn the_location_error_names_what_is_wrong() {
        let e = Client::new("tok", 0.0, 0.0).unwrap_err().to_string();
        assert!(e.contains("both 0"), "{e}");
        let e = Client::new("tok", 123.0, 0.0).unwrap_err().to_string();
        assert!(e.contains("123"), "{e}");
    }

    #[test]
    fn client_stores_coordinates() {
        let client = Client::new("test-token", 42.36, -71.06).unwrap();
        assert_eq!(client.coordinates(), (42.36, -71.06));
        assert_eq!(client.token(), "test-token");
    }

    #[test]
    fn base_url_defaults_to_public_api() {
        let client = Client::new("t", 42.36, -71.06).unwrap();
        assert_eq!(client.base_url(), API_BASE);
    }

    #[test]
    fn with_base_url_overrides_and_normalises() {
        let client = Client::new("t", 42.36, -71.06)
            .unwrap()
            .with_base_url("http://127.0.0.1:9000/api/v1/");
        // Trailing slash trimmed so the joined URL has exactly one separator.
        assert_eq!(client.base_url(), "http://127.0.0.1:9000/api/v1");
    }

    // ── location fuzzing and the upload bar (P-2) ──────────────────────

    /// Great-circle distance in metres (haversine, mean radius).
    fn metres_between(a: (f64, f64), b: (f64, f64)) -> f64 {
        let (la1, lo1, la2, lo2) = (
            a.0.to_radians(),
            a.1.to_radians(),
            b.0.to_radians(),
            b.1.to_radians(),
        );
        let h = ((la2 - la1) / 2.0).sin().powi(2)
            + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
        2.0 * 6_371_000.0 * h.sqrt().asin()
    }

    /// Slack for rounding to five decimals: up to 0.55 m on each axis, 0.8 m together.
    const ROUNDING_M: f64 = 1.5;

    #[test]
    fn a_fuzzed_position_stays_inside_the_radius_everywhere() {
        // On a grid of draws, at the equator, at 60 N (where a degree of
        // longitude is half as long), in the southern hemisphere, and close
        // to a pole. The per-axis square birdnet-go draws fails the equator
        // case (1.41 x the radius at the corners); a latitude-blind disc fails
        // the 60 N case.
        for (lat, lon) in [(0.0, 10.0), (60.0, 24.0), (-33.9, 151.2), (89.0, 5.0)] {
            for radius in [5.0, 100.0, 5_000.0] {
                for i in 0..=20 {
                    for j in 0..20 {
                        let (u, v) = (f64::from(i) / 20.0, f64::from(j) / 20.0);
                        let got = fuzz_location(lat, lon, radius, u, v);
                        let d = metres_between((lat, lon), got);
                        assert!(
                            d <= radius + ROUNDING_M,
                            "({lat},{lon}) r={radius} u={u} v={v}: moved {d:.1} m to {got:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_blur_actually_blurs_and_reaches_the_edge() {
        // The counterpart: a function that returned the input would satisfy
        // the bound above. The farthest draw (u = 1) must be close to the
        // radius, and a mid draw must have moved.
        let far = fuzz_location(51.5, -0.1, 1_000.0, 1.0, 0.25);
        let d = metres_between((51.5, -0.1), far);
        assert!(
            d > 990.0 && d <= 1_001.0,
            "the edge of the disc is {d:.1} m out"
        );
        let mid = fuzz_location(51.5, -0.1, 1_000.0, 0.25, 0.6);
        assert!(metres_between((51.5, -0.1), mid) > 100.0);
    }

    #[test]
    fn fuzzing_keeps_a_position_on_the_globe() {
        let at_pole = fuzz_location(90.0, 0.0, 50_000.0, 1.0, 0.0);
        assert!(at_pole.0 <= 90.0 && at_pole.0 >= -90.0, "{at_pole:?}");
        let near_date_line = fuzz_location(10.0, 179.99999, 5_000.0, 1.0, 0.25);
        assert!(
            (-180.0..=180.0).contains(&near_date_line.1),
            "{near_date_line:?}"
        );
        let west = fuzz_location(10.0, -179.99999, 5_000.0, 1.0, 0.75);
        assert!((-180.0..=180.0).contains(&west.1), "{west:?}");
    }

    #[test]
    fn a_zero_radius_or_nonsense_leaves_the_position_exact() {
        for r in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert_eq!(fuzz_location(51.5, -0.1, r, 0.7, 0.3), (51.5, -0.1), "{r}");
        }
    }

    #[test]
    fn a_client_posts_the_exact_position_unless_asked_to_blur() {
        let plain = Client::new("t", 51.5, -0.1).unwrap();
        assert_eq!(plain.location_accuracy(), 0.0);
        for _ in 0..20 {
            assert_eq!(plain.post_location(), Some((51.5, -0.1)));
        }
        let blurred = Client::new("t", 51.5, -0.1)
            .unwrap()
            .with_location_accuracy(500.0);
        let draws: Vec<(f64, f64)> = (0..50)
            .map(|_| blurred.post_location().expect("a seeded client posts"))
            .collect();
        assert!(
            draws.iter().all(|p| *p != (51.5, -0.1)),
            "a blurred client posted the exact position"
        );
        for p in &draws {
            assert!(
                metres_between((51.5, -0.1), *p) <= 500.0 + ROUNDING_M,
                "{p:?}"
            );
        }
        // The station's own coordinates are untouched: the doctor and the
        // dashboard read them, and they must not drift.
        assert_eq!(blurred.coordinates(), (51.5, -0.1));
    }

    /// The defect (M1): a new offset per post, so the mean of N posts is within
    /// about `radius / sqrt(N)` of the true position and a year of uploads
    /// gives the address back. Averaging 400 posts is the attack; it must land
    /// exactly where one post does, which is not the true position.
    #[test]
    fn averaging_many_posts_does_not_recover_the_true_position() {
        let truth = (51.5, -0.1);
        let c = Client::new("t", truth.0, truth.1)
            .unwrap()
            .with_location_accuracy(2_000.0);
        let posts: Vec<(f64, f64)> = (0..400).map(|_| c.post_location().unwrap()).collect();
        assert!(
            posts.windows(2).all(|w| w[0] == w[1]),
            "the offset changed between posts, so it can be averaged away"
        );
        #[allow(clippy::cast_precision_loss)]
        let n = posts.len() as f64;
        let mean = (
            posts.iter().map(|p| p.0).sum::<f64>() / n,
            posts.iter().map(|p| p.1).sum::<f64>() / n,
        );
        assert!(
            metres_between(truth, mean) > 20.0,
            "400 posts averaged to {:.1} m from the truth",
            metres_between(truth, mean)
        );
    }

    /// The counterpart, and the reason for a stored seed: the same seed gives
    /// the same point from a new client (a restart), a different seed another.
    #[test]
    fn the_offset_is_the_stations_own_and_survives_a_restart() {
        let at = |seed: [u8; BLUR_SEED_LEN]| {
            Client::new("t", 51.5, -0.1)
                .unwrap()
                .with_blur_seed(seed)
                .with_location_accuracy(2_000.0)
                .post_location()
                .unwrap()
        };
        assert_eq!(at([7; BLUR_SEED_LEN]), at([7; BLUR_SEED_LEN]));
        assert_ne!(at([7; BLUR_SEED_LEN]), at([8; BLUR_SEED_LEN]));
        // A client given a seed keeps it when the radius is set after.
        let a = Client::new("t", 51.5, -0.1)
            .unwrap()
            .with_location_accuracy(2_000.0)
            .with_blur_seed([7; BLUR_SEED_LEN]);
        assert_eq!(a.post_location().unwrap(), at([7; BLUR_SEED_LEN]));
    }

    /// Over many stations the fixed offset is still uniform over the disc: the
    /// mean distance of a uniform disc is 2/3 of its radius, and every quadrant
    /// of bearing is used. A distance taken as `radius * u` would give 1/2.
    #[test]
    fn the_fixed_offsets_are_uniform_over_the_disc_and_inside_it() {
        let truth = (51.5, -0.1);
        let radius = 1_000.0;
        let (mut sum_fractions, mut quadrants, stations) = (0.0, [0_u32; 4], 4_000_u32);
        for index in 0..stations {
            let mut seed = [0_u8; BLUR_SEED_LEN];
            seed[..4].copy_from_slice(&index.to_le_bytes());
            // `Client::post_location` is exactly this pair of calls; building
            // a client per seed would build 4 000 HTTP clients.
            let (distance_draw, bearing_draw) = blur_draws(&seed, radius);
            let posted = fuzz_location(truth.0, truth.1, radius, distance_draw, bearing_draw);
            let moved = metres_between(truth, posted);
            assert!(
                moved <= radius + ROUNDING_M,
                "seed {index}: {moved:.1} m out"
            );
            sum_fractions += moved / radius;
            let quadrant = usize::from(posted.0 > truth.0) * 2 + usize::from(posted.1 > truth.1);
            quadrants[quadrant] += 1;
        }
        let mean = sum_fractions / f64::from(stations);
        assert!((0.64..0.69).contains(&mean), "mean distance {mean:.3} r");
        assert!(
            quadrants.iter().all(|count| (850..1150).contains(count)),
            "{quadrants:?}"
        );
    }

    /// Changing the radius must not slide the point along one line through the
    /// truth: two observations on that line would fix the true position
    /// exactly. Over many seeds, the offset at 500 m and at 1 000 m do not
    /// share a bearing.
    #[test]
    fn a_different_radius_draws_an_independent_offset() {
        let truth = (51.5, -0.1);
        let bearing = |seed: [u8; BLUR_SEED_LEN], radius: f64| {
            let p = Client::new("t", truth.0, truth.1)
                .unwrap()
                .with_blur_seed(seed)
                .with_location_accuracy(radius)
                .post_location()
                .unwrap();
            let north = (p.0 - truth.0).to_radians() * 6_371_000.0;
            let east = (p.1 - truth.1).to_radians() * 6_371_000.0 * truth.0.to_radians().cos();
            east.atan2(north)
        };
        let differing = (0..50_u8)
            .filter(|i| {
                let seed = [*i; BLUR_SEED_LEN];
                (bearing(seed, 500.0) - bearing(seed, 1_000.0)).abs() > 0.05
            })
            .count();
        assert!(
            differing >= 45,
            "only {differing} of 50 seeds changed bearing with the radius"
        );
    }

    /// The RNG-failure branch (M1, low). A client with a radius and no seed has
    /// nothing to blur by; the answer must be "post nothing", never the exact
    /// position (the old comment called the zero-bytes outcome "a consistent
    /// offset", but `u = 0` is a distance of 0).
    #[test]
    fn a_blur_with_no_seed_posts_nothing_rather_than_the_exact_position() {
        let mut c = Client::new("t", 51.5, -0.1)
            .unwrap()
            .with_location_accuracy(500.0);
        c.blur_seed = None;
        assert_eq!(c.post_location(), None);
        // The draw a zero seed would have produced is not the exact position
        // either way, so this is the only safe branch.
        assert!(
            metres_between((51.5, -0.1), fuzz_location(51.5, -0.1, 500.0, 0.0, 0.0)) < ROUNDING_M
        );
        // The counterpart: no radius, no seed needed.
        let plain = Client::new("t", 51.5, -0.1).unwrap();
        assert_eq!(plain.post_location(), Some((51.5, -0.1)));
    }

    #[test]
    fn a_random_seed_is_different_each_time() {
        let a = random_blur_seed().expect("the system generator works");
        let b = random_blur_seed().expect("the system generator works");
        assert_ne!(a, b);
        assert_ne!(a, [0; BLUR_SEED_LEN]);
    }

    #[test]
    fn a_blur_radius_that_is_nonsense_is_off_and_a_huge_one_is_capped() {
        for bad in [0.0, -1.0, f64::NAN, f64::NEG_INFINITY] {
            let c = Client::new("t", 51.5, -0.1)
                .unwrap()
                .with_location_accuracy(bad);
            assert_eq!(c.location_accuracy(), 0.0, "{bad}");
        }
        let c = Client::new("t", 51.5, -0.1)
            .unwrap()
            .with_location_accuracy(1.0e9);
        assert!((c.location_accuracy() - MAX_LOCATION_ACCURACY_M).abs() < f64::EPSILON);
    }

    #[test]
    fn the_upload_bar_is_inclusive_and_absent_by_default() {
        let none = Client::new("t", 51.5, -0.1).unwrap();
        assert!(none.wants(0.0) && none.wants(0.01), "no bar, no filtering");
        let c = Client::new("t", 51.5, -0.1)
            .unwrap()
            .with_min_confidence(0.85);
        assert!(!c.wants(0.84) && c.wants(0.85) && c.wants(0.99));
        assert_eq!(c.min_confidence(), Some(0.85));
    }

    #[test]
    fn a_bar_typed_as_a_percentage_is_ignored_not_clamped_to_one() {
        // 75 typed for 75 % would clamp to 1.0 and stop every upload for good.
        for bad in [75.0_f32, 1.01, -0.5, f32::NAN, f32::INFINITY, 0.0] {
            let c = Client::new("t", 51.5, -0.1)
                .unwrap()
                .with_min_confidence(bad);
            assert_eq!(c.min_confidence(), None, "{bad}");
            assert!(c.wants(0.3), "{bad} blocked a 0.3 detection");
        }
    }

    #[test]
    fn with_base_url_ignores_blank_override() {
        // A blank env var must keep the default, never produce relative URLs.
        let client = Client::new("t", 42.36, -71.06)
            .unwrap()
            .with_base_url("   ");
        assert_eq!(client.base_url(), API_BASE);
    }
}
