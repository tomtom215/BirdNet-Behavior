//! Spectrogram generation and serving.
//!
//! Generates a PNG spectrogram from a WAV recording file on demand and
//! returns it as an `image/png` response.
//!
//! Rendered PNGs are cached in memory keyed by filename, modification time, the
//! overlay label and the look (colour map and dynamic range, see `look`), so the common case — the same recordings re-requested as
//! the list and detail pages are browsed — is served without re-decoding the
//! WAV or recomputing the mel transform. The cache is bounded by total bytes
//! and evicts oldest-first. Because the endpoint is public and the label is
//! caller-controlled (so the cache can be bypassed by varying the query
//! string), concurrent *rendering* is additionally capped by a semaphore so a
//! burst of distinct requests can't saturate a Pi's cores.
//!
//! Route:
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /api/v2/spectrogram/{filename} | Generate/serve spectrogram PNG |
//!
//! The spectrogram is rendered as a PNG from the mel spectrogram computed by
//! `birdnet-core`, in the colour map and dynamic range the station chose
//! (viridis at 80 dB until it does). The response is revalidated rather than
//! kept (`ETag`, `Cache-Control: private, no-cache`), so a change of look
//! reaches a browser that already holds the old image under the same address.

mod colormap;
mod font;
mod look;
mod png;
mod render;

pub(crate) use look::{DEFAULT_RANGE_DB, RANGES_DB, colormap_choices, is_colormap, is_range_db};

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use tokio::sync::Semaphore;

use crate::state::AppState;
use look::Look;
use render::{SpectrogramLabel, generate_spectrogram_png_with_label};

/// Mount the spectrogram generation and serving route.
pub fn router() -> Router<AppState> {
    Router::new().route("/spectrogram/{filename}", get(serve_spectrogram))
}

// ---------------------------------------------------------------------------
// Render bounding
// ---------------------------------------------------------------------------

/// Maximum spectrograms rendered concurrently.
///
/// A render decodes a WAV, runs an FFT-based mel transform and encodes a PNG —
/// all CPU-bound and dispatched to the blocking pool. The endpoint is public
/// and unauthenticated, so without a cap a burst of distinct (cache-missing)
/// requests would spawn one heavyweight job per connection and peg a Pi's
/// cores. Four matches the core count of the Pi 4/5 targets; cache hits never
/// take a permit, and waiters queue rather than fail so a page showing several
/// uncached spectrograms still renders them all (just serialised past the cap).
const MAX_CONCURRENT_RENDERS: usize = 4;

/// Fixed time-axis width, in pixels, of a Recordings-grid thumbnail (`?thumb=1`).
///
/// Roughly 3× the ~104 px the grid renders the tile at, so it stays crisp on
/// hi-dpi while a long clip ships ~a few KB instead of a multi-thousand-pixel
/// full-resolution image. Frequency rows (mel bands) are kept as-is.
const THUMB_WIDTH: u32 = 320;

/// Process-global permit pool bounding concurrent spectrogram rendering.
static RENDER_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_RENDERS)));

/// Total bytes of rendered PNGs retained in [`SPECTROGRAM_CACHE`].
///
/// Bounding by bytes (not entry count) keeps the ceiling fixed regardless of
/// recording length: a 3 s clip renders to a few hundred KB, a longer clip to
/// more, so an entry cap would let memory drift. 32 MiB holds dozens of typical
/// clips — ample for the browse-the-recordings workload — while staying
/// comfortable on a 2–4 GB Pi.
const CACHE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Process-global rendered-PNG cache.
static SPECTROGRAM_CACHE: LazyLock<Mutex<SpectrogramCache>> =
    LazyLock::new(|| Mutex::new(SpectrogramCache::with_budget(CACHE_BUDGET_BYTES)));

