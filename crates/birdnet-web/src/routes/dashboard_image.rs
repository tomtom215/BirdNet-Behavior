//! One picture of the owner's choosing on the Today page.
//!
//! A photograph of the site, or the last frame of a camera, with a title
//! (BirdNET-Pi's `CUSTOM_IMAGE` and `CUSTOM_IMAGE_TITLE`,
//! `scripts/overview.php:18-28`).
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
//! These are decided on the opened file, not on the earlier `stat` (the path can
//! change in between): it is opened without blocking, so a path swapped for a
//! FIFO is refused rather than waited on, and it is streamed, not read into
//! memory, because the route is public.
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

/// How much of the picture is read at a time when it is sent.
const STREAM_CHUNK: usize = 64 * 1024;

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

/// A picture opened and checked, ready to stream.
struct OpenPicture {
    file: std::fs::File,
    /// The length `fstat` gave on the open handle (not the earlier `stat`'s).
    len: u64,
    kind: &'static str,
}

/// Open for reading without being able to be made to wait.
///
/// `stat` has already said "regular file", but the path can change between that
/// and this: swapped for a FIFO (by anyone who can write the directory), a plain
/// blocking `open` of it parks the blocking-pool thread until something opens the
/// other end, which may be never. `O_NONBLOCK` makes the open return at once; a
/// regular file is unaffected by it. A symlink is followed on purpose: pointing
/// the setting at `latest.jpg -> frame-0412.jpg` is how a camera script keeps
/// the picture current, and `stat` above follows it too.
#[cfg(unix)]
fn open_nonblocking(path: &str) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_nonblocking(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Open the file and check it is a picture. `None` when it is not.
///
/// Everything is decided on the open handle, so what was checked is what is
/// served: the type from `fstat` (a FIFO or device swapped in after `stat` is
/// refused here), the size from `fstat`, the picture type from the first bytes.
/// The body is then streamed ([`serve_image`]) rather than read into memory:
/// this route is public, and an uncached request used to hold up to 8 MiB.
fn open_picture(path: &str) -> Option<OpenPicture> {
    use std::io::{Read as _, Seek as _};
    let mut file = open_nonblocking(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BYTES {
        return None;
    }
    // `sniff` looks at twelve bytes at most.
    let mut head = [0_u8; 16];
    let mut have = 0;
    while have < head.len() {
        match file.read(&mut head[have..]) {
            Ok(0) => break,
            Ok(n) => have += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    let kind = sniff(&head[..have])?;
    file.seek(std::io::SeekFrom::Start(0)).ok()?;
    Some(OpenPicture {
        file,
        len: meta.len(),
        kind,
    })
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
    let Ok(Stat::Ready { etag }) = tokio::task::spawn_blocking(move || stat(&for_stat)).await
    else {
        return (StatusCode::NOT_FOUND, "no picture").into_response();
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
    let Ok(Some(picture)) = tokio::task::spawn_blocking(move || open_picture(&path)).await else {
        return (StatusCode::NOT_FOUND, "no picture").into_response();
    };
    let mut headers = headers_with(&etag);
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(picture.kind));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(picture.len));
    // Streamed, and bounded again by the length `fstat` gave: a file that grows
    // while it is sent is cut off at that length rather than read past it.
    let reader =
        tokio::io::AsyncReadExt::take(tokio::fs::File::from_std(picture.file), picture.len);
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::with_capacity(
        reader,
        STREAM_CHUNK,
    ));
    (StatusCode::OK, headers, body).into_response()
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
    /// A tiny PNG-shaped file: the signature is all `sniff` reads.
    fn write_picture(path: &std::path::Path, len: usize) {
        let mut bytes = vec![0_u8; len];
        bytes[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        std::fs::write(path, bytes).unwrap();
    }

    /// `stat` said "regular file", and then the path became a FIFO. A blocking
    /// `open` of a FIFO nobody writes to never returns, so one request would park
    /// a blocking-pool thread for good. The open must come back — with `None`.
    ///
    /// Waited for on an OS thread: with the thread stuck, nothing here drives a
    /// timer, so a runtime timeout would hang instead of failing. If the open
    /// does block, the FIFO is opened for writing to release it before the
    /// assertion fires, so the test process can end.
    #[cfg(unix)]
    #[test]
    fn a_path_swapped_for_a_fifo_is_refused_instead_of_waited_on() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("snap.png");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(
            made.is_ok_and(|s| s.success()),
            "mkfifo is needed to make the fixture"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let path = fifo.to_str().unwrap().to_owned();
        std::thread::spawn(move || {
            let _ = tx.send(open_picture(&path).is_none());
        });
        let answered = rx.recv_timeout(std::time::Duration::from_secs(3));
        if answered.is_err() {
            // Release the stuck reader before failing.
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
        }
        assert_eq!(
            answered,
            Ok(true),
            "opening a FIFO blocked (or was served as a picture)"
        );
    }

    /// The counterparts: a regular file and a symlink to one (how a camera script
    /// keeps `latest.jpg` current) are opened and sniffed.
    #[cfg(unix)]
    #[test]
    fn a_regular_file_and_a_symlink_to_one_are_still_opened() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("frame-1.png");
        write_picture(&real, 100);
        let link = dir.path().join("latest.png");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        for p in [&real, &link] {
            let opened = open_picture(p.to_str().unwrap()).expect("a picture");
            assert_eq!((opened.len, opened.kind), (100, "image/png"), "{p:?}");
        }
        // Not a picture by its first bytes, or empty: refused as before.
        let text = dir.path().join("x.png");
        std::fs::write(&text, b"root:x:0:0:root:/root:/bin/bash").unwrap();
        assert!(open_picture(text.to_str().unwrap()).is_none());
        std::fs::write(&text, b"").unwrap();
        assert!(open_picture(text.to_str().unwrap()).is_none());
    }

    /// An uncached request used to read the whole file (up to 8 MiB) into memory
    /// before answering. It is sent in chunks now, with the length declared.
    #[tokio::test]
    async fn the_picture_is_streamed_not_held_in_memory() {
        use tower::ServiceExt as _;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("snap.png");
        let size = 3 * 1024 * 1024;
        write_picture(&file, size);
        let state = crate::state::AppState::new(dir.path().join("birds.db")).unwrap();
        state
            .with_db(|c| {
                birdnet_db::settings::set(
                    c,
                    PATH_KEY,
                    file.to_str().unwrap(),
                    birdnet_db::settings::SettingsCategory::System,
                )
            })
            .unwrap();
        let res = router()
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v2/dashboard-image")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(res.headers()[header::CONTENT_LENGTH], size.to_string());
        let mut stream = res.into_body().into_data_stream();
        let (mut total, mut biggest, mut chunks) = (0_usize, 0_usize, 0_usize);
        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            let chunk = chunk.unwrap();
            total += chunk.len();
            biggest = biggest.max(chunk.len());
            chunks += 1;
        }
        assert_eq!(total, size, "every byte arrived");
        assert!(
            chunks > 1 && biggest <= STREAM_CHUNK,
            "one chunk of {biggest} bytes in {chunks}: the file was held whole"
        );
    }
}
