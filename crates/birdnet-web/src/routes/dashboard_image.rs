//! One picture of the owner's choosing on the Today page: a photograph of the
//! site, or the last frame of a camera, with a title (BirdNET-Pi's
//! `CUSTOM_IMAGE` and `CUSTOM_IMAGE_TITLE`, `scripts/overview.php:18-28`).
//!
//! | Method | Path | Answer |
//! |--------|------|--------|
//! | GET | `/api/v2/dashboard-image` | the configured file, as the image it is |
//! | GET | `/pages/today-picture` | the block's HTML, polled by the page |
//!
//! # Settings
//!
//! `dashboard_image` is the absolute path of an image file; `dashboard_image_title`
//! is the heading shown over it. Both empty (the default) means no block. A
//! webcam snapshot is the same thing: something on the station (a cron job
//! running `ffmpeg -i rtsp://… -frames:v 1 snap.jpg`, a motion detector's
//! output folder) keeps the file current, and the page re-reads it every ten
//! seconds while the tab is visible. BirdNET-Pi re-reads it every second and
//! inlines it as base64; one conditional request in ten seconds is enough for
//! a snapshot and a great deal cheaper on a Pi.
//!
//! # What is served, and what is not
//!
//! The path is the administrator's, set on the settings page or through the
//! authenticated settings API, but the route is public like the rest of the
//! dashboard, so it is held to what a picture is:
//!
//! * a regular file (a directory, a device node or a FIFO is refused, so a path
//!   pointing at `/dev/zero` cannot be read forever);
//! * at most [`MAX_BYTES`];
//! * a PNG, JPEG, GIF or WebP **by its first bytes**, whatever the name says
//!   (so `/etc/passwd` renamed `x.jpg`, or the path typed with a slip, is
//!   refused). SVG is not on the list: it can carry script.
//!
//! The content type sent is the sniffed one. The answer carries an `ETag` of
//! the file's size and modification time, so the page's poll costs a `304`
//! when the camera has not written a new frame.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Router, routing::get};

use crate::routes::pages::escape_html;
use crate::state::AppState;

/// Settings key: the absolute path of the picture.
pub const PATH_KEY: &str = "dashboard_image";
/// Settings key: the title shown with it.
pub const TITLE_KEY: &str = "dashboard_image_title";

/// The largest file served. A camera frame is a few hundred kilobytes; eight
/// mebibytes holds a 24-megapixel JPEG and no more.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The longest title kept.
pub const MAX_TITLE_CHARS: usize = 120;

/// Mount the picture routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/dashboard-image", get(serve_image))
        .route("/pages/today-picture", get(picture_block))
}

/// The two stored values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picture {
    /// Absolute path of the image; empty when none is set.
    pub path: String,
    /// The heading; may be empty.
    pub title: String,
}

impl Picture {
    /// Read the setting rows. A failed read is "no picture": the block is
    /// decoration, and the page it sits on is not made an error page by it.
    pub fn load(conn: &rusqlite::Connection) -> Self {
        let get = |key: &str| match birdnet_db::settings::get(conn, key) {
            Ok(v) => v.trim().to_owned(),
            Err(birdnet_db::settings::SettingsError::NotFound(_)) => String::new(),
            Err(e) => {
                tracing::warn!(key, error = %e, "today picture: setting unreadable");
                String::new()
            }
        };
        Self {
            path: get(PATH_KEY),
            title: get(TITLE_KEY),
        }
    }
}

/// Why a stored path or title is refused, or `None` when it is fine. Shared by
/// the settings page and the settings API. An empty value is always fine: it
/// clears the setting.
#[must_use]
pub fn path_problem(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.chars().any(char::is_control) {
        return Some("Picture path cannot contain control characters.".to_owned());
    }
    if path.len() > 4096 {
        return Some("Picture path is too long.".to_owned());
    }
    if !std::path::Path::new(path).is_absolute() {
        return Some(format!(
            "Picture path must be absolute (start with /). You entered “{path}”."
        ));
    }
    None
}

