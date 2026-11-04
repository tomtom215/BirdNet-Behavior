//! The API and the exports say which classifier supplied each detection and what
//! every classifier scored it (migrations 50 and 57).
//!
//! The pieces are tested where they live: the table and its readers in
//! `birdnet-db`, the CSV writers in this crate's unit tests. What neither covers
//! is that the routes are *connected*: a handler that reads the wrong key, or a
//! route that was never registered, passes both. Each test here goes through the
//! real router.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::sqlite::{ModelScore, ModelScoreState, insert_model_score};
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

const DATE: &str = "2026-05-01";
const TIME: &str = "06:00:00";
const SCI: &str = "Dryobates villosus";

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

fn add_detection(
    state: &AppState,
    date: &str,
    time: &str,
    sci: &str,
    model: Option<&str>,
    agreement: Option<i64>,
) {
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs, model_id, model_agreement)
             VALUES (?1, ?2, ?3, 'Hairy Woodpecker', 0.9, 0.7, 18, 1.25, 0.0, ?3 || '.wav', 0, ?4, ?5)",
            rusqlite::params![date, time, sci, model, agreement],
        )
        .expect("insert detection");
    });
}

fn store(
    state: &AppState,
    date: &str,
    time: &str,
    sci: &str,
    model: &str,
    st: ModelScoreState,
    score: Option<f64>,
    own_bar: Option<f64>,
) {
    state.with_db(|conn| {
        insert_model_score(
            conn,
            date,
            time,
            sci,
            &ModelScore {
                model_id: model.to_owned(),
                state: st,
                score,
                bar: own_bar,
                window_start_secs: Some(0.0),
                window_end_secs: Some(3.0),
            },
        )
        .expect("store score");
    });
}

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

