//! An export names the model file of the classifier that made each row, not
//! the run's primary (review F19, MMP-08).
//!
//! A run is one daemon start, and `analysis_runs` records its primary
//! classifier's file. On a station running two classifiers, a row whose answer
//! came from the second was exported with the primary's name and SHA-256: a
//! checksum of a file that did not make it. Each test goes through the real
//! router.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PRIMARY_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

fn register_run(state: &AppState) -> i64 {
    let new = birdnet_db::sqlite::NewAnalysisRun {
        app_version: "0.0.0-test",
        model_name: "BirdNET_GLOBAL_6K_V2.4_Model_FP32",
        model_path: "/models/birdnet.onnx",
        model_sha256: PRIMARY_SHA,
        model_bytes: 4096,
        labels_path: "/models/labels.txt",
        labels_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
        label_count: 3,
        geomodel_sha256: None,
        confidence: 0.25,
        sensitivity: 1.25,
        overlap: 0.0,
        sf_thresh: 0.03,
        lat: None,
        lon: None,
    };
    state
        .with_db(|conn| birdnet_db::sqlite::insert_analysis_run(conn, &new))
        .expect("run")
}

fn add_row(state: &AppState, time: &str, sci: &str, run: i64, model: &str) {
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs,
                  run_id, model_id, model_agreement)
             VALUES ('2026-05-01', ?1, ?2, ?2, 0.9, ?2 || '.wav', 0, ?3, ?4, 1)",
            rusqlite::params![time, sci, run, model],
        )
        .expect("insert detection");
    });
}

async fn csv(state: &AppState) -> Vec<Vec<String>> {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(body.to_vec())
        .unwrap()
        .lines()
        .map(|l| l.split(',').map(str::to_owned).collect())
        .collect()
}

/// The `Model_SHA256` cell of the row for `sci`.
fn sha_of(table: &[Vec<String>], sci: &str) -> String {
    let col = table[0]
        .iter()
        .position(|h| h == "Model_SHA256")
        .expect("column");
    table
        .iter()
        .find(|r| r.get(2).is_some_and(|s| s == sci))
        .unwrap_or_else(|| panic!("a row for {sci}"))[col]
        .clone()
}

/// A run registered before classifiers were recorded per run, whose rows name
/// two classifiers: which file made which row is not known, and the export
/// says nothing rather than the primary's checksum for both.
#[tokio::test]
async fn a_second_classifiers_row_is_not_given_the_primarys_checksum() {
    let (_dir, state) = station();
    let run = register_run(&state);
    add_row(&state, "06:00:00", "Pica pica", run, "birdnet");
    add_row(&state, "06:00:03", "Corvus corax", run, "perch");
    let table = csv(&state).await;
    assert_ne!(
        sha_of(&table, "Corvus corax"),
        PRIMARY_SHA,
        "a row the second classifier made was exported with the primary's checksum"
    );
}

/// Counterpart: a run whose rows all name one classifier is a run with one
/// classifier answering, and its rows keep the run's file, as they always did.
#[tokio::test]
async fn a_run_whose_rows_name_one_classifier_keeps_its_checksum() {
    let (_dir, state) = station();
    let run = register_run(&state);
    add_row(&state, "06:00:00", "Pica pica", run, "birdnet");
    add_row(&state, "06:00:03", "Corvus corax", run, "birdnet");
    let table = csv(&state).await;
    assert_eq!(sha_of(&table, "Pica pica"), PRIMARY_SHA);
    assert_eq!(sha_of(&table, "Corvus corax"), PRIMARY_SHA);
}

/// A run that recorded each classifier's file exports each row with its own
/// classifier's checksum, and a classifier it did not record with none.
#[tokio::test]
async fn each_row_carries_its_own_classifiers_checksum() {
    const PERCH_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let (_dir, state) = station();
    let run = register_run(&state);
    state
        .with_db(|conn| {
            for (id, name, sha, primary) in [
                ("birdnet", "BirdNET_GLOBAL_6K_V2.4_Model_FP32", PRIMARY_SHA, true),
                ("perch", "perch_v2", PERCH_SHA, false),
            ] {
                birdnet_db::sqlite::insert_run_classifier(
                    conn,
                    &birdnet_db::sqlite::NewRunClassifier {
                        run_id: run,
                        model_id: id,
                        model_name: name,
                        model_sha256: sha,
                        labels_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
                        is_primary: primary,
                    },
                )?;
            }
            Ok::<_, birdnet_db::sqlite::DbError>(())
        })
        .expect("classifiers");
    add_row(&state, "06:00:00", "Pica pica", run, "birdnet");
    add_row(&state, "06:00:03", "Corvus corax", run, "perch");
    add_row(&state, "06:00:06", "Parus major", run, "unrecorded");
    let table = csv(&state).await;
    assert_eq!(sha_of(&table, "Pica pica"), PRIMARY_SHA);
    assert_eq!(sha_of(&table, "Corvus corax"), PERCH_SHA);
    assert_eq!(sha_of(&table, "Parus major"), "");
}
