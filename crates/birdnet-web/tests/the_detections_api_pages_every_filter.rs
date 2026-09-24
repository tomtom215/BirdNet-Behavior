//! `/api/v2/detections` honours `limit` and `offset` with every filter.
//!
//! `?date=` ignored both — `detections_by_date` has no `LIMIT`, so a busy day
//! came back whole while the response still said `"limit": 100` — and
//! `?species=` ignored `offset`, so a client paging through a species got the
//! same first page forever.

use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for minute in 0..5 {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-05-01', ?1, 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [format!("06:0{minute}:00")],
        )
        .expect("seed");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn times(state: &AppState, query: &str) -> Vec<String> {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/api/v2/detections?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).expect("json");
    json["detections"]
        .as_array()
        .expect("detections")
        .iter()
        .map(|d| {
            d["time"]
                .as_str()
                .or_else(|| d["Time"].as_str())
                .unwrap_or("?")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn a_date_and_a_species_page_like_the_unfiltered_list() {
    let st = station();
    let page = vec!["06:02:00".to_owned(), "06:01:00".to_owned()];
    // Precondition: the unfiltered list pages, so the expected page is right.
    assert_eq!(times(&st, "limit=2&offset=2").await, page);
    assert_eq!(
        times(&st, "date=2026-05-01&limit=2&offset=2").await,
        page,
        "by date"
    );
    assert_eq!(
        times(&st, "species=Eurasian+Blackbird&limit=2&offset=2").await,
        page,
        "by species"
    );
}
