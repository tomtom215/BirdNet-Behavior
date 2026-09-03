//! End-to-end proof that the operator's species lists actually suppress birds.
//!
//! The lists at `/admin/species` were maintained, confirmed on save, and given
//! their own preview page — and reached no detection. `build_species_filter_config`
//! took everything but `sf_thresh` from `Default`, so `include_list` and
//! `exclude_list` arrived empty and nothing in production ever populated them.
//! An excluded species kept being recorded, counted, notified on and uploaded.
//!
//! Two layers, matching `tests/pipeline_e2e.rs`:
//!
//! 1. **CI-runnable** (always on): the full wiring chain — a row written the way
//!    the admin page writes it, read back through the same function the daemon
//!    uses, into the real `SpeciesFilter` — asserting the species is gone from
//!    the allowed set. No 541 MB model needed, so the regression is caught on
//!    every commit.
//! 2. **Model-gated** (skipped unless `BIRDNET_TEST_MODEL` / `BIRDNET_TEST_LABELS`
//!    are set): drives the bundled Eurasian Magpie recording through
//!    `process_and_infer_filtered` — the exact function the daemon calls —
//!    twice, and asserts the Magpie is detected without an exclude list and
//!    absent with one.

mod common;

use std::path::Path;

use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig, SpeciesLists};
use birdnet_db::settings::{SettingsCategory, ensure_settings_table, set};
use birdnet_web::routes::admin::species::handler::configured_species_lists;
use birdnet_web::state::AppState;

const PICA_PICA_WAV: &str = "tests/testdata/Pica_pica_30s.wav";

/// Fresh `AppState` over an in-memory database carrying the full schema.
fn fresh_state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// Write a settings row exactly as `/admin/species` does — one comma-separated
/// string under `species_exclude` / `species_include`.
fn store_list(state: &AppState, key: &str, value: &str) {
    state.with_db(|conn| {
        ensure_settings_table(conn).unwrap();
        set(conn, key, value, SettingsCategory::Species).unwrap();
    });
}

fn test_labels() -> LabelSet {
    LabelSet::from_entries(vec![
        ("Pica pica".into(), "Eurasian Magpie".into()),
        ("Turdus merula".into(), "Eurasian Blackbird".into()),
        ("Erithacus rubecula".into(), "European Robin".into()),
        ("Homo sapiens".into(), "Human".into()),
    ])
}

/// Build the filter the daemon would build for this station.
fn filter_for(state: &AppState) -> SpeciesFilter {
    let lists: SpeciesLists = configured_species_lists(state);
    SpeciesFilter::new_passthrough(SpeciesFilterConfig {
        include_list: lists.include,
        exclude_list: lists.exclude,
        ..SpeciesFilterConfig::default()
    })
}

// ---------------------------------------------------------------------------
// Layer 1 — the wiring, no model required
// ---------------------------------------------------------------------------

#[test]
fn an_excluded_species_is_dropped_from_the_allowed_set() {
    let state = fresh_state();
    // The page collects *common* names, so that is what a real station stores.
    store_list(&state, "species_exclude", "Human");

    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &test_labels())
        .expect("passthrough filter cannot fail");

    assert!(
        !allowed.contains("Homo sapiens"),
        "a species excluded on /admin/species must not reach the detection path"
    );
    assert!(allowed.contains("Pica pica"), "others must be unaffected");
    assert_eq!(allowed.len(), 3);
}

#[test]
fn exclusion_works_on_a_station_with_no_coordinates() {
    // The metadata model needs a location; the operator's instruction does not.
    // A station that never set a latitude used to keep recording every species
    // its operator had asked to suppress, because the caller skipped the whole
    // filter when either coordinate was missing.
    let state = fresh_state();
    store_list(&state, "species_exclude", "Human");

    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(None, 20, &test_labels())
        .expect("passthrough filter cannot fail");

    assert!(!allowed.contains("Homo sapiens"));
    assert_eq!(allowed.len(), 3);
}

