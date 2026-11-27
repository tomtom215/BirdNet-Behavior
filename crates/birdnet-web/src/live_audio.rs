//! Who may listen to the live microphone (`OOB-13`).
//!
//! Stored clips pass the human-sound filter before anyone can play them; the
//! live feed passes none. Until this module, `/stream` and the HLS stream were
//! open to everyone who could reach the port, while the filtered recordings
//! were the surface the station treated with care. The setting here closes the
//! unfiltered one by default.
//!
//! # The setting
//!
//! `live_audio` in the `settings` table, changed on **Admin → Audio**:
//!
//! * `signed-in` — the audio routes need the same sign-in as the admin panel.
//!   A station with no admin password has an open admin panel, and its audio
//!   is open on the same terms (the open-admin host rule in
//!   [`crate::open_admin_host`]): requiring a sign-in nobody can perform would
//!   lock the owner out of their own microphone.
//! * `anyone` — the behaviour before this setting existed.
//!
//! # Which stations get which
//!
//! [`seed`] runs once at every start and writes a value only when there is
//! none. A station that already holds detections when it first starts on this
//! version is an existing station and is seeded `anyone`: what its household
//! could hear yesterday it can hear today. A station with no detections is a
//! new install and is seeded `signed-in`. A missing or unreadable value is read
//! as `signed-in`, so a failed seed or a failed read closes the feed rather
//! than opening it.
//!
//! # What it covers
//!
//! `/stream` and the HLS control routes (`/api/v2/streams/hls/…`: start, stop,
//! heartbeat, status). Not the HLS content under `/api/v2/streams/hls/t/…`:
//! that is addressed by a 128-bit token which only a permitted start returns,
//! and it is what a player that cannot send the station's cookie (a media
//! player handed the playlist URL) uses. Not the live spectrogram socket, which
//! is a 128-band picture the Today page draws for visitors (decision 5 of
//! `docs/audit-2026-11/decisions-2026-11-25.md`; whether speech can be
//! recovered from those frames has not been measured).
//!
//! On a station in private mode this module stands aside: the private gate
//! already requires a session for everything, and its `live_audio` carve-out
//! is the operator's explicit statement that the feed is public.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::Connection;

use crate::state::AppState;

/// The settings-table key.
pub const LIVE_AUDIO_SETTING: &str = "live_audio";

/// Who may listen to the live audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveAudioPolicy {
    /// Signed-in users only (the admin panel's sign-in).
    SignedIn,
    /// Anyone who can reach the station.
    Anyone,
}

impl LiveAudioPolicy {
    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SignedIn => "signed-in",
            Self::Anyone => "anyone",
        }
    }

    /// Parse the stored spelling. Anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "signed-in" | "signed_in" | "signedin" => Some(Self::SignedIn),
            "anyone" => Some(Self::Anyone),
            _ => None,
        }
    }
}

/// Whether a request path is one of the live-audio routes this policy covers.
///
/// `path` is as the router sees it, base path already stripped.
#[must_use]
pub fn is_live_audio_path(path: &str) -> bool {
    path == "/stream"
        || (path.starts_with("/api/v2/streams/hls/") && !path.starts_with("/api/v2/streams/hls/t/"))
}

/// The station's policy. Absent, unreadable or misspelt reads as
/// [`LiveAudioPolicy::SignedIn`]: this is a gate, and a gate that cannot read
/// its own setting stays shut.
#[must_use]
pub fn policy(conn: &Connection) -> LiveAudioPolicy {
    use birdnet_db::settings::SettingsError;
    match birdnet_db::settings::get(conn, LIVE_AUDIO_SETTING) {
        Ok(v) => LiveAudioPolicy::parse(&v).unwrap_or_else(|| {
            tracing::warn!(
                value = %v,
                "live_audio holds neither signed-in nor anyone; treating it as signed-in"
            );
            LiveAudioPolicy::SignedIn
        }),
        Err(SettingsError::NotFound(_)) => LiveAudioPolicy::SignedIn,
        Err(e) => {
            tracing::warn!(error = %e, "live_audio could not be read; treating it as signed-in");
            LiveAudioPolicy::SignedIn
        }
    }
}

