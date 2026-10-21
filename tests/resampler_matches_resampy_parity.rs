//! Does a recording that is not 48 kHz reach the model where BirdNET-Pi puts it?
//!
//! BirdNET-Pi resamples with `librosa.load(res_type='kaiser_fast')`, i.e.
//! `resampy`. This project resamples with `rubato`. The two agree on
//! everything but a start-up delay: rubato delays its output by
//! `128 * ratio - 1` samples and the module used to remove only the integer
//! part, so a 44.1 kHz recording arrived 0.69 sample early. The consequence was
//! measured on BirdNET-Pi's own reference clips (`tools/parity`): the input to
//! the model differed from BirdNET-Pi's by 28 % (RMS, relative) on a quiet
//! 44.1 kHz clip and 19 % on a spoken-word one, one borderline window fell from
//! 0.711 to 0.656 and stopped being a detection, and a speech window changed
//! its top species.
//!
//! This test needs no model and no audio: it resamples a deterministic sum of
//! sines and compares 256 consecutive samples with what resampy produced for
//! the same input (`tools/parity/golden/resampy_tones.json`, made by
//! `tools/parity/resampler_golden.py`). Two bounds, because they fail
//! differently: the distance to resampy (a shifted output fails this one) and
//! the distance to the ideal band-limited answer (a resampler that agreed with
//! resampy only by sharing its fault would fail this one).

#![allow(clippy::cast_precision_loss)]

use birdnet_core::audio::resample::resample;
use std::f64::consts::TAU;

const GOLDEN: &str = "tools/parity/golden/resampy_tones.json";

fn tone_sum(tones: &[(f64, f64)], rate: f64, i: usize) -> f64 {
    tones
        .iter()
        .map(|&(f, a)| a * (TAU * f * i as f64 / rate).sin())
        .sum()
}

#[test]
fn a_resampled_recording_lands_where_birdnet_pi_puts_it() {
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let tones: Vec<(f64, f64)> = golden["tones_hz_amplitude"]
        .as_array()
        .expect("tones")
        .iter()
        .map(|t| (t[0].as_f64().unwrap(), t[1].as_f64().unwrap()))
        .collect();
    let to_rate = u32::try_from(golden["to_rate"].as_u64().unwrap()).unwrap();
    let start = usize::try_from(golden["window_start"].as_u64().unwrap()).unwrap();
    let len = usize::try_from(golden["window_len"].as_u64().unwrap()).unwrap();
    let runs = golden["runs"].as_array().expect("runs");
    assert_eq!(runs.len(), 3, "three source rates in the golden");

    for run in runs {
        let from = u32::try_from(run["from_rate"].as_u64().unwrap()).unwrap();
        let n = usize::try_from(run["input_len"].as_u64().unwrap()).unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let x: Vec<f32> = (0..n)
            .map(|i| tone_sum(&tones, f64::from(from), i) as f32)
            .collect();
        let y = resample(&x, from, to_rate).expect("resample");
        assert_eq!(
            y.len(),
            usize::try_from(run["output_len"].as_u64().unwrap()).unwrap(),
            "{from} Hz: output length"
        );

        let pi: Vec<f64> = run["resampy_window"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let (mut vs_pi, mut vs_ideal) = (0.0_f64, 0.0_f64);
        for (j, p) in pi.iter().enumerate() {
            let k = start + j;
            let ours = f64::from(y[k]);
            vs_pi = vs_pi.max((ours - p).abs());
            vs_ideal = vs_ideal.max((ours - tone_sum(&tones, f64::from(to_rate), k)).abs());
        }
        eprintln!(
            "{from} Hz -> {to_rate}: max |ours - resampy| {vs_pi:.2e}, max |ours - ideal| {vs_ideal:.2e}"
        );
        assert_eq!(len, pi.len());
        // The signal's peak is about 0.8. What is left after the delay is
        // removed is rubato's 1/128-sample residual (0.0085 sample at
        // 44.1 kHz, 0.0117 at 32 kHz, 0.017 at 22.05 kHz), which scales with
        // the five tones' frequencies: about 2e-3 at the worst point. The
        // delay the module used to leave (0.69 sample at 44.1 kHz) is ten times
        // that: measured 4.1e-2 against resampy.
        let (tol_pi, tol_ideal) = if from == 22_050 {
            // resampy's rolloff (0.868 of Nyquist) already attenuates the
            // 9 kHz tone at this source rate (its own error against the ideal
            // is 5.3e-3); rubato keeps it. Both are "correct"; the bound is
            // the size of that choice plus the residual.
            (1e-2, 6e-3)
        } else {
            (4e-3, 4e-3)
        };
        assert!(vs_pi < tol_pi, "{from} Hz: {vs_pi:.2e} from resampy");
        assert!(vs_ideal < tol_ideal, "{from} Hz: {vs_ideal:.2e} from ideal");
    }
}
