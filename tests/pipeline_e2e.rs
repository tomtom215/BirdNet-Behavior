//! Full detection-pipeline integration test: audio → inference → DB → web.
//!
//! Two layers, by design:
//!
//! 1. **CI-runnable** (always on): exercises every stage of the real pipeline
//!    *except* the ONNX matmul — the bundled recording is decoded and resampled
//!    by the production audio code, and a detection is written through the same
//!    `insert_detection` path the daemon uses, then read back over the live
//!    HTTP API. No 541 MB model required, so it runs on every commit.
//!
//! 2. **Model-gated** (skipped unless `BIRDNET_TEST_MODEL` / `BIRDNET_TEST_LABELS`
//!    are set, exactly like `tests/inference_e2e.rs`): runs the *entire* chain —
//!    decode → resample → inference → DB → web — and asserts the Eurasian Magpie
//!    recording actually surfaces on the dashboard's detections API.
//!
//! Together they close the "no full-pipeline E2E" gap: the wiring is proven in
//! CI, and the real inference end-to-end is proven wherever the model is present.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use std::path::Path;
use tower::ServiceExt;

use birdnet_db::sqlite::{DetectionRecord, insert_detection};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

const PICA_PICA_WAV: &str = "tests/testdata/Pica_pica_30s.wav";

/// Fresh `AppState` over an in-memory database carrying the full schema.
fn fresh_state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// SQLite's notion of "today", so an inserted row lands inside the API's
/// default window — mirrors `tests/web_api_detections.rs` to avoid time-bombs.
fn sqlite_today(state: &AppState) -> String {
    state.with_db(|conn| {
        conn.query_row("SELECT DATE('now')", [], |r| r.get(0))
            .unwrap()
    })
}

