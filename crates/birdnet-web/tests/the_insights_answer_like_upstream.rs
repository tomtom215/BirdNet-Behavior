//! `/api/v2/insights/*`, `/api/v2/dashboard/kpis` and the `/api/v2/analytics/*`
//! routes shared with birdnet-go answer with its field names, defaults and
//! limits, and a station whose detections cannot be read is told so rather than
//! given an empty list.

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

/// Upstream's species summary: a bare array, the most detected first, with
/// an RFC 3339 first and last heard; a malformed or reversed range is a 400,
/// and a `limit` that is not a number is ignored, as upstream ignores it.
#[tokio::test]
async fn the_species_summary_answers_with_upstreams_shape() {
    let app = station(&[
        (1, "07:00:00", "Turdus merula"),
        (2, "06:00:00", "Turdus merula"),
        (40, "07:00:00", "Turdus merula"),
        (1, "08:00:00", "Parus major"),
    ]);
    let (status, body) = get(&app, "/api/v2/analytics/species/summary").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body.as_array().expect("a bare array");
    assert_eq!(rows.len(), 2, "{body}");
    assert_eq!(rows[0]["scientific_name"], "Turdus merula");
    assert_eq!(rows[0]["count"], 3);
    assert_eq!(rows[0]["max_confidence"], 0.9);
    let first = rows[0]["first_heard"].as_str().expect("first_heard");
    let last = rows[0]["last_heard"].as_str().expect("last_heard");
    // `YYYY-MM-DDTHH:MM:SS+HH:MM`, and the 40-days-ago row first.
    assert_eq!((first.len(), &first[10..11]), (25, "T"), "{first}");
    assert!(first < last, "{first} {last}");
    assert!(last.contains("T07:00:00"), "{last}");

    // A range: the last week holds two of the three.
    let since = &last[..10];
    let (_, body) = get(
        &app,
        &format!(
            "/api/v2/analytics/species/summary?start_date=2000-01-01&end_date={since}&limit=1"
        ),
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(1), "limit=1: {body}");
    assert_eq!(body[0]["count"], 3);
    let (_, body) = get(
        &app,
        &format!("/api/v2/analytics/species/summary?start_date={since}&limit=many"),
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(2), "{body}");
    assert_eq!(body[0]["count"], 1, "only yesterday's: {body}");

    for bad in [
        "start_date=2026-02-30",
        "end_date=yesterday",
        "start_date=2026-10-13&end_date=2026-10-12",
    ] {
        let (status, _) = get(&app, &format!("/api/v2/analytics/species/summary?{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// Upstream's new-species list: first heard in the range (by default the
/// last 30 days), newest first, as a bare array; bad paging is a 400.
#[tokio::test]
async fn new_species_are_those_first_heard_in_the_range() {
    let app = station(&[
        (3, "07:00:00", "Fringilla montifringilla"),
        (1, "07:00:00", "Fringilla montifringilla"),
        (10, "07:00:00", "Turdus iliacus"),
        // First heard 31 days ago: before the default range.
        (31, "07:00:00", "Turdus merula"),
        (2, "07:00:00", "Turdus merula"),
        // 30 days ago: the default range's first day.
        (30, "07:00:00", "Bombycilla garrulus"),
    ]);
    let (status, body) = get(&app, "/api/v2/analytics/species/detections/new").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body
        .as_array()
        .expect("a bare array")
        .iter()
        .filter_map(|r| r["scientific_name"].as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Fringilla montifringilla",
            "Turdus iliacus",
            "Bombycilla garrulus"
        ],
        "{body}"
    );
    assert_eq!(body[0]["count_in_period"], 2);
    let first = body[0]["first_heard_date"].as_str().expect("date");

    // A range ending before the brambling's first day: it is not new there.
    let (_, body) = get(
        &app,
        &format!("/api/v2/analytics/species/detections/new?start_date=2000-01-01&end_date={first}&limit=1&offset=0"),
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(body[0]["scientific_name"], "Fringilla montifringilla");
    assert_eq!(body[0]["count_in_period"], 1, "{body}");
    let (_, body) = get(
        &app,
        "/api/v2/analytics/species/detections/new?limit=0&offset=2",
    )
    .await;
    assert_eq!(
        body.as_array().map(Vec::len),
        Some(1),
        "limit=0 is the default: {body}"
    );

    for bad in [
        "limit=-1",
        "offset=x",
        "start_date=2026-02-30",
        "start_date=2026-10-13&end_date=2026-10-01",
    ] {
        let (status, _) = get(
            &app,
            &format!("/api/v2/analytics/species/detections/new?{bad}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
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
        "/api/v2/analytics/species/summary",
        "/api/v2/analytics/species/detections/new",
    ] {
        let (status, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}: {body}");
    }
}

/// Upstream's source list: `{"sources": [{"id", "name", "count"}]}`, the
/// busiest first, the owner's name where the source has one and the id where
/// it has not; a malformed or reversed range is a 400.
#[tokio::test]
async fn sources_are_listed_with_their_counts_and_names() {
    let state = station_state(&[]);
    state.with_db(|conn| {
        for (i, (days_ago, source)) in [(1, "cam1"), (2, "cam1"), (40, "cam1"), (1, "local")]
            .into_iter()
            .enumerate()
        {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Source)
                 VALUES (date('now', 'localtime', ?1), printf('07:%02d:00', ?3), 'Turdus merula', 'Blackbird', 0.9, ?2)",
                rusqlite::params![format!("-{days_ago} days"), source, i64::try_from(i).unwrap()],
            )
            .expect("insert");
        }
        conn.execute(
            "INSERT INTO audio_sources (id, kind, device_id, label)
             VALUES ('cam1', 'rtsp', 'rtsp://cam', 'Pond')",
            [],
        )
        .expect("insert source");
    });
    let app = build_router(state);
    let (status, body) = get(&app, "/api/v2/analytics/sources").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({"sources": [
            {"id": "cam1", "name": "Pond", "count": 3},
            {"id": "local", "name": "local", "count": 1},
        ]})
    );

    let (_, body) = get(&app, "/api/v2/analytics/sources?start_date=2000-01-01").await;
    assert_eq!(body["sources"][0]["count"], 3, "{body}");
    let (status, body) = get(&app, "/api/v2/analytics/sources?end_date=2000-01-01").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({"sources": []}), "never null");

    for bad in [
        "start_date=2026-02-30",
        "end_date=yesterday",
        "start_date=2026-10-13&end_date=2026-10-12",
    ] {
        let (status, _) = get(&app, &format!("/api/v2/analytics/sources?{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// Upstream's confidence distribution: camelCase, `start_date` required,
/// `bins` and `limit` defaulted and bounded as upstream bounds them.
#[tokio::test]
async fn the_confidence_distribution_answers_with_upstreams_shape() {
    let state = station_state(&[]);
    state.with_db(|conn| {
        for i in 0..25 {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-10-02', printf('06:%02d:00', ?1), 'Turdus merula', 'Blackbird', ?2)",
                rusqlite::params![i, if i < 5 { 0.97 } else { 0.51 }],
            )
            .expect("insert");
        }
        // Five more species, each less detected, for the limit: six qualify,
        // so the default of five is told apart from the maximum of ten.
        for (k, sci) in [
            "Erithacus rubecula",
            "Parus major",
            "Sitta europaea",
            "Pica pica",
            "Corvus corone",
        ]
        .into_iter()
        .enumerate()
        {
            for i in 0..(24 - k) {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                     VALUES ('2026-10-03', printf('06:%02d:00', ?1), ?2, ?2, 0.8)",
                    rusqlite::params![i64::try_from(i).unwrap(), sci],
                )
                .expect("insert");
            }
        }
    });
    let app = build_router(state);
    let (status, body) = get(
        &app,
        "/api/v2/analytics/confidence/distribution?start_date=2026-10-01",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = &body[0];
    assert_eq!(row["scientificName"], "Turdus merula", "{body}");
    assert_eq!(row["commonName"], "Blackbird");
    assert_eq!(row["total"], 25);
    let bins = row["bins"].as_array().expect("bins");
    assert_eq!(bins.len(), 20, "the default");
    assert_eq!(bins[19], 0.2);
    assert_eq!(bins[10], 0.8);

    for (query, expected) in [
        ("bins=3", 5),
        ("bins=500", 50),
        ("bins=many", 20),
        ("bins=7", 7),
    ] {
        let (_, body) = get(
            &app,
            &format!("/api/v2/analytics/confidence/distribution?start_date=2026-10-01&{query}"),
        )
        .await;
        assert_eq!(
            body[0]["bins"].as_array().map(Vec::len),
            Some(expected),
            "{query}"
        );
    }
    // end_date defaults to thirty days after start_date: the 2nd is inside a
    // range starting on 2 September, and outside one ending on 1 October.
    let (_, body) = get(
        &app,
        "/api/v2/analytics/confidence/distribution?start_date=2026-09-02",
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(1), "{body}");
    let (_, body) = get(
        &app,
        "/api/v2/analytics/confidence/distribution?start_date=2026-09-01",
    )
    .await;
    assert_eq!(body, serde_json::json!([]), "never null");
    let (_, body) = get(
        &app,
        "/api/v2/analytics/confidence/distribution?start_date=2026-10-01&species=blackbird",
    )
    .await;
    assert_eq!(body[0]["total"], 25, "by common name, in any case: {body}");
    // `limit`: upstream's ParsePaginationLimit, so 0, a non-number and
    // anything above 10 all mean the default of 5 — not the maximum.
    for (query, expected) in [
        ("limit=1", 1),
        ("limit=6", 6),
        ("limit=10", 6),
        ("limit=0", 5),
        ("limit=11", 5),
        ("limit=x", 5),
        ("limit=", 5),
    ] {
        let (_, body) = get(
            &app,
            &format!("/api/v2/analytics/confidence/distribution?start_date=2026-10-01&{query}"),
        )
        .await;
        assert_eq!(
            body.as_array().map(Vec::len),
            Some(expected),
            "{query}: {body}"
        );
    }

    for bad in [
        "",
        "start_date=2026-02-30",
        "start_date=2026-10-01&end_date=soon",
        "start_date=2026-10-13&end_date=2026-10-12",
    ] {
        let (status, _) = get(
            &app,
            &format!("/api/v2/analytics/confidence/distribution?{bad}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// Upstream's year-over-year tracker: `{currentYear, previousYear, points}`,
/// one cumulative point per day from 1 January to `date`, each year's
/// detections counted through the same calendar day.
#[tokio::test]
async fn year_over_year_answers_with_upstreams_shape() {
    let state = station_state(&[]);
    state.with_db(|conn| {
        for (date, time) in [
            ("2026-01-01", "06:00:00"),
            ("2026-01-03", "06:00:00"),
            ("2026-01-03", "07:00:00"),
            ("2025-01-02", "06:00:00"),
            ("2025-01-04", "06:00:00"),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, 'Turdus merula', 'Blackbird', 0.9)",
                rusqlite::params![date, time],
            )
            .expect("insert");
        }
    });
    let app = build_router(state);
    let (status, body) = get(
        &app,
        "/api/v2/analytics/time/year-over-year?date=2026-01-03",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({
            "currentYear": 2026,
            "previousYear": 2025,
            "points": [
                {"date": "2026-01-01", "monthDay": "01-01", "thisYear": 1, "lastYear": 0, "delta": 1},
                {"date": "2026-01-02", "monthDay": "01-02", "thisYear": 1, "lastYear": 1, "delta": 0},
                {"date": "2026-01-03", "monthDay": "01-03", "thisYear": 3, "lastYear": 1, "delta": 2},
            ],
        })
    );
    // No date is today, which is in the current year.
    let (status, body) = get(&app, "/api/v2/analytics/time/year-over-year").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let points = body["points"].as_array().expect("points");
    assert_eq!(
        points.last().and_then(|p| p["date"].as_str()),
        Some(local_today().as_str()),
        "{body}"
    );
    for bad in ["date=2026-02-30", "date=today", "date=2026-1-3"] {
        let (status, _) = get(
            &app,
            &format!("/api/v2/analytics/time/year-over-year?{bad}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// Today's local date, as SQLite's `localtime` names it — the same clock the
/// station's `Date` column is written in.
fn local_today() -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
        .unwrap()
}

/// Upstream's dawn onset: every day of the range, its detection count, and
/// its third detection's minute against civil dawn — null on a sparse day
/// and at a station with no location.
#[tokio::test]
async fn dawn_onset_answers_with_upstreams_shape() {
    let state = station_state(&[]);
    state.with_db(|conn| {
        // Two days whose third detection is at 04:30, the second with its
        // first two an hour earlier: strays before dawn that must not move
        // the onset.
        for (date, times) in [
            (
                "2026-06-01",
                ["04:10:00", "04:20:00", "04:30:00", "06:00:00", "07:00:00"],
            ),
            (
                "2026-06-04",
                ["03:10:00", "03:20:00", "04:30:00", "06:00:00", "07:00:00"],
            ),
        ] {
            for time in times {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                     VALUES (?1, ?2, 'Turdus merula', 'Blackbird', 0.9)",
                    [date, time],
                )
                .expect("insert");
            }
        }
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-06-03', '05:00:00', 'Turdus merula', 'Blackbird', 0.9)",
            [],
        )
        .expect("insert");
    });
    let app = build_router(state.clone());
    let uri = "/api/v2/analytics/time/dawn-onset?start_date=2026-06-01&end_date=2026-06-03";
    let (status, body) = get(&app, uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!([
            {"date": "2026-06-01", "onsetRelMinutes": null, "detectionCount": 5},
            {"date": "2026-06-02", "onsetRelMinutes": null, "detectionCount": 0},
            {"date": "2026-06-03", "onsetRelMinutes": null, "detectionCount": 1},
        ]),
        "no location, so no dawn to measure against"
    );

    // With a location the five-detection day has an onset. Its value depends
    // on the zone the test runs in (civil dawn is placed in local time), so
    // only its presence is asserted here; the arithmetic is unit-tested in
    // routes/charts.rs.
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "latitude",
            "51.5",
            birdnet_db::settings::SettingsCategory::Location,
        )
        .expect("lat");
        birdnet_db::settings::set(
            conn,
            "longitude",
            "-0.13",
            birdnet_db::settings::SettingsCategory::Location,
        )
        .expect("lon");
    });
    let (_, body) = get(&app, uri).await;
    let rel = body[0]["onsetRelMinutes"].as_i64();
    assert!(rel.is_some(), "{body}");
    assert_eq!(
        body[2]["onsetRelMinutes"],
        serde_json::Value::Null,
        "sparse"
    );
    // The third detection, not the first: civil dawn moves about a minute a
    // day in June, so the two days' onsets agree to within a few minutes;
    // reading the first detection would put them an hour apart.
    let (_, both) = get(
        &app,
        "/api/v2/analytics/time/dawn-onset?start_date=2026-06-01&end_date=2026-06-04",
    )
    .await;
    let (a, b) = (
        both[0]["onsetRelMinutes"].as_i64().expect("day 1"),
        both[3]["onsetRelMinutes"].as_i64().expect("day 4"),
    );
    assert!((a - b).abs() <= 5, "{a} {b}: {both}");
    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/dawn-onset?start_date=2026-06-01&end_date=2026-06-01&species=Parus%20major",
    )
    .await;
    assert_eq!(
        body,
        serde_json::json!([{"date": "2026-06-01", "onsetRelMinutes": null, "detectionCount": 0}]),
        "another species heard nothing"
    );
    let (status, _) = get(&app, "/api/v2/analytics/time/dawn-onset").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "start_date is required");
    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/dawn-onset?start_date=2026-06-01",
    )
    .await;
    assert_eq!(
        body.as_array().map(Vec::len),
        Some(31),
        "thirty days after, inclusive"
    );
}

/// Upstream's heatmap: every day of the range in `dates`, the slot width
/// for the range, and only the occupied cells, in three parallel arrays;
/// `format=csv` lists the same cells.
#[tokio::test]
async fn the_heatmap_answers_with_upstreams_shape() {
    let state = station_state(&[]);
    state.with_db(|conn| {
        for (date, time) in [
            ("2026-05-01", "06:05:00"),
            ("2026-05-01", "06:10:00"),
            ("2026-05-03", "18:45:00"),
            ("2026-08-15", "06:05:00"),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, 'Turdus merula', 'Blackbird', 0.9)",
                rusqlite::params![date, time],
            )
            .expect("insert");
        }
    });
    let app = build_router(state);
    let (status, body) = get(
        &app,
        "/api/v2/analytics/time/heatmap?start_date=2026-05-01&end_date=2026-05-03",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!({
            "dates": ["2026-05-01", "2026-05-02", "2026-05-03"],
            "slotResolutionMinutes": 15,
            "cells": {"dateIndex": [0, 2], "slot": [24, 75], "count": [2, 1]},
        })
    );
    // 107 days: half-hour slots.
    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/heatmap?start_date=2026-05-01&end_date=2026-08-15",
    )
    .await;
    assert_eq!(body["slotResolutionMinutes"], 30);
    assert_eq!(body["dates"].as_array().map(Vec::len), Some(107));
    assert_eq!(body["cells"]["dateIndex"], serde_json::json!([0, 2, 106]));
    assert_eq!(body["cells"]["slot"], serde_json::json!([12, 37, 12]));

    let res = app
        .clone()
        .oneshot(
            Request::get(
                "/api/v2/analytics/time/heatmap?start_date=2026-05-01&end_date=2026-05-03&format=CSV",
            )
            .header("host", "localhost")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "text/csv; charset=utf-8");
    assert!(
        res.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("activity-heatmap.csv")
    );
    let csv = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&csv).unwrap(),
        "date,slot,slot_start,count\n2026-05-01,24,06:00,2\n2026-05-03,75,18:45,1\n"
    );

    // Both ends count: 1 May to 30 July is 91 days, one past the 15-minute
    // limit; and the CSV's slot start follows the range's slot width.
    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/heatmap?start_date=2026-05-01&end_date=2026-07-30",
    )
    .await;
    assert_eq!(body["slotResolutionMinutes"], 30, "91 days");
    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/heatmap?start_date=2026-05-01&end_date=2026-07-29",
    )
    .await;
    assert_eq!(body["slotResolutionMinutes"], 15, "90 days");
    let res = app
        .clone()
        .oneshot(
            Request::get(
                "/api/v2/analytics/time/heatmap?start_date=2026-05-01&end_date=2026-08-15&format=csv",
            )
            .header("host", "localhost")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let csv = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&csv).unwrap(),
        "date,slot,slot_start,count\n2026-05-01,12,06:00,2\n2026-05-03,37,18:30,1\n2026-08-15,12,06:00,1\n"
    );

    let (_, body) = get(&app, "/api/v2/analytics/time/heatmap?start_date=2026-05-01").await;
    assert_eq!(body["dates"].as_array().map(Vec::len), Some(31));
    let (status, _) = get(&app, "/api/v2/analytics/time/heatmap?end_date=2026-05-01").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Upstream's hour-of-day charts: the ridgeline's buckets are fractions of