/// Write the policy a station starts with, if it has none.
///
/// `anyone` for a station that already holds detections (an upgrade keeps what
/// it had), `signed-in` otherwise (a new install). One statement, so the check
/// and the write cannot be separated by a detection landing between them.
///
/// Returns the policy written, or `None` when the station already had one.
///
/// # Errors
///
/// Any `SQLite` error. Nothing is written then, and [`policy`] reads the
/// missing value as signed-in.
pub fn seed(conn: &Connection) -> Result<Option<LiveAudioPolicy>, rusqlite::Error> {
    birdnet_db::settings::ensure_settings_table(conn).map_err(|e| match e {
        birdnet_db::settings::SettingsError::Sqlite(e) => e,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })?;
    let written = conn.execute(
        "INSERT INTO settings (key, value, category, updated_at) \
         SELECT ?1, CASE WHEN EXISTS (SELECT 1 FROM detections) THEN ?2 ELSE ?3 END, \
                'audio', datetime('now') \
         WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = ?1)",
        [
            LIVE_AUDIO_SETTING,
            LiveAudioPolicy::Anyone.as_str(),
            LiveAudioPolicy::SignedIn.as_str(),
        ],
    )?;
    if written == 0 {
        return Ok(None);
    }
    Ok(Some(policy(conn)))
}

/// [`seed`] against the station's database, logging what it did. The one call
/// the start-up path makes.
pub fn seed_at_start(state: &AppState) {
    match state.with_db(seed) {
        Ok(Some(LiveAudioPolicy::Anyone)) => tracing::info!(
            "live audio: this station already holds detections, so its live feed stays open \
             to anyone who can reach it (live_audio=anyone); change it on Admin → Audio"
        ),
        Ok(Some(LiveAudioPolicy::SignedIn)) => tracing::info!(
            "live audio: a new station; its live feed needs a sign-in (live_audio=signed-in); \
             change it on Admin → Audio"
        ),
        Ok(None) => {}
        Err(e) => tracing::warn!(
            error = %e,
            "live audio: the starting policy could not be written; the live feed needs a sign-in \
             until it is set on Admin → Audio"
        ),
    }
}

/// Whether this request may listen. Blocks on the database: call it from
/// [`AppState::run_blocking`].
///
/// `cookie` is the request's `Cookie` header, `authority` its `Host`.
#[must_use]
pub fn may_listen(state: &AppState, cookie: Option<&str>, authority: Option<&str>) -> bool {
    if state.private_mode() {
        // The private gate has already decided; see the module docs.
        return true;
    }
    if state.with_read_db(policy) == LiveAudioPolicy::Anyone {
        return true;
    }
    if !crate::auth_middleware::admin_password_configured(state) {
        // No password, so no sign-in is possible: the feed is open on the
        // terms the admin panel is, under a name no outside page can rebind.
        return crate::open_admin_host::may_open_bypass(authority);
    }
    let token = cookie
        .and_then(crate::session::extract_token)
        .and_then(crate::session::validate_token);
    crate::auth_middleware::session_user(token, state).is_some()
}

/// [`may_listen`] for a handler holding the request's headers.
#[must_use]
pub fn may_listen_with_headers(state: &AppState, headers: &HeaderMap) -> bool {
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    may_listen(state, cookie, host)
}

/// The sentence a refused listener is shown, wherever they are shown it.
pub const SIGN_IN_TO_LISTEN: &str = "Live audio on this station is for signed-in users. \
     Sign in to listen, or an administrator can open it to anyone on Admin → Audio.";

/// The refusal a live-audio route gives. `401` and a body that says what to
/// do: an `<audio>` element cannot follow a redirect to a form, and the HLS
/// start is a `fetch` that reads the JSON.
#[must_use]
pub fn refusal(path: &str) -> Response {
    let login = "/login?next=%2Frecordings%3Fview%3Dlive";
    if path.starts_with("/api/") {
        return (
            StatusCode::UNAUTHORIZED,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            serde_json::json!({
                "error": "sign in required: live audio on this station is for signed-in users",
                "login": login,
                "setting": LIVE_AUDIO_SETTING,
            })
            .to_string(),
        )
            .into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        format!("{SIGN_IN_TO_LISTEN}\nSign in: {login}\n"),
    )
        .into_response()
}
