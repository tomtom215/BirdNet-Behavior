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

/// Sample rate used for the live audio stream output (Hz).
const STREAM_SAMPLE_RATE: u32 = 44_100;

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
    /// Optional `audio_sources.id` selecting which configured source to
    /// stream. Without it, the legacy `state.audio_source()` string is
    /// used — preserves the BirdNET-Pi-compatible "one source per
    /// station" contract that pre-dates the `audio_sources` entity.
    #[serde(default)]
    pub source_id: Option<String>,
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
    // Use i64 arithmetic to avoid overflow then clamp to a safe minimum.
    let shifted = i64::from(base_rate) + i64::from(shift_hz);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let shifted_rate = shifted.max(8000) as u32;
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
///
/// When `?source_id=` is supplied, the row's `kind` + `device_id` from the
/// `audio_sources` table drives the ffmpeg backend selection — the
/// listen-now page uses this to switch between configured mics / streams
/// without restarting the daemon. An unknown id returns `404`.
async fn livestream(State(state): State<AppState>, Query(params): Query<StreamParams>) -> Response {
    // Resolve the audio source. Two paths:
    //   1. `?source_id=` → DB lookup; honour the row's kind explicitly.
    //   2. (no param) → legacy `state.audio_source()` string + heuristics.
    let (source, kind_hint) = match params.source_id.as_deref() {
        Some(id) if !id.is_empty() => match resolve_by_source_id(&state, id) {
            Some(pair) => pair,
            None => {
                return (StatusCode::NOT_FOUND, "no such audio source").into_response();
            }
        },
        _ => {
            let Some(s) = state.audio_source() else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "no audio source configured",
                )
                    .into_response();
            };
            (s.to_owned(), None)
        }
    };

    // Honour the audio_sources kind when present; otherwise fall back to
    // the URL-prefix heuristic that the single-string path has always used.
    let (is_rtsp, is_pulse) = kind_hint.map_or_else(
        || {
            (
                source.starts_with("rtsp://") || source.starts_with("rtsps://"),
                source.starts_with("pulse://") || source == "pulse" || source == "default",
            )
        },
        |k| {
            use birdnet_db::audio_sources::SourceKind;
            (matches!(k, SourceKind::Rtsp), matches!(k, SourceKind::PipeWire))
        },
    );

    // Build the audio filter chain: optional freq shift + format conversion.
    let audio_filter = freq_shift_filter(STREAM_SAMPLE_RATE, params.freq_shift_hz);

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

    cmd.args([
        "-f",
        "mp3",
        "-b:a",
        "128k",
        "-ar",
        &STREAM_SAMPLE_RATE.to_string(),
        "-ac",
        "1",
        "pipe:1",
    ]);

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

/// Resolve a `?source_id=` query value to a `(source-string, kind-hint)`
/// pair by looking up the configured `audio_sources` row.
///
/// * Returns `None` when the row is missing, disabled, or the DB read
///   itself fails. Callers treat all three as "404 no such source".
/// * The `kind-hint` is the `SourceKind` from the DB, used to bypass the
///   legacy URL-prefix heuristic so a PipeWire row whose `device_id` is
///   literally `default` (operator typed `default`) still picks the
///   PulseAudio ffmpeg backend rather than ALSA.
fn resolve_by_source_id(
    state: &AppState,
    id: &str,
) -> Option<(String, Option<birdnet_db::audio_sources::SourceKind>)> {
    use birdnet_db::audio_sources::AudioSourceStore;
    state.with_db(|conn| AudioSourceStore::list(conn).ok()).and_then(
        |sources| {
            sources.into_iter().find_map(|s| {
                if s.id == id && s.disabled_at.is_none() {
                    Some((s.device_id.clone(), Some(s.kind)))
                } else {
                    None
                }
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freq_shift_filter_off_when_zero() {
        assert!(freq_shift_filter(44_100, 0).is_none());
    }

    #[test]
    fn freq_shift_filter_clamps_to_minimum() {
        // 44100 - 50000 = -5900 → clamped to 8000.
        let filter = freq_shift_filter(44_100, -50_000).expect("non-zero shift returns filter");
        assert!(filter.contains("asetrate=8000"));
        assert!(filter.contains(",aresample=44100"));
    }

    #[test]
    fn freq_shift_filter_shifts_up_correctly() {
        let filter = freq_shift_filter(44_100, 3_000).expect("non-zero shift returns filter");
        assert!(filter.contains("asetrate=47100"));
    }
}
