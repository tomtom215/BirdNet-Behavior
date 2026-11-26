//! A station fetches a missing or damaged model file again by itself, and
//! uses it once it verifies (MDL-10).
//!
//! Before this, nothing on a running station fetched a model: a file lost or
//! damaged after the install stayed that way until somebody ran
//! `install.sh repair` or `--install-model`, and a station whose classifier
//! was damaged detected nothing until then. Each test starts the built binary
//! (`station`), breaks a file the configuration names, and reads what the
//! station did and what it said (`birdnet_model_file_state`, the detector,
//! classifier and filter state gauges, the database and the log).
//!
//! The origin is a loopback HTTP server named by `BIRDNET_MODEL_MIRROR`; every
//! station here is `--offline`, which fetches from that mirror and from
//! nothing else. Three tests need files that verify against the catalogue's
//! pins: the real BirdNET V2.4 model (`BIRDNET_TEST_V24_MODEL`) and its range
//! filter (`BIRDNET_TEST_V24_MDATA`). They skip without them, and fail instead
//! with `BIRDNET_REQUIRE_V24_MODEL=1`, which the model jobs set. The other
//! three need nothing but the fixtures: the V2.4 labels file is compiled into
//! the binary, so the station writes it again without any network.

mod common;
mod station;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use station::{Mirror, StationDir, fixture_model};

/// Long enough for a loaded runner to restart a station, load the real V2.4
/// and analyse one recording.
const WITHIN: Duration = Duration::from_secs(120);

/// The repair interval every station here runs at, in seconds.
const INTERVAL_SECS: u64 = 2;

/// The settings every station here shares: the repair interval, and one
/// round of the origins with no waits, so a failed fetch is over at once.
fn repairing(dir: &mut StationDir) -> &mut StationDir {
    dir.env("BIRDNET_MODEL_REPAIR_INTERVAL", INTERVAL_SECS)
        .env("BIRDNET_MODEL_RETRY_WAITS", "")
        .env("BIRDNET_MODEL_CONNECT_TIMEOUT", "2")
        .env("BIRDNET_MODEL_STALL_TIMEOUT", "5")
}

/// `birdnet_model_file_state` for `file` in `state`.
fn file_state(file: &str, state: &str) -> String {
    format!("birdnet_model_file_state{{file=\"{file}\",state=\"{state}\"}}")
}

/// The real V2.4 model and its range filter, or `None` (a skip, or a
/// failure under `BIRDNET_REQUIRE_V24_MODEL=1`).
fn real_v24() -> Option<(PathBuf, PathBuf)> {
    let (model, _) = common::v24_model_paths_with_mdata()?;
    let mdata = PathBuf::from(std::env::var("BIRDNET_TEST_V24_MDATA").ok()?);
    Some((model, mdata))
}

/// `bytes` with eight bytes in the middle overwritten: the same size, and not
/// the file.
fn damaged(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mid = out.len() / 2;
    for b in &mut out[mid..mid + 8] {
        *b = !*b;
    }
    out
}

