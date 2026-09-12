//! eBird recent-observations client: what other people have actually seen
//! near this station lately (`G-27`).
//!
//! # What this is for
//!
//! The station already has a geographic prior on which species are plausible —
//! the BirdNET range model. That prior is *climatological*: it knows a Common
//! Swift is expected here in July, and it has no idea that the first one of
//! the year arrived last Tuesday, or that a Waxwing irruption reached this
//! valley a fortnight ago. eBird's recent observations are the opposite kind
//! of evidence: a human being stood near here, within the last few days, and
//! wrote down what they saw.
//!
//! That makes it the best available answer to "is this bird around *right
//! now*", and the natural companion to the suspect-species report in
//! `birdnet_db::phantoms`, which otherwise judges a species purely on the
//! shape of its own detections.
//!
//! # Corroboration only ever exculpates. It never accuses.
//!
//! This is the load-bearing design decision, so it is stated before the code.
//!
//! A hit — somebody reported this species 8 km away four days ago — is strong
//! evidence the detections are real, and it is shown.
//!
//! A miss is shown as **nothing at all**, because a miss means nothing.
//! eBird coverage tracks *birder density*, not bird presence. A station in a
//! well-watched county gets hundreds of checklists a week; a station on a
//! Hebridean hillside gets none, ever, for any species. If absence from the
//! snapshot were allowed to count against a species, the second station would
//! have its entire list flagged as phantoms — and the operator who most needs
//! an automated check is exactly the one with nobody else nearby to confirm
//! anything. So [`Snapshot::reported`] returns `Some` or `None`, callers
//! render the `Some`, and `None` changes no verdict anywhere.
//!
//! # The API key is the opt-in
//!
//! There is no `BNB_EBIRD_ENABLED` flag. eBird requires an API key for every
//! `/v2/` endpoint, so a station that has not been given one cannot reach the
//! service and never tries: [`resolve`] returns `Ok(None)` and nothing is
//! spawned. A second switch would only add a state where a key is configured
//! and silently unused.
//!
//! # Nearby by default, region on request
//!
//! [`Scope::Nearby`] asks `data/obs/geo/recent` around the station's own
//! coordinates, because "reported within 25 km" is a far stronger statement
//! than "reported somewhere in this state", and because the operator has
//! already given coordinates for the range model and the solar schedule — a
//! region code is one more thing to look up.
//!
//! [`Scope::Region`] asks `data/obs/{regionCode}/recent` and exists for the
//! sparse case: a station whose 50 km radius genuinely contains no eBird
//! observers gets more corroboration from its county or state than from a
//! circle containing nobody.
//!
//! Coordinates are sent rounded to two decimal places — which is all eBird
//! documents that it accepts, and which also means the station's position
//! leaves the machine no more precisely than about a kilometre.
//!
//! # Offline degrades to the last snapshot
//!
//! The snapshot is cached to disk as JSON. A fetch that fails leaves the
//! cached file untouched, so a station whose uplink is down keeps showing the
//! corroboration it had, labelled with its age, rather than losing the
//! feature. [`Snapshot::is_stale`] is what a caller uses to decide whether to
//! say so.
//!
//! # Verification
//!
//! **The decoder is written against eBird's published API documentation, not
//! against live bytes.** Every `/v2/` endpoint answers `403` without an API
//! key, which this repository does not have; that was confirmed against four
//! distinct endpoints (`data/obs/{region}/recent`, `data/obs/geo/recent`,
//! `ref/hotspot/{region}`, `product/spplist/{region}`), each returning `403`
//! both with no key and with a bogus one.
//!
//! The three fixtures in `testdata/` are eBird's own documented example
//! response bodies, taken verbatim (the region one trimmed to its first three
//! entries) from the eBird API 2.0 Postman collection published at
//! `https://documenter.getpostman.com/view/664302/S1ENwy59`, retrieved
//! 2026-09-11. Field optionality below is read off those documented bodies:
//! `subId` is present in the region example and absent from both geo
//! examples, and the notable-observations example carries fifteen fields
//! neither of the others has — so unknown fields are ignored and everything
//! not needed for a species-presence answer is optional. The taxonomically
//! sorted geo example is kept as a third fixture because it contains
//! `Anser sp. (Domestic type)` — a row from the `domestic` category, which
//! eBird's default "all categories" returns and which is not a binomial at
//! all.
//!
//! Test literals that exercise decoder tolerance for shapes the
//! documentation does not illustrate (a missing `howMany`, a date-only
//! `obsDt`) are constructed in the test module and named as such. They are
//! not presented as eBird output.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// eBird's API host.
pub const DEFAULT_BASE_URL: &str = "https://api.ebird.org";

/// How often the snapshot is refreshed.
///
/// eBird's own window is measured in days, and a checklist submitted this
/// afternoon does not change whether a species is plausible this afternoon.
/// Six hours keeps the answer current without asking a free service for the
/// same data forty-eight times a day.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Age beyond which a snapshot is reported as stale.
///
/// Four refresh intervals: long enough that one failed fetch, or a station
/// that was powered off overnight, does not raise an alarm; short enough that
/// a persistently broken key or a revoked one becomes visible within a day.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// Default days of history to ask for. eBird's own default.
pub const DEFAULT_BACK_DAYS: u32 = 14;

/// Most days of history eBird will return.
pub const MAX_BACK_DAYS: u32 = 30;

/// Default search radius for [`Scope::Nearby`], in kilometres. eBird's own
/// default.
pub const DEFAULT_DIST_KM: u32 = 25;

/// Largest search radius eBird accepts, in kilometres.
pub const MAX_DIST_KM: u32 = 50;

/// The header eBird authenticates with.
const API_KEY_HEADER: &str = "x-ebirdapitoken";