/// GET `uri` on `router` and decode the JSON body.
async fn get_json(router: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 256 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

/// CI layer, audio half — the production decoder handles the real recording and
/// the resampler retargets it, proving the front of the pipeline on real audio.
#[test]
#[allow(clippy::cast_precision_loss)]
fn audio_decodes_and_resamples() {
    let wav = Path::new(PICA_PICA_WAV);
    assert!(wav.exists(), "test fixture missing: {PICA_PICA_WAV}");

    let audio = birdnet_core::audio::decode::decode_file(wav).expect("decode failed");
    assert_eq!(audio.sample_rate, 48_000, "fixture is 48 kHz");

    let secs = audio.samples.len() as f64 / f64::from(audio.sample_rate);
    assert!(
        (28.0..=32.0).contains(&secs),
        "expected ~30s of audio, got {secs:.1}s"
    );

    // Retarget to the V3 model rate; production resampler must succeed and
    // produce a proportionally shorter buffer.
    let resampled =
        birdnet_core::audio::resample::resample(&audio.samples, audio.sample_rate, 32_000)
            .expect("resample failed");
    assert!(
        !resampled.is_empty() && resampled.len() < audio.samples.len(),
        "48k→32k resample should shrink the buffer"
    );
}

/// CI layer, DB → web half — a detection written through the production
/// insertion path must surface on both the detections list and the stats API.
#[tokio::test]
async fn detection_persists_and_surfaces_on_web_api() {
    let state = fresh_state();
    let today = sqlite_today(&state);

    let record = DetectionRecord {
        date: &today,
        time: "06:30:00",
        sci_name: "Pica pica",
        com_name: "Eurasian Magpie",
        confidence: 0.91,
        lat: Some(51.5),
        lon: Some(-0.1),
        cutoff: Some(0.1),
        week: Some(11),
        sensitivity: Some(1.0),
        overlap: Some(0.0),
        file_name: "e2e-magpie-0630.wav",
        chunk_offset_secs: Some(3.0),
        correlation_id: Some("pipeline-e2e-0001"),
    };
    state
        .with_db(|conn| insert_detection(conn, &record))
        .expect("insert_detection failed");

    // Detections list: the row round-trips with its key fields intact.
    let (status, json) = get_json(
        build_router(state.clone()),
        &format!("/api/v2/detections?date={today}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["count"], 1);
    let det = &json["detections"][0];
    assert_eq!(det["com_name"], "Eurasian Magpie");
    assert_eq!(det["date"], today.as_str());

    // Stats: the aggregate endpoint counts it too.
    let (status, stats_json) = get_json(build_router(state), "/api/v2/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats_json["total_detections"], 1);
}

/// Load the real model + labels from env, or `None` to skip — same contract as
/// `tests/inference_e2e.rs`.
fn load_model() -> Option<birdnet_core::inference::model::BirdNetModel> {
    use birdnet_core::inference::labels::LabelSet;
    use birdnet_core::inference::model::{BirdNetModel, ModelConfig};

    let model_path = std::env::var("BIRDNET_TEST_MODEL").ok()?;
    let labels_path = std::env::var("BIRDNET_TEST_LABELS").ok()?;
    let model_path = Path::new(&model_path);
    let labels_path = Path::new(&labels_path);
    if !model_path.exists() || !labels_path.exists() {
        eprintln!("SKIP: BIRDNET_TEST_MODEL / BIRDNET_TEST_LABELS point at missing files");
        return None;
    }
    let labels = LabelSet::load(labels_path).expect("failed to load labels");
    let config = ModelConfig {
        confidence_threshold: 0.1,
        ..ModelConfig::default()
    };
    Some(BirdNetModel::load(model_path, labels, config).expect("failed to load model"))
}

/// Model-gated layer — the *entire* chain end to end. Decodes the Magpie
/// recording, runs real inference, persists every detection through the
/// production path, and asserts the Magpie surfaces on the web API.
#[tokio::test]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
async fn full_pipeline_audio_to_web_model_gated() {
    let Some(mut model) = load_model() else {
        return;
    };

    let audio =
        birdnet_core::audio::decode::decode_file(Path::new(PICA_PICA_WAV)).expect("decode failed");
    let target = model.infer_sample_rate();
    let samples = if audio.sample_rate == target {
        audio.samples
    } else {
        birdnet_core::audio::resample::resample(&audio.samples, audio.sample_rate, target)
            .expect("resample failed")
    };

    let state = fresh_state();
    let today = sqlite_today(&state);
    let chunk = (3.0 * f64::from(target)) as usize;
    let mut inserted = 0_usize;

    for start in (0..samples.len()).step_by(chunk) {
        let end = (start + chunk).min(samples.len());
        let mut buf = samples[start..end].to_vec();
        if buf.len() < chunk {
            buf.resize(chunk, 0.0);
        }
        let detections = model
            .predict(
                &buf,
                &today,
                "06:30:00",
                start as f32 / target as f32,
                end as f32 / target as f32,
                11,
            )
            .expect("inference failed");

        for d in &detections {
            let record = DetectionRecord {
                date: &d.date,
                time: &d.time,
                sci_name: &d.scientific_name,
                com_name: &d.common_name,
                confidence: f64::from(d.confidence),
                lat: None,
                lon: None,
                cutoff: None,
                week: Some(i64::from(d.week)),
                sensitivity: None,
                overlap: None,
                file_name: "magpie.wav",
                chunk_offset_secs: Some(f64::from(d.start)),
                correlation_id: None,
            };
            state
                .with_db(|conn| insert_detection(conn, &record))
                .expect("insert_detection failed");
            inserted += 1;
        }
    }

    assert!(
        inserted > 0,
        "model produced no detections on the Magpie fixture"
    );

    let (status, json) = get_json(
        build_router(state),
        &format!("/api/v2/detections?date={today}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let detections = json["detections"].as_array().unwrap();
    assert!(
        detections
            .iter()
            .any(|d| d["sci_name"] == "Pica pica" || d["com_name"] == "Eurasian Magpie"),
        "the Eurasian Magpie must surface on the API among {} detection(s)",
        detections.len()
    );
}
