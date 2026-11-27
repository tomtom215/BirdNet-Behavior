//! `/api/v2/events`, `/api/v2/events/daily` and `/api/v2/effort/sites`
//! (decisions 1 and 2 of 2026-11-25): a count of events states the policy, the
//! tolerance and the site map it used and how many rows it left out, a failed
//! read is an error and never a count of zero, and the one listened-day rule
//! `birdnet-db` names is the rule the trend queries apply.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

/// 2026-05-01 00:00:00Z.
const DAY: i64 = 1_777_593_600;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

fn add(state: &AppState, source: &str, time: &str, utc: Option<i64>) {
    let clip = format!("2026-05-01-birdnet-{source}-{time}.wav");
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs,
                  Source, detected_at_utc)
             VALUES ('2026-05-01', ?1, 'Strix aluco', 'Tawny Owl', 0.9, ?2, 0, ?3, ?4)",
            rusqlite::params![time, clip, source, utc],
        )
        .expect("insert detection");
        if utc.is_none() {
            conn.execute(
                "UPDATE detections SET detected_at_utc = NULL WHERE File_Name = ?1",
                [&clip],
            )
            .expect("clear the instant");
        }
    });
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, serde_json::from_slice(&bytes).expect("json"))
}

fn seed(state: &AppState) {
    // Two sources four seconds apart; a third row on one of them a minute on;
    // one row with no instant.
    add(state, "mic", "06:00:00", Some(DAY + 6 * 3600));
    add(state, "cam", "06:00:04", Some(DAY + 6 * 3600 + 4));
    add(state, "mic", "06:01:00", Some(DAY + 6 * 3600 + 60));
    add(state, "mic", "06:30:00", None);
}

#[tokio::test]
async fn events_state_their_policy_and_count_once_only_when_told_the_sites() {
    let (_dir, state) = station();
    seed(&state);

    let (status, body) = get(&state, "/api/v2/events?from=2026-05-01&to=2026-05-01").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["policy"]["version"], "events-2");
    assert_eq!(body["policy"]["tolerance_secs"], 4.0);
    assert_eq!(body["policy"]["site_map"]["mic"], "mic");
    assert_eq!(body["policy"]["site_map"]["cam"], "cam");
    assert_eq!(body["rows_without_instant"], 1);
    // Every source its own site: three rows, three events.
    assert_eq!(body["total_events"], 3, "{body}");

    let (_, body) = get(
        &state,
        "/api/v2/events?from=2026-05-01&to=2026-05-01&sites=mic:yard,cam:yard",
    )
    .await;
    assert_eq!(body["policy"]["site_map"]["cam"], "yard");
    assert_eq!(body["total_events"], 2, "{body}");
    assert_eq!(body["sites"][0]["site"], "yard");
    assert_eq!(body["sites"][0]["events"], 2);
    assert_eq!(body["sites"][0]["rows"], 3);
    assert_eq!(
        body["events"][0]["members"].as_array().map(Vec::len),
        Some(2)
    );

    // A tolerance below the four seconds between them separates them.
    let (_, body) = get(
        &state,
        "/api/v2/events?from=2026-05-01&to=2026-05-01&sites=mic:yard,cam:yard&tolerance_secs=3",
    )
    .await;
    assert_eq!(body["policy"]["tolerance_secs"], 3.0);
    assert_eq!(body["total_events"], 3, "{body}");

    let (status, body) = get(
        &state,
        "/api/v2/events/daily?from=2026-05-01&to=2026-05-01&sites=mic:yard,cam:yard",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["days"][0]["date"], "2026-05-01");
    assert_eq!(body["days"][0]["events"], 2);
    assert_eq!(body["days"][0]["rows"], 3);
    assert_eq!(body["rows_without_instant"], 1);
    assert!(
        !body.to_string().to_lowercase().contains("birds\""),
        "a count of events is not a count of birds: {body}"
    );
}

#[tokio::test]
async fn bad_parameters_are_refused() {
    let (_dir, state) = station();
    for uri in [
        "/api/v2/events?from=2026-05-01",
        "/api/v2/events?from=2026-05-01&to=2026-05-09",
        "/api/v2/events?from=2026-05-01&to=2026-05-01&tolerance_secs=-1",
        "/api/v2/events?from=2026-05-01&to=2026-05-01&sites=mic:a,mic:b",
        "/api/v2/events/daily?from=2026-05-01&to=2026-06-15",
        "/api/v2/effort/sites?from=2026-05-01&to=2026-04-01",
    ] {
        let (status, body) = get(&state, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
    }
}

#[tokio::test]
async fn a_failed_read_is_an_error_not_zero_events() {
    let (_dir, state) = station();
    seed(&state);
    state.with_db(|conn| {
        conn.execute_batch("DROP VIEW detections_analytic; DROP TABLE recording_effort_hourly;")
            .expect("break the reads");
    });
    for uri in [
        "/api/v2/events?from=2026-05-01&to=2026-05-01",
        "/api/v2/events/daily?from=2026-05-01&to=2026-05-01",
        "/api/v2/effort/sites?from=2026-05-01&to=2026-05-01",
    ] {
        let (status, body) = get(&state, uri).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}: {body}");
        assert!(body.get("total_events").is_none() && body.get("hours").is_none());
    }
}

#[tokio::test]
async fn a_site_listened_between_bounds_and_sensor_time_is_apart() {
    let (_dir, state) = station();
    state.with_db(|conn| {
        for (source, secs) in [("mic", 1800.0), ("cam", 2400.0)] {
            conn.execute(
                "INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up)
                 VALUES ('2026-01-10', 6, ?1, ?2, 3600)",
                rusqlite::params![source, secs],
            )
            .expect("seed");
        }
    });
    let (status, body) = get(
        &state,
        "/api/v2/effort/sites?from=2026-01-10&to=2026-01-10&sites=mic:yard,cam:yard",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let h = &body["hours"][0];
    assert_eq!(h["site"], "yard");
    assert_eq!(h["listened_secs_lower_bound"], 2400.0);
    assert_eq!(h["listened_secs_upper_bound"], 3600.0);
    assert_eq!(h["sensor_secs"], 4200.0);
    assert_eq!(body["site_map"]["mic"], "yard");
    assert_eq!(body["listened_day_rule"], "half-of-typical-day");
}

#[test]
fn the_named_listened_day_rule_is_the_trend_queries_rule() {
    use birdnet_db::sqlite::queries::site_effort::{LISTENED_DAY_SHARE, is_listened_day};
    use birdnet_timeseries::queries::trend;
    assert!((LISTENED_DAY_SHARE - trend::EFFORT_FLOOR_SHARE).abs() < f64::EPSILON);
    for typical in [None, Some(3600.0), Some(7200.0), Some(86_400.0)] {
        for seconds in [
            0.0, 60.0, 1799.0, 1800.0, 3600.0, 43_199.0, 43_200.0, 86_400.0,
        ] {
            assert_eq!(
                is_listened_day(seconds, typical),
                trend::is_listened_day(seconds, typical),
                "{seconds} s against a typical day of {typical:?}"
            );
        }
    }
}
