//! A station whose models are not doing what its configuration asks says so on
//! every surface that claims to show its health (MDL-03, MDL-04, MDL-05,
//! MDL-06), started from the built binary and broken one file at a time.
//!
//! On 2026-11-25 a real station that did not start its detector answered
//! `/api/v2/health` 200 with a green header badge and nothing on Today, a
//! second classifier that was skipped was one INFO line, and the Capture
//! panel said the occurrence filter was on because the configuration named
//! one. Each test here starts the binary in one of those situations, waits for
//! its start-up to publish what became of its models, and reads what an owner
//! reads: the header badge, the Station Health banner and its species-filter
//! cell, the Today page's notices, the Capture tab's panel, and the strict and
//! plain health endpoints. Each reads the rendered region, not the document.
//!
//! The decision on `?strict=1` is the health endpoint's own (see
//! `routes/system.rs`): 503 while nothing is identified, and 200 with the
//! reason in the body for a second classifier left out or a filter that is
//! off.

mod station;

use std::time::{Duration, Instant};

use station::{StationDir, TINY_META_LABELS, fixture_model, labels_text};

/// How long a station may take to publish its detector's state.
const SETTLED_WITHIN: Duration = Duration::from_secs(60);

/// The tiny V3.0-shaped classifier, cut short: what a download interrupted
/// part-way leaves, past any size check and not a model.
fn truncated_model() -> Vec<u8> {
    let whole = fixture_model("tiny_v30_test.onnx");
    whole[..whole.len() / 2].to_vec()
}

/// What an owner reads, from one station.
struct Pages {
    /// `?strict=1`: the status code and the body.
    strict: (u16, serde_json::Value),
    /// The plain endpoint's status code.
    plain: u16,
    /// The header badge.
    badge: String,
    /// The Station Health banner.
    banner: String,
    /// The Station Health pipeline row's species-filter cell.
    filter_cell: String,
    /// The Today page's model notices, joined.
    notices: String,
    /// The Capture tab's classifier panel.
    panel: String,
}

/// Whether `text` says `words`, whatever the case of its first letter: the
/// badge and the banner join their sentences and capitalise only the first.
fn says(text: &str, words: &str) -> bool {
    text.to_lowercase().contains(&words.to_lowercase())
}

/// The model-condition keys in the strict body.
fn keys(body: &serde_json::Value) -> Vec<String> {
    body["model_conditions"]
        .as_array()
        .unwrap_or_else(|| panic!("no model_conditions: {body}"))
        .iter()
        .filter_map(|c| c["key"].as_str().map(str::to_owned))
        .collect()
}

/// `body` from the first `start` up to the first `end` after it.
fn between<'a>(body: &'a str, start: &str, end: &str) -> &'a str {
    let from = body
        .find(start)
        .unwrap_or_else(|| panic!("no {start:?} in:\n{body}"));
    let rest = &body[from..];
    &rest[..rest.find(end).map_or(rest.len(), |e| e + end.len())]
}

fn page(station: &station::Station, path: &str) -> String {
    match station.get(path) {
        Some((200, body)) => body,
        other => panic!("GET {path} answered {other:?}"),
    }
}