/// Cache key: recording filename + modification time + overlay label + look.
///
/// `mtime` invalidates the entry when a recording is overwritten so a stale PNG
/// is never served. The label (species / confidence / time) is part of the key
/// because it alters the rendered pixels — two requests for the same file with
/// different overlays are distinct images. `thumb` likewise: the small grid
/// preview and the full-size image for one file are different renders. So is
/// the `look`: a station that changes its colour map or dynamic range must get
/// the next image in the new one, not the cached image in the old one.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    filename: String,
    mtime: Option<Duration>,
    label: Option<(String, u32, String)>,
    thumb: bool,
    look: Look,
}

impl CacheKey {
    /// A quoted entity tag naming exactly the inputs of this render.
    ///
    /// Two requests with equal tags would be handed the same bytes, which is
    /// what lets a `If-None-Match` be answered `304` without rendering or even
    /// consulting the cache. The label is user text, so it enters as a hash.
    fn etag(&self) -> String {
        let mut label_hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes.iter().chain(&[0xff]) {
                label_hash ^= u64::from(*b);
                label_hash = label_hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        if let Some((species, confidence, time)) = &self.label {
            feed(species.as_bytes());
            feed(&confidence.to_le_bytes());
            feed(time.as_bytes());
        }
        feed(self.filename.as_bytes());
        let mtime = self.mtime.map_or(0, |d| d.as_nanos());
        format!(
            "\"{mtime:x}-{}-{}-{label_hash:016x}\"",
            self.look.tag(),
            u8::from(self.thumb)
        )
    }
}

/// Whether an `If-None-Match` header names `etag` (or is `*`).
fn etag_matches(header_value: &str, etag: &str) -> bool {
    header_value
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == etag)
}

/// Byte-budgeted, insertion-order (FIFO) cache of rendered spectrogram PNGs.
///
/// A serving cache for a long-running station: the hot set drifts toward recent
/// recordings, so oldest-first eviction keeps that working set resident while
/// the byte budget caps peak memory. Values are [`Bytes`] so a hit clones a
/// refcount, not the PNG, and hands the buffer to the response body zero-copy.
struct SpectrogramCache {
    entries: HashMap<CacheKey, Bytes>,
    order: VecDeque<CacheKey>,
    total_bytes: usize,
    budget: usize,
}

