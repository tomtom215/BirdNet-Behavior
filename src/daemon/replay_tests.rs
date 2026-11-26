//! What a stop keeps, and what the next start does with it, driven through the
//! real event processor against queue batches on disk.
//!
//! Each test writes `clip-queue/<batch>/events.jsonl` the way a stopping
//! station writes it, starts the processor over a fresh `AppState` and reads
//! the database back. Writing the batch rather than calling the queue's own
//! API keeps the tests about the on-disk contract: a batch written by an
//! earlier release has to keep meaning the same thing to a later one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use birdnet_core::audio::extraction::{ExtractionConfig, Extractor};
use birdnet_core::detection::daemon::DetectionEvent;
use birdnet_core::detection::types::Detection;

const SR: u32 = 48_000;

fn detection(sci: &str, time: &str, confidence: f32, start: f32) -> Detection {
    Detection {
        date: "2026-05-19".into(),
        time: time.into(),
        scientific_name: sci.into(),
        common_name: sci.into(),
        confidence,
        start,
        stop: start + 3.0,
        week: 20,
        file_name_extr: None,
        model_id: Some("birdnet".into()),
        agreeing_models: None,
        model_scores: Vec::new(),
    }
}

fn live_event(det: Detection, source: PathBuf, correlation_id: &str) -> DetectionEvent {
    DetectionEvent {
        detection: det,
        source_file: source,
        latency_ms: 5,
        correlation_id: correlation_id.into(),
        lease: None,
        out_of_range: None,
    }
}

/// A batch line as the station before this change wrote it: no run, no
/// policy, no version.
fn v1_line(det: &Detection, source_name: &str, correlation_id: &str) -> String {
    serde_json::json!({
        "detection": det,
        "source": source_name,
        "latency_ms": 5,
        "correlation_id": correlation_id,
        "out_of_range": null,
    })
    .to_string()
}

/// A batch line carrying the run that produced it and the bars it was to be
/// judged against.
fn v2_line(
    det: &Detection,
    source_name: &str,
    correlation_id: &str,
    run_id: i64,
    global_bar: f64,
) -> String {
    serde_json::json!({
        "v": 2,
        "detection": det,
        "source": source_name,
        "latency_ms": 5,
        "correlation_id": correlation_id,
        "out_of_range": null,
        "run": {
            "run_id": run_id,
            "algorithm": null,
            "lat": 10.5,
            "lon": 20.5,
            "sensitivity": 1.0,
            "overlap": 0.0,
            "model_thresholds": {},
        },
        "policy": {"species_bar": null, "global_bar": global_bar},
    })
    .to_string()
}

/// A 16-bit mono WAV of `secs` seconds at [`SR`], every sample `level`.
fn wav(path: &Path, secs: u32, level: i16) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..(SR * secs) {
        w.write_sample(level).unwrap();
    }
    w.finalize().unwrap();
}

/// Write one batch beside the database at `db`: `lines` and the recordings
/// named in `wavs` (15 s of a constant level each).
fn write_batch(db: &Path, name: &str, lines: &[String], wavs: &[&str]) -> PathBuf {
    let batch = db.parent().unwrap().join("clip-queue").join(name);
    std::fs::create_dir_all(&batch).unwrap();
    for w in wavs {
        wav(&batch.join(w), 15, 8_000);
    }
    std::fs::write(batch.join("events.jsonl"), lines.join("\n") + "\n").unwrap();
    batch
}

fn register_run(state: &birdnet_web::state::AppState, sha: &str) -> i64 {
    let new = birdnet_db::sqlite::NewAnalysisRun {
        app_version: "0.0.0-test",
        model_name: "test-model",
        model_path: "/models/test.onnx",
        model_sha256: sha,
        model_bytes: 4096,
        labels_path: "/models/labels.csv",
        labels_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
        label_count: 3,
        geomodel_sha256: None,
        confidence: 0.25,
        sensitivity: 1.25,
        overlap: 1.5,
        sf_thresh: 0.03,
        lat: Some(51.48),
        lon: Some(-0.13),
    };
    state
        .with_db(|conn| birdnet_db::sqlite::insert_analysis_run(conn, &new))
        .unwrap()
}

