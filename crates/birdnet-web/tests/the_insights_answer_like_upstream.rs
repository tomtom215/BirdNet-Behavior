//! `/api/v2/insights/{dawn-chorus,migration}` answer with birdnet-go's field
//! names, defaults and limits, and a station whose detections cannot be read
//! is told so rather than given an empty list.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

/// A station with detections placed relative to today.
fn station(rows: &[(i32, &str, &str)]) -> axum::Router {
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
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
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

/// The counterpart to both: an empty list would read as "nothing arrived",
/// "nobody sang" — a claim about the station's records made on a read that
/// failed.
#[tokio::test]
async fn an_unreadable_station_is_an_error_not_an_empty_list() {
    let app = build_router(AppState::from_connection(
        rusqlite::Connection::open_in_memory().expect("open"),
        std::path::PathBuf::from(":memory:"),
    ));
    for uri in ["/api/v2/insights/dawn-chorus", "/api/v2/insights/migration"] {
        let (status, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{uri}: {body}");
    }
}
