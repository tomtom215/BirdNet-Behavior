//! The weather JSON API (`P-21`): what the poll stored, as it stored it.
//!
//! The values here are the ones a provider can leave out, so the gates that
//! matter are the ones about absence: a missing temperature is `null`, not `0`;
//! an empty day is a `200` with a count of zero, and a table that cannot be read
//! is a `500`, so the two are never confused; and nothing in an answer says
//! where the station is.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::weather::{WeatherRow, WeatherStore as _};
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

fn row(at: &str, temp: Option<f32>) -> WeatherRow {
    WeatherRow {
        at: at.to_owned(),
        temp_c: temp,
        precip_mm: Some(0.4),
        wind_kt: Some(7.5),
        wind_dir_deg: Some(250),
        pressure_hpa: Some(1013.2),
        cloud_pct: None,
        code: Some(3),
    }
}

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).unwrap();
    state.with_db(|c| {
        // The two shapes providers write the stamp in.
        c.upsert(&row("2026-10-21T05:00", Some(8.5))).unwrap();
        c.upsert(&row("2026-10-21T06:00:00Z", None)).unwrap();
        c.upsert(&row("2026-10-21T23:00:00Z", Some(11.0))).unwrap();
        c.upsert(&row("2026-10-22T00:00:00Z", Some(10.0))).unwrap();
    });
    (dir, state)
}

async fn get(state: &AppState, path: &str) -> (StatusCode, Value) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn the_latest_hour_is_served_with_units_in_the_names() {
    let (_d, state) = station();
    let (status, v) = get(&state, "/api/v2/weather/latest").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["at"], "2026-10-22T00:00:00Z");
    assert_eq!(v["temperature_c"], 10.0);
    assert_eq!(v["wind_speed_kt"], 7.5);
    assert_eq!(v["wind_direction_deg"], 250);
    assert!((v["pressure_hpa"].as_f64().unwrap() - 1013.2).abs() < 0.01);
    assert_eq!(v["weather_code"], 3);
}

#[tokio::test]
async fn a_value_the_provider_left_out_is_null_never_zero() {
    let (_d, state) = station();
    let (_, v) = get(&state, "/api/v2/weather/hourly/2026-10-21/6").await;
    assert!(
        v["temperature_c"].is_null(),
        "no temperature became {}",
        v["temperature_c"]
    );
    assert!(
        v["cloud_cover_pct"].is_null(),
        "no cloud cover became {}",
        v["cloud_cover_pct"]
    );
    assert_eq!(
        v["precipitation_mm"].as_f64().map(|p| (p * 10.0).round()),
        Some(4.0)
    );
}

#[tokio::test]
async fn a_day_is_its_utc_hours_in_order_and_an_hour_matches_either_stamp_shape() {
    let (_d, state) = station();
    let (status, v) = get(&state, "/api/v2/weather/hourly/2026-10-21").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        v["count"], 3,
        "the 22nd's midnight belongs to the 22nd: {v}"
    );
    assert_eq!(v["timezone"], "UTC");
    let ats: Vec<&str> = v["hours"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["at"].as_str().unwrap())
        .collect();
    assert_eq!(
        ats,
        [
            "2026-10-21T05:00:00Z",
            "2026-10-21T06:00:00Z",
            "2026-10-21T23:00:00Z"
        ]
    );
    for hour in ["5", "05", "6", "23"] {
        let (status, h) = get(&state, &format!("/api/v2/weather/hourly/2026-10-21/{hour}")).await;
        assert_eq!(status, StatusCode::OK, "hour {hour}: {h}");
    }
    let (status, _) = get(&state, "/api/v2/weather/hourly/2026-10-21/7").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_empty_day_is_an_empty_list_and_a_broken_table_is_an_error() {
    let (_d, state) = station();
    let (status, v) = get(&state, "/api/v2/weather/hourly/2026-01-01").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["count"], 0);
    assert_eq!(v["hours"].as_array().unwrap().len(), 0);

    state
        .with_db(|c| c.execute_batch("DROP TABLE weather"))
        .unwrap();
    for path in [
        "/api/v2/weather/latest",
        "/api/v2/weather/hourly/2026-10-21",
        "/api/v2/weather/hourly/2026-10-21/5",
    ] {
        let (status, v) = get(&state, path).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}: {v}");
        assert!(
            !v.to_string().contains("weather"),
            "{path} named the table: {v}"
        );
    }
}

#[tokio::test]
async fn a_station_with_no_weather_says_how_it_gets_some() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).unwrap();
    let (status, v) = get(&state, "/api/v2/weather/latest").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        v["error"].as_str().unwrap().contains("BNB_WEATHER_ENABLED"),
        "{v}"
    );
}

#[tokio::test]
async fn bad_dates_and_hours_are_refused_not_looked_up() {
    let (_d, state) = station();
    for path in [
        "/api/v2/weather/hourly/2026-13-01",
        "/api/v2/weather/hourly/2026-02-30",
        "/api/v2/weather/hourly/yesterday",
        "/api/v2/weather/hourly/2026-10-21/24",
        "/api/v2/weather/hourly/2026-10-21/-1",
        "/api/v2/weather/hourly/2026-10-21/noon",
    ] {
        let (status, v) = get(&state, path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {v}");
    }
}

#[tokio::test]
async fn the_weather_when_a_detection_was_heard_is_found_by_its_key() {
    let (_d, state) = station();
    // 06:30:12 UTC. `detected_at_utc` is what the daemon stamps.
    let instant = birdnet_core::civil::unix_secs_from_civil(
        &birdnet_core::civil::parse_civil("2026-10-21", "06:30:12").unwrap(),
    );
    state.with_db(|c| {
        c.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap, File_Name, chunk_offset_secs, detected_at_utc)
             VALUES ('2026-10-21','08:30:12','Pica pica','Eurasian Magpie',0.9,0.7,43,1.25,0.0,'x.wav',0, ?1)",
            [instant],
        )
        .unwrap();
    });
    let (status, v) = get(
        &state,
        "/api/v2/weather/detection?date=2026-10-21&time=08:30:12&sci_name=Pica%20pica",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["heard_at"], "2026-10-21T06:30:12Z");
    assert_eq!(
        v["weather"]["at"], "2026-10-21T06:00:00Z",
        "the hour it fell in: {v}"
    );

    let (status, _) = get(
        &state,
        "/api/v2/weather/detection?date=2026-10-21&time=09:00:00&sci_name=Pica%20pica",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no such detection");
    let (status, _) = get(&state, "/api/v2/weather/detection?date=2026-10-21").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn no_answer_says_where_the_station_is() {
    let (_d, state) = station();
    for path in [
        "/api/v2/weather/latest",
        "/api/v2/weather/hourly/2026-10-21",
        "/api/v2/weather/hourly/2026-10-21/5",
    ] {
        let (_, v) = get(&state, path).await;
        let text = v.to_string().to_lowercase();
        for leak in ["lat", "lon", "coordinates", "station_id", "provider"] {
            assert!(!text.contains(leak), "{path} mentions {leak:?}: {text}");
        }
    }
}
