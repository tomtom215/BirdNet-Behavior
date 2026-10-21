//! Audio resampling via rubato.
//!
//! Resamples audio to the target sample rate required by the ML model.
//! `BirdNET` models typically expect 48kHz; Perch expects 32kHz.
//!
//! Uses rubato's `Async` **sinc** resampler: a windowed-sinc low-pass
//! (256 taps, squared Blackman-Harris, cutoff chosen by rubato) runs as part
//! of the interpolation, so content above the output Nyquist is removed
//! before it can fold down.
//!
//! This was the polynomial (`Septic`) resampler, which has no such filter.
//! Capture runs at 48 kHz and the bundled V3.0 model wants 32 kHz, so every
//! file was downsampled through it, and anything between 16 and 24 kHz —
//! insects, electrical whine, ultrasonic deterrents — folded into the bird
//! band nearly unattenuated. Measured (Goertzel, 2 s tones, f32):
//!
//! | 48 kHz tone → folded to | polynomial | sinc |
//! |---|---|---|
//! | 17 kHz → 15 kHz | −0.9 dB | −141.2 dB |
//! | 20 kHz → 12 kHz | −2.3 dB | −143.0 dB |
//! | 23 kHz → 9 kHz  | −4.9 dB | −159.4 dB |
//!
//! Passband: flat (0.00 dB) through 14 kHz; −3.1 dB at 15 kHz, just under
//! the 16 kHz output Nyquist. Cost, release profile on a 2.8 GHz Xeon: 0.19 s
//! per 60 s of audio against the polynomial's 0.05 s.
//!
//! `rubato` and `audioadapter-buffers` are a version-locked pair: rubato 5
//! requires `audioadapter ^5`, so `audioadapter-buffers` must stay on 5.x until
//! rubato takes 6. Bumping either alone puts two versions of the crate that
//! defines `Adapter` in the graph, and the `InterleavedSlice` below then
//! implements the wrong one. `.github/dependabot.yml` groups them so they
//! arrive as one pull request.
//!
//! rubato's filter delays its output by `128 * ratio - 1` samples (measured, not
//! documented by rubato), and `process_all` trims only `floor(128 * ratio)`. The
//! module once trimmed nothing, and every resampled recording ran that much late
//! (see the impulse test); it then trimmed the integer part and left every
//! recording whose rate is not 48 kHz early by up to a sample (0.69 at
//! 44.1 -> 48 kHz), which moved a borderline BirdNET-Pi comparison across the
//! confidence threshold. [`resample`] now makes the delay a whole number by
//! prepending zeros and drops the surplus, so a sound lands where it went in to
//! within 0.05 sample (see the tone test).

use std::fmt;

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction};

/// Errors during resampling.
#[derive(Debug)]
pub enum ResampleError {
    /// Invalid parameters (e.g., zero sample rate).
    InvalidParams(String),
    /// Resampling computation failed.
    Process(String),
}

impl fmt::Display for ResampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParams(msg) => write!(f, "invalid resample params: {msg}"),
            Self::Process(msg) => write!(f, "resample error: {msg}"),
        }
    }
}

impl std::error::Error for ResampleError {}

/// Resample mono audio samples from `from_rate` to `to_rate`.
///
/// Returns the resampled samples. If rates are equal, returns input unchanged.
///
/// # Errors
///
/// Returns `ResampleError` if sample rates are zero or resampling fails.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, ResampleError> {
    if from_rate == 0 || to_rate == 0 {
        return Err(ResampleError::InvalidParams(
            "sample rates must be non-zero".into(),
        ));
    }

    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }

    let ratio = f64::from(to_rate) / f64::from(from_rate);

    let mut resampler = Async::<f32>::new_sinc(
        ratio,
        1.1,
        &SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2),
        1024,
        1, // mono
        FixedAsync::Input,
    )
    .map_err(|e| ResampleError::Process(e.to_string()))?;

    // The filter delays the signal by `128 * ratio - 1` output samples
    // (measured: `a_tone_comes_back_with_no_fractional_sample_delay`), which is
    // a whole number only for some ratios, while `process_all` can trim only
    // `output_delay()`, the integer part of `128 * ratio`. Left there, every
    // recording whose rate is not 48 kHz came out early by a fraction of a
    // sample (0.69 at 44.1 -> 48 kHz).
    //
    // Prepending `lead` zeros delays the signal by `lead * ratio` more output
    // samples. Choosing `lead` so that `(128 + lead) * ratio` is a whole
    // number makes the total delay a whole number too, and the surplus over
    // what `process_all` trimmed is dropped from the front below. `lead` is
    // below twice the ratio's reduced denominator; the zeros are the silence
    // the filter would have been fed anyway. `tail` zeros give the filter room
    // to produce the frames that the dropped ones displaced.
    let divisor = gcd(from_rate, to_rate);
    let (num, den) = (u64::from(to_rate / divisor), u64::from(from_rate / divisor));
    let trimmed = resampler.output_delay() as u64;
    let mut lead = (den - 128 % den) % den;
    while (128 + lead) * num / den < trimmed + 1 {
        lead += den;
    }
    let delay = (128 + lead) * num / den - 1;
    let extra = (delay - trimmed) as usize;
    let tail = (1.0 / ratio).ceil() as usize + 1;

    let mut padded = Vec::with_capacity(lead as usize + samples.len() + tail);
    padded.resize(lead as usize, 0.0_f32);
    padded.extend_from_slice(samples);
    padded.resize(padded.len() + tail, 0.0);

    // `process_all` resets the filter, trims its start-up delay and returns
    // exactly `ceil(ratio * len)` frames, so the output lines up with the input
    // instead of running late and short.
    let adapter = InterleavedSlice::new(&padded, 1, padded.len())
        .map_err(|e| ResampleError::Process(e.to_string()))?;
    let out = resampler
        .process_all(&adapter, padded.len(), None)
        .map_err(|e: rubato::ResampleError| ResampleError::Process(e.to_string()))?
        .take_data();
    // `ceil(n * to / from)` in integers: in f64 `88_200 * (48_000 / 44_100.0)` is
    // 96_000.000000000015 and the ceiling is one sample too long.
    let wanted =
        (samples.len() as u64 * u64::from(to_rate)).div_ceil(u64::from(from_rate)) as usize;
    Ok(out.into_iter().skip(extra).take(wanted).collect())
}

