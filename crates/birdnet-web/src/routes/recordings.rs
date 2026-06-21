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
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::state::AppState;

/// Mount recording routes under the given prefix.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/recordings", get(list_recordings))
        .route("/recordings/{filename}", get(serve_recording))
        .route(
            "/recordings/{filename}/spectrogram.png",
            get(serve_spectrogram),
        )
}

// ---------------------------------------------------------------------------
// GET /api/v2/recordings/{filename}/spectrogram.png
// ---------------------------------------------------------------------------

/// Generated spectrogram-thumbnail width, in pixels. ~3.3:1, scaled down by CSS
/// in the grid; rendered larger than displayed so it stays crisp on hi-dpi.
const THUMB_W: u32 = 320;
/// Generated spectrogram-thumbnail height, in pixels.
const THUMB_H: u32 = 96;

/// Serve a small spectrogram thumbnail PNG for a saved clip.
///
/// The preview is generated from the *same* audio file [`serve_recording`]
/// streams (so the picture matches what plays), then cached under
/// [`AppState::spectrogram_cache_dir`] — the first view renders it, every later
/// view (and every other clip on the page) is served straight from disk. There
/// is no schema change and historical clips get a preview too.
///
/// Returns `404` when the source audio is missing or undecodable — the grid
/// only links a thumbnail for clips whose audio is present, so this is the
/// honest "no preview" path rather than a faked tile.
async fn serve_spectrogram(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    if !is_safe_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    let rec_dir = state.recording_dir();
    let cache_dir = state.spectrogram_cache_dir();

    let result = tokio::task::spawn_blocking(move || {
        render_or_load_thumbnail(&rec_dir, &cache_dir, &filename)
    })
    .await;

    match result {
        Ok(Some(bytes)) => {
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            (StatusCode::OK, h, bytes).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "spectrogram unavailable").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "spectrogram task failed").into_response(),
    }
}

/// Load a cached thumbnail or render-and-cache one from the source audio.
///
/// Returns `None` when the audio file is absent or cannot be decoded/rendered.
/// Blocking (decode + FFT + PNG encode + disk I/O); call from a blocking task.
fn render_or_load_thumbnail(
    rec_dir: &std::path::Path,
    cache_dir: &std::path::Path,
    filename: &str,
) -> Option<Vec<u8>> {
    let cache_path = cache_dir.join(format!("{filename}.png"));
    if let Ok(bytes) = std::fs::read(&cache_path) {
        return Some(bytes);
    }

    let audio_path = rec_dir.join(filename);
    // Defence in depth: confirm the resolved audio path stays inside rec_dir
    // (is_safe_filename already forbids separators / `..`, so this is belt and
    // braces — and it cheaply doubles as the "audio missing" check).
    let canonical = audio_path.canonicalize().ok()?;
    let rec_canonical = rec_dir
        .canonicalize()
        .unwrap_or_else(|_| rec_dir.to_path_buf());
    if !canonical.starts_with(&rec_canonical) {
        return None;
    }

    match birdnet_core::audio::spectrogram::thumbnail::render_file_png(&canonical, THUMB_W, THUMB_H)
    {
        Ok(bytes) => {
            // Best-effort cache write — a read-only or full disk just means we
            // re-render next time, never a failed response.
            if std::fs::create_dir_all(cache_dir).is_ok() {
                let _ = std::fs::write(&cache_path, &bytes);
            }
            Some(bytes)
        }
        Err(e) => {
            tracing::debug!(file = filename, error = %e, "spectrogram thumbnail render failed");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// GET /api/v2/recordings/{filename}
// ---------------------------------------------------------------------------

/// Serve a single audio recording file, honouring HTTP `Range` requests.
///
/// Security: filename components are validated — only basename characters
/// allowed (no `..` or path separators) so callers cannot escape the
/// recording directory.
///
/// Range support matters: the `<audio>` player seeks by sending
/// `Range: bytes=…`, and Safari in particular refuses to play/seek a media
/// element unless the server answers with `206 Partial Content`. The previous
/// handler advertised `Accept-Ranges: bytes` but ignored the header and always
/// returned the whole file with `200`, so every seek re-downloaded from byte 0
/// (and Safari playback could break).
async fn serve_recording(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    headers: HeaderMap,
) -> Response {
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
    let Ok(meta) = file.metadata().await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "cannot stat recording").into_response();
    };
    let total = meta.len();
    let content_type = content_type_for(&filename);

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map_or(ParsedRange::None, |r| parse_range(r, total));

    match range {
        ParsedRange::Unsatisfiable => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            "requested range not satisfiable",
        )
            .into_response(),
        ParsedRange::Satisfiable { start, end } => {
            let len = end - start + 1;
            let mut file = file;
            if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "seek failed").into_response();
            }
            let body = Body::from_stream(ReaderStream::new(file.take(len)));
            let mut h = base_recording_headers(content_type);
            h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
            if let Ok(cr) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total}")) {
                h.insert(header::CONTENT_RANGE, cr);
            }
            (StatusCode::PARTIAL_CONTENT, h, body).into_response()
        }
        // No (or syntactically-ignored) Range header → full 200.
        ParsedRange::None => {
            let body = Body::from_stream(ReaderStream::new(file));
            let mut h = base_recording_headers(content_type);
            h.insert(header::CONTENT_LENGTH, HeaderValue::from(total));
            (StatusCode::OK, h, body).into_response()
        }
    }
}

