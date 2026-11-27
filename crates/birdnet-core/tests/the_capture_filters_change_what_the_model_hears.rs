//! ACC-06: the default capture filters change the samples the classifier is
//! given, and by how much, on the bundled magpie recording.
//!
//! The defaults (`AudioPipeline::default()`) are a 5 Hz DC block and a 120 Hz
//! high-pass. A file dropped into the watch directory skips capture, which is
//! how every parity measurement was made; a microphone or a camera does not.
//! This measures the change to the V2.4 model's input (raw 48 kHz samples,
//! 3 s windows) for both backends' filters, prints it, and holds it to the
//! values first measured here, so a change to either filter shows up as a
//! changed number rather than as nothing. The effect on the model's *scores*
//! needs the real model and is in `docs/book/guides/parity.md`; it is not
//! re-measured here.

use std::path::Path;

use birdnet_core::audio::capture::types::{AudioPipeline, DC_BLOCK_CUTOFF_HZ, HIGH_PASS_CUTOFF_HZ};
use birdnet_core::audio::eq::EqChain;
use birdnet_core::detection::conditioning::{
    CaptureBackend, InputChange, OnePoleHighPass, describe,
};

const RATE: u32 = 48_000;

fn magpie_at_48k() -> Vec<f32> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/testdata/Pica_pica_30s.wav");
    let audio = birdnet_core::audio::decode::decode_file(&path).expect("the fixture decodes");
    if audio.sample_rate == RATE {
        audio.samples
    } else {
        birdnet_core::audio::resample::resample(&audio.samples, audio.sample_rate, RATE)
            .expect("resamples")
    }
}

/// The tee's flags, streamed over the whole recording as capture runs them.
fn through_tee(samples: &[f32], pipeline: AudioPipeline) -> Vec<f32> {
    let mut dc = pipeline
        .dc_removal
        .then(|| OnePoleHighPass::new(RATE, DC_BLOCK_CUTOFF_HZ));
    let mut hp = pipeline
        .high_pass
        .then(|| OnePoleHighPass::new(RATE, HIGH_PASS_CUTOFF_HZ));
    samples
        .iter()
        .map(|&x| {
            let x = dc.as_mut().map_or(x, |f| f.step(x));
            hp.as_mut().map_or(x, |f| f.step(x))
        })
        .collect()
}

/// The same flags as the ffmpeg backend's chain is specified
/// (`EqChain::from_pipeline_flags`, two-pole high-passes).
fn through_chain(samples: &[f32], pipeline: AudioPipeline) -> Vec<f32> {
    let chain = EqChain::from_pipeline_flags(pipeline.high_pass, pipeline.dc_removal);
    if chain.is_empty() {
        return samples.to_vec();
    }
    let mut p = chain.build(RATE).expect("builds at 48 kHz");
    samples.iter().map(|&x| p.process(x)).collect()
}

/// Per 3 s window, the change from `before` to `after`.
fn per_window(before: &[f32], after: &[f32]) -> Vec<InputChange> {
    let w = (RATE * 3) as usize;
    before
        .chunks(w)
        .zip(after.chunks(w))
        .filter(|(b, _)| b.len() == w)
        .filter_map(|(b, a)| InputChange::between(b, a))
        .collect()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The tee's one-pole filter has the response `AudioPipeline`'s documentation
/// tabulates, so the measurement below is of the filter capture runs.
#[test]
fn the_one_pole_filter_has_the_documented_response() {
    for (hz, documented_db) in [
        (20.0_f32, -15.68),
        (30.0, -12.31),
        (50.0, -8.31),
        (60.0, -7.00),
        (80.0, -5.14),
        (120.0, -3.04),
    ] {
        let mut filter = OnePoleHighPass::new(RATE, HIGH_PASS_CUTOFF_HZ);
        let len = RATE as usize * 2;
        let sine: Vec<f32> = (0..len)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / RATE as f32;
                (2.0 * std::f32::consts::PI * hz * t).sin()
            })
            .collect();
        let out: Vec<f32> = sine.iter().map(|&v| filter.step(v)).collect();
        // The second second, after the filter has settled.
        let tail = len / 2;
        let change = InputChange::between(&sine[tail..], &out[tail..]).unwrap();
        assert!(
            (change.level_change_db - documented_db).abs() < 0.1,
            "{hz} Hz: {:.2} dB against the documented {documented_db}",
            change.level_change_db
        );
    }
}

