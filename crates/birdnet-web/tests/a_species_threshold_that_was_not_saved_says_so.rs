//! A species threshold that was not saved says so.
//!
//! `/admin/species/thresholds/set` answered a percentage slip (`75` in a 0–1
//! field) with a bare `400`, which htmx discards: nothing happened on the page,
//! nothing said why. And a write the database refused was swallowed with
//! `.ok()`, recorded in the audit log as a successful set, and answered with
//! the list re-rendered as if it had worked — so the operator believed a rare
//! bird's threshold was lowered when it was not.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn set(state: &AppState, threshold: &str) -> (u16, Option<String>, String) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/species/thresholds/set")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "sci_name=Tichodroma+muraria&threshold={threshold}"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let reswap = resp
        .headers()
        .get("hx-reswap")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, reswap, String::from_utf8_lossy(&body).into_owned())
}

fn audited(state: &AppState) -> i64 {
    state.with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'species.threshold.set'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    })
}

#[tokio::test]
async fn a_percentage_is_answered_with_what_was_meant() {
    let st = station();
    let (status, reswap, body) = set(&st, "75").await;
    assert_eq!(status, 200, "htmx discards a 4xx body: {body}");
    assert_eq!(reswap.as_deref(), Some("none"));
    assert!(body.contains("0.75"), "{body}");
    assert_eq!(audited(&st), 0);
}

#[tokio::test]
async fn a_refused_write_is_not_reported_or_audited_as_done() {
    let st = station();
    st.with_db(|c| c.execute_batch("DROP TABLE species_thresholds"))
        .unwrap();
    let (status, reswap, body) = set(&st, "0.35").await;
    assert_eq!(status, 200);
    assert_eq!(reswap.as_deref(), Some("none"), "shown as saved: {body}");
    assert_eq!(audited(&st), 0, "audited as a successful set");
}

/// The counterpart: a good value is saved, shown and audited.
#[tokio::test]
async fn a_good_value_is_saved() {
    let st = station();
    let (status, reswap, body) = set(&st, "0.35").await;
    assert_eq!(status, 200);
    assert_eq!(reswap, None);
    assert!(
        body.contains("Tichodroma muraria") || body.contains("0.35"),
        "{body}"
    );
    assert_eq!(audited(&st), 1);
}
