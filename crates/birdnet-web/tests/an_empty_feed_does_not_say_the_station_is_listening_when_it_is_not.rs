//! The empty live feed says only what the station's capture status supports.
//!
//! # What was wrong
//!
//! `quiet_yard()` said "A quiet yard. The station is listening" for every empty
//! feed. On a station whose microphone was not recording, the same dashboard
//! showed a hero pill "not recording" and a checklist row "Not listening yet"
//! beside it: three sentences about one fact, two of which were true.
//!
//! # What this gate holds
//!
//! The feed (`/pages/detections`) and the hero pill (`/pages/today-pills`) are
//! asked about the same station in each of five capture states. The feed claims
//! to be listening **exactly when** the pill says the station is recording, and
//! each of the other states says what is actually the matter (no microphone,
//! not recording, no detector, not known yet).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with no detections, with one source (or none), the gauge for it as
/// given (`None`: never published) and the detector running or not.
fn station(source: bool, gauge: Option<bool>, detector: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    if source {
        conn.insert(&NewAudioSource::defaults(
            "local",
            SourceKind::UsbAlsa,
            "plughw:1,0".to_owned(),
        ))
        .expect("insert source");
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    if let Some(up) = gauge {
        state.metrics().set_source_up("local", up);
    }
    state
        .detection_status_flag()
        .store(detector, std::sync::atomic::Ordering::Relaxed);
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

#[tokio::test]
async fn the_feed_claims_to_listen_exactly_when_the_station_is_recording() {
    // (name, source, gauge, detector, the feed says it listens, the pill says recording)
    let cases = [
        (
            "recording, detector running",
            true,
            Some(true),
            true,
            true,
            true,
        ),
        ("no microphone", false, None, true, false, false),
        ("microphone not recording", true, None, false, false, false),
        ("gauge says down", true, Some(false), true, false, false),
    ];
    for (name, source, gauge, detector, listens, recording) in cases {
        let state = station(source, gauge, detector);
        let feed = get(&state, "/pages/detections").await;
        let pill = get(&state, "/pages/today-pills").await;
        assert_eq!(
            pill.contains("> recording</span>"),
            recording,
            "{name}: precondition, the hero pill: {pill}"
        );
        assert_eq!(
            feed.contains("The station is listening"),
            listens,
            "{name}: the feed and the pill disagree about whether the station is listening.\n\
             feed: {feed}\npill: {pill}"
        );
    }
}

#[tokio::test]
async fn each_state_says_what_is_the_matter() {
    let state = station(false, None, true);
    let feed = get(&state, "/pages/detections").await;
    assert!(
        feed.contains("No microphone is set up") && feed.contains("/station/capture"),
        "{feed}"
    );

    let state = station(true, None, false);
    let feed = get(&state, "/pages/detections").await;
    assert!(feed.contains("not recording right now"), "{feed}");

    // The microphone is up, the detector is not: it is recording, so the pill
    // is right to say so, and the feed must not claim anything is listening.
    let state = station(true, Some(true), false);
    let feed = get(&state, "/pages/detections").await;
    assert!(
        feed.contains("the detector is not running") && !feed.contains("is listening"),
        "{feed}"
    );

    // Sources configured, supervisor has not reported yet, detector running:
    // not an outage, and not yet a claim to be listening.
    let state = station(true, None, true);
    let feed = get(&state, "/pages/detections").await;
    assert!(
        feed.contains("starting up") && !feed.contains("is listening"),
        "{feed}"
    );
}
