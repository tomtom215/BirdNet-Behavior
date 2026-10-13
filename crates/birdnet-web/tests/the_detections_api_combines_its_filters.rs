//! `/api/v2/detections` applies every filter it is given, together.
//!
//! It took `species` or `date`, never both: with `species` present the date
//! was dropped without a word, so `?species=X&date=Y` answered with every
//! detection of X on every day. And it had no `source` at all, which a
//! two-microphone station needs to ask "what did the feeder mic hear".

use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Four detections, laid out so each filter on its own selects a different
/// set from any two of them together.
fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for row in [
        [
            "2026-05-01",
            "06:00:00",
            "Turdus merula",
            "Eurasian Blackbird",
            "feeder",
        ],
        [
            "2026-05-01",
            "06:05:00",
            "Turdus merula",
            "Eurasian Blackbird",
            "nestbox",
        ],
        [
            "2026-05-02",
            "06:00:00",
            "Turdus merula",
            "Eurasian Blackbird",
            "feeder",
        ],
        [
            "2026-05-01",
            "07:00:00",
            "Erithacus rubecula",
            "European Robin",
            "feeder",
        ],
    ] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Source)
             VALUES (?1, ?2, ?3, ?4, 0.9, ?5)",
            row,
        )
        .expect("seed");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, query: &str) -> Value {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/api/v2/detections?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{query}: {}", resp.status());
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).expect("json")
}

/// `date time` of each row, in the order returned.
async fn rows(state: &AppState, query: &str) -> Vec<String> {
    get(state, query).await["detections"]
        .as_array()
        .expect("detections")
        .iter()
        .map(|d| {
            format!(
                "{} {}",
                d["date"].as_str().unwrap_or("?"),
                d["time"].as_str().unwrap_or("?")
            )
        })
        .collect()
}

#[tokio::test]
async fn species_and_date_together_are_both_applied() {
    let st = station();
    assert_eq!(
        rows(&st, "species=Eurasian+Blackbird&date=2026-05-01").await,
        ["2026-05-01 06:05:00", "2026-05-01 06:00:00"],
        "the blackbird on the 2nd is not on the 1st"
    );
}

#[tokio::test]
async fn source_narrows_the_list() {
    let st = station();
    assert_eq!(rows(&st, "source=nestbox").await, ["2026-05-01 06:05:00"]);
    assert_eq!(
        rows(&st, "source=feeder&species=Eurasian+Blackbird").await,
        ["2026-05-02 06:00:00", "2026-05-01 06:00:00"]
    );
}

/// The counterpart: one filter at a time still selects what it always did,
/// so the two tests above cannot pass on a list that simply returns less.
#[tokio::test]
async fn each_filter_alone_is_unchanged() {
    let st = station();
    // Precondition: the fixture is all there.
    assert_eq!(rows(&st, "").await.len(), 4);
    assert_eq!(rows(&st, "species=Eurasian+Blackbird").await.len(), 3);
    assert_eq!(
        rows(&st, "date=2026-05-01").await,
        [
            "2026-05-01 07:00:00",
            "2026-05-01 06:05:00",
            "2026-05-01 06:00:00"
        ]
    );
}

/// `total_count` is the size of the filtered set, not of the table — a
/// client paging a filtered list needs to know when it has reached the end.
#[tokio::test]
async fn total_count_counts_the_filtered_set() {
    let st = station();
    let v = get(&st, "source=feeder&limit=1").await;
    assert_eq!(v["count"], 1);
    assert_eq!(v["total_count"], 3);
}
