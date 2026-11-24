//! A comment belongs to the detection it was written on, not to every
//! detection that shares its date, time and species (MSC-05).
//!
//! Two sources that hear one bird in one second write two detections that
//! differ only in their clip. Comments were keyed by `(date, time, sci_name)`,
//! so a note written on the garden microphone's row ("this is the neighbour's
//! dog") appeared on the camera's row too. Reviews were moved to a per-clip key
//! by migration 59; comments were not.
//!
//! Observed failing on the code before the fix: the second detection's thread
//! listed the first one's comment, through the API and on the page alike.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::api_token::ApiToken;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const DATE: &str = "2026-05-01";
const TIME: &str = "06:00:00";
const SCI: &str = "Strix aluco";
const GARDEN: &str = "2026-05-01-birdnet-garden-06:00:00.wav";
const CAMERA: &str = "2026-05-01-birdnet-camera-06:00:00.wav";
const TOKEN: &str = "test-token-for-detection-comments-0123456789";

/// A station whose two sources each recorded the same species in the same
/// second.
fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db"))
        .expect("state")
        .with_api_token(ApiToken::new(TOKEN).expect("long enough"));
    state.with_db(|conn| {
        for (file, source) in [(GARDEN, "garden"), (CAMERA, "camera")] {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs, Source)
                 VALUES (?1, ?2, ?3, 'Tawny Owl', 0.9, 0.7, 18, 1.25, 0.0, ?4, 0, ?5)",
                rusqlite::params![DATE, TIME, SCI, file, source],
            )
            .expect("insert detection");
        }
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3",
                rusqlite::params![DATE, TIME, SCI],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 2, "precondition: two detections share the triple");
    });
    (dir, state)
}

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let app = birdnet_web::server::build_router(state.clone());
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    let req = match body {
        Some(v) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = app.oneshot(req).await.expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
}

fn thread_query(file: &str) -> String {
    format!(
        "date={DATE}&time={TIME}&sci_name=Strix%20aluco&file_name={}",
        file.replace(':', "%3A")
    )
}

async fn api_comments(state: &AppState, file: &str) -> Vec<serde_json::Value> {
    let (status, body) = call(
        state,
        "GET",
        &format!("/api/v2/detections/comments?{}", thread_query(file)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    json["comments"].as_array().expect("array").clone()
}

/// The acceptance gate: two detections, one triple, different clips; a comment
/// on one; the other's comment list is empty.
#[tokio::test]
async fn a_comment_on_one_sources_detection_is_not_listed_on_the_others() {
    let (_dir, state) = station();

    let (status, created) = call(
        &state,
        "POST",
        "/api/v2/detections/comments",
        Some(serde_json::json!({
            "date": DATE, "time": TIME, "sci_name": SCI, "file_name": GARDEN,
            "body": "This is the neighbour's dog, not an owl."
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let garden = api_comments(&state, GARDEN).await;
    assert_eq!(
        garden.len(),
        1,
        "the comment is on the detection it was written on: {garden:?}"
    );
    let camera = api_comments(&state, CAMERA).await;
    assert!(
        camera.is_empty(),
        "the camera's detection must not carry the garden's comment: {camera:?}"
    );

    let (status, page) = call(
        &state,
        "GET",
        &format!("/pages/detection-comments?{}", thread_query(CAMERA)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !page.contains("neighbour"),
        "nor may the camera's page thread show it: {page}"
    );
}

/// The counterpart: a comment that does not say which of two detections it is
/// about is refused, not attached to both. Without this, a handler that
/// silently stored every unnamed comment against no clip would leave the gate
/// above green.
#[tokio::test]
async fn a_comment_that_names_no_clip_among_two_detections_is_refused() {
    let (_dir, state) = station();
    let (status, body) = call(
        &state,
        "POST",
        "/api/v2/detections/comments",
        Some(serde_json::json!({
            "date": DATE, "time": TIME, "sci_name": SCI, "body": "which one?"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(api_comments(&state, GARDEN).await.is_empty());
    assert!(api_comments(&state, CAMERA).await.is_empty());
}

/// A comment written before comments named a clip, on a second two sources
/// share, cannot be given to either. It is still shown on both, and marked as
/// such, rather than hidden.
#[tokio::test]
async fn a_legacy_comment_on_a_shared_second_is_shown_and_marked() {
    let (_dir, state) = station();
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detection_comments (date, time, sci_name, author, body)
             VALUES (?1, ?2, ?3, 'ada', 'written before comments named a clip')",
            rusqlite::params![DATE, TIME, SCI],
        )
        .expect("legacy comment");
    });
    for file in [GARDEN, CAMERA] {
        let listed = api_comments(&state, file).await;
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert!(listed[0]["file_name"].is_null(), "{listed:?}");

        let (_, page) = call(
            &state,
            "GET",
            &format!("/pages/detection-comments?{}", thread_query(file)),
            None,
        )
        .await;
        assert!(
            page.contains("written before comments named a clip"),
            "{page}"
        );
        assert!(
            page.contains("dc-legacy"),
            "a comment that may be about another source's detection says so: {page}"
        );
    }
}
