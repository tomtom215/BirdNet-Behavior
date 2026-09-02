//! The corroboration filter's model of a "neighbourhood", checked against the
//! chunk timeline the real audio pipeline actually produces.
//!
//! # Why this exists
//!
//! [`ConfirmationLevel::required_confirmations_at`] and everything built on it
//! ([`ConfirmationLevel::minimum_overlap`], the startup warning, the `--doctor`
//! check) rest on one arithmetic claim: that a station whose windows are
//! `chunk_secs` long and advance by `chunk_secs - overlap_secs` has
//!
//! ```text
//! (REFERENCE_SPAN / 2 / step) * 2 + 1
//! ```
//!
//! windows within half a span either side of any given one. That is a claim
//! about `pipeline::process_file`, not about arithmetic, and nothing checked
//! it — the numbers in the module's own documentation were derived by reading
//! the formula, which is precisely the way this repository has been wrong
//! before.
//!
//! So this decodes a real recording, at four overlaps, and counts.
//!
//! It is also the only gate that exercises the corroboration filter over a
//! timeline the pipeline produced rather than one a test invented, which is
//! where a units mix-up (samples for seconds, or `end_secs` for `start_secs`)
//! would show up.

use std::path::{Path, PathBuf};

use birdnet_core::detection::corroboration::{
    ConfirmationLevel, REFERENCE_SPAN, corroborate, required_confirmations,
};
use birdnet_core::detection::pipeline::{self, PipelineConfig};
use birdnet_core::detection::types::Detection;

/// The bundled 30-second recording, staged under the capture-style name the
/// pipeline requires.
fn staged(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/testdata/Pica_pica_30s.wav");
    let dst = dir.join("2026-05-19-birdnet-06:30:00.wav");
    std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("stage {}: {e}", src.display()));
    dst
}

/// Chunk start times, in seconds, as the pipeline emits them at this overlap.
fn real_starts(overlap: f32) -> (Vec<f32>, f32) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = staged(dir.path());
    let cfg = PipelineConfig {
        watch_dir: dir.path().to_path_buf(),
        chunk_overlap_secs: overlap,
        // The chunk boundaries are computed before, and independently of, the
        // spectrogram — `process_file` derives `start_secs` from the sample
        // position and then decides what to store. Taking the raw-audio branch
        // therefore exercises the same boundary arithmetic while skipping a mel
        // transform per chunk, which is the difference between this file
        // costing a second and costing a minute and a half. Checked rather than
        // assumed: a probe ran both branches at all four overlaps and the start
        // vectors were identical.
        raw_audio_input: true,
        ..PipelineConfig::default()
    };
    let chunks = pipeline::process_file(&path, &cfg).expect("the bundled recording must decode");
    (
        chunks.iter().map(|c| c.start_secs).collect(),
        cfg.chunk_duration_secs,
    )
}

/// The neighbourhood the filter will actually see around a mid-file chunk.
fn measured_neighbourhood(starts: &[f32]) -> usize {
    let here = starts[starts.len() / 2];
    starts
        .iter()
        .filter(|s| (**s - here).abs() <= REFERENCE_SPAN / 2.0)
        .count()
}

#[test]
fn the_modelled_neighbourhood_matches_the_pipeline() {
    // Mid-file so the window is not clipped by the start or end of the
    // recording — which is what `required_confirmations_at` describes, and is
    // the only place the formula is meant to be exact.
    for overlap in [0.0_f32, 1.5, 2.0, 2.5] {
        let (starts, chunk_secs) = real_starts(overlap);
        let measured = measured_neighbourhood(&starts);

        // Every level must agree, because they share the neighbourhood and
        // differ only in the fraction of it they demand.
        for level in [
            ConfirmationLevel::Lenient,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Strict,
        ] {
            assert_eq!(
                level.required_confirmations_at(overlap, chunk_secs),
                required_confirmations(level, measured),
                "at overlap {overlap}s the pipeline emits {} chunks, {measured} of them \
                 within {REFERENCE_SPAN}s of each other, but {} models a different \
                 neighbourhood — every number this filter reports to an operator is \
                 derived from that model",
                starts.len(),
                level.as_str(),
            );
        }
    }
}

