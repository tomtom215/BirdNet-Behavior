//! A station runs on the model files it can load, and says which it could not
//! (MDL-01, MDL-02, MDL-04, MDL-05, MDL-07, MDL-16).
//!
//! On 2026-11-25 a real station, started from the built binary with the real
//! files, showed that one bad *optional* file stopped all detection: a
//! truncated Perch file beside a working V2.4 gave "failed to start detection
//! daemon … classifier `perch` could not be loaded" and 0 files analysed, and
//! a missing `METADATA_LABELS_PATH` did the same. It also showed that the
//! station mostly did not say what state its models were in: a geomodel with
//! no coordinates was reported "on", and a second classifier routed to the
//! only microphone judged nothing.
//!
//! Each test here starts the binary in one of those situations, hands it a
//! recording, and asserts both what the station did (the primary classifier
//! still detects, or the detector does not start) and what it said: the
//! state-set gauges `birdnet_detector_state`, `birdnet_classifier_state` and
//! `birdnet_occurrence_filter_state`, and `birdnet_occurrence_filter_active`.

mod station;

use std::time::Duration;

use station::{StationDir, TINY_META_LABELS, fixture_model, labels_text};

/// Long enough for a loaded CI runner to analyse one 30 s recording with the
/// tiny models (about a second here) and write its detections.
const ANALYSED_WITHIN: Duration = Duration::from_secs(60);

/// How long a test waits for something that must not happen: a recording a
/// stopped detector must not analyse. The counterpart test
/// (`a_primary_with_its_own_labels_runs`) shows a running one analyses it
/// well inside this.
const NOTHING_WITHIN: Duration = Duration::from_secs(20);

/// The tiny V3.0-shaped classifier, cut short: what a download interrupted
/// part-way leaves, past any size check and not a model.
fn truncated_model() -> Vec<u8> {
    let whole = fixture_model("tiny_v30_test.onnx");
    whole[..whole.len() / 2].to_vec()
}

/// Hand the station one recording and wait until it has been analysed and
/// its detections written; return the classifiers they were recorded under.
fn detects(station: &mut station::Station) -> std::collections::BTreeSet<String> {
    station.hand_recording(None);
    let analysed = station.wait_for_analysed(1, ANALYSED_WITHIN);
    assert_eq!(
        analysed,
        1,
        "the recording was not analysed:\n{}",
        station.log()
    );
    let rows = station.wait_for_detections(ANALYSED_WITHIN);
    assert!(
        !rows.is_empty(),
        "the recording was analysed and nothing was recorded:\n{}",
        station.log()
    );
    station.detecting_classifiers()
}

/// The one state a state-set family is in, for one label set prefix
/// (`classifier="perch",`), or the whole label set when `prefix` is empty.
fn state_of(station: &station::Station, family: &str, prefix: &str) -> Vec<String> {
    station
        .states_of(family)
        .into_iter()
        .filter_map(|labels| {
            labels
                .strip_prefix(prefix)
                .map(|rest| rest.trim_start_matches(',').to_owned())
        })
        .collect()
}

fn detector(station: &station::Station) -> Vec<String> {
    state_of(station, "birdnet_detector_state", "")
}

fn classifier(station: &station::Station, id: &str) -> Vec<String> {
    state_of(
        station,
        "birdnet_classifier_state",
        &format!("classifier=\"{id}\","),
    )
}

fn filter(station: &station::Station) -> Vec<String> {
    state_of(station, "birdnet_occurrence_filter_state", "")
}

fn running() -> Vec<String> {
    vec!["state=\"running\"".to_owned()]
}

fn state(code: &str) -> Vec<String> {
    vec![format!("state=\"{code}\"")]
}

// ── MDL-01: an extra classifier that will not load ─────────────────────