/// Common response headers for both the full and partial recording responses.
fn base_recording_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    headers
}

/// Outcome of parsing a `Range` request header against a known content length.
#[derive(Debug, PartialEq, Eq)]
enum ParsedRange {
    /// No usable range — serve the whole file with `200`. Covers a missing
    /// header and (per RFC 7233) a syntactically-invalid one, which must be
    /// ignored rather than rejected.
    None,
    /// A single satisfiable byte range, inclusive on both ends.
    Satisfiable { start: u64, end: u64 },
    /// A syntactically-valid but unsatisfiable range → `416`.
    Unsatisfiable,
}

/// Parse a single-range `Range: bytes=…` header against `total` bytes.
///
/// Supports `bytes=N-M`, `bytes=N-` (to end) and `bytes=-N` (last N). Only the
/// first range of a multi-range request is honoured (audio players send one);
/// anything else is treated as "no range" so the whole file is served.
fn parse_range(value: &str, total: u64) -> ParsedRange {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return ParsedRange::None; // unsupported unit → ignore
    };
    // Honour only the first range if several are listed.
    let first = spec.split(',').next().unwrap_or("").trim();
    let Some((start_s, end_s)) = first.split_once('-') else {
        return ParsedRange::None;
    };

    if total == 0 {
        return ParsedRange::Unsatisfiable;
    }
    let last = total - 1;

    let (start, end) = if start_s.is_empty() {
        // Suffix range `bytes=-N`: the final N bytes.
        let Ok(n) = end_s.parse::<u64>() else {
            return ParsedRange::None;
        };
        if n == 0 {
            return ParsedRange::Unsatisfiable; // `bytes=-0` requests nothing
        }
        (total.saturating_sub(n), last)
    } else {
        let Ok(start) = start_s.parse::<u64>() else {
            return ParsedRange::None;
        };
        let end = if end_s.is_empty() {
            last
        } else {
            match end_s.parse::<u64>() {
                Ok(e) => e.min(last), // clamp an over-long end to EOF
                Err(_) => return ParsedRange::None,
            }
        };
        (start, end)
    };

    if start > last || start > end {
        return ParsedRange::Unsatisfiable;
    }
    ParsedRange::Satisfiable { start, end }
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
    metas.sort_by_key(|m| std::cmp::Reverse(m.modified_secs));

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
    fn parse_range_full_open_ended() {
        // `bytes=0-` → whole file as one satisfiable range.
        assert_eq!(
            parse_range("bytes=0-", 1000),
            ParsedRange::Satisfiable { start: 0, end: 999 }
        );
    }

    #[test]
    fn parse_range_explicit_window() {
        assert_eq!(
            parse_range("bytes=100-199", 1000),
            ParsedRange::Satisfiable {
                start: 100,
                end: 199
            }
        );
    }

    #[test]
    fn parse_range_clamps_overlong_end_to_eof() {
        // A player commonly asks for `bytes=500-` or an end past EOF; clamp it.
        assert_eq!(
            parse_range("bytes=500-99999", 1000),
            ParsedRange::Satisfiable {
                start: 500,
                end: 999
            }
        );
    }

    #[test]
    fn parse_range_suffix_last_n_bytes() {
        assert_eq!(
            parse_range("bytes=-200", 1000),
            ParsedRange::Satisfiable {
                start: 800,
                end: 999
            }
        );
        // Suffix larger than the file → whole file.
        assert_eq!(
            parse_range("bytes=-5000", 1000),
            ParsedRange::Satisfiable { start: 0, end: 999 }
        );
    }

    #[test]
    fn parse_range_unsatisfiable_start_past_eof() {
        assert_eq!(
            parse_range("bytes=1000-1100", 1000),
            ParsedRange::Unsatisfiable
        );
        assert_eq!(parse_range("bytes=2000-", 1000), ParsedRange::Unsatisfiable);
        // Empty file: any range is unsatisfiable.
        assert_eq!(parse_range("bytes=0-0", 0), ParsedRange::Unsatisfiable);
        // `bytes=-0` requests zero bytes → unsatisfiable.
        assert_eq!(parse_range("bytes=-0", 1000), ParsedRange::Unsatisfiable);
    }

    #[test]
    fn parse_range_invalid_is_ignored() {
        // Unsupported unit / garbage → treat as no range (serve full 200).
        assert_eq!(parse_range("items=0-10", 1000), ParsedRange::None);
        assert_eq!(parse_range("bytes=abc-def", 1000), ParsedRange::None);
        assert_eq!(parse_range("bytes=", 1000), ParsedRange::None);
        assert_eq!(parse_range("nonsense", 1000), ParsedRange::None);
    }

    #[test]
    fn parse_range_takes_first_of_multiple() {
        assert_eq!(
            parse_range("bytes=0-99,200-299", 1000),
            ParsedRange::Satisfiable { start: 0, end: 99 }
        );
    }

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

    /// Write a minimal valid 48 kHz / 16-bit mono PCM WAV (a 1 kHz sine) so
    /// symphonia can decode it — enough for the spectrogram renderer.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn write_test_wav(path: &std::path::Path, secs: f32) {
        let sr = 48_000u32;
        let n = (secs * sr as f32) as u32;
        let data_len = n * 2;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // mono
        buf.extend_from_slice(&sr.to_le_bytes());
        buf.extend_from_slice(&(sr * 2).to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..n {
            let t = i as f32 / sr as f32;
            let s = ((t * 2.0 * std::f32::consts::PI * 1000.0).sin() * 16_384.0) as i16;
            buf.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, buf).unwrap();
    }

    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

    #[test]
    fn thumbnail_renders_and_caches_from_real_audio() {
        let rec = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        write_test_wav(&rec.path().join("robin.wav"), 1.0);

        let bytes = render_or_load_thumbnail(rec.path(), cache.path(), "robin.wav")
            .expect("a thumbnail should render from decodable audio");
        assert!(bytes.starts_with(&PNG_MAGIC), "output is not a PNG");

        // The cache file was written, so a second call short-circuits to disk.
        let cached = std::fs::read(cache.path().join("robin.wav.png")).expect("cache written");
        assert_eq!(cached, bytes, "served bytes must match the cached file");
    }

    #[test]
    fn thumbnail_returns_none_when_audio_missing() {
        let rec = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        assert!(render_or_load_thumbnail(rec.path(), cache.path(), "ghost.wav").is_none());
    }

    #[test]
    fn thumbnail_cache_hit_short_circuits_without_audio() {
        let rec = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        // A cached PNG with no corresponding audio file: the cache read must
        // win before any decode is attempted.
        std::fs::write(cache.path().join("old.wav.png"), b"\x89PNG-cached").unwrap();
        let bytes = render_or_load_thumbnail(rec.path(), cache.path(), "old.wav")
            .expect("cache hit returns bytes even with the audio gone");
        assert_eq!(bytes, b"\x89PNG-cached");
    }
}