impl SpectrogramCache {
    fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            budget,
        }
    }

    fn get(&self, key: &CacheKey) -> Option<Bytes> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: CacheKey, png: Bytes) {
        let len = png.len();
        // A single render larger than the whole budget is never cached — it
        // would immediately evict everything (including itself) for no benefit.
        if len > self.budget {
            return;
        }
        // Replacing an existing key keeps its single `order` slot; only a
        // genuinely new key is queued, so `order` never holds duplicates.
        match self.entries.insert(key.clone(), png) {
            Some(old) => self.total_bytes -= old.len(),
            None => self.order.push_back(key),
        }
        self.total_bytes += len;
        // Evict oldest until back within budget.
        while self.total_bytes > self.budget {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&evicted) {
                self.total_bytes -= removed.len();
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Look up a rendered PNG, recovering from a poisoned lock.
fn cache_get(key: &CacheKey) -> Option<Bytes> {
    SPECTROGRAM_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
}

/// Store a rendered PNG, recovering from a poisoned lock.
fn cache_insert(key: CacheKey, png: Bytes) {
    SPECTROGRAM_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, png);
}

// ---------------------------------------------------------------------------
// GET /api/v2/spectrogram/{filename}?species=...&confidence=...&time=...
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct SpectrogramQuery {
    species: Option<String>,
    confidence: Option<u32>,
    time: Option<String>,
    /// When set (`?thumb=1`), render a small fixed-width preview for the
    /// Recordings grid instead of the full-resolution image. A thumbnail is
    /// always bare (no overlay label).
    thumb: Option<u32>,
}

/// The full-size, unlabelled spectrogram for `filename`, for the share link
/// (O-4): the same handler as `/spectrogram/{filename}` with no query.
pub(crate) async fn serve_shared(state: State<AppState>, filename: Path<String>) -> Response {
    let query = SpectrogramQuery {
        species: None,
        confidence: None,
        time: None,
        thumb: None,
    };
    serve_spectrogram(
        state,
        filename,
        axum::extract::Query(query),
        axum::http::HeaderMap::new(),
    )
    .await
}

async fn serve_spectrogram(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    axum::extract::Query(query): axum::extract::Query<SpectrogramQuery>,
    request_headers: axum::http::HeaderMap,
) -> Response {
    if !is_safe_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    let rec_dir = state.recording_dir();
    let file_path = rec_dir.join(&filename);

    // Confirm the resolved path stays within the recording directory.
    match tokio::fs::canonicalize(&file_path).await {
        Ok(canonical) => {
            let rec_canonical = tokio::fs::canonicalize(&rec_dir)
                .await
                .unwrap_or_else(|_| rec_dir.clone());
            if !canonical.starts_with(&rec_canonical) {
                return (StatusCode::FORBIDDEN, "path traversal denied").into_response();
            }
        }
        Err(_) => {
            return (StatusCode::NOT_FOUND, "recording not found").into_response();
        }
    }

    // A thumbnail (the Recordings grid preview) is always bare — no overlay,
    // small fixed width — so it ignores the label params entirely.
    let thumb = query.thumb.is_some_and(|v| v != 0);

    // `raw_spectrogram` suppresses the overlay everywhere, for operators who
    // embed these images — an OBS bird-feeder stream, a kiosk, a paper. It is
    // BirdNET-Pi's `RAW_SPECTROGRAM` with one honest difference: their setting
    // also removes *axes*, and we have never drawn any. The image is a bare mel
    // surface either way; the only thing to suppress is the text.
    let (raw, look) = state
        .run_blocking(|state| {
            state.with_read_db(|conn| {
                let raw = birdnet_db::settings::get_or(conn, "raw_spectrogram", "false")
                    .unwrap_or_else(|_| "false".to_owned())
                    == "true";
                (raw, Look::load(conn))
            })
        })
        .await;

    // Build the optional overlay label, plus a matching cache-key fragment.
    let label = if thumb || raw {
        None
    } else {
        query.species.map(|species| SpectrogramLabel {
            species,
            confidence_pct: query.confidence.unwrap_or(0),
            time: query.time.unwrap_or_default(),
        })
    };
    let label_key = label
        .as_ref()
        .map(|l| (l.species.clone(), l.confidence_pct, l.time.clone()));

    // mtime keys the cache so an overwritten recording re-renders rather than
    // serving a stale image; absent metadata simply keys without it.
    let mtime = tokio::fs::metadata(&file_path)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok());

    let key = CacheKey {
        filename: filename.clone(),
        mtime,
        label: label_key,
        thumb,
        look,
    };

    // Revalidation: the tag names every input of the render, so a browser that
    // holds this exact image is told so without a render or a cache lookup,
    // and one that holds an image drawn under another look is not.
    let etag = key.etag();
    if request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| etag_matches(v, &etag))
    {
        return not_modified(&etag);
    }

    // Fast path: serve a previously rendered PNG without a render permit or CPU.
    if let Some(bytes) = cache_get(&key) {
        return png_response(bytes, &etag);
    }

    // Slow path: bound how many renders run at once (see `MAX_CONCURRENT_RENDERS`).
    // Waiters queue here; the permit is held across the blocking render and
    // dropped when this function returns. Acquire can only fail if the semaphore
    // is closed, which never happens.
    let Ok(_render_permit) = RENDER_SLOTS.clone().acquire_owned().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "spectrogram renderer unavailable",
        )
            .into_response();
    };

    // Re-check under the permit: a concurrent request for the same key may have
    // populated the cache while we queued, so we don't render it twice.
    if let Some(bytes) = cache_get(&key) {
        return png_response(bytes, &etag);
    }

    let result = tokio::task::spawn_blocking(move || {
        generate_spectrogram_png_with_label(
            &file_path,
            label.as_ref(),
            thumb.then_some(THUMB_WIDTH),
            look,
        )
    })
    .await;

    match result {
        Ok(Ok(png_bytes)) => {
            let bytes = Bytes::from(png_bytes);
            cache_insert(key, bytes.clone());
            png_response(bytes, &etag)
        }
        Ok(Err(e)) => {
            // The detail (codec/decode/path specifics) is for the operator's
            // logs, not the unauthenticated caller — return a generic message.
            tracing::warn!(file = %filename, err = %e, "spectrogram generation failed");
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "could not render a spectrogram for this recording",
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, "spectrogram task panicked");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}

