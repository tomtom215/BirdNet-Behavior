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
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::state::AppState;

/// Mount livestream and i18n routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/languages", get(list_languages))
}

/// Mount the raw audio stream route (top-level, not under /api/v2).
pub fn stream_router() -> Router<AppState> {
    Router::new().route("/stream", get(livestream))
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
/// Uses `ffmpeg` to capture from ALSA or RTSP and encode to MP3, streaming
/// stdout directly as the HTTP response body with `Content-Type: audio/mpeg`.
///
/// If no audio source is configured, returns `503 Service Unavailable`.
async fn livestream(State(state): State<AppState>) -> Response {
    let Some(source) = state.audio_source() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no audio source configured",
        )
            .into_response();
    };

    let source = source.to_owned();
    let is_rtsp = source.starts_with("rtsp://") || source.starts_with("rtsps://");

    let child = if is_rtsp {
        // RTSP source
        tokio::process::Command::new("ffmpeg")
            .args([
                "-i",
                &source,
                "-vn", // no video
                "-f",
                "mp3",
                "-b:a",
                "128k",
                "-ar",
                "44100",
                "-ac",
                "1",
                "pipe:1",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
    } else {
        // ALSA source
        tokio::process::Command::new("ffmpeg")
            .args([
                "-f",
                "alsa",
                "-i",
                &source,
                "-f",
                "mp3",
                "-b:a",
                "128k",
                "-ar",
                "44100",
                "-ac",
                "1",
                "pipe:1",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
    };

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

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            tracing::error!("ffmpeg process has no stdout handle");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to capture audio stream",
            )
                .into_response();
        }
    };

    tracing::info!(source = %source, "starting live audio stream");

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
