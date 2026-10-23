//! A recording whose name the daemon cannot read is refused with a warning in
//! the log and left where it is, so the person who dropped it in sees nothing.
//! Station Health (the page and the header badge) now says so, with the folder,
//! the count and the name that works; the Capture tab says where the folder is
//! and what a file must be called.
//!
//! Driven through the real router against a real directory. The counterpart is
//! the same station with only good names in the folder: healthy, and silent.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::audio::capture::DiskUsage;
use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station(watch: Option<&std::path::Path>) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.insert(&NewAudioSource::defaults(
        "local",
        SourceKind::UsbAlsa,
        "plughw:1,0".to_owned(),
    ))
    .expect("source");
    birdnet_db::sqlite::record_run_result(
        &conn,
        birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
        1_700_000_000,
        Some(true),
    )
    .expect("record");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    state.metrics().set_source_up("local", true);
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    state.pin_disk_usage(DiskUsage {
        total_bytes: 1_000_000_000,
        used_bytes: 100_000_000,
        available_bytes: 900_000_000,
    });
    if let Some(dir) = watch {
        state.set_watch_dir(dir.to_path_buf());
    }
    state
}

async fn get(state: &AppState, uri: &str) -> String {
    let resp = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn touch(dir: &std::path::Path, name: &str) {
    std::fs::write(dir.join(name), b"RIFF").unwrap();
}

#[tokio::test]
async fn files_with_names_the_daemon_cannot_read_are_counted_on_the_page_and_the_badge() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "2026-08-12-birdnet-11:30:16.wav");
    touch(dir.path(), "dawn chorus.wav");
    touch(dir.path(), "recording-2.mp3");
    touch(dir.path(), "notes.txt");
    let state = station(Some(dir.path()));

    let page = get(&state, "/station").await;
    let shown = dir.path().display().to_string();
    assert!(
        page.contains(&format!("2 audio files in {shown} were not analysed"))
            && page.contains("YYYY-MM-DD-birdnet-HH:MM:SS.wav"),
        "the page must name the count, the folder and the name that works: {page}"
    );
    assert!(!page.contains("All systems healthy"), "{page}");

    let badge = get(&state, "/pages/health-badge").await;
    assert!(
        badge.contains("Files not analysed") && badge.contains(r#"data-health="warn""#),
        "{badge}"
    );
}

#[tokio::test]
async fn a_folder_of_good_names_is_healthy_and_silent() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "2026-08-12-birdnet-11:30:16.wav");
    touch(dir.path(), "2026-08-12-birdnet-RTSP_1-11:30:19.wav");
    touch(dir.path(), "notes.txt");
    let state = station(Some(dir.path()));
    let page = get(&state, "/station").await;
    assert!(page.contains("All systems healthy"), "{page}");
    assert!(!page.contains("not analysed"), "{page}");
    let badge = get(&state, "/pages/health-badge").await;
    assert!(badge.contains(r#"data-health="ok""#), "{badge}");
}

#[tokio::test]
async fn the_capture_tab_says_where_the_folder_is_and_what_a_file_is_called() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(Some(dir.path()));
    let page = get(&state, "/station/capture").await;
    let shown = dir.path().display().to_string();
    assert!(
        page.contains("watch-folder-note")
            && page.contains(&format!("Recordings are read from <code>{shown}</code>"))
            && page.contains("YYYY-MM-DD-birdnet-HH:MM:SS.wav"),
        "{page}"
    );
}
