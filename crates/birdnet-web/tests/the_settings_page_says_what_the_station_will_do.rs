//! Two settings that said more than they did.
//!
//! # The audio addresses
//!
//! *Microphone address*, *Network camera address* and *Several network
//! cameras* carried an "Applies after restart" badge. They only seed the
//! audio-source list while it is empty (`src/capture/sources.rs`); once it has
//! an active row, capture reads the list alone (`src/capture.rs`). A station
//! set up with the wizard has one the moment it has a microphone, so a camera
//! address typed there was saved, badged "applies after restart", and did
//! nothing for ever: the log said "capture sources resolved from `audio_sources`
//! table count=1" and the Audio page said "0 streams".
//!
//! The list is the one place that decides what the station records from, so the
//! settings page no longer offers a second, ignored way in: while the list has
//! an active source the card shows the sources, read-only, and links to where
//! they are edited. While it has none (a new station, or one whose sources are
//! all disabled, which capture falls back from) the fields are offered and the
//! badge is true.
//!
//! # The picture path
//!
//! The field says anything that is not a picture is refused. It is refused when
//! the Today page asks for it, with a 404; the save said "Settings saved" in
//! green for `/etc/passwd`. The path is still stored (a camera script may write
//! the file later), but the save now says the picture will not show, and why.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn get(state: &AppState, uri: &str) -> String {
    let res = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("response");
    assert_eq!(res.status(), StatusCode::OK, "GET {uri}");
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn save(state: &AppState, fields: &[(&str, &str)]) -> String {
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let res = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

fn add_source(state: &AppState, id: &str, kind: SourceKind, device: &str, label: Option<&str>) {
    let mut new = NewAudioSource::defaults(id, kind, device);
    new.label = label.map(str::to_owned);
    state
        .with_db(|conn| conn.insert(&new))
        .expect("insert a source");
}

const ADDRESS_FIELDS: [&str; 3] = ["alsa_device", "rtsp_url", "rtsp_urls"];

fn offers_the_addresses(html: &str) -> [bool; 3] {
    ADDRESS_FIELDS.map(|k| html.contains(&format!(r#"id="{k}""#)))
}

fn badges_the_addresses(html: &str) -> [bool; 3] {
    ADDRESS_FIELDS.map(|k| html.contains(&format!(r#"data-applies-for="{k}""#)))
}

/// Seen failing before the change: the three fields and their badges were on the
/// page of a station with a wizard-configured microphone.
#[tokio::test]
async fn once_the_source_list_has_a_source_the_ignored_addresses_are_not_offered() {
    let (_dir, state) = station();
    // A new station: nothing to read yet, so the addresses are real and badged.
    let page = get(&state, "/admin/settings").await;
    assert_eq!(offers_the_addresses(&page), [true; 3]);
    assert_eq!(badges_the_addresses(&page), [true; 3]);
    assert!(!page.contains(r#"class="set-sources""#));

    add_source(
        &state,
        "mic1",
        SourceKind::UsbAlsa,
        "plughw:1,0",
        Some("Garden mic"),
    );
    add_source(
        &state,
        "cam1",
        SourceKind::Rtsp,
        "rtsp://viewer:camPASSWORD@camera.local/stream",
        None,
    );
    for uri in ["/admin/settings", "/station/capture"] {
        let page = get(&state, uri).await;
        assert_eq!(offers_the_addresses(&page), [false; 3], "{uri}");
        assert_eq!(badges_the_addresses(&page), [false; 3], "{uri}");
        assert!(page.contains(r#"class="set-sources""#), "{uri}");
        assert!(page.contains(r#"href="/station/capture#audio""#), "{uri}");
        assert!(page.contains("Garden mic"), "{uri}");
        assert!(page.contains("1 microphone and 1 network stream"), "{uri}");
        assert!(
            page.contains("rtsp://viewer:***REDACTED***@camera.local/stream"),
            "{uri}: a stream is listed by its masked address"
        );
        assert!(!page.contains("camPASSWORD"), "{uri}");
        // Counterpart: the rest of the card is still there to edit.
        assert!(page.contains(r#"id="segment_duration""#), "{uri}");
        assert!(page.contains(r#"id="audio_format""#), "{uri}");
    }
}

/// Capture falls back to the addresses when every row is disabled
/// (`resolve_sources_from_db` returns `None` for an empty active set), so there
/// the fields do something and are offered.
#[tokio::test]
async fn a_list_of_only_disabled_sources_does_not_hide_the_addresses() {
    let (_dir, state) = station();
    add_source(&state, "mic1", SourceKind::UsbAlsa, "plughw:1,0", None);
    state
        .with_db(|conn| conn.execute("UPDATE audio_sources SET disabled_at = datetime('now')", []))
        .expect("disable");
    let page = get(&state, "/admin/settings").await;
    assert_eq!(offers_the_addresses(&page), [true; 3]);
    assert_eq!(badges_the_addresses(&page), [true; 3]);
}

/// A save from a page that no longer carries the fields must not clear them: the
/// addresses stored from before are left alone (they are ignored, not deleted).
#[tokio::test]
async fn saving_the_card_without_the_addresses_leaves_them_stored() {
    let (_dir, state) = station();
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "rtsp_url",
            "rtsp://cam.local/stream",
            birdnet_db::settings::SettingsCategory::Audio,
        )
        .unwrap();
    });
    add_source(&state, "mic1", SourceKind::UsbAlsa, "plughw:1,0", None);
    save(&state, &[("segment_duration", "18")]).await;
    let kept = state
        .with_db(|conn| birdnet_db::settings::get(conn, "rtsp_url"))
        .unwrap();
    assert_eq!(kept, "rtsp://cam.local/stream");
}

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

fn stored_picture(state: &AppState) -> String {
    state
        .with_db(|conn| birdnet_db::settings::get(conn, "dashboard_image"))
        .unwrap_or_default()
}

/// Seen failing before the change: every one of these answered with the green
/// "Settings saved" banner and a success toast.
#[tokio::test]
async fn a_picture_path_that_is_not_a_picture_is_saved_with_a_warning() {
    let (dir, state) = station();
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "root:x:0:0:root:/root:/bin/bash\n").unwrap();
    let missing = dir.path().join("not-yet.jpg");
    let directory = dir.path().to_path_buf();
    for (path, why) in [
        (text.clone(), "not a PNG, JPEG, GIF or WebP"),
        (missing.clone(), "cannot be read"),
        (directory, "not a regular file"),
    ] {
        let path = path.to_str().unwrap().to_owned();
        let reply = save(&state, &[("dashboard_image", &path)]).await;
        assert!(reply.contains("Settings saved"), "still saved: {reply}");
        assert!(
            reply.contains("will not show") && reply.contains(why),
            "{path}: {reply}"
        );
        assert!(
            reply.contains("alert-warn") && !reply.contains("alert-success"),
            "{path}: a save with a warning is not green: {reply}"
        );
        assert!(
            reply.contains(r#"data-kind="warn""#),
            "{path}: the toast is the warning one: {reply}"
        );
        assert_eq!(
            stored_picture(&state),
            path,
            "the path is stored all the same"
        );
    }
}

/// The counterpart: a real picture, and no picture at all, save quietly.
#[tokio::test]
async fn a_real_picture_and_an_empty_path_save_without_a_warning() {
    let (dir, state) = station();
    let png = dir.path().join("latest.png");
    std::fs::write(&png, PNG).unwrap();
    let reply = save(&state, &[("dashboard_image", png.to_str().unwrap())]).await;
    assert!(reply.contains("alert-success"), "{reply}");
    assert!(reply.contains(r#"data-kind="success""#), "{reply}");
    assert!(!reply.contains("will not show"), "{reply}");
    assert!(!reply.contains("alert-warn"), "{reply}");

    let reply = save(&state, &[("dashboard_image", "")]).await;
    assert!(reply.contains("alert-success"), "{reply}");
    assert!(!reply.contains("will not show"), "{reply}");
}
