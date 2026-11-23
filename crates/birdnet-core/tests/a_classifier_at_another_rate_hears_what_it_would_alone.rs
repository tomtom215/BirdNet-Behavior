//! A classifier at another sample rate than the primary's is fed, chunk for
//! chunk, exactly the audio it would be fed as a station's only classifier
//! (MMP-01).
//!
//! # Why this exists
//!
//! The registry used to refuse a 32 kHz classifier (Perch v2) beside the
//! default 48 kHz primary (BirdNET V2.4), and the refusal stopped detection.
//! The pipeline now resamples each recording once more per extra rate and cuts
//! a copy of every chunk at that rate. "The daemon starts and both classifiers
//! answer" says nothing about *what* the second one heard: a copy cut from the
//! primary's 48 kHz samples, or at a start scaled by the wrong ratio, would
//! answer just as readily. So this compares the copy with the chunks a
//! pipeline whose only rate is 32 kHz cuts from the same recording, sample for
//! sample.

use std::path::{Path, PathBuf};

use birdnet_core::detection::pipeline::{self, PipelineConfig, PreparedChunk};

/// The bundled 30-second recording, staged under the capture-style name the
/// pipeline requires.
fn staged(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/testdata/Pica_pica_30s.wav");
    let dst = dir.join("2026-05-19-birdnet-06:30:00.wav");
    std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("stage {}: {e}", src.display()));
    dst
}

/// The chunks of the staged recording under `rate` as the primary's rate, cut
/// to Perch's 5 s window and stepped by V2.4's 3 s, as the daemon fits them
/// for that pair.
fn chunks(path: &Path, rate: u32, extra: Vec<u32>) -> Vec<PreparedChunk> {
    let cfg = PipelineConfig {
        watch_dir: path.parent().unwrap().to_path_buf(),
        target_sample_rate: rate,
        chunk_duration_secs: 5.0,
        chunk_overlap_secs: 0.0,
        chunk_step_secs: Some(3.0),
        raw_audio_input: true,
        extra_sample_rates: extra,
        ..PipelineConfig::default()
    };
    pipeline::process_file(path, &cfg).expect("the bundled recording must decode")
}

/// Observed failing two ways, each a mutant of `process_file`'s copy: cutting
/// the 32 kHz copy from the primary's 48 kHz samples (`&samples` in place of
/// the extra resample) failed at chunk 0, and starting it at the primary's
/// sample index unscaled (`pos` in place of `scale(pos)` in `cut_at_rate`)
/// failed at chunk 1.
#[test]
fn the_copy_at_another_rate_is_what_that_rate_alone_would_cut() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = staged(dir.path());

    let beside_v24 = chunks(&path, 48_000, vec![32_000]);
    let alone = chunks(&path, 32_000, Vec::new());

    assert!(alone.len() > 2, "several chunks, got {}", alone.len());
    assert_eq!(
        beside_v24.len(),
        alone.len(),
        "one timeline, one chunk count"
    );
    for (i, (ours, theirs)) in beside_v24.iter().zip(&alone).enumerate() {
        assert!(
            (ours.start_secs - theirs.start_secs).abs() < 1e-4,
            "chunk {i} starts at {} beside V2.4 and {} alone",
            ours.start_secs,
            theirs.start_secs
        );
        let copy = ours
            .samples_at(32_000)
            .unwrap_or_else(|| panic!("chunk {i} has no 32 kHz copy"));
        assert_eq!(
            copy.len(),
            theirs.spectrogram.data.len(),
            "chunk {i} length"
        );
        assert!(
            copy == theirs.spectrogram.data.as_slice(),
            "chunk {i}: the 32 kHz copy differs from what a 32 kHz pipeline cuts"
        );
    }
}

/// Counterpart: with no extra rate the pipeline prepares no copy, so a
/// one-rate station does no second resample and carries no second buffer.
#[test]
fn no_extra_rate_prepares_no_copy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = staged(dir.path());
    let only = chunks(&path, 48_000, Vec::new());
    assert!(!only.is_empty());
    assert!(only.iter().all(|c| c.at_rates.is_empty()));
    assert!(only.iter().all(|c| c.samples_at(32_000).is_none()));
}