#[test]
fn a_scientific_name_entered_by_hand_also_excludes() {
    // The per-species *threshold* control on the same page takes scientific
    // names, so operators reasonably type either form here.
    let state = fresh_state();
    store_list(&state, "species_exclude", "Homo sapiens");

    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &test_labels())
        .expect("passthrough filter cannot fail");

    assert!(!allowed.contains("Homo sapiens"));
}

#[test]
fn an_include_list_narrows_the_station_to_those_species() {
    let state = fresh_state();
    store_list(&state, "species_include", "Eurasian Magpie, Turdus merula");

    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &test_labels())
        .expect("passthrough filter cannot fail");

    assert_eq!(allowed.len(), 2);
    assert!(allowed.contains("Pica pica"));
    assert!(allowed.contains("Turdus merula"));
}

#[test]
fn an_all_typos_include_list_does_not_silence_the_station() {
    // The destructive reading of a typo. Intersecting with an empty resolved
    // set would suppress every detection the station makes — one misspelt name
    // must not take a station off the air.
    let state = fresh_state();
    store_list(&state, "species_include", "Eurasian Magpiee");

    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &test_labels())
        .expect("passthrough filter cannot fail");

    assert_eq!(allowed.len(), 4, "a typo must not suppress everything");
}

#[test]
fn a_trailing_comma_does_not_exclude_everything() {
    // `parse_list` drops blanks, and `matches_species` refuses to match an
    // empty entry — belt and braces, because a blank matching every species
    // would blank the station.
    let state = fresh_state();
    store_list(&state, "species_exclude", "Human,, ,");

    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &test_labels())
        .expect("passthrough filter cannot fail");

    assert_eq!(allowed.len(), 3);
    assert!(!allowed.contains("Homo sapiens"));
}

#[test]
fn a_list_change_applies_without_rebuilding_the_filter() {
    // The live-reload contract the daemon's TTL refresh relies on: the same
    // filter instance must honour a changed list, because excluding a species
    // is something an operator does while it is spamming them.
    let state = fresh_state();
    let mut filter = filter_for(&state);
    let labels = test_labels();

    assert_eq!(
        filter
            .filter_species(Some((42.36, -71.06)), 20, &labels)
            .unwrap()
            .len(),
        4
    );

    // Operator adds an exclusion on /admin/species …
    store_list(&state, "species_exclude", "Human");
    // … and the daemon's next refresh pushes it into the running filter.
    let fresh = configured_species_lists(&state);
    filter.set_lists(fresh.include, fresh.exclude);

    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &labels)
        .unwrap();
    assert!(
        !allowed.contains("Homo sapiens"),
        "the change must apply to the next file, not the next restart"
    );
}

#[test]
fn no_lists_configured_leaves_every_species_allowed() {
    let state = fresh_state();
    let mut filter = filter_for(&state);
    let allowed = filter
        .filter_species(Some((42.36, -71.06)), 20, &test_labels())
        .unwrap();
    assert_eq!(allowed.len(), 4);
}

// ---------------------------------------------------------------------------
// Layer 2 — the real detection path, model-gated
// ---------------------------------------------------------------------------

/// Load the real model + labels from env, or `None` to skip — same contract as
/// `tests/pipeline_e2e.rs`.
fn load_model() -> Option<birdnet_core::inference::model::BirdNetModel> {
    use birdnet_core::inference::model::{BirdNetModel, ModelConfig};

    let (model_path, labels_path) = common::model_paths()?;
    let labels = LabelSet::load(&labels_path).expect("failed to load labels");
    let config = ModelConfig {
        confidence_threshold: 0.1,
        ..ModelConfig::default()
    };
    Some(BirdNetModel::load(&model_path, labels, config).expect("failed to load model"))
}