/// A directory holding `files` under their catalogue names, for a mirror.
fn mirror_dir(files: &[(&str, &Path)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("mirror dir");
    for (name, from) in files {
        std::fs::copy(from, dir.path().join(name)).expect("stage the mirror");
    }
    dir
}

/// Hand the station the magpie and wait until its detections are written.
fn detections(station: &mut station::Station) -> Vec<(String, String)> {
    station.hand_recording(None);
    let analysed = station.wait_for_analysed(1, WITHIN);
    assert_eq!(
        analysed,
        1,
        "the recording was not analysed:\n{}",
        station.log()
    );
    let rows = station.wait_for_detections(WITHIN);
    assert!(!rows.is_empty(), "nothing was recorded:\n{}", station.log());
    rows
}

/// **A damaged primary classifier is fetched again, the station restarts
/// once to load it, and detects the magpie with it.**
///
/// The model has eight bytes overwritten (the right size, so only a hash
/// finds it) and its labels file is missing: the detector cannot start. Both
/// are put back in the same pass — the model from the mirror, the labels from
/// the binary — and the station restarts once, not once per file.
///
/// Observed failing before the repair existed: "the detector did not come
/// back running", the station logging "failed to start detection daemon"
/// and nothing after it.
#[test]
fn a_damaged_classifier_is_fetched_again_and_the_station_detects_with_it() {
    let Some((model, _)) = real_v24() else {
        return;
    };
    let mirror_files = mirror_dir(&[("birdnet-v24.onnx", &model)]);
    let mirror = Mirror::reserve();
    mirror.serve(mirror_files.path());

    let mut dir = StationDir::new();
    let bytes = std::fs::read(&model).expect("read the model");
    let model_path = dir.put("birdnet-v24.onnx", &damaged(&bytes));
    let labels_path = dir.model_path("birdnet-v24_labels.txt");
    dir.set("MODEL_PATH", model_path.display())
        .set("LABELS_PATH", labels_path.display());
    repairing(&mut dir).env("BIRDNET_MODEL_MIRROR", mirror.url());
    let mut station = dir.start();

    assert!(
        station.wait_for_state("birdnet_detector_state{state=\"running\"}", WITHIN),
        "the detector did not come back running:\n{}",
        station.log()
    );
    assert!(
        std::fs::read(&model_path).expect("read") == bytes,
        "the model file is not the pinned one"
    );
    assert!(labels_path.is_file());
    assert_eq!(
        station.starts(),
        2,
        "one restart for both files:\n{}",
        station.log()
    );
    let rows = detections(&mut station);
    assert!(
        rows.iter().any(|(sci, _)| sci == "Pica pica"),
        "the repaired V2.4 did not find the magpie: {rows:?}"
    );
    assert_eq!(
        station.metric(&file_state("birdnet-v24.onnx", "verified")),
        Some(1.0)
    );
    assert_eq!(station.starts(), 2);
    station.stop();
}

/// **A damaged filter whose origin is down leaves the station detecting,
/// retried at the interval; when the origin comes back the filter is fetched,
/// the station restarts once to load it, and the filter runs.**
///
/// The damage is eight bytes of the filter's weights, which still load: the
/// station ran on them, so it restarts once the file verifies even though
/// the filter reports itself active (first seen here: the filter was
/// `active` on the damaged file, and the first version of the repair, which
/// restarted only for a reader left without its file, never restarted).
///
/// Observed failing before the repair existed: "the filter file was never
/// retried" (no `birdnet_model_file_state` series at all).
#[test]
fn a_filter_whose_origin_was_down_is_fetched_when_it_comes_back() {
    let Some((model, mdata)) = real_v24() else {
        return;
    };
    let mirror_files = mirror_dir(&[("birdnet-v24-range-v2.onnx", &mdata)]);
    let mirror = Mirror::reserve();

    let mut dir = StationDir::new();
    let model_path = dir.put("birdnet-v24.onnx", &std::fs::read(&model).expect("model"));
    let labels_path = dir.put(
        "birdnet-v24_labels.txt",
        birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS.as_bytes(),
    );
    let filter_path = dir.put(
        "birdnet-v24-range-v2.onnx",
        &damaged(&std::fs::read(&mdata).expect("mdata")),
    );
    dir.set("MODEL_PATH", model_path.display())
        .set("LABELS_PATH", labels_path.display())
        .set("METADATA_MODEL_PATH", filter_path.display())
        .set("SF_THRESH", "0.03")
        .with_coordinates();
    repairing(&mut dir).env("BIRDNET_MODEL_MIRROR", mirror.url());
    let mut station = dir.start();

    let will_retry = file_state("birdnet-v24-range-v2.onnx", "will_retry");
    assert!(
        station.wait_for_state(&will_retry, WITHIN),
        "the filter file was never retried:\n{}",
        station.log()
    );
    // Detecting on what it has. Eight bytes of a filter's weights do not stop
    // it loading, so it runs, on the wrong weights, until the file is fetched.
    let rows = detections(&mut station);
    assert!(rows.iter().any(|(sci, _)| sci == "Pica pica"), "{rows:?}");
    // Tried again, an interval apart.
    std::thread::sleep(Duration::from_secs(INTERVAL_SECS * 2 + 1));
    let attempts = station.log_lines("fetching it again");
    assert!(
        attempts >= 2,
        "tried {attempts} time(s):\n{}",
        station.log()
    );
    assert_eq!(station.starts(), 1);

    mirror.serve(mirror_files.path());
    let deadline = Instant::now() + WITHIN;
    while station.starts() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(
        station.starts(),
        2,
        "the station did not restart once to load the fetched filter:\n{}",
        station.log()
    );
    assert!(
        std::fs::read(&filter_path).expect("read") == std::fs::read(&mdata).expect("read"),
        "the filter file is not the pinned one"
    );
    assert!(
        station.wait_for_state("birdnet_occurrence_filter_state{state=\"active\"}", WITHIN),
        "the filter did not run once its file came back:\n{}",
        station.log()
    );
    assert!(
        station.wait_for_state(&file_state("birdnet-v24-range-v2.onnx", "verified"), WITHIN),
        "{}",
        station.log()
    );
    std::thread::sleep(Duration::from_secs(INTERVAL_SECS * 2));
    assert_eq!(station.starts(), 2, "{}", station.log());
    station.stop();
}

/// **An origin that is down is tried once per interval, not in a loop, and
/// the station keeps detecting on what it has.**
///
/// Needs no real model: the damaged file is the V2.4 range filter's name
/// holding bytes that are not it, beside the tiny fixture classifier.
///
/// Observed failing before the repair existed: "the filter file was never
/// retried".
#[test]
fn an_origin_that_is_down_is_tried_at_the_interval_while_the_station_detects() {
    let mirror = Mirror::reserve();
    let mut dir = StationDir::new();
    dir.with_primary();
    let filter_path = dir.put("birdnet-v24-range-v2.onnx", b"not the range filter");
    dir.set("METADATA_MODEL_PATH", filter_path.display())
        .with_coordinates();
    repairing(&mut dir).env("BIRDNET_MODEL_MIRROR", mirror.url());
    let started = Instant::now();
    let mut station = dir.start();

    let will_retry = file_state("birdnet-v24-range-v2.onnx", "will_retry");
    assert!(
        station.wait_for_state(&will_retry, WITHIN),
        "the filter file was never retried:\n{}",
        station.log()
    );
    assert!(!detections(&mut station).is_empty());
    std::thread::sleep(Duration::from_secs(INTERVAL_SECS * 3));
    let attempts = station.log_lines("fetching it again");
    let bound = usize::try_from(started.elapsed().as_secs() / INTERVAL_SECS + 2).expect("fits");
    assert!(
        (2..=bound).contains(&attempts),
        "{attempts} attempts in {:?} at a {INTERVAL_SECS} s interval:\n{}",
        started.elapsed(),
        station.log()
    );
    assert_eq!(
        station.metric("birdnet_detector_state{state=\"running\"}"),
        Some(1.0)
    );
    assert_eq!(station.starts(), 1);
    station.stop();
}

/// **An `--offline` station with no mirror fetches nothing, says so once
/// however often it looks, and keeps detecting.**
///
/// Observed failing before the repair existed: no `not_fetched` state.
#[test]
fn an_offline_station_with_no_mirror_says_so_once_and_keeps_detecting() {
    let mut dir = StationDir::new();
    dir.with_primary();
    let filter_path = dir.put("birdnet-v24-range-v2.onnx", b"not the range filter");
    dir.set("METADATA_MODEL_PATH", filter_path.display())
        .with_coordinates();
    repairing(&mut dir);
    let mut station = dir.start();

    assert!(
        station.wait_for_state(
            &file_state("birdnet-v24-range-v2.onnx", "not_fetched"),
            WITHIN
        ),
        "the station did not say it is not fetching the file:\n{}",
        station.log()
    );
    assert!(!detections(&mut station).is_empty());
    std::thread::sleep(Duration::from_secs(INTERVAL_SECS * 3));
    assert_eq!(
        station.log_lines("is not being fetched"),
        1,
        "{}",
        station.log()
    );
    assert_eq!(station.log_lines("fetching it again"), 0);
    assert!(std::fs::read(&filter_path).expect("read") == b"not the range filter");
    station.stop();
}

/// **A file that verifies and still will not load does not restart the
/// station in a loop.**
///
/// The second classifier is the tiny fixture (11 classes) with the V2.4
/// labels file (6 522 names) configured beside it, missing. The station
/// writes the labels again from the binary — a file it was without, so it
/// restarts once to load it — and then the classifier is left out for the
/// labels' length, a fault no restart mends. The file verifies from then on,
/// so the station never restarts again, at a repair interval of 2 s.
///
/// Observed failing before the repair existed (the labels never written:
/// `left_out_labels_missing`), and with `Repair::verified` asking for a
/// restart whenever the reader is without its file rather than only after
/// the file was broken: "left: 11, right: 2", the station restarting at
/// every look.
#[test]
fn a_file_that_verifies_and_will_not_load_does_not_restart_the_station_again() {
    let mut dir = StationDir::new();
    dir.with_primary();
    let second = dir.put("second.onnx", &fixture_model("tiny_v24_test.onnx"));
    let labels = dir.model_path("birdnet-v24_labels.txt");
    dir.set("MODEL_2_PATH", second.display())
        .set("MODEL_2_LABELS", labels.display())
        .set("MODEL_2_ID", "second");
    repairing(&mut dir);
    let mut station = dir.start();

    let wrong_length =
        "birdnet_classifier_state{classifier=\"second\",state=\"left_out_labels_wrong_length\"}";
    assert!(
        station.wait_for_state(wrong_length, WITHIN),
        "the labels were not written again and loaded:\n{}",
        station.log()
    );
    std::thread::sleep(Duration::from_secs(INTERVAL_SECS * 6));
    assert!(!station.has_exited());
    assert_eq!(station.starts(), 2, "{}", station.log());
    assert_eq!(
        station.metric(&file_state("birdnet-v24_labels.txt", "verified")),
        Some(1.0)
    );
    // And the primary still detects.
    assert!(!detections(&mut station).is_empty());
    station.stop();
}