/// The cache file's name inside the station's data directory.
const CACHE_FILE: &str = "ebird-nearby.json";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an eBird lookup could not be made or could not be read.
///
/// Hand-rolled, like the rest of this crate's error types: the binary owns
/// the runtime and the error vocabulary.
#[derive(Debug)]
pub enum EbirdError {
    /// HTTP transport (timeout, DNS, TLS).
    Http(reqwest::Error),
    /// The response body was not shaped as documented.
    Decode(serde_json::Error),
    /// eBird refused the API key (`401`/`403`).
    ///
    /// Its own variant because this is the one failure that will never fix
    /// itself: a wrong, revoked or unset key returns `403` on every request
    /// forever, and in a log it is otherwise indistinguishable from a network
    /// blip that is worth retrying.
    Unauthorized(u16),
    /// eBird returned some other non-success status.
    Api(String),
    /// The integration cannot be used as configured.
    Config(String),
    /// The cache file could not be read or written.
    Io(std::io::Error),
}

impl std::fmt::Display for EbirdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "http error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Unauthorized(code) => {
                write!(f, "eBird refused the API key ({code}); check EBIRD_API_KEY")
            }
            Self::Api(m) => write!(f, "api error: {m}"),
            Self::Config(m) => write!(f, "{m}"),
            Self::Io(e) => write!(f, "cache error: {e}"),
        }
    }
}

impl std::error::Error for EbirdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            Self::Decode(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Unauthorized(_) | Self::Api(_) | Self::Config(_) => None,
        }
    }
}

impl From<reqwest::Error> for EbirdError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<serde_json::Error> for EbirdError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e)
    }
}

impl From<std::io::Error> for EbirdError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// Where to ask about.
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    /// A circle around the station's own coordinates.
    Nearby {
        /// Latitude, degrees.
        lat: f64,
        /// Longitude, degrees.
        lng: f64,
        /// Search radius, kilometres (0..=[`MAX_DIST_KM`]).
        dist_km: u32,
    },
    /// An eBird region code — a country, state, county or location.
    Region(String),
}

impl Scope {
    /// A circle around `(lat, lng)`.
    ///
    /// # Errors
    ///
    /// [`EbirdError::Config`] if the coordinates are outside their valid
    /// range or the radius exceeds [`MAX_DIST_KM`].
    pub fn nearby(lat: f64, lng: f64, dist_km: u32) -> Result<Self, EbirdError> {
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
            return Err(EbirdError::Config(format!(
                "station coordinates ({lat}, {lng}) are not a point on Earth"
            )));
        }
        if dist_km > MAX_DIST_KM {
            return Err(EbirdError::Config(format!(
                "EBIRD_DIST_KM is {dist_km}; eBird accepts at most {MAX_DIST_KM}"
            )));
        }
        Ok(Self::Nearby { lat, lng, dist_km })
    }

    /// An eBird region code such as `US-MA` or `GB-ENG-CAM`.
    ///
    /// # Errors
    ///
    /// [`EbirdError::Config`] if the code is empty or carries characters no
    /// eBird region code has. The check is deliberately shallow — eBird owns
    /// the list, and a code this rejects would be one it also rejects — but it
    /// stops a stray path segment or query string being pasted into a URL.
    pub fn region(code: &str) -> Result<Self, EbirdError> {
        let code = code.trim();
        if code.is_empty() {
            return Err(EbirdError::Config("EBIRD_REGION is empty".to_owned()));
        }
        if !code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(EbirdError::Config(format!(
                "EBIRD_REGION {code:?} is not an eBird region code \
                 (letters, digits, - and _ only, e.g. US-MA or GB-ENG)"
            )));
        }
        Ok(Self::Region(code.to_owned()))
    }

    /// A stable identity for this scope, stored in the cache file.
    ///
    /// A snapshot taken for one scope must not answer questions about
    /// another, so changing the region or moving the station invalidates the
    /// cache rather than silently corroborating from the wrong place.
    /// Coordinates appear here at the same two decimal places they are sent
    /// at, so a sub-kilometre GPS jitter does not throw the cache away.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Nearby { lat, lng, dist_km } => format!("geo:{lat:.2},{lng:.2}:{dist_km}"),
            Self::Region(code) => format!("region:{code}"),
        }
    }

    /// How this scope reads in a sentence to an operator.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Nearby { dist_km, .. } => format!("within {dist_km} km"),
            Self::Region(code) => format!("in {code}"),
        }
    }

    /// The request path and query for `back_days` of history.
    ///
    /// `sppLocale` is not sent: matching is done on scientific names, and the
    /// common names this station shows come from its own label file, so a
    /// localised eBird common name would be a second vocabulary to reconcile
    /// for no gain.
    fn path_and_query(&self, back_days: u32) -> String {
        match self {
            // Two decimal places is what eBird documents that it takes, and
            // it is also as much of the station's position as needs to leave
            // the machine.
            Self::Nearby { lat, lng, dist_km } => format!(
                "/v2/data/obs/geo/recent?lat={lat:.2}&lng={lng:.2}&dist={dist_km}&back={back_days}"
            ),
            Self::Region(code) => format!("/v2/data/obs/{code}/recent?back={back_days}"),
        }
    }
}

// ---------------------------------------------------------------------------
// The wire shape
// ---------------------------------------------------------------------------

