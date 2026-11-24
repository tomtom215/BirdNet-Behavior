//! RES-02. A species' detection history marks an occasion the station did not
//! listen to as `NA`, never `0`: a day without effort is not a day without
//! birds, and an occupancy model told otherwise reads the outage as absence.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
}

#[tokio::test]
async fn hours_the_source_was_down_are_na_and_listened_hours_without_the_bird_are_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        for (hour, analysed) in [(4, 3600.0), (5, 0.0), (6, 0.0), (7, 3600.0)] {
            conn.execute(
                "INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up)
                 VALUES ('2026-05-01', ?1, 'local', ?2, ?2)",
                rusqlite::params![hour, analysed],
            )
            .expect("effort");
        }
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs, Source)
             VALUES ('2026-05-01', '04:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7,
                     18, 1.25, 0.0, 'a.wav', 0, 'local')",
            [],
        )
        .expect("detection");
    });
    let (status, csv) = get(
        &state,
        "/api/v2/detections/export/history?sci_name=Turdus%20merula&from=2026-05-01&to=2026-05-01&grain=hour",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{csv}");
    let row = |hour: u8| {
        csv.lines()
            .find(|l| l.starts_with(&format!("2026-05-01,{hour},")))
            .unwrap_or_else(|| panic!("no row for hour {hour}: {csv}"))
            .rsplit(',')
            .next()
            .expect("a last column")
            .to_owned()
    };
    assert_eq!(row(4), "1");
    assert_eq!(row(5), "NA");
    assert_eq!(row(6), "NA");
    assert_eq!(row(7), "0");

    let (status, csv) = get(
        &state,
        "/api/v2/effort/export?from=2026-05-01&to=2026-05-01",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{csv}");
    assert!(csv.contains("2026-05-01,7,local,3600.0,3600.0"), "{csv}");
}
