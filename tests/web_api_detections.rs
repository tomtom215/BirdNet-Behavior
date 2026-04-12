//! Integration tests for the detection API endpoints.
//!
//! Test data dates are resolved from `SQLite`'s `DATE('now')` so the fixture
//! always lives inside any "last N days" window. Hard-coding calendar dates
//! creates time-bombs that pass on the day they were written and silently
//! fail later once the data drifts outside the window — which is exactly
//! what broke the first release of this crate.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use serde_json::Value;
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Common name shared by the duplicate-species rows in the fixture.
const REPEATED_SPECIES_COM: &str = "Eurasian Blackbird";

/// Total number of records inserted by [`Fixture::new`].
const FIXTURE_TOTAL_ROWS: usize = 5;
/// Number of fixture rows dated "today".
const FIXTURE_TODAY_ROWS: usize = 4;
/// Number of fixture rows dated "yesterday".
const FIXTURE_YESTERDAY_ROWS: usize = FIXTURE_TOTAL_ROWS - FIXTURE_TODAY_ROWS;
/// Number of fixture rows whose `Com_Name` equals [`REPEATED_SPECIES_COM`].
const FIXTURE_REPEATED_SPECIES_ROWS: usize = 2;

/// `MAX_LIMIT` enforced by the detections route handler. Mirrored here so the
/// "limit is capped" test fails loudly if the production constant changes
/// without the test being updated.
const ROUTE_MAX_LIMIT: u32 = 1000;

/// Test fixture: an in-memory `SQLite` seeded with detections whose dates
/// are always relative to "now", plus the resolved date strings so individual
/// tests can build URLs without re-deriving them.
struct Fixture {
    state: AppState,
    today: String,
    yesterday: String,
}

impl Fixture {
    fn new() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS detections (
                Date TEXT NOT NULL,
                Time TEXT NOT NULL,
                Sci_Name TEXT NOT NULL,
                Com_Name TEXT NOT NULL,
                Confidence REAL NOT NULL,
                Lat REAL,
                Lon REAL,
                Cutoff REAL,
                Week INTEGER,
                Sens REAL,
                Overlap REAL,
                File_Name TEXT
            );",
        )
        .unwrap();

        // Resolve dates from the same engine the production query uses so we
        // never disagree with it about what "today" is (timezone, DST, etc.).
        let today: String = conn
            .query_row("SELECT DATE('now')", [], |r| r.get(0))
            .unwrap();
        let yesterday: String = conn
            .query_row("SELECT DATE('now', '-1 day')", [], |r| r.get(0))
            .unwrap();

        // Four rows "today" (two of them sharing a species so the species
        // filter has something meaningful to assert) and one row "yesterday"
        // so the daily-counts endpoint sees more than one bucket.
        let today_str = today.as_str();
        let yesterday_str = yesterday.as_str();
        let records: [(&str, &str, &str, &str, f64); FIXTURE_TOTAL_ROWS] = [
            (
                today_str,
                "06:30:00",
                "Turdus merula",
                REPEATED_SPECIES_COM,
                0.87,
            ),
            (
                today_str,
                "06:35:00",
                "Erithacus rubecula",
                "European Robin",
                0.92,
            ),
            (
                today_str,
                "06:45:00",
                "Turdus merula",
                REPEATED_SPECIES_COM,
                0.78,
            ),
            (today_str, "07:00:00", "Parus major", "Great Tit", 0.81),
            (
                yesterday_str,
                "18:00:00",
                "Cyanistes caeruleus",
                "Eurasian Blue Tit",
                0.75,
            ),
        ];

        for (date, time, sci, com, conf) in &records {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![date, time, sci, com, conf],
            )
            .unwrap();
        }

        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        Self {
            state,
            today,
            yesterday,
        }
    }

    fn router(&self) -> axum::Router {
        build_router(self.state.clone())
    }
}

