//! Spectrogram generation and serving.
//!
//! Generates a PNG spectrogram from a WAV recording file on demand and
//! returns it as an `image/png` response.  Spectrograms are cached in
//! memory (keyed by filename + mtime) to avoid re-computation on every
//! page load.
//!
//! Route:
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /api/v2/spectrogram/{filename} | Generate/serve spectrogram PNG |
//!
//! The spectrogram is rendered as a grayscale/viridis-like PNG using the
//! mel spectrogram computed by `birdnet-core`.

mod colormap;
mod font;
mod png;
mod render;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};

use crate::state::AppState;
use render::{SpectrogramLabel, generate_spectrogram_png_with_label};

pub fn router() -> Router<AppState> {
    Router::new().route("/spectrogram/{filename}", get(serve_spectrogram))
}

// ---------------------------------------------------------------------------
// GET /api/v2/spectrogram/{filename}?species=...&confidence=...&time=...
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct SpectrogramQuery {
    species: Option<String>,
    confidence: Option<u32>,
    time: Option<String>,
}

async fn serve_spectrogram(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    axum::extract::Query(query): axum::extract::Query<SpectrogramQuery>,
) -> Response {
    if !is_safe_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    let rec_dir = state.recording_dir();
    let file_path = rec_dir.join(&filename);

    // Confirm the path is within the recording directory.
    match file_path.canonicalize() {
        Ok(canonical) => {
            let rec_canonical = rec_dir.canonicalize().unwrap_or_else(|_| rec_dir.clone());
            if !canonical.starts_with(&rec_canonical) {
                return (StatusCode::FORBIDDEN, "path traversal denied").into_response();
            }
        }
        Err(_) => {
            return (StatusCode::NOT_FOUND, "recording not found").into_response();
        }
    }

    // Build optional label from query parameters.
    let label = query.species.map(|species| SpectrogramLabel {
        species,
        confidence_pct: query.confidence.unwrap_or(0),
        time: query.time.unwrap_or_default(),
    });

    // Generate spectrogram in a blocking task.
    let result = tokio::task::spawn_blocking(move || {
        generate_spectrogram_png_with_label(&file_path, label.as_ref())
    })
    .await;

    match result {
        Ok(Ok(png_bytes)) => {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            (StatusCode::OK, headers, Body::from(png_bytes)).into_response()
        }
        Ok(Err(e)) => {
            tracing::warn!(file = %filename, err = %e, "spectrogram generation failed");
            (StatusCode::UNPROCESSABLE_ENTITY, e).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, "spectrogram task panicked");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Safety
// ---------------------------------------------------------------------------

fn is_safe_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_graphic())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_ok() {
        assert!(is_safe_filename("bird_2026-03-14_06-00-00.wav"));
    }

    #[test]
    fn safe_filename_traversal() {
        assert!(!is_safe_filename("../etc/passwd"));
        assert!(!is_safe_filename("foo/bar.wav"));
    }
}