/// A pipeline config matching the loaded model, the way `run_daemon` builds one.
fn pipeline_for(
    model: &birdnet_core::inference::model::BirdNetModel,
) -> birdnet_core::detection::pipeline::PipelineConfig {
    let mut pipeline = birdnet_core::detection::pipeline::PipelineConfig::default();
    let rate = model.infer_sample_rate();
    pipeline.target_sample_rate = rate;
    pipeline.raw_audio_input = rate == 32_000;
    pipeline.chunk_duration_secs = model.recommended_chunk_secs();
    pipeline
}

/// Copy the bundled recording under the capture-style name the pipeline
/// requires (`YYYY-MM-DD-birdnet-HH:MM:SS.wav`), which is how a real captured
/// segment reaches the daemon.
/// Stage the bundled recording under a name the daemon can parse.
///
/// The date in this name is load-bearing: since the `week` parameter was
/// removed, `process_and_infer_filtered` derives the geomodel week from the
/// recording's own filename. 19 May is week 19 of the 48-week year — the
/// literal `20` this call site used to pass was not that week, and nothing
/// noticed, because the daemon passed `0` here regardless.
fn staged_recording(dir: &Path) -> std::path::PathBuf {
    let staged = dir.join("2026-05-19-birdnet-06:30:00.wav");
    std::fs::copy(PICA_PICA_WAV, &staged).expect("stage the bundled recording");
    staged
}

/// Run the bundled Magpie recording through the daemon's own filtered
/// processing function and report the common names it detected.
fn detect_with_exclusions(exclude: Vec<String>) -> Option<Vec<String>> {
    use birdnet_core::detection::daemon::process_and_infer_filtered;
    use birdnet_core::detection::privacy::PrivacyFilter;
    use birdnet_core::detection::{ChunkFilters, noise::NoiseFilter};

    let mut model = load_model()?;
    let pipeline = pipeline_for(&model);
    let chunk_filters = ChunkFilters {
        privacy: PrivacyFilter::new(0.0),
        noise: NoiseFilter::with_default_classes(0.0),
        confirmation: birdnet_core::detection::corroboration::ConfirmationLevel::Off,
    };
    let mut filter = SpeciesFilter::new_passthrough(SpeciesFilterConfig {
        exclude_list: exclude,
        ..SpeciesFilterConfig::default()
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let path = staged_recording(dir.path());

    let events = process_and_infer_filtered(
        &path,
        &pipeline,
        &mut model,
        &chunk_filters,
        &mut filter,
        None,
        Some(42.36),
        Some(-71.06),
        "species-filter-e2e",
    )
    .expect("processing the bundled recording must not fail");

    Some(
        events
            .into_iter()
            .map(|e| e.detection.common_name)
            .collect(),
    )
}

#[test]
fn model_gated_excluded_species_never_becomes_a_detection() {
    // Baseline: without an exclude list the Magpie is detected. If this does
    // not hold the second half proves nothing, so assert it rather than
    // assuming it.
    let Some(baseline) = detect_with_exclusions(Vec::new()) else {
        return;
    };
    assert!(
        baseline.iter().any(|n| n == "Eurasian Magpie"),
        "the bundled Magpie recording should detect a Magpie; got {baseline:?}"
    );

    // Now exclude it the way an operator would, by common name.
    let filtered = detect_with_exclusions(vec!["Eurasian Magpie".to_string()])
        .expect("the model was available a moment ago");
    assert!(
        !filtered.iter().any(|n| n == "Eurasian Magpie"),
        "an excluded species must produce no detection event at all; got {filtered:?}"
    );
}

#[test]
fn model_gated_exclusion_also_accepts_a_scientific_name() {
    let Some(baseline) = detect_with_exclusions(Vec::new()) else {
        return;
    };
    assert!(baseline.iter().any(|n| n == "Eurasian Magpie"));

    let filtered = detect_with_exclusions(vec!["Pica pica".to_string()])
        .expect("the model was available a moment ago");
    assert!(
        !filtered.iter().any(|n| n == "Eurasian Magpie"),
        "a scientific-name exclusion must work too; got {filtered:?}"
    );
}
