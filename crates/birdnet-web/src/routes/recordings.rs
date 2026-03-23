//! Audio recording file serving routes.
//!
//! Serves extracted bird-call audio clips from the recording directory,
//! enabling the web UI to embed `<audio>` players next to each detection.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET /api/v2/recordings/{filename}` | Stream a single audio file |
//! | `GET /api/v2/recordings`            | List available recordings with metadata |

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

use crate::state::AppState;

/// Mount recording routes under the given prefix.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/recordings", get(list_recordings))
        .route("/recordings/{filename}", get(serve_recording))
}

// ---------------------------------------------------------------------------
// GET /api/v2/recordings/{filename}
// ---------------------------------------------------------------------------

/// Serve a single audio recording file.
///
/// Security: filename components are validated — only basename characters
/// allowed (no `..` or path separators) so callers cannot escape the
/// recording directory.
async fn serve_recording(State(state): State<AppState>, Path(filename): Path<String>) -> Response {
    if !is_safe_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    let rec_dir = state.recording_dir();
    let file_path = rec_dir.join(&filename);

    // Security: resolve canonical path and confirm it is inside rec_dir.
    let Ok(canonical) = file_path.canonicalize() else {
        return (StatusCode::NOT_FOUND, "recording not found").into_response();
    };

    let rec_dir_canonical = rec_dir.canonicalize().unwrap_or_else(|_| rec_dir.clone());
    if !canonical.starts_with(&rec_dir_canonical) {
        return (StatusCode::FORBIDDEN, "access denied").into_response();
    }

    let Ok(file) = File::open(&canonical).await else {
        return (StatusCode::NOT_FOUND, "recording not found").into_response();
    };

    let content_type = content_type_for(&filename);
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    // Allow browsers to range-request (seek in audio player)
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    // Prevent caching of potentially large files
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );

    (StatusCode::OK, headers, body).into_response()
}

// ---------------------------------------------------------------------------
// GET /api/v2/recordings
// ---------------------------------------------------------------------------

/// Recording metadata for the listing API.
#[derive(Debug, Serialize)]
pub struct RecordingMeta {
    /// Filename (basename only, no path).
    pub filename: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last-modified timestamp (Unix seconds).
    pub modified_secs: u64,
    /// Inferred MIME type.
    pub content_type: &'static str,
}

/// Query parameters for the recording list.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Optional species name filter (matches against filename).
    pub species: Option<String>,
    /// Maximum number of results (default 50, max 500).
    pub limit: Option<usize>,
    /// Offset for pagination.
    pub offset: Option<usize>,
}

/// List recordings in the recording directory.
async fn list_recordings(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let rec_dir = state.recording_dir();
    let species_filter = query.species.map(|s| s.to_lowercase());
    let limit = query.limit.unwrap_or(50).min(500);
    let offset = query.offset.unwrap_or(0);

    let result = tokio::task::spawn_blocking(move || {
        collect_recordings(&rec_dir, species_filter.as_deref(), limit, offset)
    })
    .await;

    result.map_or_else(
        |_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to list recordings",
            )
                .into_response()
        },
        |recordings| Json(recordings).into_response(),
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Validate that a filename is safe to use as a path component.
///
/// Only allows: ASCII alphanumeric, hyphens, underscores, dots, colons.
/// Rejects: path separators, null bytes, `..`, or non-UTF-8 sequences.
fn is_safe_filename(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// Return the MIME content-type for an audio filename.
fn content_type_for(filename: &str) -> &'static str {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("ogg" | "oga") => "audio/ogg",
        Some("m4a" | "aac") => "audio/aac",
        _ => "application/octet-stream",
    }
}

/// Collect recording metadata from a directory, applying filters and pagination.
fn collect_recordings(
    dir: &PathBuf,
    species_filter: Option<&str>,
    limit: usize,
    offset: usize,
) -> Vec<RecordingMeta> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut metas: Vec<RecordingMeta> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let filename = path.file_name()?.to_str()?.to_owned();
            if !is_audio_extension(&filename) {
                return None;
            }
            if let Some(filter) = species_filter
                && !filename.to_lowercase().contains(filter)
            {
                return None;
            }
            let meta = e.metadata().ok()?;
            let size_bytes = meta.len();
            let modified_secs = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let content_type = content_type_for(&filename);
            Some(RecordingMeta {
                filename,
                size_bytes,
                modified_secs,
                content_type,
            })
        })
        .collect();

    // Sort by most-recently-modified first
    metas.sort_by(|a, b| b.modified_secs.cmp(&a.modified_secs));

    metas.into_iter().skip(offset).take(limit).collect()
}