/// each species' total, the succession's counts are counts, and a repeated
/// `species` picks the species, eleven being too many.
#[tokio::test]
async fn the_hour_of_day_charts_answer_with_upstreams_shape() {
    let state = station_state(&[]);
    state.with_db(|conn| {
        for (sci, com, time) in [
            ("Turdus merula", "Blackbird", "05:10:00"),
            ("Turdus merula", "Blackbird", "05:20:00"),
            ("Turdus merula", "Blackbird", "05:30:00"),
            ("Turdus merula", "Blackbird", "20:00:00"),
            ("Strix aluco", "Tawny Owl", "23:00:00"),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-05-01', ?1, ?2, ?3, 0.9)",
                rusqlite::params![time, sci, com],
            )
            .expect("insert");
        }
    });
    let app = build_router(state);
    let (status, body) = get(
        &app,
        "/api/v2/analytics/time/distribution/species?start_date=2026-05-01",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[0]["scientificName"], "Turdus merula", "{body}");
    assert_eq!(body[0]["commonName"], "Blackbird");
    assert_eq!(body[0]["total"], 4);
    let buckets = body[0]["buckets"].as_array().expect("buckets");
    assert_eq!(buckets.len(), 24);
    assert_eq!(buckets[5], 0.75);
    assert_eq!(buckets[20], 0.25);
    assert_eq!(body[1]["buckets"][23], 1.0);

    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/succession?start_date=2026-05-01&species=tawny%20owl",
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(body[0]["scientificName"], "Strix aluco");
    assert_eq!(body[0]["counts"][23], 1);
    assert_eq!(body[0]["counts"].as_array().map(Vec::len), Some(24));
    let (_, body) = get(
        &app,
        "/api/v2/analytics/time/succession?start_date=2026-05-01&limit=1",
    )
    .await;
    assert_eq!(body.as_array().map(Vec::len), Some(1));
    assert_eq!(body[0]["counts"][5], 3);

    let eleven = (0..11)
        .map(|i| format!("&species=S{i}"))
        .collect::<Vec<_>>()
        .concat();
    for path in ["distribution/species", "succession"] {
        let (status, body) = get(
            &app,
            &format!("/api/v2/analytics/time/{path}?start_date=2026-05-01{eleven}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(body["error"], "Too many items requested. Maximum: 10");
        let (status, _) = get(&app, &format!("/api/v2/analytics/time/{path}")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path}: start_date is required"
        );
    }
}