/// One row of an eBird recent-observations response.
///
/// Only the fields this station has a use for are named; serde ignores the
/// rest, which matters because the notable-observations endpoint returns
/// fifteen more of them. Everything that is not needed to answer "which
/// species, when, where" is optional, per the module's Verification note.
#[derive(Debug, Clone, Deserialize)]
struct RawObservation {
    /// Scientific name, in eBird/Clements taxonomy.
    #[serde(rename = "sciName")]
    sci_name: String,
    /// eBird's English common name.
    #[serde(rename = "comName")]
    com_name: Option<String>,
    /// Observation timestamp, `YYYY-MM-DD HH:MM` or `YYYY-MM-DD`.
    #[serde(rename = "obsDt")]
    obs_dt: Option<String>,
    /// Locality name as the observer recorded it.
    #[serde(rename = "locName")]
    loc_name: Option<String>,
    /// How many were counted. Absent when the observer recorded presence
    /// without a count.
    #[serde(rename = "howMany")]
    how_many: Option<i64>,
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// One species somebody else reported, and the most recent time they did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearbySpecies {
    /// Scientific name as eBird gave it.
    pub sci_name: String,
    /// eBird's common name, when it gave one.
    pub com_name: String,
    /// The date of the most recent report, `YYYY-MM-DD`.
    pub last_seen: String,
    /// The locality of that report, as the observer named it.
    pub locality: String,
    /// How many were counted, when a count was recorded.
    pub how_many: Option<i64>,
}

/// What eBird said, when it was asked, and what it was asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Seconds since the Unix epoch at which this was fetched.
    pub fetched_at: u64,
    /// [`Scope::key`] of the scope it was fetched for.
    pub scope: String,
    /// How the scope reads in a sentence, so a reader need not parse `scope`.
    pub scope_label: String,
    /// Days of history requested.
    pub back_days: u32,
    /// Every species reported, sorted by normalised scientific name so the
    /// file is stable across fetches and [`Snapshot::reported`] can binary
    /// search it.
    pub species: Vec<NearbySpecies>,
}

/// Seconds since the Unix epoch, now.
#[must_use]
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Lowercase and collapse whitespace, for comparing two spellings of a name.
fn normalise(name: &str) -> String {
    name.split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first two words of an already-normalised name, when it has more.
///
/// eBird returns subspecies rows (`Junco hyemalis hyemalis`) alongside
/// species rows, because its default taxonomic category filter is "all". The
/// station's classifier only knows binomials, so a subspecies report would
/// corroborate nothing unless it is also indexed under its species. A
/// subspecies *is* the species, so this collapse is taxonomically sound —
/// unlike collapsing in the other direction, which is why nothing here ever
/// expands a binomial into subspecies.
fn binomial(normalised: &str) -> Option<&str> {
    let mut it = normalised.char_indices().filter(|&(_, c)| c == ' ');
    it.next()?;
    let (second_space, _) = it.next()?;
    Some(&normalised[..second_space])
}

impl Snapshot {
    /// Fold a response into a species-presence snapshot.
    ///
    /// eBird documents that a recent-observations response already holds only
    /// the most recent observation per species, so this is mostly a
    /// projection. It deduplicates anyway — keeping the latest `obsDt` —
    /// because subspecies rows collapse onto their binomial here, and because
    /// a boundary should not trust a documented invariant it can cheaply
    /// enforce.
    #[must_use]
    fn from_raw(scope: &Scope, back_days: u32, fetched_at: u64, raw: Vec<RawObservation>) -> Self {
        let mut by_name: std::collections::HashMap<String, NearbySpecies> =
            std::collections::HashMap::with_capacity(raw.len());
        for obs in raw {
            let sci = normalise(&obs.sci_name);
            if sci.is_empty() {
                continue;
            }
            // Index subspecies under their species; keep a binomial as it is.
            let key = binomial(&sci).unwrap_or(&sci).to_owned();
            let last_seen = obs
                .obs_dt
                .as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let entry = NearbySpecies {
                sci_name: key,
                com_name: obs.com_name.unwrap_or_default(),
                last_seen,
                locality: obs.loc_name.unwrap_or_default(),
                how_many: obs.how_many,
            };
            match by_name.entry(entry.sci_name.clone()) {
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    // Dates are `YYYY-MM-DD`, so lexicographic order is
                    // chronological order.
                    if slot.get().last_seen < entry.last_seen {
                        slot.insert(entry);
                    }
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(entry);
                }
            }
        }
        let mut species: Vec<NearbySpecies> = by_name.into_values().collect();
        species.sort_by(|a, b| a.sci_name.cmp(&b.sci_name));
        Self {
            fetched_at,
            scope: scope.key(),
            scope_label: scope.label(),
            back_days,
            species,
        }
    }

    /// Whether `sci_name` was reported, and by whom, where and when.
    ///
    /// Matching is on the scientific name, normalised for case and spacing,
    /// and on the binomial of a longer name. BirdNET's labels are derived from
    /// the eBird/Clements taxonomy, so most names should match exactly — but
    /// the two can be on different taxonomy *versions*, and how often that
    /// bites is not measured here, because the label files are downloaded by
    /// the installer and are not in this repository to compare against.
    ///
    /// It does not need to be measured to be safe, because every way this can
    /// be wrong fails in the same direction. A name that does not match yields
    /// `None`, `None` means "eBird said nothing", and "eBird said nothing"
    /// already changes no verdict anywhere. There is no input for which a
    /// mismatch invents corroboration: the lookup returns an entry only when
    /// that entry's own name equals the key.
    ///
    /// `None` is therefore never evidence the species is absent. See the
    /// module docs.
    #[must_use]
    pub fn reported(&self, sci_name: &str) -> Option<&NearbySpecies> {
        let wanted = normalise(sci_name);
        let key = binomial(&wanted).unwrap_or(&wanted);
        self.species
            .binary_search_by(|s| s.sci_name.as_str().cmp(key))
            .ok()
            .map(|i| &self.species[i])
    }

    /// How old this snapshot is, in seconds, as of `now`.
    ///
    /// Saturating: a snapshot stamped in the future — a station whose clock
    /// was corrected after it was written — reads as age zero rather than
    /// wrapping to eighty years.
    #[must_use]
    pub const fn age_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.fetched_at)
    }

    /// Whether this snapshot is old enough to say so, as of `now`.
    #[must_use]
    pub const fn is_stale(&self, now: u64) -> bool {
        self.age_secs(now) > STALE_AFTER.as_secs()
    }

    /// Whether this snapshot answers for `scope`.
    #[must_use]
    pub fn covers(&self, scope: &Scope) -> bool {
        self.scope == scope.key()
    }

    /// How many species it holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.species.len()
    }

    /// Whether it holds no species at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.species.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Disk cache
