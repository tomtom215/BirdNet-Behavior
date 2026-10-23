//! The Today page can show one picture of the owner's choosing with a title
//! (BirdNET-Pi's `CUSTOM_IMAGE` and `CUSTOM_IMAGE_TITLE`; P-13, N-6), and only
//! ever serves a picture.
//!
//! # What is guarded
//!
//! 1. With nothing set there is no block and no route answer: a station that
//!    never asked for it sees the page it always had.
//! 2. With a PNG set, the block names the title (escaped) and the image route
//!    returns the file's own bytes with the sniffed content type and an `ETag`;
//!    the same tag is answered `304`, and a rewritten file is a new tag and the
//!    new bytes (a camera snapshot is the point).
//! 3. **Only pictures.** A text file called `snap.jpg`, a directory, an empty
//!    file and one over the size limit are refused, so setting the path to
//!    `/etc/passwd` serves nothing. SVG is refused by the same rule.
//! 4. A configured file that cannot be read shows a placeholder that keeps
//!    polling, not a vanished block.
//! 5. The block is an ordinary Today block: it can be hidden in the layout.
//! 6. The settings page refuses a relative path and a control character.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The smallest PNG signature a sniff accepts, plus filler.
fn png(tag: u8) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend_from_slice(&[tag; 64]);
    v
}

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "onboarding_complete",
            "true",
            birdnet_db::settings::SettingsCategory::General,
        )
        .expect("onboarded");
    });
    (dir, state)
}

fn store(state: &AppState, key: &str, value: &str) {
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::System,
        )
        .expect("set");
    });
}

async fn send(
    state: &AppState,
    req: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 16 << 20)
        .await
        .expect("body")
        .to_vec();
    (status, headers, bytes)
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    send(
        state,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await
}

async fn page(state: &AppState, uri: &str) -> String {
    let (status, _, body) = get(state, uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}");
    String::from_utf8_lossy(&body).into_owned()
}

const IMAGE: &str = "/api/v2/dashboard-image";

#[tokio::test]
async fn with_nothing_set_there_is_no_block_and_no_picture() {
    let (_d, st) = station();
    let today = page(&st, "/").await;
    assert!(!today.contains("today-picture"), "no block without a path");
    assert_eq!(get(&st, IMAGE).await.0, StatusCode::NOT_FOUND);
    assert_eq!(page(&st, "/pages/today-picture").await, "");
}