/// The reproduction: a working primary and a damaged second classifier.
#[test]
fn a_damaged_second_classifier_leaves_the_primary_detecting() {
    let mut dir = StationDir::new();
    dir.with_primary().with_second(
        "perch",
        Some(&truncated_model()),
        Some(&labels_text("Secundus", 11)),
    );
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(detector(&station), running());
    assert_eq!(classifier(&station, "birdnet"), state("judging"));
    assert_eq!(
        classifier(&station, "perch"),
        state("left_out_will_not_load")
    );
    assert!(
        station.log().contains("classifier `perch`"),
        "the journal names the classifier it left out:\n{}",
        station.log()
    );
    station.stop();
}

/// A second classifier whose labels file is missing.
#[test]
fn a_second_classifier_without_its_labels_leaves_the_primary_detecting() {
    let mut dir = StationDir::new();
    dir.with_primary()
        .with_second("perch", Some(&fixture_model("tiny_v30_test.onnx")), None);
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(detector(&station), running());
    assert_eq!(
        classifier(&station, "perch"),
        state("left_out_labels_missing")
    );
    station.stop();
}

/// A second classifier whose labels file is not its model's: 12 names for
/// 11 classes. It used to load and name every species by the wrong row.
#[test]
fn a_second_classifier_with_the_wrong_labels_is_left_out() {
    let mut dir = StationDir::new();
    dir.with_primary().with_second(
        "perch",
        Some(&fixture_model("tiny_v30_test.onnx")),
        Some(&labels_text("Secundus", 12)),
    );
    dir.set("MODEL_ROUTES", "local:birdnet+perch");
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(
        classifier(&station, "perch"),
        state("left_out_labels_wrong_length")
    );
    station.stop();
}

/// A second classifier whose file is not there: this one already left the
/// primary running, and said so only at INFO. Now it is published.
#[test]
fn a_missing_second_classifier_is_published_as_missing() {
    let mut dir = StationDir::new();
    dir.with_primary()
        .with_second("perch", None, Some(&labels_text("Secundus", 11)));
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(
        classifier(&station, "perch"),
        state("left_out_file_missing")
    );
    station.stop();
}

// ── MDL-16: the primary's labels do not match its model ─────────────────

/// The primary with 12 names for 11 classes is refused: the detector does
/// not start, and the station says exactly why.
#[test]
fn a_primary_with_the_wrong_labels_stops_the_detector_and_says_why() {
    let mut dir = StationDir::new();
    let model = dir.put("primary.onnx", &fixture_model("tiny_v30_test.onnx"));
    let labels = dir.put("primary_labels.txt", labels_text("Primus", 12).as_bytes());
    dir.set("MODEL_PATH", model.display())
        .set("LABELS_PATH", labels.display());
    let mut station = dir.start();

    station.hand_recording(None);
    assert_eq!(
        station.wait_for_analysed(1, NOTHING_WITHIN),
        0,
        "{}",
        station.log()
    );
    assert_eq!(detector(&station), state("failed"));
    assert_eq!(
        classifier(&station, "birdnet"),
        state("left_out_labels_wrong_length")
    );
    let log = station.log();
    assert!(
        log.contains("scores 11 classes") && log.contains("names 12"),
        "the journal names both counts:\n{log}"
    );
    station.stop();
}

/// The counterpart: the matched pair starts and analyses.
#[test]
fn a_primary_with_its_own_labels_runs() {
    let mut dir = StationDir::new();
    let model = dir.put("primary.onnx", &fixture_model("tiny_v30_test.onnx"));
    let labels = dir.put("primary_labels.txt", labels_text("Primus", 11).as_bytes());
    dir.set("MODEL_PATH", model.display())
        .set("LABELS_PATH", labels.display());
    let mut station = dir.start();

    station.hand_recording(None);
    assert_eq!(
        station.wait_for_analysed(1, NOTHING_WITHIN),
        1,
        "the same pair, matched, is analysed within the window the test above \
         waits: {}",
        station.log()
    );
    // Analysed, not necessarily detected: the tiny V3.0-shaped model scores
    // each class as one of the window's first samples, which in the first
    // six seconds of this recording are all under the station's bar.
    assert_eq!(detector(&station), running());
    assert_eq!(classifier(&station, "birdnet"), state("judging"));
    assert_eq!(filter(&station), state("off_not_configured"));
    assert_eq!(
        station.metric("birdnet_occurrence_filter_active"),
        Some(0.0)
    );
    station.stop();
}

