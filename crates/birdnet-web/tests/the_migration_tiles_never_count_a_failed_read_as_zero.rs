//! The Migration tab's four tiles never report a failed read as zeros.
//!
//! `stats_partial` defaulted its whole result with
//! `.unwrap_or((0, 0, 0, None, 0))` and then **cached** it, so one database
//! error rendered "First-of-year arrivals 0 · Peak diversity week w0 ·
//! no overdue migrants" as fact, and kept serving it after the database
//! recovered. The peak week and the "still expected" count defaulted to zero
//! individually as well. Where a surface's whole content is a claim about the
//! reader's data, propagate and let the caller say it failed.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

async fn tiles(state: &AppState) -> String {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/pages/migration-stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&body).into_owned()
}

fn state() -> (AppState, String) {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let view: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'detections_analytic'",
            [],
            |r| r.get(0),
        )
        .expect("the view exists");
    (
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:")),
        view,
    )
}

#[tokio::test]
async fn a_failed_read_is_not_zeros_and_is_not_kept() {
    let (st, view) = state();
    st.with_db(|c| c.execute_batch("DROP VIEW detections_analytic"))
        .unwrap();
    let broken = tiles(&st).await;
    assert!(
        !broken.contains(r#"<span class="value">0</span>"#) && !broken.contains(">w0<"),
        "a failed read rendered as zeros: {broken}"
    );
    assert!(broken.contains("unavailable"), "{broken}");

    // The database recovers; the failure must not have been cached.
    st.with_db(|c| c.execute_batch(&view)).unwrap();
    let healed = tiles(&st).await;
    assert!(healed.contains("First-of-year arrivals"), "{healed}");
}

/// The counterpart: a healthy station with nothing yet this year does show
/// its zero, because that zero is true.
#[tokio::test]
async fn a_true_zero_is_still_shown() {
    let (st, _) = state();
    let html = tiles(&st).await;
    assert!(html.contains("First-of-year arrivals"), "{html}");
    assert!(html.contains(r#"<span class="value">0</span>"#), "{html}");
}
