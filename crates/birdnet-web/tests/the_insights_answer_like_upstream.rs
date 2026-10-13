//! `/api/v2/insights/*` and `/api/v2/dashboard/kpis` answer with birdnet-go's
//! field names, defaults and limits, and a station whose detections cannot be read
//! is told so rather than given an empty list.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

/// A station with detections placed relative to today.
fn station(rows: &[(i32, &str, &str)]) -> axum::Router {
    build_router(station_state(rows))
}

fn station_state(rows: &[(i32, &str, &str)]) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for (days_ago, time, sci) in rows {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (date('now', 'localtime', ?1), ?2, ?3, ?3 || ' common', 0.9)",
            rusqlite::params![format!("-{days_ago} days"), time, sci],
        )
        .expect("insert");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
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
async fn the_dawn_chorus_answers_with_upstreams_fields_and_limits() {
    let app = station(&[
        (1, "04:30:00", "Erithacus rubecula"),
        (2, "05:30:00", "Erithacus rubecula"),
        (1, "05:10:00", "Troglodytes troglodytes"),
        (2, "05:10:00", "Troglodytes troglodytes"),
    ]);
    let (status, body) = get(
        &app,
        "/api/v2/insights/dawn-chorus?period_days=7&min_days=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["period_days"], 7);
    assert_eq!(body["min_days"], 2);
    assert_eq!(body["start_hour"], 4);
    assert_eq!(body["end_hour"], 10);
    let first = &body["species"][0];
    assert_eq!(first["scientific_name"], "Erithacus rubecula", "{body}");
    assert_eq!(first["avg_first_detection"], "05:00");
    assert_eq!(first["earliest_detection"], "04:30");
    assert_eq!(first["days_observed"], 2);
    assert_eq!(
        first["thumbnail_url"],
        "/api/v2/species/image/Erithacus%20rubecula"
    );

    // The defaults, echoed: a client sees what was applied.
    let (_, body) = get(&app, "/api/v2/insights/dawn-chorus").await;
    assert_eq!(body["period_days"], 30);
    assert_eq!(body["min_days"], 3);

    // min_days is clamped to the period actually applied.
    let (_, body) = get(
        &app,
        "/api/v2/insights/dawn-chorus?period_days=2&min_days=9",
    )
    .await;
    assert_eq!(body["min_days"], 2);

    for bad in ["period_days=0", "period_days=week", "min_days=-1"] {
        let (status, _) = get(&app, &format!("/api/v2/insights/dawn-chorus?{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn migration_lists_arrivals_and_silences() {
    let mut rows = vec![(3, "07:00:00", "Fringilla montifringilla")];
    for d in 20..25 {
        rows.push((d, "07:00:00", "Sylvia atricapilla"));
    }
    let app = station(&rows);
    let (status, body) = get(&app, "/api/v2/insights/migration").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recent_days"], 14);
    assert_eq!(body["min_total_detections"], 5);
    assert_eq!(
        body["new_arrivals"][0]["scientific_name"], "Fringilla montifringilla",
        "{body}"
    );
    let quiet = &body["gone_quiet"][0];
    assert_eq!(quiet["scientific_name"], "Sylvia atricapilla", "{body}");
    assert_eq!(quiet["days_since"], 20);
    assert_eq!(quiet["total_detections"], 5);
}

/// Upstream's expected-today: this date in earlier years, not this year.
#[tokio::test]
async fn expected_today_is_this_date_in_earlier_years() {
    // 365 days ago is this date last year (or a day off across a leap day,
    // still inside the ±3-day window); today's own detection is not
    // "expected", it is here.
    let app = station(&[
        (365, "07:00:00", "Turdus iliacus"),
        (0, "07:00:00", "Sturnus vulgaris"),
    ]);
    let (status, body) = get(&app, "/api/v2/insights/expected-today").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["years_of_data"], 1, "{body}");
    assert_eq!(body["species"].as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(body["species"][0]["scientific_name"], "Turdus iliacus");
    assert_eq!(body["species"][0]["years_seen"], 1);
    assert!(
        body["day_of_year"]
            .as_i64()
            .is_some_and(|d| (1..=366).contains(&d))
    );
}

/// eBird's nearby reports, less what this station usually hears now. Without
/// a snapshot the answer says it is not available rather than "nothing".
#[tokio::test]
async fn the_regional_list_is_ebird_less_the_stations_regulars() {
    let state = station_state(&[(365, "07:00:00", "Turdus iliacus")]);
    let app = build_router(state.clone());
    let (status, body) = get(&app, "/api/v2/insights/expected-today/regional").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["available"], false, "{body}");

    let nearby = |sci: &str, com: &str| birdnet_integrations::ebird::NearbySpecies {
        sci_name: sci.to_owned(),
        com_name: com.to_owned(),
        last_seen: "2026-10-12".to_owned(),
        locality: "Marsh hide".to_owned(),
        how_many: None,
    };
    // Normalised and sorted, as the snapshot stores them.
    state.set_nearby(birdnet_integrations::ebird::Snapshot {
        fetched_at: 0,
        scope: "test".to_owned(),
        scope_label: "near the station".to_owned(),
        back_days: 14,
        species: vec![
            nearby("bombycilla garrulus", "Bohemian Waxwing"),
            nearby("turdus iliacus", "Redwing"),
        ],
    });
    let (status, body) = get(&app, "/api/v2/insights/expected-today/regional").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["available"], true);
    let names: Vec<&str> = body["species"]
        .as_array()
        .expect("species")
        .iter()
        .filter_map(|s| s["scientific_name"].as_str())
        .collect();
    assert_eq!(
        names,
        ["Bombycilla garrulus"],
        "the redwing is a regular here: {body}"
    );
    assert_eq!(body["species"][0]["location_name"], "Marsh hide");
}

/// Upstream's phantom-species fields and thresholds, echoed.
#[tokio::test]
async fn phantom_species_answer_with_upstreams_fields() {
    let state = station_state(&[]);
    state
        .with_db(|conn| {
            for (days_ago, conf) in [(0, 0.3), (1, 0.4), (2, 0.5)] {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                     VALUES (date('now', 'localtime', ?1), '03:00:00', 'Strix aluco',
                             'Tawny Owl', ?2)",
                    rusqlite::params![format!("-{days_ago} days"), conf],
                )?;
            }
            Ok::<_, rusqlite::Error>(())
        })
        .expect("insert");
    let app = build_router(state);
    let (status, body) = get(&app, "/api/v2/insights/phantom-species").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["period_days"], 30);
    assert_eq!(body["min_detections"], 3);
    assert_eq!(body["confidence_threshold"], 0.6);
    let owl = &body["species"][0];
    assert_eq!(owl["scientific_name"], "Strix aluco", "{body}");
    assert_eq!(owl["common_name"], "Tawny Owl");
    assert_eq!(owl["detection_count"], 3);
    assert_eq!(owl["max_confidence"], 0.5);
    assert!(
        owl["avg_confidence"]
            .as_f64()
            .is_some_and(|m| (m - 0.4).abs() < 1e-9),
        "{body}"
    );
}