/// See [`path_problem`].
#[must_use]
pub fn title_problem(title: &str) -> Option<String> {
    let title = title.trim();
    if title.chars().any(char::is_control) {
        return Some("Picture title cannot contain control characters.".to_owned());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Some(format!(
            "Picture title is longer than {MAX_TITLE_CHARS} characters."
        ));
    }
    None
}

/// The image type a file's first bytes name, or `None`.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// What the file looks like from its metadata alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stat {
    /// A regular file of an allowed size; the tag names this version of it.
    Ready {
        /// The quoted `ETag`.
        etag: String,
    },
    /// Not usable, and why (for the log and the placeholder, not the reader).
    Unusable(&'static str),
}

/// Look at the file without reading it.
#[must_use]
pub fn stat(path: &str) -> Stat {
    if path.is_empty() {
        return Stat::Unusable("no picture configured");
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return Stat::Unusable("the file cannot be read");
    };
    if !meta.is_file() {
        return Stat::Unusable("not a regular file");
    }
    if meta.len() == 0 || meta.len() > MAX_BYTES {
        return Stat::Unusable("empty or larger than the limit");
    }
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Stat::Ready {
        etag: format!("\"{nanos:x}-{:x}\"", meta.len()),
    }
}

/// Read the file and check it is a picture. `None` when it is not.
fn read_picture(path: &str) -> Option<(Vec<u8>, &'static str)> {
    use std::io::Read as _;
    // The size was checked from metadata; the read is bounded again so a file
    // that grows between the two cannot be read past the limit.
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let kind = sniff(&bytes)?;
    Some((bytes, kind))
}

fn headers_with(etag: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    // `private`, like the recordings: on a private station a shared cache must
    // not hand a signed-in reader's picture to the next person. `no-cache`
    // with an ETag, not a max-age: the point of the picture is that it changes.
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if let Ok(v) = HeaderValue::from_str(etag) {
        h.insert(header::ETAG, v);
    }
    h
}

async fn serve_image(State(state): State<AppState>, request: HeaderMap) -> Response {
    let cfg = state
        .run_blocking(|state| state.with_read_db(Picture::load))
        .await;
    let path = cfg.path;
    let for_stat = path.clone();
    let etag = match tokio::task::spawn_blocking(move || stat(&for_stat)).await {
        Ok(Stat::Ready { etag }) => etag,
        _ => return (StatusCode::NOT_FOUND, "no picture").into_response(),
    };
    // The tag names the file's size and modification time, so a poll that
    // matches is answered without reading the file.
    if request
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        })
    {
        return (StatusCode::NOT_MODIFIED, headers_with(&etag)).into_response();
    }
    let Ok(Some((bytes, kind))) = tokio::task::spawn_blocking(move || read_picture(&path)).await
    else {
        return (StatusCode::NOT_FOUND, "no picture").into_response();
    };
    let mut headers = headers_with(&etag);
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    (StatusCode::OK, headers, bytes).into_response()
}

