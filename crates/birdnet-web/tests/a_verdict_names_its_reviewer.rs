//! A verdict given on a page is kept with the name of whoever gave it (RES-06).
//!
//! The database keeps an append-only history of verdicts (`review_events`,
//! migration 64), but the history is only as good as what the handlers hand
//! it: a page that wrote through the unattributed path recorded every verdict
//! with no reviewer, and the audit row that named them is pruned after 180
//! days. These go through the real router, signed in as the station's open-
//! bypass admin, and read the history the way an export would.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const DATE: &str = "2026-09-08";

fn station() -> (axum::Router, AppState) {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (?1, '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav')",
        [DATE],
    )
    .expect("seed");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    (build_router(state.clone()), state)
}

async fn post(app: &axum::Router, uri: &str, form: &str) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .body(Body::from(form.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    assert!(
        res.headers().get("hx-reswap").is_none(),
        "{uri}: the write was refused"
    );
    String::from_utf8_lossy(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned()
}

/// `(reviewer_name, verdict, via)` of every event, oldest first.
fn history(state: &AppState) -> Vec<(Option<String>, Option<String>, String)> {
    state.with_db(|conn| {
        conn.prepare("SELECT reviewer_name, verdict, via FROM review_events ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

const FORM: &str = "date=2026-09-08&time=06%3A00%3A00&sci_name=Turdus+merula\
                    &com_name=Eurasian+Blackbird&file_name=a.wav";

/// The queue's form, the detection page's inline widget and the queue's undo
/// each record the signed-in person against what they did.
#[tokio::test]
async fn every_review_form_records_who_gave_the_verdict() {
    let (app, state) = station();
    let admin = || Some("admin".to_owned());

    post(
        &app,
        "/pages/detection-review",
        &format!("{FORM}&status=confirmed"),
    )
    .await;
    post(
        &app,
        "/pages/detection-review-inline",
        &format!("{FORM}&status=rejected"),
    )
    .await;
    post(&app, "/pages/detection-review-clear", FORM).await;

    assert_eq!(
        history(&state),
        [
            (admin(), Some("confirmed".to_owned()), "review".to_owned()),
            (admin(), Some("rejected".to_owned()), "detection".to_owned()),
            (admin(), None, "review".to_owned()),
        ],
    );
}
