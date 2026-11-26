//! `/api/v2/health` says what became of the station's models (MDL-03).
//!
//! On 2026-11-25 a station whose detector had not started answered the plain
//! endpoint 200 with nothing in its body about why, and a second classifier
//! that was left out appeared on no endpoint at all. The body now carries the
//! station's published model state and every condition the dashboard shows
//! for it, in the same words; the status code follows one rule:
//!
//! * plain: liveness, as before — 200 while the database serves;
//! * `?strict=1`: 503 while nothing is being identified (the detector failed,
//!   stopped or is not configured), and **200** for a second classifier left
//!   out or unrouted, or an occurrence filter that is off. The station is
//!   still identifying birds with its primary classifier; a pager woken for an
//!   optional file is a pager taught to be ignored. The reason is in the body
//!   either way, under `model_conditions`, and on every page.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::inference::model_state::{
    ClassifierFault, ClassifierState, ClassifierStatus, DetectorState, FilterOff, FilterState,
};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let classifier = |id: &str, primary: bool| ClassifierState {
        id: id.to_owned(),
        primary,
        model_path: std::path::PathBuf::from(format!("/m/{id}.onnx")),
        labels_path: None,
        status: ClassifierStatus::Judging {
            sources: Vec::new(),
            unrouted_sources: primary,
        },
    };
    let m = state.metrics();
    m.set_classifier_states(vec![
        classifier("birdnet", true),
        classifier("perch", false),
    ]);
    m.set_occurrence_filter(FilterState::Active {
        candidates: Some(140),
    });
    m.set_detector_state(DetectorState::Running);
    state
}

async fn health(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn keys(body: &serde_json::Value) -> Vec<String> {
    body["model_conditions"]
        .as_array()
        .unwrap_or_else(|| panic!("no model_conditions: {body}"))
        .iter()
        .map(|c| c["key"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The counterpart: a station doing what it was set up for is healthy on
/// both, with no condition and its model state in the body.
#[tokio::test]
async fn a_station_whose_models_run_as_configured_is_healthy() {
    let state = station();
    for uri in ["/api/v2/health", "/api/v2/health?strict=1"] {
        let (status, body) = health(&state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(keys(&body), Vec::<String>::new(), "{uri}");
        assert_eq!(body["models"]["detector"]["state"], "running", "{body}");
    }
}

/// A detector the configuration asks for that is not running is a strict
/// fault, with the reason in the body; the plain probe stays liveness. Read
/// from the published model state, so a daemon flag that says running cannot
/// hide a detector that says it failed.
#[tokio::test]
async fn a_detector_that_is_not_running_is_a_strict_fault_with_its_reason() {
    for detector in [
        DetectorState::Failed {
            why: "its labels file names 12".to_owned(),
        },
        DetectorState::Stopped,
        DetectorState::NotConfigured {
            why: "no classifier (MODEL_PATH)".to_owned(),
        },
    ] {
        let code = detector.code();
        let state = station();
        state.metrics().set_detector_state(detector);
        let (status, body) = health(&state, "/api/v2/health?strict=1").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{code}: {body}");
        assert_eq!(keys(&body), ["models:detector"], "{code}");
        assert_eq!(body["model_conditions"][0]["state"], code);
        assert_eq!(body["model_conditions"][0]["stops_detection"], true);
        assert!(
            body["model_conditions"][0]["sentence"]
                .as_str()
                .is_some_and(|s| s.starts_with("nothing is being identified")),
            "{body}"
        );
        let (status, body) = health(&state, "/api/v2/health").await;
        assert_eq!(status, StatusCode::OK, "plain stays liveness: {body}");
        assert_eq!(keys(&body), ["models:detector"]);
    }
}

/// One way to break a station's published model state.
type Break = fn(&AppState);

/// A second classifier left out or unrouted, and a filter that is off, are
/// not strict faults — the station is identifying birds — and are named in
/// the body with what to do.
#[tokio::test]
async fn an_optional_model_not_running_is_named_and_is_not_a_strict_fault() {
    let cases: [(&str, Break); 3] = [
        ("models:classifier:perch", |s| {
            let mut c = s.models_state().classifiers;
            c[1].status = ClassifierStatus::left_out(
                ClassifierFault::WillNotLoad,
                "its model file will not load; install it again",
            );
            s.metrics().set_classifier_states(c);
        }),
        ("models:classifier:perch", |s| {
            let mut c = s.models_state().classifiers;
            c[1].status = ClassifierStatus::Unrouted {
                primary: "birdnet".to_owned(),
            };
            s.metrics().set_classifier_states(c);
        }),
        ("models:occurrence-filter", |s| {
            s.metrics().set_occurrence_filter(FilterState::off(
                FilterOff::NoCoordinates,
                "the station has no position; set its location",
            ));
        }),
    ];
    for (key, break_it) in cases {
        let state = station();
        break_it(&state);
        let (status, body) = health(&state, "/api/v2/health?strict=1").await;
        assert_eq!(status, StatusCode::OK, "{key}: {body}");
        assert_eq!(keys(&body), [key], "{body}");
        assert_eq!(body["model_conditions"][0]["stops_detection"], false);
        assert!(
            body["model_conditions"][0]["sentence"]
                .as_str()
                .is_some_and(|s| s.contains("install it again")
                    || s.contains("MODEL_ROUTES")
                    || s.contains("set its location")),
            "{body}"
        );
    }
}