/// The block's HTML for a configured picture, or the empty string when none is
/// configured. `etag` is the file's current tag when it is readable.
#[must_use]
pub fn block_html(cfg: &Picture, state: &Stat) -> String {
    if cfg.path.is_empty() {
        return String::new();
    }
    let title = escape_html(&cfg.title);
    let heading = if title.is_empty() {
        String::new()
    } else {
        format!(r#"<h2 class="sh-h">{title}</h2>"#)
    };
    let alt = if title.is_empty() {
        "A picture from the station".to_owned()
    } else {
        title
    };
    let body = match state {
        Stat::Ready { etag } => {
            // The tag is the cache key in the address: a new frame is a new URL.
            let version: String = etag.chars().filter(char::is_ascii_alphanumeric).collect();
            format!(
                r#"<img class="td-picture-img" src="/api/v2/dashboard-image?v={version}" alt="{alt}">"#
            )
        }
        Stat::Unusable(_) => r#"<p class="bnb-meta td-picture-wait" role="status">The picture file cannot be read right now. This block keeps trying.</p>"#.to_owned(),
    };
    format!(
        r#"<div class="bnb-card pad td-picture" id="today-picture" data-screen-label="Station picture" hx-get="/pages/today-picture" hx-trigger="every 10s" hx-swap="outerHTML">
  <div class="section-header"><div><div class="bnb-eyebrow">From the station</div>{heading}</div></div>
  {body}
</div>"#
    )
}

/// The block for the Today page, read now.
pub async fn block_for(state: &AppState) -> String {
    let cfg = state
        .run_blocking(|state| state.with_read_db(Picture::load))
        .await;
    if cfg.path.is_empty() {
        return String::new();
    }
    let path = cfg.path.clone();
    let st = tokio::task::spawn_blocking(move || stat(&path))
        .await
        .unwrap_or(Stat::Unusable("the check did not finish"));
    if let Stat::Unusable(why) = &st {
        tracing::debug!(why, "today picture: not showing the file");
    }
    block_html(&cfg, &st)
}

async fn picture_block(State(state): State<AppState>) -> Html<String> {
    Html(block_for(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_known_by_its_first_bytes() {
        assert_eq!(
            sniff(&[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0]),
            Some("image/png")
        );
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a...."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        // Not pictures, whatever the file is called.
        assert_eq!(sniff(b"root:x:0:0:root:/root:/bin/bash"), None);
        assert_eq!(
            sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script/></svg>"),
            None
        );
        assert_eq!(sniff(b"RIFF\x10\0\0\0WAVEfmt "), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn paths_and_titles_are_checked_where_they_are_set() {
        assert!(path_problem("").is_none());
        assert!(path_problem("/var/lib/camera/snap.jpg").is_none());
        assert!(path_problem("snap.jpg").is_some());
        assert!(path_problem("~/snap.jpg").is_some());
        assert!(path_problem("/tmp/a\nb.jpg").is_some());
        assert!(path_problem(&format!("/{}", "a".repeat(5000))).is_some());
        assert!(title_problem("").is_none());
        assert!(title_problem("The feeder, 08:00").is_none());
        assert!(title_problem("bad\u{7}").is_some());
        assert!(title_problem(&"x".repeat(MAX_TITLE_CHARS + 1)).is_some());
    }

    #[test]
    fn no_path_is_no_block_and_a_title_is_escaped() {
        assert_eq!(block_html(&Picture::default(), &Stat::Unusable("x")), "");
        let cfg = Picture {
            path: "/x.jpg".into(),
            title: "<b>\"Feeder\"</b>".into(),
        };
        let html = block_html(
            &cfg,
            &Stat::Ready {
                etag: "\"1a-2b\"".into(),
            },
        );
        assert!(html.contains("&lt;b&gt;"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(html.contains("dashboard-image?v=1a2b"), "{html}");
        assert!(html.contains(r#"hx-trigger="every 10s""#));
    }

    #[test]
    fn an_unreadable_file_says_so_instead_of_vanishing() {
        let cfg = Picture {
            path: "/x.jpg".into(),
            title: String::new(),
        };
        let html = block_html(&cfg, &Stat::Unusable("the file cannot be read"));
        assert!(html.contains("cannot be read right now"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        // Still polling, so the block comes back when the file does.
        assert!(html.contains("hx-get=\"/pages/today-picture\""));
    }

    #[test]
    fn the_stat_refuses_what_is_not_a_regular_file_of_a_sane_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("snap.png");
        assert!(
            matches!(stat(p.to_str().unwrap()), Stat::Unusable(_)),
            "missing"
        );
        assert!(
            matches!(stat(dir.path().to_str().unwrap()), Stat::Unusable(_)),
            "directory"
        );
        std::fs::write(&p, b"").unwrap();
        assert!(
            matches!(stat(p.to_str().unwrap()), Stat::Unusable(_)),
            "empty"
        );
        std::fs::write(&p, b"x").unwrap();
        let Stat::Ready { etag: a } = stat(p.to_str().unwrap()) else {
            panic!("a one-byte file is a file");
        };
        std::fs::write(&p, b"xy").unwrap();
        let Stat::Ready { etag: b } = stat(p.to_str().unwrap()) else {
            panic!("ready");
        };
        assert_ne!(a, b, "a new version of the file is a new tag");
        #[cfg(unix)]
        assert!(
            matches!(stat("/dev/zero"), Stat::Unusable(_)),
            "a device is not a file"
        );
    }
}
