//! `GET /api/v2/analytics/assembly` and `/answers`: the two routes the Sequences tab's Method
//! panels link to, so that each picture's numbers can be taken away.
//!
//! Both run against a real engine, because what they are for is the *answer*: the depth the
//! funnel reaches within each window, and the pairs counted within a minute. The station is two
//! mornings in March 2026 with every number worked out by hand:
//!
//! * 03-03: Robin 05:00:00, Blackbird 05:10:00, Wren 05:30:00. Two voices exactly ten minutes
//!   apart and three exactly thirty, so the windows 9, 10, 29 and 30 sit either side of both.
//! * 03-04: Robin 05:00:00 and Blackbird 05:00:30. Two voices from the first minute, no third.
//!
//! The counterparts ask for what must be refused (a window list that does not rise, a date that is
//! not a date, a sequence of one, a window of nothing) and for the defaults, which must still
//! answer with the documented ladder and not an error.

// Needs the DuckDB analytics store: without the feature `AppState` has no analytics methods.
#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use serde_json::{Value, json};
use tower::ServiceExt as _;

/// Seconds since the epoch for a UTC date and clock time (`YYYY-MM-DD`, `HH:MM:SS`).
fn epoch(date: &str, time: &str) -> i64 {
    let n = |s: &str| s.parse::<i64>().expect("number");
    let (y, m, d) = (n(&date[0..4]), n(&date[5..7]), n(&date[8..10]));
    let (y, mp) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + n(&time[0..2]) * 3600 + n(&time[3..5]) * 60 + n(&time[6..8])
}

fn station(dir: &std::path::Path) -> AppState {
    let sqlite = dir.join("birds.db");
    {
        let conn = birdnet_db::sqlite::open_or_create(&sqlite).expect("sqlite");
        let rows = [
            ("2026-03-03", "05:00:00", "Erithacus rubecula", "Robin"),
            ("2026-03-03", "05:10:00", "Turdus merula", "Blackbird"),
            ("2026-03-03", "05:30:00", "Troglodytes troglodytes", "Wren"),
            ("2026-03-04", "05:00:00", "Erithacus rubecula", "Robin"),
            ("2026-03-04", "05:00:30", "Turdus merula", "Blackbird"),
        ];
        for (i, (date, time, sci, com)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
                 VALUES (?1, ?2, ?3, ?4, 0.9, ?5, ?6)",
                rusqlite::params![
                    date,
                    time,
                    sci,
                    com,
                    format!("clip-{i}.wav"),
                    epoch(date, time)
                ],
            )
            .expect("seed a detection");
        }
    }
    AppState::new_with_analytics(sqlite, &dir.join("analytics.duckdb"))
        .expect("a state with the analytics store")
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