// ── MDL-02, MDL-05: the occurrence filter's files ───────────────────────

/// The reproduction: the geomodel's labels file is missing.
#[test]
fn a_missing_filter_labels_file_turns_the_filter_off_and_keeps_detecting() {
    let mut dir = StationDir::new();
    dir.with_primary()
        .with_coordinates()
        .with_filter(Some(&fixture_model("tiny_meta_test.onnx")), None);
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(detector(&station), running());
    assert_eq!(filter(&station), state("off_labels_missing"));
    assert_eq!(
        station.metric("birdnet_occurrence_filter_active"),
        Some(0.0)
    );
    station.stop();
}

/// A damaged geomodel: off, and said to be off, and detection goes on.
#[test]
fn a_damaged_filter_model_turns_the_filter_off_and_keeps_detecting() {
    let mut dir = StationDir::new();
    let whole = fixture_model("tiny_meta_test.onnx");
    dir.with_primary()
        .with_coordinates()
        .with_filter(Some(&whole[..whole.len() / 2]), Some(TINY_META_LABELS));
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(filter(&station), state("off_model_unusable"));
    assert_eq!(
        station.metric("birdnet_occurrence_filter_active"),
        Some(0.0)
    );
    station.stop();
}

/// A geomodel path naming no file.
#[test]
fn a_missing_filter_model_turns_the_filter_off_and_keeps_detecting() {
    let mut dir = StationDir::new();
    dir.with_primary()
        .with_coordinates()
        .with_filter(None, Some(TINY_META_LABELS));
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(filter(&station), state("off_model_missing"));
    station.stop();
}

// ── MDL-04: no coordinates ──────────────────────────────────────────────

/// The reproduction: the filter is configured, the station has no position,
/// and the filter used to be reported active.
#[test]
fn a_filter_without_coordinates_is_off() {
    let mut dir = StationDir::new();
    dir.with_primary().with_filter(
        Some(&fixture_model("tiny_meta_test.onnx")),
        Some(TINY_META_LABELS),
    );
    let mut station = dir.start();

    detects(&mut station);
    assert_eq!(filter(&station), state("off_no_coordinates"));
    assert_eq!(
        station.metric("birdnet_occurrence_filter_active"),
        Some(0.0)
    );
    station.stop();
}

/// The counterpart: with a position it is on, and says how many species it
/// admits.
#[test]
fn a_filter_with_coordinates_is_on() {
    let mut dir = StationDir::new();
    dir.with_primary().with_coordinates().with_filter(
        Some(&fixture_model("tiny_meta_test.onnx")),
        Some(TINY_META_LABELS),
    );
    let mut station = dir.start();

    station.hand_recording(None);
    assert_eq!(
        station.wait_for_analysed(1, ANALYSED_WITHIN),
        1,
        "{}",
        station.log()
    );
    assert_eq!(filter(&station), state("active"));
    assert_eq!(
        station.metric("birdnet_occurrence_filter_active"),
        Some(1.0)
    );
    assert!(
        station.metric("birdnet_occurrence_candidates").is_some(),
        "{}",
        station.metrics()
    );
    station.stop();
}

// ── MDL-07: a second classifier on the only microphone ──────────────────