/// Return true if the filename has a known audio extension.
fn is_audio_extension(filename: &str) -> bool {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    matches!(
        ext.as_deref(),
        Some("wav" | "mp3" | "flac" | "ogg" | "oga" | "m4a" | "aac")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_valid() {
        assert!(is_safe_filename("2026-03-13-birdnet-07:12:34.wav"));
        assert!(is_safe_filename("robin_detection.wav"));
        assert!(is_safe_filename("clip-001.mp3"));
    }

    #[test]
    fn safe_filename_rejects_traversal() {
        assert!(!is_safe_filename("../etc/passwd"));
        assert!(!is_safe_filename("../../secrets"));
        assert!(!is_safe_filename("foo/bar.wav"));
        assert!(!is_safe_filename("foo\\bar.wav"));
    }

    #[test]
    fn safe_filename_rejects_empty_and_long() {
        assert!(!is_safe_filename(""));
        assert!(!is_safe_filename(&"a".repeat(256)));
    }

    #[test]
    fn content_type_wav() {
        assert_eq!(content_type_for("recording.WAV"), "audio/wav");
        assert_eq!(content_type_for("clip.wav"), "audio/wav");
    }

    #[test]
    fn content_type_mp3() {
        assert_eq!(content_type_for("clip.mp3"), "audio/mpeg");
    }

    #[test]
    fn content_type_flac() {
        assert_eq!(content_type_for("clip.flac"), "audio/flac");
    }

    #[test]
    fn content_type_unknown() {
        assert_eq!(content_type_for("clip.xyz"), "application/octet-stream");
    }

    #[test]
    fn audio_extension_check() {
        assert!(is_audio_extension("clip.wav"));
        assert!(is_audio_extension("clip.MP3"));
        assert!(is_audio_extension("clip.flac"));
        assert!(!is_audio_extension("clip.txt"));
        assert!(!is_audio_extension("image.jpg"));
    }

    #[test]
    fn collect_recordings_nonexistent_dir() {
        let dir = PathBuf::from("/nonexistent/path");
        let result = collect_recordings(&dir, None, 50, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn collect_recordings_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = collect_recordings(&tmp.path().to_path_buf(), None, 50, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn collect_recordings_with_wav_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("robin.wav"), b"RIFF....").unwrap();
        std::fs::write(tmp.path().join("wren.wav"), b"RIFF....").unwrap();
        std::fs::write(tmp.path().join("ignore.txt"), b"text").unwrap();

        let result = collect_recordings(&tmp.path().to_path_buf(), None, 50, 0);
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|r| r.filename.to_ascii_lowercase().ends_with(".wav"))
        );
    }

    #[test]
    fn collect_recordings_species_filter() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("2026-robin-07_00.wav"), b"RIFF").unwrap();
        std::fs::write(tmp.path().join("2026-wren-07_05.wav"), b"RIFF").unwrap();

        let result = collect_recordings(&tmp.path().to_path_buf(), Some("robin"), 50, 0);
        assert_eq!(result.len(), 1);
        assert!(result[0].filename.contains("robin"));
    }

    #[test]
    fn collect_recordings_pagination() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..10_u8 {
            std::fs::write(tmp.path().join(format!("clip-{i:02}.wav")), b"RIFF").unwrap();
        }
        let page1 = collect_recordings(&tmp.path().to_path_buf(), None, 3, 0);
        let page2 = collect_recordings(&tmp.path().to_path_buf(), None, 3, 3);
        assert_eq!(page1.len(), 3);
        assert_eq!(page2.len(), 3);
        // Pages should not overlap
        let names1: std::collections::HashSet<_> = page1.iter().map(|r| &r.filename).collect();
        let names2: std::collections::HashSet<_> = page2.iter().map(|r| &r.filename).collect();
        assert!(names1.is_disjoint(&names2));
    }
}