fn provenance(run_id: i64) -> super::processor::RunProvenance {
    super::processor::RunProvenance {
        run_id: run_id.into(),
        algorithm: None,
        lat: Some(51.48),
        lon: Some(-0.13),
        sensitivity: 1.25,
        overlap: 1.5,
        model_thresholds: HashMap::new(),
    }
}

/// Run the processor once over `events`, then let it drain and exit.
async fn drive(
    state: &birdnet_web::state::AppState,
    events: Vec<DetectionEvent>,
    global_confidence: f32,
    duplicate_interval_secs: i64,
    provenance: super::processor::RunProvenance,
    extractor: Extractor,
) {
    let (tx, rx) = mpsc::channel();
    for e in events {
        tx.send(e).unwrap();
    }
    drop(tx);
    let filter = birdnet_integrations::notification::NotificationFilter {
        trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
        species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
    };
    let template = birdnet_integrations::notification::NotificationTemplate::default();
    let rt = tokio::runtime::Handle::current();
    let st = state.clone();
    let broadcast = state.detection_broadcast();
    tokio::task::spawn_blocking(move || {
        super::processor::event_processor(
            rx,
            st,
            broadcast,
            None,
            None,
            None,
            None,
            filter,
            template,
            rt,
            HashMap::new(),
            global_confidence,
            extractor,
            duplicate_interval_secs,
            super::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
            birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default(),
            ),
            provenance,
            None,
        );
    })
    .await
    .unwrap();
}

fn extractor_into(dir: &Path, own_segment_only: bool) -> Extractor {
    Extractor::new(ExtractionConfig {
        output_dir: dir.to_path_buf(),
        own_segment_only,
        ..ExtractionConfig::default()
    })
}

