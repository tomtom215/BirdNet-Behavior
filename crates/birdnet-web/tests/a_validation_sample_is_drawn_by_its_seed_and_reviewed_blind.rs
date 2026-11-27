//! RES-07: a validation sample is drawn by a recorded seed, its items are shown
//! without a score when the protocol is blind, and its export carries each
//! item's inclusion probability and verdict. Through the real router.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use serde_json::{Value, json};
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        for i in 0..30 {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-05-01', ?1, 'Pica pica', 'Magpie', ?2, ?3)",
                rusqlite::params![
                    format!("06:{i:02}:00"),
                    0.512_345 + f64::from(i) / 100.0,
                    format!("c{i}.wav")
                ],
            )
            .expect("insert");
        }
    });
    (dir, state)
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    let body = body.map_or_else(Body::empty, |v| Body::from(v.to_string()));
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn protocol(seed: u64, blind: bool) -> Value {
    json!({
        "from": "2026-05-01", "to": "2026-05-01",
        "score_edges": [0.5, 0.7, 1.0], "per_stratum": 4, "seed": seed, "blind": blind
    })
}

#[tokio::test]
async fn a_blind_sample_shows_no_score_and_exports_weights_and_verdicts() {
    let (_d, state) = station();
    let (status, body) = call(
        &state,
        "POST",
        "/admin/validation/samples",
        Some(protocol(42, true)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();

    let (status, shown) = call(
        &state,
        "GET",
        &format!("/admin/validation/samples/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    let items = serde_json::from_str::<Value>(&shown).unwrap()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 8, "{shown}");
    // Blind: no score anywhere in what the reviewer is shown. Every score in
    // this fixture has the digits 2345 after its second decimal place.
    assert!(
        !shown.contains("2345"),
        "a score reached the blind view: {shown}"
    );
    assert!(!shown.contains("confidence"), "{shown}");

    let (status, recorded) = call(
        &state,
        "POST",
        &format!("/admin/validation/samples/{id}/verdicts"),
        Some(json!({"position": 0, "verdict": "absent"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{recorded}");
    let (status, _) = call(
        &state,
        "POST",
        &format!("/admin/validation/samples/{id}/verdicts"),
        Some(json!({"position": 0, "verdict": "probably"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, csv) = call(
        &state,
        "GET",
        &format!("/admin/validation/samples/{id}/export.csv"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{csv}");
    let rows: Vec<&str> = csv.lines().collect();
    assert_eq!(rows.len(), 9, "{csv}");
    assert!(
        rows[0].contains("inclusion_probability,weight,confidence"),
        "{}",
        rows[0]
    );
    assert!(rows[1].contains(",absent,"), "{}", rows[1]);
    assert!(csv.contains("2345"), "the export carries the score: {csv}");

    // Counterpart: the same protocol and seed draw the same items.
    let (_, body) = call(
        &state,
        "POST",
        "/admin/validation/samples",
        Some(protocol(42, true)),
    )
    .await;
    let again = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    let (_, shown_again) = call(
        &state,
        "GET",
        &format!("/admin/validation/samples/{again}"),
        None,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(&shown_again).unwrap()["items"],
        items
    );
}

#[tokio::test]
async fn a_sample_that_is_not_blind_shows_the_score() {
    let (_d, state) = station();
    let (_, body) = call(
        &state,
        "POST",
        "/admin/validation/samples",
        Some(protocol(1, false)),
    )
    .await;
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    let (_, shown) = call(
        &state,
        "GET",
        &format!("/admin/validation/samples/{id}"),
        None,
    )
    .await;
    assert!(shown.contains("2345"), "{shown}");
}
