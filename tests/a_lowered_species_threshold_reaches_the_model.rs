//! A per-species threshold below the global one must reach the classifier.
//!
//! The classifier discards every score below its own threshold inside
//! `predict_chunk`; the processor that applies per-species thresholds only
//! ever sees what survived. The daemon ran the model at the global confidence,
//! so lowering one species under it — the tuning guide's "a rare bird… you can
//! lower its threshold" — changed nothing: every detection it existed to
//! recover was dropped inside the model.
//!
//! The daemon now runs each classifier at the lower of its own threshold and
//! a floor the processor publishes (`ThresholdFloor`, `apply_threshold_floor`).
//! This gate drives the real 11 K-species model over the bundled Eurasian
//! Magpie recording, where the magpie scores ~0.93: a model loaded at 0.95
//! misses it, and the same model with a 0.5 floor applied finds it.
//!
//! Model-gated like `inference_e2e`: listed in the CI inference job, which
//! exports `BIRDNET_REQUIRE_MODEL=1`, so a skip there is a failure.

mod common;

use std::path::Path;

use birdnet_core::detection::daemon::{
    ThresholdFloor, apply_threshold_floor, loaded_thresholds, process_and_infer_filtered,
};
use birdnet_core::detection::privacy::PrivacyFilter;
use birdnet_core::detection::{ChunkFilters, noise::NoiseFilter};
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::model::{BirdNetModel, ModelConfig};
use birdnet_core::inference::registry::ClassifierRegistry;
use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig};

const PICA_PICA_WAV: &str = "tests/testdata/Pica_pica_30s.wav";

fn magpies(registry: &mut ClassifierRegistry, dir: &Path) -> usize {
    let model = &registry.primary().model;
    let mut pipeline = birdnet_core::detection::pipeline::PipelineConfig::default();
    let rate = model.infer_sample_rate();
    pipeline.target_sample_rate = rate;
    pipeline.raw_audio_input = rate == 32_000;
    pipeline.chunk_duration_secs = model.recommended_chunk_secs();

    let path = dir.join("2026-05-19-birdnet-06:30:00.wav");
    std::fs::copy(PICA_PICA_WAV, &path).expect("stage the bundled recording");
    let filters = ChunkFilters {
        privacy: PrivacyFilter::new(0.0),
        noise: NoiseFilter::with_default_classes(0.0),
        confirmation: birdnet_core::detection::corroboration::ConfirmationLevel::Off,
    };
    let mut species = SpeciesFilter::new_passthrough(SpeciesFilterConfig::default());
    process_and_infer_filtered(
        &path,
        &pipeline,
        registry,
        &ClassifierRegistry::default_route(),
        &filters,
        &mut species,
        None,
        None,
        None,
        "threshold-floor-e2e",
    )
    .expect("processing the bundled recording")
    .iter()
    .filter(|e| e.detection.scientific_name == "Pica pica")
    .count()
}

#[test]
fn a_lowered_threshold_reaches_a_model_loaded_above_it() {
    let Some((model_path, labels_path)) = common::model_paths() else {
        return;
    };
    let labels = LabelSet::load(&labels_path).expect("labels");
    let model = BirdNetModel::load(
        &model_path,
        labels,
        ModelConfig {
            confidence_threshold: 0.95,
            ..ModelConfig::default()
        },
    )
    .expect("model");
    let mut registry = ClassifierRegistry::single("birdnet", model);
    let base = loaded_thresholds(&registry);
    assert_eq!(base, vec![0.95]);
    let dir = tempfile::tempdir().expect("tempdir");

    // Precondition: at its own 0.95 the model discards the ~0.93 magpie. If it
    // did not, the rest of this test would prove nothing.
    apply_threshold_floor(&mut registry, &base, None);
    assert_eq!(
        magpies(&mut registry, dir.path()),
        0,
        "the precondition failed"
    );

    // A species lowered to 0.5 publishes a 0.5 floor; the model now runs there.
    let floor = ThresholdFloor::new(0.5);
    apply_threshold_floor(&mut registry, &base, Some(&floor));
    let found = magpies(&mut registry, dir.path());
    assert!(
        found >= 3,
        "the lowered threshold did not reach the model: {found} magpies"
    );

    // And back: raising the floor above the model's own threshold never lifts
    // the model above what it was loaded at.
    floor.set(0.99);
    apply_threshold_floor(&mut registry, &base, Some(&floor));
    assert_eq!(registry.primary().model.config().confidence_threshold, 0.95);
}