/// The manual's example on a one-microphone station: a route written for
/// `local`, the source its detections carry, reaches recordings whose names
/// carry no source id.
#[test]
fn a_route_for_the_local_microphone_reaches_its_recordings() {
    let mut dir = StationDir::new();
    dir.with_primary().with_second(
        "perch",
        Some(&fixture_model("tiny_v24_test.onnx")),
        Some(&labels_text("Secundus", 11)),
    );
    dir.set("MODEL_ROUTES", "local:birdnet+perch");
    let mut station = dir.start();

    let by = detects(&mut station);
    assert!(
        by.contains("perch"),
        "the second classifier judged nothing on the local microphone: {by:?}\n{}",
        station.log()
    );
    assert_eq!(classifier(&station, "perch"), state("judging"));
    station.stop();
}

/// The counterpart: with no route, the second classifier judges nothing, on
/// purpose, and the station publishes that it is loaded and idle.
#[test]
fn an_unrouted_second_classifier_is_published_as_judging_nothing() {
    let mut dir = StationDir::new();
    dir.with_primary().with_second(
        "perch",
        Some(&fixture_model("tiny_v24_test.onnx")),
        Some(&labels_text("Secundus", 11)),
    );
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    assert_eq!(classifier(&station, "perch"), state("unrouted"));
    station.stop();
}

/// A route for a named source still reaches that source's recordings, and
/// not the local microphone's.
#[test]
fn a_route_for_a_named_source_reaches_only_that_source() {
    let mut dir = StationDir::new();
    dir.with_primary().with_second(
        "perch",
        Some(&fixture_model("tiny_v24_test.onnx")),
        Some(&labels_text("Secundus", 11)),
    );
    dir.set("MODEL_ROUTES", "RTSP_1:birdnet+perch");
    let mut station = dir.start();

    assert_eq!(detects(&mut station), ["birdnet".to_owned()].into());
    station.hand_recording(Some("RTSP_1"));
    assert_eq!(station.wait_for_analysed(2, ANALYSED_WITHIN), 2);
    let before = station.detections().len();
    let deadline = std::time::Instant::now() + ANALYSED_WITHIN;
    while station.detections().len() == before && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    station.wait_for_detections(ANALYSED_WITHIN);
    assert!(
        station.detecting_classifiers().contains("perch"),
        "{}",
        station.log()
    );
    station.stop();
}

// ── the detector itself ─────────────────────────────────────────────────

/// A station with no classifier configured says the detector is not
/// configured, rather than nothing at all.
#[test]
fn a_station_with_no_classifier_says_the_detector_is_not_configured() {
    let station = StationDir::new().start();
    assert_eq!(detector(&station), state("not_configured"));
}

/// **A filter turned off in the configuration is off, and nothing tries to
/// load it.** `METADATA_MODEL_PATH=off` (or `none`, in any case, from the
/// file, the environment or the command line) is an owner running unfiltered
/// on purpose: the filter is published as not configured, the station says so
/// once at INFO, and nothing looks for a file called `off`.
///
/// Observed failing before the predicate existed: the station looked for a
/// model file named `off`, and the filter was `off_model_missing`.
#[test]
fn a_filter_turned_off_is_not_configured_and_not_looked_for() {
    for (how, value) in [("file", "off"), ("environment", "NONE")] {
        let mut dir = StationDir::new();
        dir.with_primary().with_coordinates();
        if how == "file" {
            dir.set("METADATA_MODEL_PATH", value);
        } else {
            dir.env("BIRDNET_METADATA_MODEL", value);
        }
        let mut station = dir.start();
        detects(&mut station);
        assert_eq!(
            filter(&station),
            state("off_not_configured"),
            "{how}:\n{}",
            station.log()
        );
        let log = station.log();
        assert_eq!(
            log.lines()
                .filter(|l| l.contains("turned off in the configuration"))
                .count(),
            1,
            "{how}: said once:\n{log}"
        );
        for absent in [
            "loading metadata ONNX model",
            "no metadata model configured",
            " WARN birdnet_core::detection::daemon::run: species occurrence",
        ] {
            assert!(!log.contains(absent), "{how}: {absent:?}:\n{log}");
        }
        station.stop();
    }
}
