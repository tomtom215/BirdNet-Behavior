//! `GET /api/v2/analytics/phenology` and `/abundance` answered 503 "analytics
//! database not configured" on every station, configured or not.
//!
//! Both run their SQL through `run_phenology_query`, which asked the prepared
//! statement for its column names *before* executing it. The `duckdb` crate
//! this workspace builds against (1.10506) panics on that ("The statement was
//! not executed yet"); the panic happened inside `spawn_blocking`, so the
//! handler saw a `JoinError`, and its catch-all arm turned that into the answer
//! for "no analytics store". Nothing exercised either route through a real
//! DuckDB, so the routes shipped, and were documented, without ever answering.
//!
//! This seeds a SQLite file, lets the analytics store mirror it as it does at
//! start-up, adds listening effort, and asks both routes for the rows. The
//! counterpart asks for a year nothing was recorded in: that must be an empty
//! 200, not a refusal. Each live row is also held to the shape `openapi.json`
//! documents for it, because the two documents were written without either
//! route ever having answered.

// Needs the DuckDB analytics store: without the feature `AppState` has no
// analytics methods to call.
#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use serde_json::Value;
use std::collections::BTreeSet;
use tower::ServiceExt as _;

/// The property names `openapi.json` says each row of `path`'s 200 answer has.
fn documented_row_keys(path: &str, array: &str) -> BTreeSet<String> {
    let doc: Value = serde_json::from_str(include_str!("../openapi.json")).expect("openapi.json");
    doc["paths"][path]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
        ["properties"][array]["items"]["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("{path} documents no row shape under {array}"))
        .keys()
        .cloned()
        .collect()
}

fn keys_of(row: &Value) -> BTreeSet<String> {
    row.as_object()
        .expect("a row is an object")
        .keys()
        .cloned()
        .collect()
}

/// A station with seven consecutive days of one species in 2026, mirrored into
/// the analytics store.
fn station(dir: &std::path::Path) -> AppState {
    let sqlite = dir.join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&sqlite).expect("sqlite");
        for day in 3..=9 {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES (?1, '06:00:00', 'Parus major', 'Great Tit', 0.9, ?2)",
                rusqlite::params![format!("2026-03-{day:02}"), format!("tit-{day}.wav")],
            )
            .expect("seed a detection");
        }
    }
    let state = AppState::new_with_analytics(sqlite, &dir.join("analytics.duckdb"))
        .expect("a state with the analytics store");
    // Listening effort: an hour of station time on the 3rd (two microphones
    // listening to the same garden are one hour) and an hour on the 4th.
    state
        .with_analytics(|adb| {
            adb.conn()
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS recording_effort
                         (date TEXT, source TEXT, seconds DOUBLE);
                     INSERT INTO recording_effort VALUES
                         ('2026-03-03','mic-a',3600), ('2026-03-03','mic-b',3600),
                         ('2026-03-04','mic-a',3600);",
                )
                .expect("seed effort");
        })
        .expect("analytics store");
    state
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phenology_names_its_columns_and_returns_the_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    let (status, body) = get(
        &app,
        "/api/v2/analytics/phenology?year=2026&min_detections=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["phenology"].as_array().expect("a phenology array");
    assert_eq!(rows.len(), 1, "one species was recorded: {body}");
    let row = &rows[0];
    assert_eq!(row["species"], "Great Tit", "{row}");
    assert_eq!(row["detection_count"], 7, "{row}");
    assert_eq!(row["detected_days"], 7, "{row}");
    assert_eq!(row["first_detection"], "2026-03-03", "{row}");
    assert_eq!(row["last_detection"], "2026-03-09", "{row}");
    assert_eq!(row["year"], 2026, "{row}");
    assert_eq!(
        keys_of(row),
        documented_row_keys("/analytics/phenology", "phenology"),
        "the live row and the documented row have different keys: {row}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abundance_answers_instead_of_reporting_no_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    let (status, body) = get(&app, "/api/v2/analytics/abundance?year=2026").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a configured store must not read as 'not configured': {body}"
    );
    let rows = body["abundance"].as_array().expect("an abundance array");
    // 2026-03-03..=09 is Tuesday to Monday: six detections in ISO week 10 (the
    // two days with effort among them) and one in week 11, which has none.
    let week = |n: i64| {
        rows.iter()
            .find(|r| r["iso_week"] == n)
            .unwrap_or_else(|| panic!("week {n}: {body}"))
    };
    let w10 = week(10);
    assert_eq!(w10["species"], "Great Tit", "{w10}");
    assert_eq!(w10["raw_count"], 6, "{w10}");
    assert_eq!(w10["effort_count"], 2, "{w10}");
    assert_eq!(w10["effort_hours"], 2.0, "{w10}");
    assert_eq!(w10["detections_per_hour"], 1.0, "{w10}");
    let w11 = week(11);
    assert_eq!(w11["raw_count"], 1, "{w11}");
    assert!(
        w11["effort_hours"].is_null() && w11["detections_per_hour"].is_null(),
        "a week nobody listened in has no rate, not a zero: {w11}"
    );
    assert_eq!(
        keys_of(w10),
        documented_row_keys("/analytics/abundance", "abundance"),
        "the live row and the documented row have different keys: {w10}"
    );
}

/// The counterpart: a year with nothing in it is an empty answer, and the store
/// being there is what says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_year_with_no_detections_is_an_empty_answer_not_a_refusal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    let (status, body) = get(
        &app,
        "/api/v2/analytics/phenology?year=1999&min_detections=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["phenology"].as_array().map(Vec::len),
        Some(0),
        "{body}"
    );
}