async fn json(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let (status, body) = get(state, uri).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn models_uri() -> String {
    format!("/api/v2/detections/models?date={DATE}&time={TIME}&sci_name=Dryobates%20villosus")
}

/// A station with one detection two classifiers answered, one of which has no
/// class for the species.
fn two_answers() -> (tempfile::TempDir, AppState) {
    let (dir, state) = station();
    add_detection(&state, DATE, TIME, SCI, Some("birdnet-v3"), Some(1));
    store(
        &state,
        DATE,
        TIME,
        SCI,
        "birdnet-v3",
        ModelScoreState::Heard,
        Some(0.91),
        Some(0.70),
    );
    store(
        &state,
        DATE,
        TIME,
        SCI,
        "perch-v2",
        ModelScoreState::BelowBar,
        Some(0.33),
        Some(0.50),
    );
    store(
        &state,
        DATE,
        TIME,
        SCI,
        "regional",
        ModelScoreState::NoLabel,
        None,
        None,
    );
    (dir, state)
}

#[tokio::test]
async fn the_models_endpoint_lists_each_classifiers_own_answer_against_its_own_bar() {
    let (_dir, state) = two_answers();
    let (status, v) = json(&state, &models_uri()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["model_id"], "birdnet-v3");
    assert_eq!(v["model_agreement"], 1);
    let models = v["models"].as_array().expect("a list");
    let seen: Vec<(&str, &str)> = models
        .iter()
        .map(|m| {
            (
                m["model_id"].as_str().unwrap(),
                m["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            ("birdnet-v3", "heard"),
            ("perch-v2", "below_bar"),
            ("regional", "no_label")
        ],
        "heard first, then below the bar, then no class"
    );
    assert!((models[1]["score"].as_f64().unwrap() - 0.33).abs() < 1e-9);
    assert!((models[1]["bar"].as_f64().unwrap() - 0.50).abs() < 1e-9);
    assert!(
        models[2]["score"].is_null() && models[2]["bar"].is_null(),
        "a classifier with no class has no score, not a zero: {}",
        models[2]
    );
}

#[tokio::test]
async fn a_row_from_before_the_columns_has_no_model_keys_and_an_empty_list() {
    let (_dir, state) = station();
    add_detection(&state, DATE, TIME, SCI, None, None);
    let (status, v) = json(&state, &models_uri()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(
        v.get("model_id").is_none() && v.get("model_agreement").is_none(),
        "{v}"
    );
    assert_eq!(v["models"], serde_json::json!([]));
}

#[tokio::test]
async fn a_detection_that_does_not_exist_is_a_404_and_a_malformed_request_is_a_400() {
    let (_dir, state) = two_answers();
    let missing = format!("/api/v2/detections/models?date={DATE}&time=07:00:00&sci_name=Nope");
    assert_eq!(json(&state, &missing).await.0, StatusCode::NOT_FOUND);
    for uri in [
        "/api/v2/detections/models".to_owned(),
        format!("/api/v2/detections/models?date={DATE}&time={TIME}"),
        format!("/api/v2/detections/models?date=yesterday&time={TIME}&sci_name=x"),
    ] {
        assert_eq!(json(&state, &uri).await.0, StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test]
async fn the_detections_list_carries_the_classifier_and_agreement_and_omits_what_is_unknown() {
    let (_dir, state) = station();
    add_detection(&state, DATE, TIME, SCI, Some("birdnet-v3"), Some(2));
    add_detection(&state, DATE, "07:00:00", "Pica pica", None, None);
    let (status, v) = json(&state, &format!("/api/v2/detections?date={DATE}")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let rows = v["detections"].as_array().expect("rows");
    let by = |sci: &str| rows.iter().find(|r| r["sci_name"] == sci).expect("row");
    assert_eq!(by(SCI)["model_id"], "birdnet-v3");
    assert_eq!(by(SCI)["model_agreement"], 2);
    let legacy = by("Pica pica");
    assert!(
        legacy.get("model_id").is_none() && legacy.get("model_agreement").is_none(),
        "{legacy}"
    );
}

#[tokio::test]
async fn the_detections_csv_ends_with_the_classifier_and_agreement_columns() {
    let (_dir, state) = two_answers();
    add_detection(&state, DATE, "07:00:00", "Pica pica", None, None);
    let (status, csv) = get(&state, "/api/v2/detections/export").await;
    assert_eq!(status, StatusCode::OK);
    let lines: Vec<&str> = csv.lines().collect();
    assert!(
        lines[0].ends_with(",Model_Id,Model_Agreement"),
        "{}",
        lines[0]
    );
    let hairy = lines.iter().find(|l| l.contains(SCI)).expect("row");
    assert!(hairy.ends_with(",birdnet-v3,1"), "{hairy}");
    let legacy = lines.iter().find(|l| l.contains("Pica pica")).expect("row");
    assert!(
        legacy.ends_with(",,"),
        "empty means nobody was counting: {legacy}"
    );
    // Every row has the header's column count.
    let n = lines[0].matches(',').count();
    assert!(lines.iter().all(|l| l.matches(',').count() == n), "{csv}");
}

#[tokio::test]
async fn the_model_scores_export_has_one_row_per_classifier_and_empty_cells_for_silence() {
    let (_dir, state) = two_answers();
    let (status, csv) = get(&state, "/api/v2/detections/export/model-scores").await;
    assert_eq!(status, StatusCode::OK);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "Date,Time,Sci_Name,Model_Id,State,Score,Bar,Window_Start_Secs,Window_End_Secs"
    );
    assert_eq!(lines.len(), 4, "{csv}");
    assert!(
        lines[1].contains(",birdnet-v3,heard,0.9100,0.7000,"),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].contains(",perch-v2,below_bar,0.3300,0.5000,"),
        "{}",
        lines[2]
    );
    assert!(
        lines[3].contains(",regional,no_label,,,"),
        "no score is empty, not 0: {}",
        lines[3]
    );
}

#[tokio::test]
async fn the_model_scores_export_honours_dates_leaves_out_rejected_detections_and_has_a_json_form()
{
    let (_dir, state) = two_answers();
    add_detection(
        &state,
        "2026-06-01",
        TIME,
        "Pica pica",
        Some("birdnet-v3"),
        Some(1),
    );
    store(
        &state,
        "2026-06-01",
        TIME,
        "Pica pica",
        "birdnet-v3",
        ModelScoreState::Heard,
        Some(0.8),
        Some(0.7),
    );
    add_detection(
        &state,
        "2026-06-02",
        TIME,
        "Parus major",
        Some("birdnet-v3"),
        Some(1),
    );
    store(
        &state,
        "2026-06-02",
        TIME,
        "Parus major",
        "birdnet-v3",
        ModelScoreState::Heard,
        Some(0.8),
        Some(0.7),
    );
    state.with_db(|c| {
        c.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Sci_Name = 'Parus major'",
            [],
        )
        .expect("reject");
    });

    let (_, v) = json(
        &state,
        "/api/v2/detections/export/model-scores?format=json&from=2026-06-01",
    )
    .await;
    let rows = v["model_scores"].as_array().expect("rows");
    assert_eq!(v["total"], 1, "{v}");
    assert_eq!(
        rows[0]["sci_name"], "Pica pica",
        "May is before `from`, and the rejected one is out"
    );
    assert_eq!(rows[0]["state"], "heard");

    let (_, all) = json(&state, "/api/v2/detections/export/model-scores?format=json").await;
    assert_eq!(
        all["total"], 4,
        "3 answers for the May detection and 1 for Pica pica: {all}"
    );

    let (status, _) = get(&state, "/api/v2/detections/export/model-scores?from=June").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