/// Greatest common divisor (Euclid); both arguments are non-zero here.
const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops
)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_returns_input() {
        let samples = vec![0.1, 0.2, 0.3];
        let result = resample(&samples, 48000, 48000).unwrap();
        assert_eq!(result, samples);
    }

    #[test]
    fn zero_rate_returns_error() {
        let samples = vec![0.1, 0.2];
        assert!(resample(&samples, 0, 48000).is_err());
        assert!(resample(&samples, 48000, 0).is_err());
    }

    fn tone(freq: f64, secs: f64) -> Vec<f32> {
        (0..(48_000.0 * secs) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / 48_000.0).sin() as f32)
            .collect()
    }

    /// Amplitude of `freq` in `x` at 32 kHz, in dB relative to a unit sine,
    /// by Goertzel over the middle half (edge transients excluded).
    fn level_db(x: &[f32], freq: f64) -> f64 {
        let x = &x[x.len() / 4..3 * x.len() / 4];
        let w = 2.0 * std::f64::consts::PI * freq / 32_000.0;
        let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
        for &v in x {
            let s0 = f64::from(v) + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let power = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
        20.0 * (2.0 * power.sqrt() / x.len() as f64).max(1e-12).log10()
    }

    /// Downsampling 48 → 32 kHz (every capture, for the V3.0 model) removes
    /// what lies above the new Nyquist instead of folding it into the bird
    /// band. The polynomial resampler this replaced folded a 20 kHz tone to
    /// 12 kHz at −2.3 dB, 17 kHz to 15 kHz at −0.9 dB.
    #[test]
    fn downsampling_does_not_fold_ultrasound_into_the_bird_band() {
        for (src, folded) in [
            (17_000.0, 15_000.0),
            (20_000.0, 12_000.0),
            (23_000.0, 9_000.0),
        ] {
            let out = resample(&tone(src, 2.0), 48_000, 32_000).unwrap();
            let l = level_db(&out, folded);
            assert!(l < -80.0, "{src} Hz folded to {folded} Hz at {l:.1} dB");
        }
    }

    /// The counterpart: the bird band itself passes unchanged, so the test
    /// above is a filter and not a mute.
    #[test]
    fn downsampling_keeps_the_bird_band() {
        for f in [1_000.0, 5_000.0, 10_000.0, 12_000.0, 14_000.0] {
            let out = resample(&tone(f, 2.0), 48_000, 32_000).unwrap();
            let l = level_db(&out, f);
            assert!(l.abs() < 0.1, "{f} Hz at {l:.2} dB");
        }
    }

    /// A resampler's filter delays its output; the output must not be shifted.
    /// An impulse at input sample `n` has to come back at `n * ratio`, to
    /// within one output sample. The first version of this module left rubato's
    /// start-up delay in, so every resampled recording ran `128 * ratio`
    /// samples late (+138 at 44.1 -> 48 kHz, +85 at 48 -> 32 kHz, +278 at
    /// 22.05 -> 48 kHz) and lost that much from its end; measured against
    /// BirdNET-Pi's resampled samples as a lag of exactly those numbers.
    #[test]
    fn an_impulse_comes_back_where_it_went_in() {
        for (from, to) in [
            (44_100_u32, 48_000_u32),
            (48_000, 32_000),
            (22_050, 48_000),
            (32_000, 48_000),
        ] {
            let at = 20_000_usize;
            let mut x = vec![0.0_f32; 40_000];
            x[at] = 1.0;
            let y = resample(&x, from, to).unwrap();
            let peak = y
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(i, _)| i)
                .unwrap();
            let expected = at as f64 * f64::from(to) / f64::from(from);
            assert!(
                (peak as f64 - expected).abs() <= 1.0,
                "{from} -> {to} Hz: impulse at {at} came back at {peak}, expected {expected:.1}"
            );
        }
    }

    /// Delay of the output against the input, in OUTPUT samples, measured on a
    /// sine of `freq` Hz: the phase of the middle half of `resample`'s output
    /// against the same sine evaluated at the output times. Positive = late.
    fn measured_delay(from: u32, to: u32, freq: f64) -> f64 {
        use std::f64::consts::TAU;
        let n = from as usize * 2;
        let x: Vec<f32> = (0..n)
            .map(|i| (TAU * freq * i as f64 / f64::from(from)).sin() as f32)
            .collect();
        let y = resample(&x, from, to).unwrap();
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for (k, v) in y.iter().enumerate().take(3 * y.len() / 4).skip(y.len() / 4) {
            let w = TAU * freq * k as f64 / f64::from(to);
            re += f64::from(*v) * w.cos();
            im += f64::from(*v) * w.sin();
        }
        // y = A sin(w k - phi):  sum y cos ~ -sin(phi), sum y sin ~ cos(phi).
        (-re).atan2(im) / (TAU * freq / f64::from(to))
    }

    /// The output is not just near where it should be, it is *there*: a tone
    /// comes out with no fractional-sample delay. rubato's filter delays the
    /// signal by `128 * ratio - 1` output samples, an integer only for some
    /// ratios, and `process_all` trims `floor(128 * ratio)`; the module used to
    /// stop there and left every non-48 kHz recording early by
    /// `1 - frac(128 * ratio)` samples: 0.69 at 44.1 -> 48 kHz, 0.38 at
    /// 22.05 -> 48 kHz, 0.67 at 48 -> 32 kHz, 1.0 at 32 -> 48 kHz. BirdNET-Pi's
    /// resampler (resampy) leaves none, and on a very quiet recording that
    /// shift alone moved one window's top score from 0.711 to 0.656, across the
    /// 0.7 line (`tools/parity`, clip `ij_mono_44k1_12s`). The impulse test
    /// below could not see it: it allows one whole output sample.
    #[test]
    fn a_tone_comes_back_with_no_fractional_sample_delay() {
        for (from, to) in [
            (44_100_u32, 48_000_u32),
            (22_050, 48_000),
            (48_000, 32_000),
            (32_000, 48_000),
            (16_000, 48_000),
            (96_000, 48_000),
            (11_025, 48_000),
            (88_200, 48_000),
            (48_000, 44_100),
            (48_000, 22_050),
            (48_000, 16_000),
        ] {
            for freq in [1_000.0, 4_000.0] {
                let d = measured_delay(from, to, freq);
                assert!(
                    d.abs() < 0.05,
                    "{from} -> {to} Hz, {freq} Hz tone: delayed by {d:+.3} output samples"
                );
            }
        }
    }

    /// The output is the length the ratio says, with nothing trimmed from the
    /// end to pay for the delay.
    #[test]
    fn the_output_has_the_length_the_ratio_says() {
        for (from, to, n) in [
            (44_100_u32, 48_000_u32, 44_100_usize),
            (48_000, 32_000, 720_000),
            (22_050, 48_000, 22_050),
            (48_000, 32_000, 48_001),
        ] {
            let y = resample(&vec![0.1_f32; n], from, to).unwrap();
            let expected = (n as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
            assert_eq!(y.len(), expected, "{from} -> {to} Hz, {n} samples");
        }
    }

    /// Inputs shorter than the filter (a clip of a few samples, or none) come
    /// back at the length the ratio says and do not panic: the padding added to
    /// align the output must never leak into the length.
    #[test]
    fn tiny_inputs_keep_the_length_the_ratio_says() {
        for (from, to) in [(44_100_u32, 48_000_u32), (48_000, 32_000), (22_050, 48_000)] {
            for n in [0_usize, 1, 2, 7, 300] {
                let y = resample(&vec![0.25_f32; n], from, to).unwrap();
                let expected = (n as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
                assert_eq!(y.len(), expected, "{from} -> {to} Hz, {n} samples");
            }
        }
    }

    #[test]
    fn downsample_produces_output() {
        // 2048 samples at 48kHz -> resample to 16kHz
        let samples: Vec<f32> = (0..2048).map(|i| (i as f32 / 48000.0).sin()).collect();
        let result = resample(&samples, 48000, 16000).unwrap();
        assert!(!result.is_empty());
        assert!(result.len() < samples.len());
    }
}