// ---------------------------------------------------------------------------

/// Where the snapshot lives, given the station's data directory.
#[must_use]
pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_FILE)
}

/// Read the cached snapshot, or `None` when there isn't a readable one.
///
/// A missing, unreadable or corrupt file is `None` rather than an error: the
/// only thing a caller can do about any of them is fetch again, and a station
/// whose cache file was truncated by a power cut should come up working.
#[must_use]
pub fn load(path: &Path) -> Option<Snapshot> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice::<Snapshot>(&bytes) {
        Ok(snapshot) => Some(snapshot),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "eBird cache unreadable; ignoring it");
            None
        }
    }
}

/// Write `snapshot` to `path`, atomically.
///
/// Written to a sibling temporary file and renamed, so a power cut during the
/// write leaves the previous snapshot intact rather than a half-written one
/// that [`load`] would discard.
///
/// # Errors
///
/// [`EbirdError::Io`] if the directory cannot be created or the file cannot
/// be written or renamed, [`EbirdError::Decode`] if the snapshot cannot be
/// serialised.
pub fn store(path: &Path, snapshot: &Snapshot) -> Result<(), EbirdError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(snapshot)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Everything needed to run the poll, once the configuration has been read.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The operator's eBird API key.
    pub api_key: String,
    /// What to ask about.
    pub scope: Scope,
    /// Days of history to ask for.
    pub back_days: u32,
}

/// Resolve the eBird configuration.
///
/// Returns `Ok(None)` when no API key is set — the integration is off and
/// nothing is fetched, which is the default state of a fresh install. Returns
/// `Err` when a key *is* set but the rest of the configuration cannot be used,
/// so an operator who asked for this and is not getting it is told why rather
/// than left with a silently dead feature.
///
/// `EBIRD_REGION` wins over the station's coordinates when both are present:
/// setting it is an explicit act, and the only reason to set it is to override
/// the default.
///
/// # Errors
///
/// [`EbirdError::Config`] when a key is set but there is neither a region nor
/// a pair of coordinates to ask about, or when a numeric setting is not a
/// number or is outside the range eBird accepts.
pub fn resolve(
    api_key: Option<&str>,
    region: Option<&str>,
    dist_km: Option<&str>,
    back_days: Option<&str>,
    lat: Option<f64>,
    lng: Option<f64>,
) -> Result<Option<Settings>, EbirdError> {
    let Some(api_key) = api_key.map(str::trim).filter(|k| !k.is_empty()) else {
        return Ok(None);
    };

    let back_days = parse_range(
        "EBIRD_BACK_DAYS",
        back_days,
        DEFAULT_BACK_DAYS,
        1,
        MAX_BACK_DAYS,
    )?;

    let region = region.map(str::trim).filter(|r| !r.is_empty());
    let scope = match (region, lat, lng) {
        (Some(code), _, _) => Scope::region(code)?,
        (None, Some(lat), Some(lng)) => {
            let dist = parse_range("EBIRD_DIST_KM", dist_km, DEFAULT_DIST_KM, 0, MAX_DIST_KM)?;
            Scope::nearby(lat, lng, dist)?
        }
        (None, _, _) => {
            return Err(EbirdError::Config(
                "EBIRD_API_KEY is set but the station has no coordinates; \
                 set LATITUDE and LONGITUDE, or name an EBIRD_REGION"
                    .to_owned(),
            ));
        }
    };

    Ok(Some(Settings {
        api_key: api_key.to_owned(),
        scope,
        back_days,
    }))
}