#[test]
fn minimum_overlap_is_the_boundary_it_claims_to_be() {
    let (_, chunk_secs) = real_starts(0.0);
    for level in [
        ConfirmationLevel::Lenient,
        ConfirmationLevel::Moderate,
        ConfirmationLevel::Balanced,
        ConfirmationLevel::Strict,
    ] {
        let need = level
            .minimum_overlap(chunk_secs)
            .expect("an enabled level always has one");

        assert!(
            level.required_confirmations_at(need, chunk_secs) >= 2,
            "{} claims {need}s is enough overlap to demand a second opinion, but at \
             {need}s it still demands only itself",
            level.as_str()
        );
        // The counterpart, and the half that makes it a boundary rather than
        // just some sufficient value: one tenth less must not be enough.
        if need > 0.0 {
            assert_eq!(
                level.required_confirmations_at(need - 0.1, chunk_secs),
                1,
                "{} reports {need}s as its *minimum*, but {}s already works — the \
                 `--doctor` advice would be telling operators to overlap more than \
                 they need to",
                level.as_str(),
                need - 0.1
            );
        }
    }
}

#[test]
fn the_filter_cuts_exactly_where_the_arithmetic_says_on_a_real_timeline() {
    // The filter's purpose, over chunk starts the pipeline produced, pinned at
    // the boundary rather than at the extremes.
    //
    // A first version of this test used a species present in *every* chunk
    // against one present in *exactly one*, and it was worthless: it stayed
    // green when `corroborate`'s half-span was doubled, and green again when
    // the neighbourhood was shifted a whole chunk off the timeline. Both are
    // real defects; neither can be seen from the extremes, because a species
    // in every window clears any bar and a species in one window clears none.
    //
    // 2.0 s overlap steps the windows by 1 s, so the neighbourhood is seven
    // windows wide and `Balanced` asks for four of them. A four-window burst is
    // therefore the first length that survives, and a three-window burst the
    // last that does not.
    let overlap = 2.0;
    let (starts, chunk_secs) = real_starts(overlap);
    assert_eq!(
        ConfirmationLevel::Balanced.required_confirmations_at(overlap, chunk_secs),
        4,
        "the premise of the burst lengths below: {starts:?}"
    );
    assert!(
        starts.len() > 24,
        "needs a long recording; got {}",
        starts.len()
    );

    // Three species, chosen so the answer differs for each.
    let long_burst = 10..=13; // four windows: survives, everywhere
    let short_burst = 18..=20; // three windows: dropped, everywhere
    let one_off = 25; // a single window: dropped

    let predictions: Vec<Vec<Detection>> = starts
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut chunk = Vec::new();
            if long_burst.contains(&i) {
                chunk.push(det("Pica pica", *s, chunk_secs));
            }
            if short_burst.contains(&i) {
                chunk.push(det("Turdus merula", *s, chunk_secs));
            }
            if i == one_off {
                chunk.push(det("Bubo bubo", *s, chunk_secs));
            }
            chunk
        })
        .collect();

    let out = corroborate(ConfirmationLevel::Balanced, &starts, &predictions);
    let kept = |sci: &str| {
        out.iter()
            .flatten()
            .filter(|d| d.scientific_name == sci)
            .count()
    };

    assert_eq!(
        kept("Pica pica"),
        4,
        "a four-window burst meets the bar in every one of its windows and must \
         survive whole; kept {} of 4",
        kept("Pica pica")
    );
    assert_eq!(
        kept("Turdus merula"),
        0,
        "a three-window burst is one short of the bar and must go entirely; kept {}",
        kept("Turdus merula")
    );
    assert_eq!(
        kept("Bubo bubo"),
        0,
        "a single-window artefact was recorded anyway"
    );
}

fn det(sci: &str, start: f32, chunk_secs: f32) -> Detection {
    Detection {
        date: "2026-05-19".into(),
        time: "06:30:00".into(),
        scientific_name: sci.into(),
        common_name: sci.into(),
        confidence: 0.9,
        start,
        stop: start + chunk_secs,
        week: 20,
        file_name_extr: None,
    }
}
