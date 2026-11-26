//! A stop with a backlog exits inside the unit's timeout, and the detections
//! it had not processed are recorded, with their clips, at the next start.
//!
//! One busy recording on a two-classifier station is about 300 detections,
//! each a clip the event processor cuts and a row it writes. The processor
//! worked through all of them after a SIGTERM, about 52 s, and the unit's
//! `TimeoutStopSec` is 30: systemd killed the process part-way, and every
//! detection still queued was lost, because the queue is memory and the
//! recordings live in the service's `PrivateTmp`, which the next start does
//! not see. This test stops the station the moment the recording has been
//! analysed, empties the watch folder as `PrivateTmp` does, starts it again,
//! and compares what it recorded with what the same station records when
//! nothing interrupts it.

mod station;

use std::path::Path;
use std::time::Duration;

use station::{Station, StationDir, fixture_model, labels_text};

/// How long systemd waits after SIGTERM before it kills the process
/// (`TimeoutStopSec=30` in the unit), less a margin for a slower machine.
const STOP_WITHIN: Duration = Duration::from_secs(20);

/// Long enough to analyse the recording and to write its detections.
const WITHIN: Duration = Duration::from_secs(240);

/// A station with two classifiers judging every window: the tiny fixture
/// models, at a confidence low enough that nearly every class in every
/// window is a detection.
fn busy() -> StationDir {
    let mut dir = StationDir::new();
    dir.with_primary();
    dir.with_second(
        "second",
        Some(&fixture_model("tiny_v24_test.onnx")),
        Some(&labels_text("Secundus", 11)),
    );
    dir.set("MODEL_ROUTES", "local:birdnet+second");
    dir
}

/// Every file under `dir`, by name.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name() {
                out.push(name.to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// The detections, as (scientific name, classifier) sorted, and whether every
/// one has its clip on the disk.
fn recorded(station: &Station) -> (Vec<(String, String)>, usize) {
    let mut rows = station.detections();
    rows.sort();
    let clips = files_under(&station.path().join("data/recordings"));
    let missing = station
        .clip_names()
        .iter()
        .filter(|name| name.is_empty() || !clips.contains(name))
        .count();
    (rows, missing)
}

/// **A stop with about 300 detections queued exits well inside the unit's
/// timeout, and every one of them is recorded, with its clip, after the next
/// start.**
#[test]
fn a_stop_with_a_backlog_exits_in_time_and_loses_nothing() {
    // What the station records when nothing interrupts it.
    let mut reference = busy().start();
    reference.hand_whole_recording();
    assert_eq!(
        reference.wait_for_analysed(1, WITHIN),
        1,
        "{}",
        reference.log()
    );
    let mut expected = reference.wait_for_detections(WITHIN);
    expected.sort();
    assert!(
        expected.len() >= 200,
        "the fixture is meant to queue a backlog; it made {} detections",
        expected.len()
    );
    let (_, missing) = recorded(&reference);
    assert_eq!(
        missing, 0,
        "the uninterrupted station has a clip for every row"
    );
    reference.stop();

    // The same, stopped as soon as the recording is analysed.
    let mut station = busy().start();
    station.hand_whole_recording();
    assert_eq!(station.wait_for_analysed(1, WITHIN), 1, "{}", station.log());
    let took = station.stop_within(Duration::from_secs(90));
    let before_restart = station.detections().len();
    eprintln!(
        "stopped in {took:?} with {before_restart} of {} detections written",
        expected.len()
    );
    assert!(
        took.is_some_and(|t| t < STOP_WITHIN),
        "SIGTERM with {} of {} detections written took {took:?} to exit, against the \
         unit's 30 s:\n{}",
        before_restart,
        expected.len(),
        station.log()
    );

    // The service's PrivateTmp is empty at the next start.
    for entry in std::fs::read_dir(station.path().join("watch"))
        .expect("watch")
        .flatten()
    {
        let _ = std::fs::remove_file(entry.path());
    }
    station.restart();
    let deadline = std::time::Instant::now() + WITHIN;
    while station.detections().len() < expected.len() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    let (rows, missing) = recorded(&station);
    assert_eq!(
        rows.len(),
        expected.len(),
        "{before_restart} rows were written before the stop; after the restart there are {}:\n{}",
        rows.len(),
        station.log()
    );
    assert!(
        rows == expected,
        "the rows recorded are not the uninterrupted station's"
    );
    assert_eq!(missing, 0, "{missing} rows have no clip on the disk");
    station.stop();
}
