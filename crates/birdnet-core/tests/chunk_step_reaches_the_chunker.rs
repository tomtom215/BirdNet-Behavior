//! The chunk step the daemon computes, checked against the timeline
//! `pipeline::process_file` actually produces (`G-10` Stage 4).
//!
//! # Why this exists
//!
//! The unit tests beside `chunk_step_samples` prove the arithmetic: given a
//! step, which spans a classifier sees, and that stepping by the shortest
//! window leaves no gap. They prove nothing about whether the chunker *uses*
//! that number — `process_file` computed the step inline until this change, and
//! a version that kept doing so would pass every one of them while the station
//! chunked exactly as before.
//!
//! So this decodes a real recording and counts, in the same spirit as
//! `confirmation_premise.rs`: the claim is about `process_file`, not about
//! arithmetic.

use std::path::{Path, PathBuf};

use birdnet_core::detection::pipeline::{self, PipelineConfig};

/// Perch v2's window at 32 kHz — the longest, so the chunk length.
const LONGEST_SECS: f32 = 5.0;
/// BirdNET+ V3.0's window at 32 kHz — the shortest, so the step.
const SHORTEST_SECS: f32 = 4.5;

/// The bundled 30-second recording, staged under the capture-style name the
/// pipeline requires.
fn staged(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/testdata/Pica_pica_30s.wav");
    let dst = dir.join("2026-05-19-birdnet-06:30:00.wav");
    std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("stage {}: {e}", src.display()));
    dst
}

/// `(start_secs, end_secs)` for every chunk the pipeline emits under `step`.
///
/// The raw-audio branch is taken deliberately: `process_file` fixes the chunk
/// boundaries from sample positions before it decides what to store, so this
/// exercises the same arithmetic without a mel transform per chunk.
fn spans(step: Option<f32>) -> Vec<(f32, f32)> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = staged(dir.path());
    let cfg = PipelineConfig {
        watch_dir: dir.path().to_path_buf(),
        target_sample_rate: 32_000,
        chunk_duration_secs: LONGEST_SECS,
        chunk_overlap_secs: 0.0,
        chunk_step_secs: step,
        raw_audio_input: true,
        ..PipelineConfig::default()
    };
    pipeline::process_file(&path, &cfg)
        .expect("the bundled recording must decode")
        .iter()
        .map(|c| (c.start_secs, c.end_secs))
        .collect()
}

/// **The step reaches the chunker.** With two classifiers the chunk is 5.0 s
/// and the step is 4.5 s, so chunk *n* starts at 4.5 *n* — not 5.0 *n*, which
/// is what the length alone would give and what this file exists to
/// distinguish.
///
/// Observed failing with `process_file` computing the step inline again
/// (`chunk_samples.saturating_sub(overlap_samples).max(1)` in place of the
/// `chunk_step_samples(config)` call): starts came back 5.0 s apart and the
/// second assertion went red at chunk 1.
#[test]
fn the_configured_step_is_what_the_chunker_advances_by() {
    let with_step = spans(Some(SHORTEST_SECS));
    assert!(
        with_step.len() > 1,
        "a 30 s recording must yield several chunks, got {}",
        with_step.len()
    );
    for (i, pair) in with_step.windows(2).enumerate() {
        let advance = pair[1].0 - pair[0].0;
        assert!(
            (advance - SHORTEST_SECS).abs() < 1e-3,
            "chunk {i} → {} advanced {advance} s, expected {SHORTEST_SECS}",
            i + 1
        );
    }
}

/// Its counterpart: **without a step the chunker is untouched.** Every station
/// running one classifier passes `None`, and must get exactly the timeline it
/// got before Stage 4: starts 5.0 s apart, and so fewer chunks over the same
/// recording than the 4.5 s step produces.
#[test]
fn no_configured_step_leaves_the_old_timeline_exactly() {
    let without = spans(None);
    for (i, pair) in without.windows(2).enumerate() {
        let advance = pair[1].0 - pair[0].0;
        assert!(
            (advance - LONGEST_SECS).abs() < 1e-3,
            "chunk {i} → {} advanced {advance} s, expected {LONGEST_SECS}",
            i + 1
        );
    }
    assert!(
        spans(Some(SHORTEST_SECS)).len() > without.len(),
        "the shorter step must produce more chunks over the same recording"
    );
}

/// **What the step is for.** A classifier whose window is the shortest reads a
/// 4.5 s prefix of each 5.0 s chunk; those prefixes must tile the recording
/// with no unheard audio between them. Stepping by the chunk length instead
/// would leave half a second unheard per chunk, which is the defect the whole
/// arithmetic exists to prevent — asserted here against the real timeline
/// rather than a modelled one.
#[test]
fn the_shortest_window_classifier_hears_the_whole_recording() {
    let with_step = spans(Some(SHORTEST_SECS));
    let heard: Vec<(f32, f32)> = with_step
        .iter()
        .map(|(s, e)| (*s, (s + SHORTEST_SECS).min(*e)))
        .collect();
    for (i, pair) in heard.windows(2).enumerate() {
        assert!(
            pair[1].0 <= pair[0].1 + 1e-3,
            "gap between chunk {i} (heard to {}) and {} (starts {})",
            pair[0].1,
            i + 1,
            pair[1].0
        );
    }

    // And the same classifier under the naive step would have had one.
    let naive = spans(None);
    let naive_heard: Vec<(f32, f32)> = naive
        .iter()
        .map(|(s, e)| (*s, (s + SHORTEST_SECS).min(*e)))
        .collect();
    assert!(
        naive_heard.windows(2).any(|w| w[1].0 > w[0].1 + 1e-3),
        "stepping by the chunk length must leave the 4.5 s classifier a gap, \
         or this test is not describing the problem"
    );
}
