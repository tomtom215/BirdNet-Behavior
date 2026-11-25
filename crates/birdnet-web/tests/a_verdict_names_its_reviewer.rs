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

/// One form post that answers 200 whatever happened, as the bulk action and
/// quarantine approval do: what they did is read back from the database.
async fn post_any(app: &axum::Router, uri: &str, form: &str) {
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
}

/// N24-04: the bulk confirm and reject on Search record who gave each verdict.
#[tokio::test]
async fn the_bulk_verdicts_on_search_record_who_gave_them() {
    let (app, state) = station();
    let selected = "selected=2026-09-08%7C06%3A00%3A00%7Ca.wav%7CTurdus+merula";
    post_any(
        &app,
        "/pages/search-bulk",
        &format!("action=confirm&{selected}"),
    )
    .await;
    post_any(
        &app,
        "/pages/search-bulk",
        &format!("action=reject&{selected}"),
    )
    .await;
    assert_eq!(
        history(&state),
        [
            (
                Some("admin".to_owned()),
                Some("confirmed".to_owned()),
                "bulk".to_owned()
            ),
            (
                Some("admin".to_owned()),
                Some("rejected".to_owned()),
                "bulk".to_owned()
            ),
        ],
    );
}

/// N24-04: approving a quarantined detection records who approved it.
#[tokio::test]
async fn a_quarantine_approval_records_who_approved_it() {
    let (app, state) = station();
    let id: i64 = state.with_db(|conn| {
        conn.execute(
            "INSERT INTO quarantine (date, time, sci_name, com_name, confidence, reason, file_name)
             VALUES (?1, '07:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.4,
                     'low_confidence', 'b.wav')",
            [DATE],
        )
        .expect("quarantine a detection");
        conn.last_insert_rowid()
    });
    post_any(&app, "/pages/quarantine-approve", &format!("id={id}")).await;
    assert_eq!(
        history(&state),
        [(
            Some("admin".to_owned()),
            Some("confirmed".to_owned()),
            "quarantine".to_owned()
        )],
    );
}

/// N24-04, the API half. A bearer token is not a signed-in person, so no name
/// is recorded (the audit row names nobody either, for the same reason); what
/// the history must say is that the verdict came through the API, for the
/// single endpoint and the batch, rather than "unattributed".
#[tokio::test]
async fn an_api_verdict_is_recorded_as_one() {
    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    let (_, state) = station();
    let state = state.with_api_token(birdnet_web::api_token::ApiToken::new(TOKEN).expect("token"));
    let app = build_router(state.clone());
    for (uri, body) in [
        (
            "/api/v2/detections/review",
            r#"{"date":"2026-09-08","time":"06:00:00","sci_name":"Turdus merula","file_name":"a.wav","status":"confirmed"}"#,
        ),
        (
            "/api/v2/detections/batch",
            r#"{"op":"review","status":"rejected","detections":[{"date":"2026-09-08","time":"06:00:00","sci_name":"Turdus merula","file_name":"a.wav"}]}"#,
        ),
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
    }
    assert_eq!(
        history(&state),
        [
            (None, Some("confirmed".to_owned()), "api".to_owned()),
            (None, Some("rejected".to_owned()), "api".to_owned()),
        ],
    );
}
