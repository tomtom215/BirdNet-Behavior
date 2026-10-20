//! The badge in every page's header and the banner on the Health page say the
//! same thing about the same station.
//!
//! The badge (`/pages/health-badge`, polled every 30 s on every route) graded
//! three things: the daily integrity verdict, whether *any* source was up, and
//! one disk percentage. The Health page banner graded those and also scratch
//! space, queued uploads, a source that keeps dropping out, a species filter
//! that admits nothing and a classifier that fails every window — and named
//! each source that had stopped. On a station with one microphone up and two
//! cameras stalled the header said **Healthy** in green while the page it
//! links to said **Needs attention: Front-yard RTSP and Pond RTSP stopped
//! sending audio**. For the reader the badge *is* the answer to "is my
//! station working?", so a header that contradicts the page is a trust
//! defect, not a styling one.
//!
//! Both now read one issue list (`pages::station_issues`). Each test builds a
//! station with exactly one thing wrong, asks both surfaces, and requires them
//! to agree — in both directions, so a badge that always cried wolf would fail
//! the healthy station.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::audio::capture::{
    CaptureStatus, SourceState, SourceStatus, UPTIME_SEGMENTS, UptimeSegment, new_capture_status,
    publish_capture_status,
};
use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn status(label: &str, state: SourceState, flapping: bool) -> SourceStatus {
    SourceStatus {
        label: label.to_owned(),
        state,
        uptime_secs: None,
        last_audio_age_secs: Some(2),
        restart_attempts: 0,
        restarts_last_hour: if flapping { 5 } else { 0 },
        flapping,
        next_retry_in_secs: None,
        uptime_24h: vec![UptimeSegment::Up; UPTIME_SEGMENTS],
    }
}

/// A station with a microphone and two cameras, all configured, with the given
/// supervisor states published. The per-source gauge the badge has always read
/// follows the state, as the supervisor publishes both from one place.
fn station(sources: &[(&str, SourceState, bool)]) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for (id, _, _) in sources {
        let kind = if *id == "local" {
            SourceKind::UsbAlsa
        } else {
            SourceKind::Rtsp
        };
        conn.insert(&NewAudioSource::defaults(*id, kind, format!("dev-{id}")))
            .expect("insert source");
    }
    // The daily integrity check has run and passed, so the badge has a verdict.
    birdnet_db::sqlite::record_run_result(
        &conn,
        birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
        1_700_000_000,
        Some(true),
    )
    .expect("record");
    let handle = new_capture_status();
    publish_capture_status(
        &handle,
        CaptureStatus {
            sources: sources
                .iter()
                .map(|(id, s, flap)| status(id, *s, *flap))
                .collect(),
            published_unix: 1_700_000_000,
        },
    );
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_capture_status(handle);
    for (id, s, _) in sources {
        state
            .metrics()
            .set_source_up(id, matches!(s, SourceState::Connected));
    }
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
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

/// `(page says healthy, badge says healthy)`.
async fn verdicts(state: &AppState) -> (bool, bool, String, String) {
    let page = get(state, "/station").await;
    let badge = get(state, "/pages/health-badge").await;
    let page_healthy = page.contains("All systems healthy");
    assert!(
        page_healthy || page.contains("Needs attention"),
        "the Health page must carry a verdict"
    );
    let badge_healthy = badge.contains(r#"data-health="ok""#);
    (page_healthy, badge_healthy, page, badge)
}

#[tokio::test]
async fn a_station_with_one_source_up_and_two_stalled_is_not_healthy_in_the_header() {
    let state = station(&[
        ("local", SourceState::Connected, false),
        ("RTSP_1", SourceState::BackingOff, false),
        ("RTSP_2", SourceState::Stalled, false),
    ]);
    let (page_ok, badge_ok, _page, badge) = verdicts(&state).await;
    assert!(
        !page_ok,
        "the page must say two sources stopped (precondition)"
    );
    assert!(
        !badge_ok,
        "the header says Healthy while the page it links to says Needs attention: {badge}"
    );
    assert!(
        badge.contains("RTSP_1") && badge.contains("RTSP_2"),
        "the header must name what the page names, not only change colour: {badge}"
    );
}

#[tokio::test]
async fn a_source_that_keeps_dropping_out_is_not_healthy_in_the_header() {
    let state = station(&[
        ("local", SourceState::Connected, false),
        ("RTSP_1", SourceState::Connected, true),
    ]);
    let (page_ok, badge_ok, _, badge) = verdicts(&state).await;
    assert!(!page_ok, "a flapping source is an issue on the page");
    assert!(!badge_ok, "…and so in the header: {badge}");
}

#[tokio::test]
async fn a_species_filter_that_admits_nothing_is_not_healthy_in_the_header() {
    let state = station(&[("local", SourceState::Connected, false)]);
    state.metrics().set_occurrence_filter(true, Some(0));
    let (page_ok, badge_ok, _, badge) = verdicts(&state).await;
    assert!(!page_ok);
    assert!(!badge_ok, "{badge}");
}

#[tokio::test]
async fn a_failing_classifier_is_not_healthy_in_the_header() {
    use birdnet_core::inference::health::{ErrorClass, FAILING_AFTER, ModelHealth};
    let state = station(&[("local", SourceState::Connected, false)]);
    let mut h = ModelHealth::default();
    for _ in 0..FAILING_AFTER {
        h.failed(ErrorClass::Inference, 1_700_000_000);
    }
    state
        .metrics()
        .set_model_health(&[("birdnet-v2.4".to_owned(), h)]);
    let (page_ok, badge_ok, _, badge) = verdicts(&state).await;
    assert!(!page_ok);
    assert!(!badge_ok, "{badge}");
}

#[tokio::test]
async fn uploads_waiting_for_the_network_are_not_healthy_in_the_header() {
    let state = station(&[("local", SourceState::Connected, false)]);
    state.with_db(|conn| {
        birdnet_db::outbound_queue::enqueue(
            conn,
            birdnet_integrations::birdweather::QUEUE_KIND,
            "{}",
            1_700_000_000,
        )
        .expect("enqueue");
    });
    let (page_ok, badge_ok, _, badge) = verdicts(&state).await;
    assert!(!page_ok);
    assert!(!badge_ok, "{badge}");
}

/// The counterpart: a station with nothing wrong must read healthy on both, so
/// the two surfaces cannot "agree" by both crying wolf.
#[tokio::test]
async fn a_station_with_nothing_wrong_is_healthy_on_both() {
    // The page and the badge both read the real disk of the machine running
    // the test; on a machine that is itself nearly full, "healthy" would be
    // the wrong answer from both.
    for dir in [std::path::PathBuf::from("."), std::env::temp_dir()] {
        if birdnet_core::audio::capture::disk_usage(&dir).is_ok_and(|d| d.is_low()) {
            eprintln!("skipped: {} is nearly full on this machine", dir.display());
            return;
        }
    }
    let state = station(&[
        ("local", SourceState::Connected, false),
        ("RTSP_1", SourceState::Connected, false),
    ]);
    let (page_ok, badge_ok, page, badge) = verdicts(&state).await;
    assert!(page_ok, "{page}");
    assert!(badge_ok, "{badge}");
}
