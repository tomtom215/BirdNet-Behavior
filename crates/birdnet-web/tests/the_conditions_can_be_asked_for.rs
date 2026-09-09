//! The station-health conditions can be asked for, not only pushed (OP-4).
//!
//! `evaluate` was private and its sole caller the notifier, so an operator
//! who missed a push had no way to ask "what is wrong right now?". The
//! notifier publishes what it finds; this endpoint answers from it, and says
//! when it looked.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use birdnet_web::station_conditions::{Condition, ConditionsSnapshot};
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
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

#[tokio::test]
async fn the_last_evaluation_is_served_with_its_time() {
    let state = station();
    let (status, body) = get(&state, "/api/v2/health/conditions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["evaluated_at"],
        serde_json::Value::Null,
        "nothing evaluated yet"
    );
    assert_eq!(body["count"], 0);

    state.set_station_conditions(ConditionsSnapshot {
        evaluated_at: Some(1_788_973_200),
        conditions: vec![Condition {
            key: "data-volume".into(),
            title: "Data volume has gone away — detections are being lost".into(),
            body: "Check the card.".into(),
        }],
    });
    let (status, body) = get(&state, "/api/v2/health/conditions").await;
    assert_eq!(status, StatusCode::OK, "the answer, not a verdict: {body}");
    assert_eq!(body["evaluated_at"], 1_788_973_200);
    assert_eq!(body["count"], 1);
    assert_eq!(body["conditions"][0]["key"], "data-volume");
    assert!(
        body["conditions"][0]["title"]
            .as_str()
            .unwrap()
            .contains("gone away")
    );
}