/// Parse an optional numeric setting and check it against eBird's own bounds.
fn parse_range(
    key: &str,
    raw: Option<&str>,
    default: u32,
    min: u32,
    max: u32,
) -> Result<u32, EbirdError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(default);
    };
    let value: u32 = raw.parse().map_err(|_| {
        EbirdError::Config(format!("{key} is {raw:?}, which is not a whole number"))
    })?;
    if value < min || value > max {
        return Err(EbirdError::Config(format!(
            "{key} is {value}; eBird accepts {min}..={max}"
        )));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// An eBird API client.
#[derive(Debug, Clone)]
pub struct Client {
    /// The HTTP client, with the station's identifying user agent.
    http: reqwest::Client,
    /// Host to ask, overridable so the tests can point at a local socket.
    base_url: String,
    /// The operator's API key, sent as the `x-ebirdapitoken` header.
    api_key: String,
}

impl Client {
    /// Build a client authenticating with `api_key`.
    ///
    /// `base_url` overrides eBird's host; it exists so the gates can point at
    /// a local socket, and so an operator behind a caching proxy can use it.
    ///
    /// # Errors
    ///
    /// [`EbirdError::Http`] if the underlying HTTP client cannot be built.
    pub fn new(api_key: impl Into<String>, base_url: Option<String>) -> Result<Self, EbirdError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!(
                "BirdNet-Behavior/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/tomtom215/BirdNet-Behavior)"
            ))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            api_key: api_key.into(),
        })
    }

    /// Fetch the recent observations for `scope` and fold them into a
    /// snapshot stamped `fetched_at`.
    ///
    /// # Errors
    ///
    /// [`EbirdError::Http`] on network, TLS or timeout failure;
    /// [`EbirdError::Api`] when eBird answers with a non-success status —
    /// `403` is what a missing, wrong or revoked key produces, and the status
    /// is named so that is diagnosable; [`EbirdError::Decode`] when the body
    /// is not the documented array of observations.
    pub async fn recent(
        &self,
        scope: &Scope,
        back_days: u32,
        fetched_at: u64,
    ) -> Result<Snapshot, EbirdError> {
        let url = format!("{}{}", self.base_url, scope.path_and_query(back_days));
        let res = self
            .http
            .get(&url)
            .header(API_KEY_HEADER, &self.api_key)
            .send()
            .await?;
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(EbirdError::Unauthorized(status.as_u16()));
        }
        if !status.is_success() {
            // The URL is safe to name — unlike the Wunderground one, the key
            // travels in a header, not the query string — but it is still
            // left out so one log line cannot grow a key by a later edit
            // moving the credential.
            return Err(EbirdError::Api(format!(
                "eBird returned {status} for recent observations {}",
                scope.label()
            )));
        }
        let body = res.bytes().await?;
        let raw: Vec<RawObservation> = serde_json::from_slice(&body)?;
        Ok(Snapshot::from_raw(scope, back_days, fetched_at, raw))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Client, DEFAULT_BACK_DAYS, DEFAULT_DIST_KM, EbirdError, MAX_BACK_DAYS, MAX_DIST_KM,
        NearbySpecies, RawObservation, Scope, Snapshot, load, now_unix, resolve, store,
    };

    /// eBird's own documented example body for `data/obs/geo/recent`
    /// ("Default Sort By Date"), verbatim. Three species, no `subId`.
    const DOCUMENTED_GEO: &str = include_str!("testdata/ebird_recent_geo.json");

    /// eBird's own documented example body for `data/obs/{regionCode}/recent`
    /// ("Simple Detail"), trimmed to its first three entries. Carries `subId`,
    /// which this decoder does not name.
    const DOCUMENTED_REGION: &str = include_str!("testdata/ebird_recent_region.json");

    /// eBird's own documented example body for the same endpoint sorted
    /// taxonomically, verbatim. Carries `Anser sp. (Domestic type)` — a row
    /// from the `domestic` taxonomic category, which eBird returns by default
    /// and which is not a binomial at all.
    const DOCUMENTED_GEO_TAXONOMIC: &str = include_str!("testdata/ebird_recent_geo_taxonomic.json");

    fn decode(body: &str) -> Vec<RawObservation> {
        serde_json::from_str(body).expect("the documented body decodes")
    }

    fn snapshot_of(body: &str) -> Snapshot {
        Snapshot::from_raw(
            &some_scope(),
            DEFAULT_BACK_DAYS,
            1_700_000_000,
            decode(body),
        )
    }

    fn some_scope() -> Scope {
        Scope::nearby(42.45, -76.48, DEFAULT_DIST_KM).expect("valid scope")
    }

    // ── the documented shape ────────────────────────────────────────────

    /// The geo fixture is eBird's own documented response. If this stops
    /// decoding, either the fixture was edited or the struct drifted.
    ///
    /// Observed failing by renaming the `sciName` serde attribute to
    /// `scientificName`; serde then reported a missing `sciName` field.
    #[test]
    fn the_documented_nearby_response_decodes_to_the_species_it_names() {
        let snap = snapshot_of(DOCUMENTED_GEO);
        assert_eq!(snap.len(), 3, "{:?}", snap.species);
        let names: Vec<&str> = snap.species.iter().map(|s| s.sci_name.as_str()).collect();
        assert_eq!(
            names,
            [
                "catharus fuscescens",
                "catharus ustulatus",
                "seiurus aurocapilla"
            ]
        );
    }

    /// The region fixture carries `subId`, and the notable-observations
    /// endpoint carries fifteen more fields than this decoder names. Unknown
    /// fields must be ignored rather than refused.
    ///
    /// Observed failing by adding `#[serde(deny_unknown_fields)]` to
    /// `RawObservation`; serde then rejected the unknown `speciesCode`.
    #[test]
    fn fields_this_decoder_does_not_name_are_ignored_rather_than_fatal() {
        let snap = snapshot_of(DOCUMENTED_REGION);
        assert_eq!(snap.len(), 3, "{:?}", snap.species);
        assert!(
            snap.reported("Corvus cornix").is_some(),
            "{:?}",
            snap.species
        );
    }

    /// Constructed, not from eBird: the documented bodies all carry
    /// `howMany`, but eBird records presence without a count as well, and a
    /// whole snapshot must not fail to decode because one observer wrote "X".
    ///
    /// Observed failing with `how_many: i64` rather than an `Option`; serde
    /// then reported a missing `howMany` field.
    #[test]
    fn a_count_the_observer_never_recorded_is_absent_rather_than_zero() {
        let constructed =
            r#"[{"sciName":"Turdus merula","comName":"Blackbird","obsDt":"2026-09-01 07:14"}]"#;
        let snap = snapshot_of(constructed);
        let bird = snap.reported("Turdus merula").expect("decoded");
        assert_eq!(
            bird.how_many, None,
            "an unrecorded count must not become a count of zero"
        );
    }

    /// Constructed, not from eBird: a checklist with no start time yields a
    /// date-only `obsDt`. Taking the text before the first space handles both
    /// shapes; a fixed-width slice would not.
    #[test]
    fn a_date_only_timestamp_is_read_as_that_date() {
        let constructed =
            r#"[{"sciName":"Turdus merula","obsDt":"2026-09-01","locName":"A hedge","howMany":2}]"#;
        let snap = snapshot_of(constructed);
        let bird = snap.reported("Turdus merula").expect("decoded");
        assert_eq!(bird.last_seen, "2026-09-01");
    }

    /// eBird's default taxonomic category is "all", so a response carries
    /// rows that are not binomials at all — `Anser sp. (Domestic type)` is in
    /// its own documented example. Such a row must neither be dropped nor
    /// corroborate a real species.
    #[test]
    fn a_row_that_is_not_a_binomial_neither_breaks_nor_corroborates() {
        let snap = snapshot_of(DOCUMENTED_GEO_TAXONOMIC);
        assert_eq!(snap.len(), 3, "{:?}", snap.species);
        assert!(snap.reported("Branta canadensis").is_some());
        // `Anser sp. (Domestic type)` collapses to `anser sp.`, which is not
        // any species the classifier can name.
        assert!(snap.reported("Anser anser").is_none(), "{:?}", snap.species);
        assert!(snap.reported("Anser sp.").is_some(), "{:?}", snap.species);
    }

    // ── matching ────────────────────────────────────────────────────────

    /// eBird's default taxonomic category is "all", so it returns subspecies
    /// rows. The classifier only knows binomials, so a subspecies report must
    /// corroborate its species or it corroborates nothing.
    ///
    /// Observed failing with `binomial` returning `None` unconditionally:
    /// the lookup missed and `reported` returned `None`.
    #[test]
    fn a_subspecies_report_corroborates_the_species() {
        let constructed = r#"[{"sciName":"Junco hyemalis hyemalis","obsDt":"2026-09-01 08:00"}]"#;
        let snap = snapshot_of(constructed);
        assert!(
            snap.reported("Junco hyemalis").is_some(),
            "{:?}",
            snap.species
        );
    }

    /// The counterpart to the subspecies gate: collapsing to a binomial must
    /// not collapse to a genus. A different species in the same genus is a
    /// different bird and corroborates nothing.
    ///
    /// Observed failing with `binomial` rewritten to take the first word:
    /// both juncos collapsed to `junco` and the wrong one matched.
    #[test]
    fn a_different_species_in_the_same_genus_is_not_a_match() {
        let constructed = r#"[{"sciName":"Junco phaeonotus","obsDt":"2026-09-01 08:00"}]"#;
        let snap = snapshot_of(constructed);
        assert!(
            snap.reported("Junco hyemalis").is_none(),
            "Yellow-eyed Junco must not corroborate Dark-eyed Junco: {:?}",
            snap.species
        );
    }

    /// Case and spacing differ between label files; the taxonomy does not.
    #[test]
    fn a_name_spelled_with_different_case_and_spacing_still_matches() {
        let snap = snapshot_of(DOCUMENTED_GEO);
        assert!(snap.reported("  catharus   FUSCESCENS ").is_some());
    }

    /// A species nobody reported is simply absent. The module's whole
    /// premise is that this means nothing, so it must not be an error, an
    /// empty-string match, or a panic.
    #[test]
    fn a_species_nobody_reported_is_absent() {
        let snap = snapshot_of(DOCUMENTED_GEO);
        assert!(snap.reported("Turdus merula").is_none());
        assert!(snap.reported("").is_none());
    }

    /// eBird documents one row per species, but subspecies collapse onto
    /// their binomial here, so two rows can land on one key. The later report
    /// is the one worth showing.
    ///
    /// Observed failing with the comparison reversed (`>` for `<`): the
    /// snapshot kept 2026-08-02 and the assertion read it back.
    #[test]
    fn the_most_recent_report_of_a_species_is_the_one_kept() {
        let constructed = r#"[
            {"sciName":"Junco hyemalis hyemalis","obsDt":"2026-08-02 08:00","locName":"Old"},
            {"sciName":"Junco hyemalis oreganus","obsDt":"2026-09-09 08:00","locName":"New"}
        ]"#;
        let snap = snapshot_of(constructed);
        assert_eq!(snap.len(), 1, "{:?}", snap.species);
        let bird = snap.reported("Junco hyemalis").expect("decoded");
        assert_eq!(bird.last_seen, "2026-09-09");
        assert_eq!(bird.locality, "New");
    }

    // ── scope ───────────────────────────────────────────────────────────

    /// eBird documents that it takes coordinates "to 2 decimal places", and
    /// two decimal places is also about a kilometre — as much of the
    /// station's position as needs to leave the machine.
    ///
    /// Observed failing with `{lat}` for `{lat:.2}`: the query carried
    /// `lat=42.4478603`.
    #[test]
    fn the_nearby_scope_rounds_the_stations_position_to_a_kilometre() {
        let scope = Scope::nearby(42.447_860_3, -76.485_951_2, 25).expect("valid");
        let target = scope.path_and_query(14);
        assert!(
            target.contains("lat=42.45") && target.contains("lng=-76.49"),
            "{target}"
        );
        assert!(
            !target.contains("42.4478"),
            "the full-precision position must not leave the machine: {target}"
        );
    }

    /// The two scopes ask two different endpoints.
    #[test]
    fn each_scope_asks_its_own_endpoint() {
        let nearby = Scope::nearby(42.45, -76.48, 10).expect("valid");
        let target = nearby.path_and_query(7);
        assert!(target.starts_with("/v2/data/obs/geo/recent?"), "{target}");
        assert!(
            target.contains("dist=10") && target.contains("back=7"),
            "{target}"
        );

        let region = Scope::region("US-MA").expect("valid");
        let target = region.path_and_query(7);
        assert_eq!(target, "/v2/data/obs/US-MA/recent?back=7");
    }

    /// A region code is interpolated straight into a URL path, so it must not
    /// be able to carry a path segment or a query string.
    ///
    /// Observed failing with the character check removed: `../../../ref`
    /// was accepted and produced `/v2/data/obs/../../../ref/recent`.
    #[test]
    fn a_region_code_cannot_smuggle_a_path_or_a_query_into_the_url() {
        for bad in [
            "../../../ref/hotspot/US-MA",
            "US-MA/recent?back=30&x=",
            "US MA",
            "",
            "   ",
        ] {
            match Scope::region(bad) {
                Err(EbirdError::Config(_)) => {}
                Err(other) => panic!("{bad:?} was refused for the wrong reason: {other}"),
                Ok(scope) => panic!("{bad:?} was accepted as {:?}", scope.key()),
            }
        }
        assert!(Scope::region("GB-ENG-CAM").is_ok());
    }

    /// A radius eBird would refuse is refused here, with the operator's own
    /// key named, rather than being silently clamped to something they did
    /// not ask for.
    #[test]
    fn a_radius_ebird_would_refuse_is_refused_here() {
        match Scope::nearby(42.45, -76.48, MAX_DIST_KM + 1) {
            Err(EbirdError::Config(m)) => assert!(m.contains("EBIRD_DIST_KM"), "{m}"),
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(_) => panic!("a radius above eBird's maximum was accepted"),
        }
        assert!(Scope::nearby(42.45, -76.48, MAX_DIST_KM).is_ok());
    }

    /// Coordinates that are not a point on Earth are refused.
    #[test]
    fn coordinates_that_are_not_a_point_on_earth_are_refused() {
        assert!(Scope::nearby(91.0, 0.0, 25).is_err());
        assert!(Scope::nearby(0.0, 181.0, 25).is_err());
        assert!(Scope::nearby(-90.0, 180.0, 25).is_ok());
    }

    /// A snapshot taken for one scope must not answer for another: moving the
    /// station or changing the region invalidates it.
    #[test]
    fn a_snapshot_does_not_answer_for_a_scope_it_was_not_taken_for() {
        let snap = snapshot_of(DOCUMENTED_GEO);
        assert!(snap.covers(&some_scope()));
        assert!(!snap.covers(&Scope::region("US-MA").expect("valid")));
        assert!(!snap.covers(&Scope::nearby(42.45, -76.48, 50).expect("valid")));
        // Sub-kilometre GPS jitter must not throw the cache away.
        assert!(snap.covers(&Scope::nearby(42.4502, -76.4798, DEFAULT_DIST_KM).expect("valid")));
    }

    // ── staleness ───────────────────────────────────────────────────────

    /// A station whose clock is corrected forwards after a snapshot was
    /// written must not read that snapshot as decades old.
    ///
    /// Observed failing with `now - self.fetched_at`: the subtraction
    /// overflowed and the test aborted with "attempt to subtract with
    /// overflow".
    #[test]
    fn a_snapshot_stamped_in_the_future_reads_as_new_rather_than_ancient() {
        let snap = snapshot_of(DOCUMENTED_GEO);
        assert_eq!(snap.age_secs(snap.fetched_at - 10_000), 0);
        assert!(!snap.is_stale(snap.fetched_at - 10_000));
    }

    /// Staleness is measured, not assumed.
    #[test]
    fn a_snapshot_becomes_stale_only_after_the_stated_window() {
        let snap = snapshot_of(DOCUMENTED_GEO);
        let stale_at = snap.fetched_at + super::STALE_AFTER.as_secs();
        assert!(!snap.is_stale(stale_at), "not stale at exactly the window");
        assert!(snap.is_stale(stale_at + 1));
    }

    // ── the disk cache ──────────────────────────────────────────────────

    /// The point of the cache: an offline station keeps what it had.
    #[test]
    fn a_snapshot_survives_a_round_trip_through_the_cache_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = super::cache_path(dir.path());
        let snap = snapshot_of(DOCUMENTED_GEO);
        store(&path, &snap).expect("stored");
        let back = load(&path).expect("loaded");
        assert_eq!(back, snap);
    }

    /// The write is a write-then-rename, so nothing must be left behind for
    /// the next `load` to trip over or for the disk to fill with.
    ///
    /// Observed failing with the `rename` replaced by a plain write to the
    /// temporary path: `ebird-nearby.json.tmp` was still there.
    #[test]
    fn storing_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = super::cache_path(dir.path());
        store(&path, &snapshot_of(DOCUMENTED_GEO)).expect("stored");
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
            .collect();
        assert_eq!(left, ["ebird-nearby.json"], "{left:?}");
    }

    /// A cache file truncated by a power cut must not stop the station
    /// coming up.
    #[test]
    fn an_unreadable_cache_file_is_ignored_rather_than_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = super::cache_path(dir.path());
        std::fs::write(&path, b"{\"fetched_at\": ").expect("write");
        assert!(load(&path).is_none());
        assert!(load(&dir.path().join("never-written.json")).is_none());
    }

    // ── configuration ───────────────────────────────────────────────────

    /// No key is off, silently and without error: that is the state of every
    /// fresh install, and it must not log a warning every boot.
    ///
    /// Observed failing with the empty-key filter removed: a configured-but-
    /// blank `EBIRD_API_KEY` produced `Ok(Some(..))` and the poll started
    /// with an empty credential.
    #[test]
    fn no_api_key_is_off_rather_than_an_error() {
        for key in [None, Some(""), Some("   ")] {
            match resolve(key, None, None, None, Some(42.45), Some(-76.48)) {
                Ok(None) => {}
                Ok(Some(s)) => panic!("{key:?} started the poll: {s:?}"),
                Err(e) => panic!("{key:?} was an error: {e}"),
            }
        }
    }

    /// A key with nowhere to ask is an error, not a silent off: the operator
    /// asked for this and must be told why they are not getting it.
    #[test]
    fn a_key_with_no_region_and_no_coordinates_says_so() {
        match resolve(Some("k"), None, None, None, None, None) {
            Err(EbirdError::Config(m)) => {
                assert!(m.contains("LATITUDE") && m.contains("EBIRD_REGION"), "{m}");
            }
            Err(other) => panic!("wrong error: {other}"),
            Ok(v) => panic!("accepted with nothing to ask about: {v:?}"),
        }
    }

    /// Setting a region is an explicit act, and the only reason to do it is
    /// to override the coordinates the station already has.
    ///
    /// Observed failing with the match arms reordered so coordinates are
    /// tried first: the scope came back as `geo:42.45,-76.48:25`.
    #[test]
    fn a_named_region_wins_over_the_stations_coordinates() {
        let s = resolve(
            Some("k"),
            Some("US-MA"),
            None,
            None,
            Some(42.45),
            Some(-76.48),
        )
        .expect("resolved")
        .expect("enabled");
        assert_eq!(s.scope, Scope::Region("US-MA".to_owned()));
        assert_eq!(s.back_days, DEFAULT_BACK_DAYS);
    }

    /// Coordinates are used when no region overrides them.
    #[test]
    fn the_stations_coordinates_are_used_when_no_region_is_named() {
        let s = resolve(
            Some("k"),
            None,
            Some("10"),
            Some("3"),
            Some(42.45),
            Some(-76.48),
        )
        .expect("resolved")
        .expect("enabled");
        assert_eq!(
            s.scope,
            Scope::Nearby {
                lat: 42.45,
                lng: -76.48,
                dist_km: 10
            }
        );
        assert_eq!(s.back_days, 3);
        assert_eq!(s.api_key, "k");
    }

    /// A number eBird would refuse, and a value that is not a number at all,
    /// are both named rather than silently defaulted.
    ///
    /// Observed failing with `parse_range` falling back to the default on a
    /// parse error: `"fourteen"` resolved to 14 and the operator's typo was
    /// invisible.
    #[test]
    fn a_setting_outside_ebirds_range_is_named_rather_than_defaulted() {
        for bad in ["0", "31", "fourteen", "-1"] {
            match resolve(Some("k"), None, None, Some(bad), Some(42.45), Some(-76.48)) {
                Err(EbirdError::Config(m)) => assert!(m.contains("EBIRD_BACK_DAYS"), "{m}"),
                Err(other) => panic!("{bad:?} refused for the wrong reason: {other}"),
                Ok(v) => panic!("{bad:?} was accepted: {v:?}"),
            }
        }
        for good in ["1", &MAX_BACK_DAYS.to_string()] {
            assert!(
                resolve(Some("k"), None, None, Some(good), Some(42.45), Some(-76.48)).is_ok(),
                "{good:?} is inside eBird's range and must be accepted"
            );
        }
    }

    // ── the request on the wire ─────────────────────────────────────────

    /// The API key travels in the `x-ebirdapitoken` header and **never** in
    /// the URL: a key in a query string ends up in proxy logs, in the
    /// station's own journal on every error, and in any referrer.
    ///
    /// Observed failing by moving the key to `?key=` in `path_and_query`:
    /// the request target came back carrying `key=super-secret-token`.
    #[tokio::test]
    async fn the_api_key_travels_in_a_header_and_never_in_the_url() {
        let (addr, request) = spawn_one_shot_server().await;
        let client = Client::new("super-secret-token", Some(format!("http://{addr}"))).expect("c");
        // The body is `[]`, so the snapshot is empty; the request is the gate.
        let _ = client.recent(&some_scope(), 14, now_unix()).await;

        let req = request.await.expect("the server saw a request");
        let target = req
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        assert!(
            !target.contains("super-secret-token"),
            "the key must not be in the URL: {target}"
        );
        assert!(
            req.to_ascii_lowercase()
                .contains("x-ebirdapitoken: super-secret-token"),
            "the key must be in the documented header; request was:\n{req}"
        );
    }

    /// A 403 — what a missing, wrong or revoked key produces — is reported as
    /// an API error naming the status, not as a decode failure or an empty
    /// snapshot that reads as "nobody reported anything".
    ///
    /// Observed failing with the status check removed: the empty body decoded
    /// and `recent` returned an empty snapshot for a rejected key.
    #[tokio::test]
    async fn a_rejected_key_is_an_error_and_not_an_empty_snapshot() {
        let (addr, _req) = spawn_refusing_server().await;
        let client = Client::new("wrong-key", Some(format!("http://{addr}"))).expect("c");
        match client.recent(&some_scope(), 14, now_unix()).await {
            Err(e @ EbirdError::Unauthorized(403)) => {
                let shown = e.to_string();
                assert!(shown.contains("EBIRD_API_KEY"), "{shown}");
                assert!(
                    !shown.contains("wrong-key"),
                    "the key must not be logged: {shown}"
                );
            }
            Err(other) => panic!("wrong error: {other}"),
            Ok(snap) => panic!(
                "a rejected key produced a snapshot of {} species",
                snap.len()
            ),
        }
    }

    /// A listener that accepts one request, hands back the whole request
    /// text, and answers with an empty JSON array.
    async fn spawn_one_shot_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n[]",
        )
        .await
    }

    /// A listener that refuses with eBird's own rejection status.
    async fn spawn_refusing_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        serve_once("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").await
    }

    async fn serve_once(
        response: &'static str,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
            request
        });
        (addr, handle)
    }

    /// Snapshots are ordinary data and must serialise the way the cache file
    /// expects; this pins the field names so a rename is caught here rather
    /// than by every station silently losing its cache after an upgrade.
    #[test]
    fn the_cache_file_names_its_fields_the_way_it_reads_them() {
        let snap = Snapshot {
            fetched_at: 1,
            scope: "region:US-MA".to_owned(),
            scope_label: "in US-MA".to_owned(),
            back_days: 14,
            species: vec![NearbySpecies {
                sci_name: "turdus merula".to_owned(),
                com_name: "Eurasian Blackbird".to_owned(),
                last_seen: "2026-09-01".to_owned(),
                locality: "A hedge".to_owned(),
                how_many: Some(2),
            }],
        };
        let json = serde_json::to_string(&snap).expect("serialised");
        let back: Snapshot = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back, snap);
        for field in [
            "fetched_at",
            "scope",
            "scope_label",
            "back_days",
            "species",
            "sci_name",
            "last_seen",
            "how_many",
        ] {
            assert!(json.contains(field), "{field} missing from {json}");
        }
    }
}
