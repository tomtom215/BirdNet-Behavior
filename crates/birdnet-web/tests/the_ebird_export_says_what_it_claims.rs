//! The eBird checklist can be narrowed to what a person confirmed, and every
//! record says it is an automated acoustic detection (RES-27).
//!
//! The export claims presence in a public database. It read every detection
//! the station had not rejected, above a confidence floor, with no way to ask
//! for only the ones a reviewer had confirmed; and its comment said
//! "BirdNET: 3 detections this hour", which does not tell an eBird reviewer
//! that no person saw or heard the bird, nor which model decided it.
//!
//! The default (every detection not rejected) is unchanged: whether eBird
//! accepts unreviewed recorder detections is not settled here.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A confirmed Blackbird made by a recorded run, and an unreviewed Robin from
/// imported history (no run), both above the floor.
fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO analysis_runs
            (id, app_version, model_name, model_path, model_sha256, model_bytes, labels_path,
             labels_sha256, label_count, confidence, sensitivity, overlap, sf_thresh)
         VALUES (7, 'test', 'BirdNET_GLOBAL_6K_V2.4_Model', '/m', ?1, 1, '/l', ?1, 1,
                 0.7, 1.0, 0.0, 0.03)",
        [SHA],
    )
    .expect("run");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, review_verdict, run_id)
         VALUES ('2026-03-12', '06:05:00', 'Turdus merula', 'Eurasian Blackbird', 0.9,
                 'confirmed', 7),
                ('2026-03-12', '06:10:00', 'Erithacus rubecula', 'European Robin', 0.9,
                 NULL, NULL);",
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn export(query: &str) -> (StatusCode, String) {
    let resp = build_router(station())
        .oneshot(
            Request::builder()
                .uri(format!("/api/v2/detections/export/ebird{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// `verdict=confirmed` claims only what a reviewer confirmed.
#[tokio::test]
async fn verdict_confirmed_claims_only_confirmed_detections() {
    let (status, body) = export("?verdict=confirmed").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Eurasian Blackbird"), "{body}");
    assert!(
        !body.contains("European Robin"),
        "an unreviewed detection was claimed under verdict=confirmed: {body}"
    );
}

/// The counterpart: without the parameter the default is unchanged, and an
/// unreviewed detection is still exported.
#[tokio::test]
async fn without_the_parameter_unreviewed_detections_still_export() {
    for query in ["", "?verdict=any"] {
        let (status, body) = export(query).await;
        assert_eq!(status, StatusCode::OK, "{query}: {body}");
        assert!(body.contains("Eurasian Blackbird"), "{query}: {body}");
        assert!(body.contains("European Robin"), "{query}: {body}");
    }
}

/// A value the export does not know is refused, not read as "any".
#[tokio::test]
async fn an_unknown_verdict_is_refused() {
    let (status, _) = export("?verdict=rejected").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Every record's comment says it is an automated acoustic detection, names
/// the model the run recorded (or says none was recorded), and says how many
/// of its detections a person confirmed.
#[tokio::test]
async fn every_record_says_it_is_an_automated_acoustic_detection_and_by_what() {
    let (_, body) = export("").await;
    let line = |species: &str| {
        body.lines()
            .find(|l| l.starts_with(species))
            .unwrap_or_else(|| panic!("no record for {species}: {body}"))
            .to_owned()
    };
    let blackbird = line("Eurasian Blackbird");
    assert!(
        blackbird.contains("Automated acoustic detection"),
        "{blackbird}"
    );
    assert!(
        blackbird.contains("BirdNET_GLOBAL_6K_V2.4_Model (sha256 0123456789ab)"),
        "the model is named from its run: {blackbird}"
    );
    assert!(
        blackbird.contains("1 confirmed by a reviewer"),
        "{blackbird}"
    );
    let robin = line("European Robin");
    assert!(robin.contains("Automated acoustic detection"), "{robin}");
    assert!(
        robin.contains("a model this station did not record"),
        "a row with no run is not given a model: {robin}"
    );
    assert!(robin.contains("0 confirmed by a reviewer"), "{robin}");
}