/// Send a `GET` request to `router` and decode the response body as `JSON`.
async fn get_json(router: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

#[tokio::test]
async fn detections_by_date_returns_only_that_days_rows() {
    let fixture = Fixture::new();
    let uri = format!("/api/v2/detections?date={}", fixture.today);

    let (status, json) = get_json(fixture.router(), &uri).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(json["count"], FIXTURE_TODAY_ROWS);
    let detections = json["detections"].as_array().unwrap();
    assert_eq!(detections.len(), FIXTURE_TODAY_ROWS);

    // Every returned row must be from the requested date.
    assert!(
        detections
            .iter()
            .all(|d| d["date"] == fixture.today.as_str()),
        "all rows should be dated {}, got {detections:?}",
        fixture.today,
    );

    // Rows should be ordered by Time DESC — the latest "today" insert is 07:00.
    assert_eq!(detections[0]["time"], "07:00:00");
}

#[tokio::test]
async fn detections_by_species_filter_returns_only_that_species() {
    let fixture = Fixture::new();
    // The fixture's repeated species name only contains ASCII letters and a
    // single space, so a literal `%20` substitution is sufficient and avoids
    // pulling in a URL-encoding crate.
    let uri = format!(
        "/api/v2/detections?species={}",
        REPEATED_SPECIES_COM.replace(' ', "%20"),
    );

    let (status, json) = get_json(fixture.router(), &uri).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(json["count"], FIXTURE_REPEATED_SPECIES_ROWS);
    let detections = json["detections"].as_array().unwrap();
    assert!(
        detections
            .iter()
            .all(|d| d["com_name"] == REPEATED_SPECIES_COM),
        "every row should match {REPEATED_SPECIES_COM}, got {detections:?}",
    );
}

#[tokio::test]
async fn recent_detections_respects_limit() {
    let fixture = Fixture::new();
    let limit: u32 = 3;

    let (status, json) = get_json(
        fixture.router(),
        &format!("/api/v2/detections/recent?limit={limit}"),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], limit);
}

#[tokio::test]
async fn daily_detections_endpoint_returns_buckets_within_window() {
    let fixture = Fixture::new();

    // 30-day window comfortably covers the fixture's "today" + "yesterday" rows.
    let (status, json) = get_json(fixture.router(), "/api/v2/detections/daily?days=30").await;
    assert_eq!(status, StatusCode::OK);

    assert!(json["daily"].is_array());
    let daily = json["daily"].as_array().unwrap();

    // Two distinct dates were seeded, so we expect exactly two buckets.
    assert_eq!(
        daily.len(),
        2,
        "expected two daily buckets (today + yesterday), got {daily:?}",
    );
    assert_eq!(json["total"], 2);

    // Bucket counts should match what we inserted, regardless of ordering.
    let count_for = |date: &str| -> Value {
        daily
            .iter()
            .find(|row| row["date"] == date)
            .unwrap_or_else(|| panic!("no daily bucket for {date} in {daily:?}"))["count"]
            .clone()
    };
    assert_eq!(count_for(&fixture.today), FIXTURE_TODAY_ROWS);
    assert_eq!(count_for(&fixture.yesterday), FIXTURE_YESTERDAY_ROWS);
}

#[tokio::test]
async fn detections_pagination_first_page() {
    let fixture = Fixture::new();
    let (status, json) =
        get_json(fixture.router(), "/api/v2/detections?limit=2&offset=0").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(json["count"], 2);
    assert_eq!(json["limit"], 2);
    assert_eq!(json["offset"], 0);
    assert_eq!(json["detections"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn detections_pagination_second_page() {
    let fixture = Fixture::new();
    let (status, json) =
        get_json(fixture.router(), "/api/v2/detections?limit=2&offset=2").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(json["count"], 2);
    assert_eq!(json["offset"], 2);
}

#[tokio::test]
async fn detections_pagination_beyond_data_is_empty() {
    let fixture = Fixture::new();
    let (status, json) =
        get_json(fixture.router(), "/api/v2/detections?limit=10&offset=100").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn detections_invalid_date_returns_400() {
    let fixture = Fixture::new();
    let (status, json) =
        get_json(fixture.router(), "/api/v2/detections?date=not-a-date").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["error"].as_str().unwrap().contains("invalid date"));
}

#[tokio::test]
async fn detections_pagination_includes_total_count() {
    let fixture = Fixture::new();
    let (status, json) =
        get_json(fixture.router(), "/api/v2/detections?limit=2&offset=0").await;
    assert_eq!(status, StatusCode::OK);

    // `total_count` is the count of all matching rows, not just the page.
    assert_eq!(json["total_count"], FIXTURE_TOTAL_ROWS);
    assert_eq!(json["count"], 2);
    assert_eq!(json["limit"], 2);
    assert_eq!(json["offset"], 0);
}

#[tokio::test]
async fn detections_limit_is_capped_at_max() {
    let fixture = Fixture::new();
    let (status, json) =
        get_json(fixture.router(), "/api/v2/detections?limit=999999").await;
    assert_eq!(status, StatusCode::OK);

    // The handler must clamp the requested limit at `ROUTE_MAX_LIMIT`.
    assert_eq!(json["limit"], ROUTE_MAX_LIMIT);
}