/// Upstream's KPI shape, a streak counted back from today, and its empty
/// values on a station with nothing yet.
#[tokio::test]
async fn the_dashboard_kpis_answer_with_upstreams_shape() {
    let app = station(&[
        (0, "07:00:00", "Turdus merula"),
        (0, "08:00:00", "Turdus merula"),
        (1, "07:00:00", "Erithacus rubecula"),
        (3, "07:00:00", "Parus major"),
    ]);
    let (status, body) = get(&app, "/api/v2/dashboard/kpis").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["lifetime_species"], 3, "{body}");
    assert_eq!(body["today_detections"], 2);
    assert_eq!(body["best_day"]["count"], 2);
    assert_eq!(body["detection_streak"]["days"], 2, "{body}");
    // The busiest day is today, and the streak began the day before it.
    let best = body["best_day"]["date"].as_str().expect("date");
    let start = body["detection_streak"]["start_date"]
        .as_str()
        .expect("date");
    assert_eq!((best.len(), start.len()), (10, 10), "{body}");
    assert!(start < best, "{body}");

    let (status, body) = get(&station(&[]), "/api/v2/dashboard/kpis").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["best_day"],
        serde_json::json!({ "date": "", "count": 0 })
    );
    assert_eq!(
        body["detection_streak"],
        serde_json::json!({ "days": 0, "start_date": "" })
    );
}

/// The counterpart to all of them: an empty list would read as "nothing arrived",
/// "nobody sang" — a claim about the station's records made on a read that
/// failed.
#[tokio::test]
async fn an_unreadable_station_is_an_error_not_an_empty_list() {
    let state = AppState::from_connection(
        rusqlite::Connection::open_in_memory().expect("open"),
        std::path::PathBuf::from(":memory:"),
    );
    // With a snapshot, so the regional list reaches its read of the
    // station's own history: unfiltered, it would pass the station's
    // regulars off as birds it does not hear.
    state.set_nearby(birdnet_integrations::ebird::Snapshot {
        fetched_at: 0,
        scope: "test".to_owned(),
        scope_label: "near the station".to_owned(),
        back_days: 14,
        species: Vec::new(),
    });
    let app = build_router(state);
    for uri in [
        "/api/v2/insights/dawn-chorus",
        "/api/v2/insights/migration",
        "/api/v2/insights/phantom-species",
        "/api/v2/insights/expected-today",
        "/api/v2/insights/expected-today/regional",
        "/api/v2/dashboard/kpis",
    ] {
        let (status, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}: {body}");
    }
}