/// `(Sci_Name, run_id, Sens, Cutoff)` of every stored row, by name.
fn rows(
    state: &birdnet_web::state::AppState,
) -> Vec<(String, Option<i64>, Option<f64>, Option<f64>)> {
    state.with_db(|c| {
        let mut s = c
            .prepare(
                "SELECT Sci_Name, run_id, Sens, Cutoff FROM detections ORDER BY Sci_Name, Time",
            )
            .unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
}

fn station() -> (tempfile::TempDir, birdnet_web::state::AppState, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("birds.db");
    let state = birdnet_web::state::AppState::new(db.clone()).unwrap();
    (tmp, state, db)
}

const SEG: &str = "2026-05-19-birdnet-09:00:00.wav";

/// F01. A batch written before batches carried their run says nothing about
/// which run analysed it, and the row must say the same: unknown, not the run
/// that happens to be replaying it.
#[tokio::test(flavor = "current_thread")]
async fn a_kept_detection_with_no_run_is_stored_with_no_run() {
    let (tmp, state, db) = station();
    let now = register_run(&state, &"b".repeat(64));
    write_batch(
        &db,
        "000000000000000000000001-0",
        &[v1_line(
            &detection("Pica pica", "09:00:00", 0.9, 0.0),
            SEG,
            "c1",
        )],
        &[SEG],
    );
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    let got = rows(&state);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(
        got[0].1, None,
        "a replayed detection with no recorded run was attributed to run {now}, \
         which did not analyse it"
    );
}

/// F01. A kept detection is stored under the run that analysed it, with that
/// run's settings, whatever run replays it.
#[tokio::test(flavor = "current_thread")]
async fn a_kept_detection_keeps_the_run_that_analysed_it() {
    let (tmp, state, db) = station();
    let then = register_run(&state, &"a".repeat(64));
    let now = register_run(&state, &"b".repeat(64));
    write_batch(
        &db,
        "000000000000000000000001-0",
        &[v2_line(
            &detection("Pica pica", "09:00:00", 0.9, 0.0),
            SEG,
            "c1",
            then,
            0.25,
        )],
        &[SEG],
    );
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    let got = rows(&state);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, Some(then), "run");
    assert_eq!(got[0].2, Some(1.0), "the run's own sensitivity");
}

/// F01. A kept detection is judged by the bar in force when it was analysed,
/// not by a bar raised between the stop and the start.
#[tokio::test(flavor = "current_thread")]
async fn a_kept_detection_is_judged_by_the_bar_it_was_kept_under() {
    let (tmp, state, db) = station();
    let then = register_run(&state, &"a".repeat(64));
    let now = register_run(&state, &"b".repeat(64));
    write_batch(
        &db,
        "000000000000000000000001-0",
        &[v2_line(
            &detection("Pica pica", "09:00:00", 0.6, 0.0),
            SEG,
            "c1",
            then,
            0.5,
        )],
        &[SEG],
    );
    drive(
        &state,
        vec![],
        0.9,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    let got = rows(&state);
    assert_eq!(
        got.len(),
        1,
        "a detection kept at a 0.5 bar was re-judged at 0.9: {got:?}"
    );
    assert_eq!(got[0].3, Some(0.5), "the cutoff it was admitted at");
}

/// F02. A kept detection the database refuses stays kept: the next start
/// stores it. Counterpart: once stored, it is not offered again.
#[tokio::test(flavor = "current_thread")]
async fn a_kept_detection_the_database_refuses_is_still_kept() {
    let (tmp, state, db) = station();
    let now = register_run(&state, &"b".repeat(64));
    write_batch(
        &db,
        "000000000000000000000001-0",
        &[
            v1_line(&detection("Pica pica", "09:00:00", 0.9, 0.0), SEG, "c1"),
            v1_line(&detection("Corvus corax", "09:00:03", 0.9, 3.0), SEG, "c1"),
        ],
        &[SEG],
    );
    state.with_db(|c| {
        c.execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON detections
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .unwrap();
    });
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    assert!(rows(&state).is_empty());
    state.with_db(|c| c.execute_batch("DROP TRIGGER refuse").unwrap());

    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    let names: Vec<String> = rows(&state).into_iter().map(|r| r.0).collect();
    assert_eq!(
        names,
        ["Corvus corax", "Pica pica"],
        "detections the database refused during a replay were deleted with their batch"
    );

    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    assert_eq!(
        rows(&state).len(),
        2,
        "a stored detection was replayed again"
    );
    assert!(
        !db.parent()
            .unwrap()
            .join("clip-queue")
            .join("000000000000000000000001-0")
            .exists(),
        "a batch whose every detection is stored is removed"
    );
}

/// F02. A start killed after a kept detection's row was written and before
/// that was recorded in the batch replays it, and must not write it twice.
#[tokio::test(flavor = "current_thread")]
async fn a_kept_detection_already_stored_is_not_stored_twice() {
    let (tmp, state, db) = station();
    let now = register_run(&state, &"b".repeat(64));
    let line = v1_line(&detection("Pica pica", "09:00:00", 0.9, 0.0), SEG, "c1");
    write_batch(
        &db,
        "000000000000000000000001-0",
        std::slice::from_ref(&line),
        &[SEG],
    );
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    assert_eq!(rows(&state).len(), 1);
    // The same batch again, as a start killed before it recorded the row
    // would leave it.
    write_batch(&db, "000000000000000000000001-0", &[line], &[SEG]);
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    assert_eq!(
        rows(&state).len(),
        1,
        "the replay wrote the detection a second time"
    );
}

/// F05. A line that cannot be read is not deleted with its batch because a
/// later line was processed: it stays, for an operator to look at.
#[tokio::test(flavor = "current_thread")]
async fn an_unreadable_kept_line_keeps_its_batch() {
    let (tmp, state, db) = station();
    let now = register_run(&state, &"b".repeat(64));
    let batch = write_batch(
        &db,
        "000000000000000000000001-0",
        &[
            v1_line(&detection("Pica pica", "09:00:00", 0.9, 0.0), SEG, "c1"),
            "{\"detection\": {\"truncated".to_owned(),
            v1_line(&detection("Corvus corax", "09:00:06", 0.9, 6.0), SEG, "c1"),
        ],
        &[SEG],
    );
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    assert_eq!(rows(&state).len(), 2, "the readable ones are stored");
    let text = std::fs::read_to_string(batch.join("events.jsonl"))
        .expect("the batch holding an unreadable line was deleted");
    assert!(text.contains("truncated"));
}

/// F16. A replayed detection proves a recording existed once; it says nothing
/// about whether its source is capturing now.
#[tokio::test(flavor = "current_thread")]
async fn a_replayed_detection_does_not_mark_its_source_up() {
    let (tmp, state, db) = station();
    let now = register_run(&state, &"b".repeat(64));
    let seg = "2026-05-19-birdnet-cam9-09:00:00.wav";
    write_batch(
        &db,
        "000000000000000000000001-0",
        &[v1_line(
            &detection("Pica pica", "09:00:00", 0.9, 0.0),
            seg,
            "c1",
        )],
        &[seg],
    );
    drive(
        &state,
        vec![],
        0.25,
        0,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    assert_eq!(
        rows(&state).len(),
        1,
        "precondition: the replay stored the row"
    );
    assert_ne!(
        state.metrics().source_up("cam9"),
        Some(true),
        "an old recording replayed at start marked its source as up"
    );
}

/// F07. A detection the database refuses does not open a duplicate interval
/// that suppresses the next real one.
#[tokio::test(flavor = "current_thread")]
async fn a_refused_detection_does_not_shadow_the_next() {
    let (tmp, state, _db) = station();
    let now = register_run(&state, &"b".repeat(64));
    state.with_db(|c| {
        c.execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON detections WHEN NEW.Time = '09:00:00'
             BEGIN SELECT RAISE(ABORT, 'database is locked'); END;",
        )
        .unwrap();
    });
    let src = tmp.path().join(SEG);
    wav(&src, 15, 8_000);
    let events = vec![
        live_event(
            detection("Pica pica", "09:00:00", 0.9, 0.0),
            src.clone(),
            "c1",
        ),
        live_event(detection("Pica pica", "09:00:05", 0.9, 5.0), src, "c1"),
    ];
    drive(
        &state,
        events,
        0.25,
        30,
        provenance(now),
        extractor_into(tmp.path(), false),
    )
    .await;
    let got = rows(&state);
    assert_eq!(
        got.len(),
        1,
        "the second detection was suppressed as a repeat of one the database refused"
    );
}

/// F03. On a station that keeps clips inside their own segment (the privacy
/// filter is on), a stop keeps the audio the clip will be cut from and no
/// other: no neighbouring recording, and silence outside the clip's window.
#[tokio::test(flavor = "current_thread")]
async fn a_stop_keeps_only_the_audio_of_the_clip_on_a_privacy_filtered_station() {
    let (tmp, state, db) = station();
    let now = register_run(&state, &"b".repeat(64));
    let stream = tmp.path().join("stream");
    std::fs::create_dir_all(&stream).unwrap();
    for t in ["08:59:45", "09:00:00", "09:00:15"] {
        wav(
            &stream.join(format!("2026-05-19-birdnet-{t}.wav")),
            15,
            8_000,
        );
    }
    let src = stream.join(SEG);
    state.begin_shutdown();
    drive(
        &state,
        vec![live_event(
            detection("Pica pica", "09:00:06", 0.9, 6.0),
            src,
            "c1",
        )],
        0.25,
        0,
        provenance(now),
        extractor_into(&tmp.path().join("clips"), true),
    )
    .await;
    let queue = db.parent().unwrap().join("clip-queue");
    let batch = std::fs::read_dir(&queue)
        .unwrap()
        .flatten()
        .next()
        .expect("a batch")
        .path();
    let audio: Vec<String> = std::fs::read_dir(&batch)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".wav"))
        .collect();
    assert_eq!(audio, [SEG], "neighbouring recordings were kept");
    let decoded = birdnet_core::audio::decode::decode_file(&batch.join(SEG)).unwrap();
    // Default extraction: 6 s clip around a 3 s window, so [4.5, 10.5) s.
    let (from, to) = ((4.5 * 48_000.0) as usize, (10.5 * 48_000.0) as usize);
    let outside = decoded
        .samples
        .iter()
        .enumerate()
        .filter(|(i, s)| (*i < from || *i >= to) && **s != 0.0)
        .count();
    assert_eq!(outside, 0, "audio outside the clip's window was kept");
    assert!(
        decoded.samples[from..to].iter().all(|s| *s != 0.0),
        "the clip's own audio was kept"
    );
}
