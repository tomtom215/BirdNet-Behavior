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
use birdnet_core::inference::model_state::{
    ClassifierFault, ClassifierState, ClassifierStatus, DetectorState, FilterOff, FilterState,
    ModelsState,
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
    // The detector the configuration asks for, running with its one
    // classifier; each test breaks the part it is about.
    publish_models(&state, &running_models());
    state
}

/// A station's models doing what they were set up for: the detector running,
/// a primary and a second classifier judging, the occurrence filter on.
fn running_models() -> ModelsState {
    let classifier = |id: &str, primary: bool| ClassifierState {
        id: id.to_owned(),
        primary,
        model_path: std::path::PathBuf::from(format!("/m/{id}.onnx")),
        labels_path: Some(std::path::PathBuf::from(format!("/m/{id}_labels.txt"))),
        status: ClassifierStatus::Judging {
            sources: Vec::new(),
            unrouted_sources: primary,
        },
    };
    ModelsState {
        detector: DetectorState::Running,
        classifiers: vec![classifier("birdnet", true), classifier("perch", false)],
        occurrence_filter: FilterState::Active {
            candidates: Some(140),
        },
    }
}

/// Publish `models` the way the start-up does: the classifiers and the filter
/// as they were loaded, then the detector, which takes them with it when it
/// is not running.
fn publish_models(state: &AppState, models: &ModelsState) {
    let metrics = state.metrics();
    metrics.set_classifier_states(models.classifiers.clone());
    metrics.set_occurrence_filter(models.occurrence_filter.clone());
    metrics.set_detector_state(models.detector.clone());
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
    state.metrics().set_occurrence_filter(
        birdnet_core::inference::model_state::FilterState::Active {
            candidates: Some(0),
        },
    );
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

/// The model states that are not what the configuration asks for (MDL-03,
/// MDL-06): each breaks one part of [`running_models`], and both surfaces must
/// say so, in the words that say what to do.
fn broken_models() -> Vec<(&'static str, ModelsState, &'static str)> {
    let mut out = Vec::new();
    for (name, detector, words) in [
        (
            "the detector failed",
            DetectorState::Failed {
                why: "its model scores 6522 classes and its labels file names 12".to_owned(),
            },
            "its labels file names 12",
        ),
        (
            "the detector stopped",
            DetectorState::Stopped,
            "restart the station to resume detection",
        ),
        (
            "the detector is not configured",
            DetectorState::NotConfigured {
                why: "no classifier (MODEL_PATH) is configured".to_owned(),
            },
            "no classifier (MODEL_PATH) is configured",
        ),
    ] {
        let mut models = running_models();
        models.set_detector(detector);
        out.push((name, models, words));
    }
    let mut left_out = running_models();
    left_out.classifiers[1].status = ClassifierStatus::left_out(
        ClassifierFault::WillNotLoad,
        "its model file /m/perch.onnx will not load; install it again",
    );
    out.push((
        "a second classifier is left out",
        left_out,
        "`perch` is not running",
    ));
    let mut unrouted = running_models();
    unrouted.classifiers[1].status = ClassifierStatus::Unrouted;
    out.push((
        "a second classifier is unrouted",
        unrouted,
        "local:&lt;primary&gt;+perch",
    ));
    let mut no_position = running_models();
    no_position.occurrence_filter = FilterState::off(
        FilterOff::NoCoordinates,
        "the station has no LATITUDE and LONGITUDE; set its location",
    );
    out.push((
        "the filter has no coordinates",
        no_position,
        "set its location",
    ));
    let mut no_labels = running_models();
    no_labels.occurrence_filter = FilterState::off(
        FilterOff::LabelsMissing,
        "its labels file /m/geo_labels.txt is not there; install it again",
    );
    out.push((
        "the filter's labels are missing",
        no_labels,
        "geo_labels.txt is not there",
    ));
    out
}

#[tokio::test]
async fn models_not_doing_what_they_were_set_up_for_are_not_healthy_in_the_header() {
    for (name, models, words) in broken_models() {
        let state = station(&[("local", SourceState::Connected, false)]);
        publish_models(&state, &models);
        let (page_ok, badge_ok, page, badge) = verdicts(&state).await;
        let banner = region(&page, "st-status");
        assert!(!page_ok, "{name}: the Health page says healthy: {banner}");
        assert!(!badge_ok, "{name}: the header says healthy: {badge}");
        assert!(
            banner.contains(words),
            "{name}: the Health page's banner must say {words:?}: {banner}"
        );
        assert!(
            badge.contains(words),
            "{name}: the header must carry what the page says, {words:?}: {badge}"
        );
    }
}

/// The element of `page` whose class list starts with `class`, up to its
/// closing tag's line: the banner, not the template's comments.
fn region<'a>(page: &'a str, class: &str) -> &'a str {
    let start = page
        .find(&format!("class=\"{class}"))
        .unwrap_or_else(|| panic!("no {class} on the page"));
    let rest = &page[start..];
    &rest[..rest
        .find("</div></div></div>")
        .map_or(rest.len(), |e| e + 18)]
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