#[tokio::test]
async fn a_picture_is_shown_with_its_title_and_follows_the_file() {
    let (d, st) = station();
    let file = d.path().join("snap.png");
    std::fs::write(&file, png(1)).unwrap();
    store(&st, "dashboard_image", file.to_str().unwrap());
    store(&st, "dashboard_image_title", "The <b>feeder</b>");

    let today = page(&st, "/").await;
    assert!(today.contains(r#"id="today-picture""#), "{today}");
    assert!(
        today.contains("The &lt;b&gt;feeder&lt;/b&gt;"),
        "title is escaped"
    );
    assert!(!today.contains("The <b>feeder</b>"));
    assert!(
        today.contains(r#"hx-trigger="every 10s""#),
        "the block re-checks itself"
    );
    assert!(
        !today.contains("{{picture_block}}"),
        "the placeholder is filled"
    );

    let (status, headers, bytes) = get(&st, IMAGE).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert!(
        headers[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("private")
    );
    assert_eq!(bytes, png(1));
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    let revalidate = Request::builder()
        .uri(IMAGE)
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&st, revalidate).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());

    // The camera writes a new frame (a different size, so the tag moves even on
    // a filesystem with coarse modification times).
    let mut next = png(2);
    next.extend_from_slice(&[9; 10]);
    std::fs::write(&file, &next).unwrap();
    let stale = Request::builder()
        .uri(IMAGE)
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let (status, headers, bytes) = send(&st, stale).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the old tag must not validate a new frame"
    );
    assert_eq!(bytes, next);
    assert_ne!(headers[header::ETAG].to_str().unwrap(), etag);

    // The poll's answer is the block, and it points at the new frame.
    let block = page(&st, "/pages/today-picture").await;
    assert!(block.contains("dashboard-image?v="), "{block}");
}

#[tokio::test]
async fn only_pictures_are_ever_served() {
    let (d, st) = station();
    let cases: Vec<(&str, std::path::PathBuf)> = {
        let text = d.path().join("snap.jpg");
        std::fs::write(&text, b"root:x:0:0:root:/root:/bin/bash\n").unwrap();
        let svg = d.path().join("logo.png");
        std::fs::write(
            &svg,
            b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>",
        )
        .unwrap();
        let empty = d.path().join("empty.png");
        std::fs::write(&empty, b"").unwrap();
        let big = d.path().join("big.png");
        let mut bytes = png(0);
        bytes.resize(8 * 1024 * 1024 + 1, 0);
        std::fs::write(&big, bytes).unwrap();
        vec![
            ("text called .jpg", text),
            ("svg called .png", svg),
            ("empty", empty),
            ("over the limit", big),
            ("a directory", d.path().to_path_buf()),
            ("missing", d.path().join("nope.png")),
        ]
    };
    for (what, path) in cases {
        store(&st, "dashboard_image", path.to_str().unwrap());
        let (status, _, body) = get(&st, IMAGE).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{what} must not be served");
        assert!(body.len() < 64, "{what}: the body is not the file");
    }
    #[cfg(unix)]
    {
        store(&st, "dashboard_image", "/dev/zero");
        assert_eq!(get(&st, IMAGE).await.0, StatusCode::NOT_FOUND, "a device");
        store(&st, "dashboard_image", "/etc/passwd");
        assert_eq!(
            get(&st, IMAGE).await.0,
            StatusCode::NOT_FOUND,
            "/etc/passwd"
        );
    }
}

#[tokio::test]
async fn a_file_that_cannot_be_read_shows_a_placeholder_that_keeps_polling() {
    let (d, st) = station();
    store(
        &st,
        "dashboard_image",
        d.path().join("not-yet.jpg").to_str().unwrap(),
    );
    let block = page(&st, "/pages/today-picture").await;
    assert!(block.contains("cannot be read right now"), "{block}");
    assert!(
        block.contains(r#"hx-get="/pages/today-picture""#),
        "{block}"
    );
    assert!(!block.contains("<img"), "no broken image");
}

#[tokio::test]
async fn the_block_can_be_hidden_like_any_other() {
    let (d, st) = station();
    let file = d.path().join("snap.png");
    std::fs::write(&file, png(1)).unwrap();
    store(&st, "dashboard_image", file.to_str().unwrap());
    st.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [],
        )
        .expect("seed");
    });
    assert!(page(&st, "/").await.contains("today-picture"));
    store(
        &st,
        "today_layout",
        "hero,nudge,daystrip,-picture,log,top-species,best,station",
    );
    assert!(!page(&st, "/").await.contains(r#"id="today-picture""#));
}

#[tokio::test]
async fn the_settings_page_refuses_a_path_that_is_not_one() {
    let (_d, st) = station();
    for (body, what) in [
        ("dashboard_image=snap.jpg", "a relative path"),
        (
            "dashboard_image=%2Ftmp%2Fa%0Ab.jpg",
            "a newline in the path",
        ),
        (
            "dashboard_image_title=bad%07",
            "a control character in the title",
        ),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri("/admin/settings")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let (_, _, out) = send(&st, req).await;
        let out = String::from_utf8_lossy(&out);
        assert!(out.contains("Nothing was saved"), "{what}: {out}");
    }
    let req = Request::builder()
        .method("POST")
        .uri("/admin/settings")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(
            "dashboard_image=%2Fvar%2Flib%2Fcamera%2Fsnap.jpg&dashboard_image_title=Feeder",
        ))
        .unwrap();
    let (_, _, out) = send(&st, req).await;
    assert!(!String::from_utf8_lossy(&out).contains("Nothing was saved"));
    let stored = st.with_db(|c| birdnet_db::settings::get(c, "dashboard_image").ok());
    assert_eq!(stored.as_deref(), Some("/var/lib/camera/snap.jpg"));
}