/// The headers every answer for a spectrogram carries.
///
/// `private`, like the recording it is drawn from: on a private station a
/// shared cache licensed by `public` could hand a signed-in reader's image to
/// the next person who asked for the same URL. `no-cache` with an `ETag`
/// rather than the `max-age=3600` it used to send: the address does not change
/// when the owner changes the colour map or the dynamic range, so an hour of
/// "no need to ask" was an hour of the old look. A revalidation that matches
/// costs a `304` and no render.
fn cache_headers(etag: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    // The tag is built from hex digits, a colour-map name and quotes.
    if let Ok(v) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, v);
    }
    headers
}

/// Build the standard `image/png` response for a rendered (or cached) PNG.
fn png_response(bytes: Bytes, etag: &str) -> Response {
    let mut headers = cache_headers(etag);
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    (StatusCode::OK, headers, Body::from(bytes)).into_response()
}

/// `304 Not Modified` for a browser that already holds this exact render.
fn not_modified(etag: &str) -> Response {
    (StatusCode::NOT_MODIFIED, cache_headers(etag)).into_response()
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

    fn key(name: &str) -> CacheKey {
        CacheKey {
            filename: name.to_string(),
            mtime: None,
            label: None,
            thumb: false,
            look: Look::default(),
        }
    }

    #[test]
    fn cache_hit_returns_inserted_bytes() {
        let mut c = SpectrogramCache::with_budget(1000);
        c.insert(key("a"), Bytes::from_static(b"png-a"));
        assert_eq!(c.get(&key("a")).unwrap(), Bytes::from_static(b"png-a"));
        assert!(c.get(&key("absent")).is_none());
    }

    #[test]
    fn cache_evicts_oldest_when_over_budget() {
        let mut c = SpectrogramCache::with_budget(1000);
        c.insert(key("a"), Bytes::from(vec![0u8; 400]));
        c.insert(key("b"), Bytes::from(vec![0u8; 400]));
        assert!(c.get(&key("a")).is_some());
        assert!(c.get(&key("b")).is_some());

        // 400 + 400 + 400 = 1200 > 1000 → oldest ("a") is evicted.
        c.insert(key("c"), Bytes::from(vec![0u8; 400]));
        assert!(c.get(&key("a")).is_none());
        assert!(c.get(&key("b")).is_some());
        assert!(c.get(&key("c")).is_some());
        assert!(c.total_bytes <= 1000);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn cache_replace_same_key_keeps_single_entry() {
        let mut c = SpectrogramCache::with_budget(1000);
        c.insert(key("a"), Bytes::from(vec![0u8; 100]));
        c.insert(key("a"), Bytes::from(vec![0u8; 200]));
        assert_eq!(c.len(), 1);
        assert_eq!(c.total_bytes, 200);
        assert_eq!(c.get(&key("a")).unwrap().len(), 200);
    }

    #[test]
    fn cache_skips_entry_larger_than_budget() {
        let mut c = SpectrogramCache::with_budget(1000);
        c.insert(key("big"), Bytes::from(vec![0u8; 2000]));
        assert!(c.get(&key("big")).is_none());
        assert_eq!(c.total_bytes, 0);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn cache_key_distinguishes_label_and_mtime() {
        let mut c = SpectrogramCache::with_budget(10_000);
        let base = key("x.wav");
        let labeled = CacheKey {
            label: Some(("Robin".to_string(), 90, "06:00".to_string())),
            ..key("x.wav")
        };
        let newer = CacheKey {
            mtime: Some(Duration::from_secs(5)),
            ..key("x.wav")
        };
        // The grid thumbnail and the full-size image for one file are distinct
        // renders, so they must not collide in the cache.
        let thumb = CacheKey {
            thumb: true,
            ..key("x.wav")
        };
        // So is the look: same file, same label, another colour map.
        let magma = CacheKey {
            look: Look {
                colormap: colormap::Colormap::Magma,
                ..Look::default()
            },
            ..key("x.wav")
        };
        let narrow = CacheKey {
            look: Look {
                range_db: 60,
                ..Look::default()
            },
            ..key("x.wav")
        };
        c.insert(magma.clone(), Bytes::from_static(b"magma"));
        c.insert(narrow.clone(), Bytes::from_static(b"narrow"));
        c.insert(base.clone(), Bytes::from_static(b"base"));
        c.insert(labeled.clone(), Bytes::from_static(b"labeled"));
        c.insert(newer.clone(), Bytes::from_static(b"newer"));
        c.insert(thumb.clone(), Bytes::from_static(b"thumb"));
        assert_eq!(c.get(&base).unwrap(), Bytes::from_static(b"base"));
        assert_eq!(c.get(&labeled).unwrap(), Bytes::from_static(b"labeled"));
        assert_eq!(c.get(&newer).unwrap(), Bytes::from_static(b"newer"));
        assert_eq!(c.get(&thumb).unwrap(), Bytes::from_static(b"thumb"));
        assert_eq!(c.get(&magma).unwrap(), Bytes::from_static(b"magma"));
        assert_eq!(c.get(&narrow).unwrap(), Bytes::from_static(b"narrow"));
        assert_eq!(c.len(), 6);
    }

    #[test]
    fn etags_differ_exactly_when_the_render_does() {
        let base = key("x.wav");
        let tags = [
            base.etag(),
            CacheKey {
                mtime: Some(Duration::from_secs(5)),
                ..key("x.wav")
            }
            .etag(),
            CacheKey {
                thumb: true,
                ..key("x.wav")
            }
            .etag(),
            CacheKey {
                label: Some(("Robin".into(), 90, "06:00".into())),
                ..key("x.wav")
            }
            .etag(),
            CacheKey {
                label: Some(("Robin".into(), 91, "06:00".into())),
                ..key("x.wav")
            }
            .etag(),
            CacheKey {
                look: Look {
                    range_db: 60,
                    ..Look::default()
                },
                ..key("x.wav")
            }
            .etag(),
            CacheKey {
                look: Look {
                    colormap: colormap::Colormap::Grey,
                    ..Look::default()
                },
                ..key("x.wav")
            }
            .etag(),
            key("y.wav").etag(),
        ];
        for (i, a) in tags.iter().enumerate() {
            assert!(a.starts_with('"') && a.ends_with('"'), "{a}");
            for b in &tags[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(
            base.etag(),
            key("x.wav").etag(),
            "a tag is a function of the inputs"
        );
    }

    #[test]
    fn if_none_match_is_read_like_a_list() {
        let tag = "\"abc\"";
        assert!(etag_matches("\"abc\"", tag));
        assert!(etag_matches("W/\"abc\"", tag));
        assert!(etag_matches("\"zzz\", \"abc\"", tag));
        assert!(etag_matches("*", tag));
        assert!(!etag_matches("\"abx\"", tag));
        assert!(!etag_matches("", tag));
    }
}