/// Wait until the start-up has published its detector's state, then read
/// every surface.
fn pages(station: &station::Station) -> Pages {
    let deadline = Instant::now() + SETTLED_WITHIN;
    let strict = loop {
        let (status, body) = station
            .get("/api/v2/health?strict=1")
            .unwrap_or_else(|| panic!("no answer:\n{}", station.log()));
        let body: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        if body["models"]["detector"]["state"] != "starting" {
            break (status, body);
        }
        assert!(
            Instant::now() < deadline,
            "the detector's state was never published:\n{}",
            station.log()
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    let plain = station.get("/api/v2/health").map_or(0, |(s, _)| s);
    let health = page(station, "/station");
    let nudge = page(station, "/pages/today-nudge");
    let capture = page(station, "/station/capture");
    Pages {
        strict,
        plain,
        badge: page(station, "/pages/health-badge"),
        banner: between(&health, "class=\"st-status", "</div></div></div>").to_owned(),
        filter_cell: between(&health, "Species filter</div>", "</div></div>").to_owned(),
        notices: nudge
            .split("<div class=\"x-nudge\"")
            .filter(|n| n.contains("Models banner"))
            .collect::<Vec<_>>()
            .join("\n"),
        panel: between(&capture, "data-cls-panel", "<div class=\"cls-actions\">").to_owned(),
    }
}

/// The counterpart of every test below: a station whose models run as
/// configured has no model condition anywhere, and its panel says the filter
/// is on because the station says it is filtering.
#[test]
fn a_station_whose_models_run_as_configured_says_nothing_about_them() {
    let mut dir = StationDir::new();
    dir.with_primary().with_coordinates().with_filter(
        Some(&fixture_model("tiny_meta_test.onnx")),
        Some(TINY_META_LABELS),
    );
    let mut station = dir.start();
    let p = pages(&station);
    assert_eq!(p.strict.1["models"]["detector"]["state"], "running");
    assert_eq!(keys(&p.strict.1), Vec::<String>::new(), "{}", p.strict.1);
    assert_eq!(p.notices, "", "no model notice on Today");
    for words in [
        "Nothing is being identified",
        "is not running",
        "filter is off",
    ] {
        assert!(!says(&p.banner, words), "{words}: {}", p.banner);
        assert!(!says(&p.badge, words), "{words}: {}", p.badge);
    }
    assert!(p.filter_cell.contains("on ·"), "{}", p.filter_cell);
    assert!(
        p.panel.contains("The local species filter is on"),
        "{}",
        p.panel
    );
    station.stop();
}

/// A damaged second classifier: the primary detects, and every surface says
/// which classifier is not running and why; strict stays 200.
#[test]
fn a_damaged_second_classifier_is_said_everywhere_and_is_not_a_strict_fault() {
    let mut dir = StationDir::new();
    dir.with_primary().with_second(
        "perch",
        Some(&truncated_model()),
        Some(&labels_text("Secundus", 11)),
    );
    let mut station = dir.start();
    let p = pages(&station);
    assert_eq!(p.strict.0, 200, "{}", p.strict.1);
    assert_eq!(keys(&p.strict.1), ["models:classifier:perch"]);
    assert_eq!(
        p.strict.1["model_conditions"][0]["state"],
        "left_out_will_not_load"
    );
    for (surface, text) in [
        ("badge", &p.badge),
        ("banner", &p.banner),
        ("Today", &p.notices),
    ] {
        assert!(
            says(text, "`perch` is not running") && says(text, "will not load"),
            "{surface}: {text}"
        );
    }
    station.stop();
}

/// The geomodel's labels missing: the detector runs unfiltered, and every
/// surface says the filter is off and why.
#[test]
fn a_filter_without_its_labels_is_said_off_everywhere() {
    let mut dir = StationDir::new();
    dir.with_primary()
        .with_coordinates()
        .with_filter(Some(&fixture_model("tiny_meta_test.onnx")), None);
    let mut station = dir.start();
    let p = pages(&station);
    assert_eq!(p.strict.0, 200, "{}", p.strict.1);
    assert_eq!(keys(&p.strict.1), ["models:occurrence-filter"]);
    assert_eq!(
        p.strict.1["model_conditions"][0]["state"],
        "off_labels_missing"
    );
    for (surface, text) in [
        ("badge", &p.badge),
        ("banner", &p.banner),
        ("Today", &p.notices),
        ("panel", &p.panel),
    ] {
        assert!(
            says(text, "filter is off") && says(text, "geo_labels-not-there.txt"),
            "{surface}: {text}"
        );
    }
    assert!(
        p.filter_cell.contains("its labels file is missing"),
        "{}",
        p.filter_cell
    );
    station.stop();
}

/// A filter with no coordinates (MDL-04): off, with the reason and the way to
/// set the location; the panel does not call it on.
#[test]
fn a_filter_without_coordinates_is_said_off_with_the_way_to_fix_it() {
    let mut dir = StationDir::new();
    dir.with_primary().with_filter(
        Some(&fixture_model("tiny_meta_test.onnx")),
        Some(TINY_META_LABELS),
    );
    let mut station = dir.start();
    let p = pages(&station);
    assert_eq!(p.strict.0, 200, "{}", p.strict.1);
    assert_eq!(
        p.strict.1["model_conditions"][0]["state"],
        "off_no_coordinates"
    );
    assert!(
        p.notices.contains("/station/capture#set-location"),
        "Today offers the location form: {}",
        p.notices
    );
    assert!(
        p.filter_cell.contains("no coordinates"),
        "{}",
        p.filter_cell
    );
    assert!(
        !p.panel.contains("filter is on") && p.panel.contains("filter is off"),
        "the panel says what the station does, not what it is configured with: {}",
        p.panel
    );
    for (surface, text) in [("badge", &p.badge), ("banner", &p.banner)] {
        assert!(says(text, "filter is off"), "{surface}: {text}");
    }
    station.stop();
}

/// A primary whose labels are the wrong length: the detector does not start,
/// strict is 503 with the reason, plain stays 200, and every surface says
/// nothing is being identified and why.
#[test]
fn a_detector_that_did_not_start_is_a_strict_fault_and_said_everywhere() {
    let mut dir = StationDir::new();
    let model = dir.put("primary.onnx", &fixture_model("tiny_v30_test.onnx"));
    let labels = dir.put("primary_labels.txt", labels_text("Primus", 12).as_bytes());
    dir.set("MODEL_PATH", model.display())
        .set("LABELS_PATH", labels.display());
    let mut station = dir.start();
    let p = pages(&station);
    assert_eq!(p.strict.0, 503, "{}", p.strict.1);
    assert_eq!(p.plain, 200, "plain stays liveness");
    assert_eq!(keys(&p.strict.1), ["models:detector"]);
    assert_eq!(p.strict.1["model_conditions"][0]["state"], "failed");
    for (surface, text) in [
        ("badge", &p.badge),
        ("banner", &p.banner),
        ("Today", &p.notices),
    ] {
        assert!(
            says(text, "nothing is being identified") && says(text, "names 12"),
            "{surface}: {text}"
        );
    }
    station.stop();
}
