//! Live audio streaming routes.
//!
//! Provides live audio streaming from the microphone (ALSA) or RTSP source
//! to the browser, replacing BirdNET-Pi's Icecast2 dependency.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET /stream` | Raw MP3 audio stream via HTTP chunked transfer |
//! | `GET /api/v2/languages` | List available i18n languages |

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::state::AppState;

/// Query parameters for the live audio stream.
#[derive(Debug, Deserialize)]
pub struct StreamParams {
    /// Frequency shift in Hz applied to the live stream (positive = shift up).
    ///
    /// Uses ffmpeg `asetrate` + `aresample` filter chain. Useful for accessibility
    /// (hearing loss compensation) or monitoring bat calls shifted into audible range.
    /// BirdNET-Pi equivalent: rubberband pitch shift filter.
    #[serde(default)]
    pub freq_shift_hz: i32,
}

/// Mount livestream and i18n routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/languages", get(list_languages))
}

/// Mount the raw audio stream route (top-level, not under /api/v2).
pub fn stream_router() -> Router<AppState> {
    Router::new().route("/stream", get(livestream))
}

/// Build the ffmpeg filter string for an optional frequency shift.
///
/// A non-zero shift applies `asetrate` (reinterpret sample rate) followed by
/// `aresample` (resample back to 44100 Hz), which shifts the perceived pitch
/// without stretching duration — equivalent to BirdNET-Pi's rubberband filter.
fn freq_shift_filter(base_rate: u32, shift_hz: i32) -> Option<String> {
    if shift_hz == 0 {
        return None;
    }
    let shifted_rate = (base_rate as i64 + shift_hz as i64).max(8000) as u32;
    Some(format!(
        "asetrate={shifted_rate},aresample={base_rate}:resampler=swr"
    ))
}

// ---------------------------------------------------------------------------
// GET /api/v2/languages
// ---------------------------------------------------------------------------

/// List available languages for species name translation.
///
/// Returns a JSON array of `{"code": "...", "name": "..."}` objects for all
/// loaded language packs. If no i18n manager is configured, returns the full
/// list of supported languages (indicating they *could* be loaded).
async fn list_languages(State(state): State<AppState>) -> Json<Value> {
    let langs = state.with_i18n_ref(|mgr| {
        mgr.available_languages()
            .into_iter()
            .map(|(code, name)| {
                json!({
                    "code": code,
                    "name": name,
                })
            })
            .collect::<Vec<_>>()
    });

    let langs = langs.unwrap_or_else(|| {
        birdnet_core::i18n::SUPPORTED_LANGUAGES
            .iter()
            .map(|(code, name)| {
                json!({
                    "code": code,
                    "name": name,
                })
            })
            .collect()
    });

    Json(json!({
        "languages": langs,
        "count": langs.len(),
    }))
}

// ---------------------------------------------------------------------------
// GET /stream
// ---------------------------------------------------------------------------

/// Stream live audio as MP3 via HTTP chunked transfer.
///
/// Uses `ffmpeg` to capture from ALSA, PulseAudio/PipeWire, or RTSP and encode
/// to MP3, streaming stdout directly as the HTTP response body.
///
/// Supports optional frequency shifting via `?freq_shift_hz=<N>` query param
/// (positive = shift up, negative = shift down). Uses the same
/// `asetrate`+`aresample` technique as the extraction pipeline.
///
/// If no audio source is configured, returns `503 Service Unavailable`.
async fn livestream(
    State(state): State<AppState>,
    Query(params): Query<StreamParams>,
) -> Response {
    let Some(source) = state.audio_source() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no audio source configured",
        )
            .into_response();
    };

    const BASE_RATE: u32 = 44_100;
    let source = source.to_owned();
    let is_rtsp = source.starts_with("rtsp://") || source.starts_with("rtsps://");
    let is_pulse = source.starts_with("pulse://") || source == "pulse" || source == "default";

    // Build the audio filter chain: optional freq shift + format conversion.
    let audio_filter = freq_shift_filter(BASE_RATE, params.freq_shift_hz);

    let mut cmd = tokio::process::Command::new("ffmpeg");

    if is_rtsp {
        cmd.args(["-rtsp_transport", "tcp", "-i", &source, "-vn"]);
    } else if is_pulse {
        let pulse_src = source.trim_start_matches("pulse://");
        cmd.args(["-f", "pulse", "-i", pulse_src]);
    } else {
        // ALSA source (default)
        cmd.args(["-f", "alsa", "-i", &source]);
    }

    if let Some(ref filter) = audio_filter {
        cmd.args(["-af", filter.as_str()]);
    }

    cmd.args(["-f", "mp3", "-b:a", "128k", "-ar", &BASE_RATE.to_string(), "-ac", "1", "pipe:1"]);

    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to spawn ffmpeg for livestream");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to start audio stream",
            )
                .into_response();
        }
    };

    let Some(stdout) = child.stdout.take() else {
        tracing::error!("ffmpeg process has no stdout handle");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to capture audio stream",
        )
            .into_response();
    };

    tracing::info!(
        source = %source,
        freq_shift_hz = params.freq_shift_hz,
        "starting live audio stream"
    );

    let stream = ReaderStream::new(stdout).map(|result| {
        result.map_err(|e| {
            tracing::debug!(error = %e, "livestream read error");
            std::io::Error::other(e)
        })
    });

    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/mpeg"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store"),
    );
    // Indicate this is a continuous stream
    headers.insert(
        header::TRANSFER_ENCODING,
        HeaderValue::from_static("chunked"),
    );
    // ICY-compatible metadata
    headers.insert(
        axum::http::HeaderName::from_static("icy-name"),
        HeaderValue::from_static("BirdNet-Behavior Live"),
    );

    (StatusCode::OK, headers, body).into_response()
}