const THREE: &str = "species=Robin,Blackbird,Wren&hour_start=4&hour_end=8";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assembly_gives_the_depth_within_each_window_newest_morning_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    let (status, body) = get(
        &app,
        &format!("/api/v2/analytics/assembly?{THREE}&windows=1,9,10,29,30&since=2026-03-01"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2, "{body}");
    assert_eq!(body["windows_minutes"], json!([1, 9, 10, 29, 30]), "{body}");
    assert_eq!(
        body["species_sequence"],
        json!(["Robin", "Blackbird", "Wren"]),
        "{body}"
    );
    assert_eq!(body["since"], "2026-03-01", "{body}");
    assert_eq!(
        body["assembly"],
        json!([
            // 03-04: two voices from the first minute, never a third.
            { "date": "2026-03-04", "depths": [2, 2, 2, 2, 2] },
            // 03-03: two voices from ten minutes (and not nine), three from thirty (not 29).
            { "date": "2026-03-03", "depths": [1, 1, 2, 2, 3] },
        ]),
        "{body}"
    );

    // `since` is a floor on the morning, inclusive.
    let (_, body) = get(
        &app,
        &format!("/api/v2/analytics/assembly?{THREE}&windows=1,30&since=2026-03-04"),
    )
    .await;
    assert_eq!(body["total"], 1, "{body}");
    assert_eq!(body["assembly"][0]["date"], "2026-03-04", "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assembly_refuses_what_it_cannot_ask_and_answers_a_bare_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    for (what, query) in [
        ("windows that do not rise", "windows=5,5"),
        ("windows that go down", "windows=30,5"),
        ("a window of nothing", "windows=0,5"),
        ("a window that is not a number", "windows=a,b"),
        ("a date that is not a date", "since=2026-3-1"),
        ("a date with a quote in it", "since=2026-03-01'%20OR%201=1"),
        ("a sequence of one", "species=Robin"),
    ] {
        let (status, body) = get(&app, &format!("/api/v2/analytics/assembly?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
        assert_eq!(body["status"], "invalid_request", "{what}: {body}");
    }

    // Nothing asked for: the default voices and the documented ladder, answered, not refused.
    // (Default since is 90 days back, so March 2026 is out of it and there are no rows.)
    let (status, body) = get(&app, "/api/v2/analytics/assembly").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["windows_minutes"],
        json!([1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120, 180, 300]),
        "{body}"
    );
    assert!(body["assembly"].is_array(), "{body}");
}

/// A browser submits a control it renders as `hour_start=`, blank, and a blank number is "not
/// given", as it is on every other route that shares a name with a control (the repo's
/// `no_query_field_with_a_rendered_control_reads_a_blank_strictly` holds the rule); a refusal
/// here would turn a form that had been left alone into a 400.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blank_hour_is_not_given_rather_than_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));
    let (status, blank) = get(&app, "/api/v2/analytics/assembly?hour_start=&hour_end=").await;
    assert_eq!(status, StatusCode::OK, "{blank}");
    let (_, bare) = get(&app, "/api/v2/analytics/assembly").await;
    assert_eq!(blank, bare, "blank hours read as the defaults");
    // The counterpart: a number is still read, and what is not a number is still refused, so
    // reading a blank leniently has not made the field lenient about anything else.
    let (status, _) = get(&app, "/api/v2/analytics/assembly?hour_start=5&hour_end=8").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&app, "/api/v2/analytics/assembly?hour_start=dawn").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answers_counts_each_pair_within_the_window_and_is_not_symmetric() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    // Ranked by how often heard, ties by name: Blackbird 2, Robin 2, Wren 1.
    let (status, body) = get(
        &app,
        "/api/v2/analytics/answers?from=2026-03-01&within=1&top=3",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["answers"]["species"],
        json!(["Blackbird", "Robin", "Wren"]),
        "{body}"
    );
    assert_eq!(body["answers"]["heard"], json!([2, 2, 1]), "{body}");
    assert_eq!(body["total"], 3, "{body}");
    // Within a minute only 03-04's Robin then Blackbird (30 s) counts; the 03-03 pair is ten
    // minutes apart. Row = heard first, column = heard after: the other way round is zero.
    assert_eq!(
        body["answers"]["exchanges"],
        json!([[0, 0, 0], [1, 0, 0], [0, 0, 0]]),
        "{body}"
    );
    assert_eq!(body["answers"]["within_secs"], 60, "{body}");

    // Fifteen minutes lets 03-03's Robin then Blackbird in as well (ten minutes), and still not
    // the Wren (twenty after the Blackbird, thirty after the Robin).
    let (_, body) = get(
        &app,
        "/api/v2/analytics/answers?from=2026-03-01&within=15&top=3",
    )
    .await;
    assert_eq!(
        body["answers"]["exchanges"],
        json!([[0, 0, 0], [2, 0, 0], [0, 0, 0]]),
        "{body}"
    );
    assert_eq!(body["answers"]["within_secs"], 900, "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answers_refuses_what_it_cannot_ask() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = build_router(station(dir.path()));

    for (what, query) in [
        ("a date that is not a date", "from=yesterday"),
        ("one species is no comparison", "top=1"),
        ("more species than the grid draws", "top=99"),
        ("a window of nothing", "within=0"),
    ] {
        let (status, body) = get(&app, &format!("/api/v2/analytics/answers?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
        assert_eq!(body["status"], "invalid_request", "{what}: {body}");
    }
    // Nothing asked for: the default is a one-minute window (what the tab draws), and it answers.
    let (status, body) = get(&app, "/api/v2/analytics/answers").await;
    assert_eq!(status, StatusCode::OK, "defaults answer: {body}");
    assert_eq!(body["answers"]["within_secs"], 60, "{body}");
}
