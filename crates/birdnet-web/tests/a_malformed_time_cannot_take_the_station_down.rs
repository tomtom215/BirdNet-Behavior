//! A malformed `Time` in the database is bad data, not a reason to stop.
//!
//! `/pages/hourly-totals` read each row's hour as `CAST(SUBSTR(Time,1,2) AS
//! INTEGER)` into a `u8` and indexed a 24-slot array with it. `Time` carries no
//! constraint, and the BirdNET-Pi importer deliberately copies malformed values
//! through with a warning — so one imported `25:00:00` row panicked the
//! handler, and the release profile's `panic = "abort"` took the whole station
//! down with it: capture, detection, everything. The sibling heat-map paths
//! already guarded the index; this one did not.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

#[tokio::test]
async fn an_hour_past_23_is_skipped_not_indexed() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES
           (date('now','localtime'), '25:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9),
           (date('now','localtime'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9);",
    )
    .expect("seed");
    let resp = build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
    .oneshot(
        Request::builder()
            .uri("/pages/hourly-totals?days=7")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .expect("the handler must answer, not panic");
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8_lossy(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    // Counterpart: the well-formed row still reaches the chart.
    assert!(html.contains("<svg"), "the chart did not render: {html}");
}
