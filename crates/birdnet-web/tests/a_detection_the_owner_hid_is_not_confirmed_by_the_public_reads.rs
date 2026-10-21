//! A detection the owner rejected is not on the public pages, and the public
//! JSON must not say otherwise.
//!
//! Two reads still did:
//!
//! * `GET /api/v2/weather/detection` looked the detection up in `detections`
//!   rather than `detections_analytic`, so a rejected detection answered `200`
//!   with the hour it was heard in while an absent one answered `404` — an
//!   oracle for "did the station once record this?".
//! * `GET /api/v2/dynamic-thresholds` listed every learned row, and a learned
//!   row is not purged when the detections it was learned from are rejected or
//!   deleted, so the list went on naming a species the public pages no longer
//!   show (its own header said "which its public detections already say").
//!
//! The counterparts keep the discrimination honest: a visible detection and a
//! species that still has one are answered as before, and the owner's own view
//! of the learned rows is not narrowed.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::dynamic_thresholds::LearnedThreshold;
use birdnet_db::weather::{WeatherRow, WeatherStore as _};
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

const HOUR_MS: i64 = 3_600_000;

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn learned(name: &str) -> LearnedThreshold {
    let now = now_ms();
    LearnedThreshold {
        sci_name: name.to_owned(),
        level: 2,
        confirmations: 3,
        expires_at_ms: now + 10 * HOUR_MS,
        first_learned_ms: now - 5 * HOUR_MS,
        last_confirmed_ms: now - HOUR_MS,
    }
}

fn insert_detection(state: &AppState, time: &str, sci: &str, verdict: Option<&str>) {
    let instant = birdnet_core::civil::unix_secs_from_civil(
        &birdnet_core::civil::parse_civil("2026-10-21", time).unwrap(),
    );
    state.with_db(|c| {
        c.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
             Sens, Overlap, File_Name, chunk_offset_secs, detected_at_utc, review_verdict)
             VALUES ('2026-10-21', ?1, ?2, 'Bird', 0.9, 0.7, 43, 1.25, 0.0, 'x.wav', 0, ?3, ?4)",
            rusqlite::params![time, sci, instant, verdict],
        )
        .unwrap();
    });
}

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).unwrap();
    state.with_db(|c| {
        c.upsert(&WeatherRow {
            at: "2026-10-21T06:00:00Z".to_owned(),
            temp_c: Some(8.5),
            precip_mm: None,
            wind_kt: None,
            wind_dir_deg: None,
            pressure_hpa: None,
            cloud_pct: None,
            code: None,
        })
        .unwrap();
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
async fn a_rejected_detection_answers_the_weather_route_like_an_absent_one() {
    let (_d, state) = station();
    insert_detection(&state, "06:30:12", "Pica pica", None);
    insert_detection(&state, "06:40:00", "Pica pica", Some("rejected"));

    let uri = |time: &str| {
        format!("/api/v2/weather/detection?date=2026-10-21&time={time}&sci_name=Pica%20pica")
    };
    let (status, v) = get(&state, &uri("06:30:12")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a visible detection still answers: {v}"
    );

    let (rejected, rejected_body) = get(&state, &uri("06:40:00")).await;
    let (absent, absent_body) = get(&state, &uri("07:00:00")).await;
    assert_eq!(
        rejected, absent,
        "a rejected detection answered differently from one that never existed"
    );
    assert_eq!(absent, StatusCode::NOT_FOUND);
    assert_eq!(
        rejected_body, absent_body,
        "the two refusals must be indistinguishable"
    );
}

#[tokio::test]
async fn a_learned_threshold_is_public_only_while_the_species_has_a_visible_detection() {
    let (_d, state) = station();
    state
        .with_db(|c| {
            birdnet_db::dynamic_thresholds::replace_all(
                c,
                &[
                    learned("Pica pica"),
                    learned("Strix aluco"),
                    learned("Turdus merula"),
                ],
            )
        })
        .unwrap();
    insert_detection(&state, "06:30:12", "Pica pica", None);
    // Every detection of the owl was rejected; the blackbird was never heard.
    insert_detection(&state, "06:31:00", "Strix aluco", Some("rejected"));

    let (_, list) = get(&state, "/api/v2/dynamic-thresholds").await;
    let names: Vec<&str> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["scientific_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Pica pica"], "{list}");
    assert_eq!(list["total"], 1);

    let (_, stats) = get(&state, "/api/v2/dynamic-thresholds/stats").await;
    assert_eq!(stats["total_count"], 1, "{stats}");

    let (status, _) = get(&state, "/api/v2/dynamic-thresholds/Strix%20aluco").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the rejected owl is named");
    let (status, _) = get(&state, "/api/v2/dynamic-thresholds/Pica%20pica").await;
    assert_eq!(status, StatusCode::OK, "the magpie is still public");

    // The owner's page and the reset read the table itself: nothing is
    // hidden from the person who can fix it, and a rejected detection that is
    // later restored brings its row back.
    let all = birdnet_web::routes::dynamic_thresholds::snapshot(&state, now_ms()).unwrap();
    assert_eq!(all.rows.len(), 3, "the owner's view lost rows");
    state.with_db(|c| {
        c.execute(
            "UPDATE detections SET review_verdict = NULL WHERE Sci_Name = 'Strix aluco'",
            [],
        )
        .unwrap();
    });
    let (status, _) = get(&state, "/api/v2/dynamic-thresholds/Strix%20aluco").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a restored detection restores the row"
    );
}