#[test]
fn the_default_filters_change_the_magpie_windows_the_model_is_given() {
    let x = magpie_at_48k();
    let defaults = AudioPipeline::default();
    let off = AudioPipeline {
        high_pass: false,
        dc_removal: false,
        ..defaults
    };
    let dc_only = AudioPipeline {
        high_pass: false,
        ..defaults
    };

    // The input before any filter: its offset, which the DC block removes.
    #[allow(clippy::cast_precision_loss)]
    let mean = x.iter().map(|v| f64::from(*v)).sum::<f64>() / x.len() as f64;
    let dc = through_tee(&x, dc_only);
    let tee = per_window(&x, &through_tee(&x, defaults));
    let chain = per_window(&x, &through_chain(&x, defaults));
    let tee_dc = per_window(&x, &dc);
    // The high-pass's own share: from the DC-blocked signal to the defaults.
    let tee_hp = per_window(&dc, &through_tee(&x, defaults));
    let chain_dc = through_chain(&x, dc_only);
    let chain_hp = per_window(&chain_dc, &through_chain(&x, defaults));
    let none = per_window(&x, &through_tee(&x, off));
    assert_eq!(tee.len(), 10, "precondition: ten whole 3 s windows");

    let residual = |v: &[InputChange]| median(v.iter().map(|c| c.residual_db).collect());
    let level = |v: &[InputChange]| median(v.iter().map(|c| c.level_change_db).collect());
    eprintln!(
        "magpie at 48 kHz, mean {mean:.5}; per 3 s window, median: defaults (tee) residual \
         {:.2} dB, level {:.2} dB; DC block alone residual {:.2} dB; high-pass after the DC \
         block: tee residual {:.2} dB level {:.2} dB, two-pole residual {:.2} dB level {:.2} dB",
        residual(&tee),
        level(&tee),
        residual(&tee_dc),
        residual(&tee_hp),
        level(&tee_hp),
        residual(&chain_hp),
        level(&chain_hp),
    );
    for (i, c) in tee_hp.iter().enumerate() {
        eprintln!(
            "  window {i}: high-pass residual {:.2} dB, level {:.2} dB, max |change| {:.5}",
            c.residual_db, c.level_change_db, c.max_abs_change
        );
    }

    // The defaults change the input on both backends, in every window.
    for (name, v) in [
        ("tee", &tee),
        ("chain", &chain),
        ("tee hp", &tee_hp),
        ("chain hp", &chain_hp),
    ] {
        for c in v {
            assert!(c.residual_db > -60.0, "{name}: {c:?}");
        }
    }
    // The values first measured here (see the commit that added this test).
    for (what, got, measured) in [
        ("defaults, tee", residual(&tee), MEASURED_TEE_RESIDUAL_DB),
        ("DC block alone", residual(&tee_dc), MEASURED_DC_RESIDUAL_DB),
        (
            "high-pass, tee",
            residual(&tee_hp),
            MEASURED_TEE_HP_RESIDUAL_DB,
        ),
        (
            "high-pass, two-pole",
            residual(&chain_hp),
            MEASURED_CHAIN_HP_RESIDUAL_DB,
        ),
    ] {
        assert!(
            (got - measured).abs() < 0.5,
            "{what}: {got:.2} dB, first measured {measured} dB"
        );
    }

    // The counterpart: with both flags off nothing changes, and the descriptor
    // a run would record says so.
    assert!(none.iter().all(|c| c.residual_db.is_infinite()));
    let d = describe(CaptureBackend::Tee, off, &EqChain::default(), 0.0);
    assert!(d.contains("dc=off") && d.contains("hp=off"), "{d}");
    let d = describe(CaptureBackend::Tee, defaults, &EqChain::default(), 0.0);
    assert!(d.contains("hp=highpass:120Hz:1pole"), "{d}");
}

/// Median residual, dB, of the tee's default filters on the magpie windows.
const MEASURED_TEE_RESIDUAL_DB: f64 = -0.19;
/// The tee's DC block alone.
const MEASURED_DC_RESIDUAL_DB: f64 = -0.38;
/// The tee's high-pass, after its DC block.
const MEASURED_TEE_HP_RESIDUAL_DB: f64 = -3.80;
/// The two-pole high-pass, after the two-pole DC block.
const MEASURED_CHAIN_HP_RESIDUAL_DB: f64 = -0.42;
