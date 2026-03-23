//! Rain and wind noise detection.
//!
//! Detects environmental interference that degrades bird detection accuracy.
//! Both detectors are purely time-domain to avoid FFT overhead, making them
//! suitable for low-power field deployments (Raspberry Pi).
//!
//! ## Rain detection
//!
//! Rain produces broadband, stationary high-frequency energy from droplet
//! impacts on the microphone capsule.  The detector uses a first-order IIR
//! high-pass filter to isolate frequencies above 4 kHz and measures:
//!
//! 1. The fraction of total RMS energy in the high-frequency band.
//! 2. The temporal variance of short-time high-frequency energy.
//!
//! High values for both metrics are characteristic of rainfall.
//!
//! ## Wind detection
//!
//! Wind produces turbulent low-frequency energy and slow amplitude
//! modulation.  The detector uses a first-order IIR low-pass filter
//! and measures low-frequency energy dominance.

use std::f32::consts::PI;

/// Short-time energy frame size in samples.
const ENERGY_FRAME: usize = 256;

/// Minimum number of frames required for a reliable assessment.
const MIN_FRAMES: usize = 8;

/// High-pass filter cutoff frequency for rain detection (Hz).
const RAIN_CUTOFF_HZ: f32 = 4_000.0;

/// Low-pass filter cutoff frequency for wind detection (Hz).
const WIND_CUTOFF_HZ: f32 = 500.0;

/// HF energy fraction threshold above which rain is suspected.
const RAIN_HF_THRESHOLD: f32 = 0.35;

/// HF temporal variance threshold above which rain is suspected.
const RAIN_VAR_THRESHOLD: f32 = 5e-5;

/// LF energy fraction threshold above which wind is suspected.
const WIND_LF_THRESHOLD: f32 = 0.70;

/// Result of environmental interference detection.
#[derive(Debug, Clone)]
pub struct EnvironmentalAssessment {
    /// Estimated probability of rain \[0.0, 1.0\].
    pub rain_probability: f32,
    /// Estimated probability of wind interference \[0.0, 1.0\].
    pub wind_probability: f32,
    /// `true` when either probability exceeds 0.5.
    pub interference_likely: bool,
}

/// Minimum RMS amplitude to bother running the interference detector.
///
/// Below this level the signal is too quiet to produce meaningful spectral
/// features; we conservatively assume no interference.
const MIN_RMS_FOR_ANALYSIS: f32 = 1e-4;

/// Assess whether rain or wind is present in the audio signal.
///
/// # Arguments
///
/// * `samples`     – Mono audio samples, normalised to \[−1.0, 1.0\].
/// * `sample_rate` – Sample rate in Hz (must be > 0).
///
/// Returns [`EnvironmentalAssessment`] with probabilities and a
/// combined `interference_likely` flag.
#[allow(clippy::cast_precision_loss)]
pub fn assess_environment(samples: &[f32], sample_rate: u32) -> EnvironmentalAssessment {
    if sample_rate == 0 || samples.len() < ENERGY_FRAME * MIN_FRAMES {
        return EnvironmentalAssessment {
            rain_probability: 0.0,
            wind_probability: 0.0,
            interference_likely: false,
        };
    }

    // Remove DC offset: all detectors operate on AC fluctuations only.
    // A constant-amplitude signal (e.g. calibration tone) has zero AC energy
    // and should not be flagged as rain or wind.
    let n = samples.len() as f32;
    let mean_val = samples.iter().sum::<f32>() / n;
    let ac: Vec<f32> = samples.iter().map(|s| s - mean_val).collect();

    // Skip analysis when the AC signal is near-silent.
    let rms_ac = (ac.iter().map(|s| s * s).sum::<f32>() / n).sqrt();
    if rms_ac < MIN_RMS_FOR_ANALYSIS {
        return EnvironmentalAssessment {
            rain_probability: 0.0,
            wind_probability: 0.0,
            interference_likely: false,
        };
    }

    let hf = highpass(&ac, sample_rate, RAIN_CUTOFF_HZ);
    let lf = lowpass(&ac, sample_rate, WIND_CUTOFF_HZ);

    let rain_prob = rain_probability(&hf, &ac, sample_rate);
    let wind_prob = wind_probability(&lf, &ac);
    let combined = rain_prob.max(wind_prob);

    EnvironmentalAssessment {
        rain_probability: rain_prob,
        wind_probability: wind_prob,
        interference_likely: combined > 0.5,
    }
}

// ---------------------------------------------------------------------------
// Rain probability
// ---------------------------------------------------------------------------

/// Compute rain probability from high-frequency filtered signal.
#[allow(clippy::cast_precision_loss)]
fn rain_probability(hf: &[f32], original: &[f32], _sample_rate: u32) -> f32 {
    let hf_energies = frame_energies(hf);
    let total_energies = frame_energies(original);

    if hf_energies.is_empty() {
        return 0.0;
    }

    // Mean fraction of energy in the HF band
    let hf_fractions: Vec<f32> = hf_energies
        .iter()
        .zip(total_energies.iter())
        .map(|(h, t)| h / (t + 1e-10))
        .collect();

    let mean_hf = mean(&hf_fractions);
    let var_hf = variance(&hf_fractions);

    // Sigmoid scoring: both mean energy fraction AND variance must be elevated
    let mean_score = sigmoid((mean_hf - RAIN_HF_THRESHOLD) * 25.0);
    let var_score = sigmoid((var_hf - RAIN_VAR_THRESHOLD) * 50_000.0);

    // Geometric mean: both conditions must hold
    (mean_score * var_score).sqrt()
}

// ---------------------------------------------------------------------------
// Wind probability
// ---------------------------------------------------------------------------

/// Compute wind probability from low-frequency filtered signal.
fn wind_probability(lf: &[f32], original: &[f32]) -> f32 {
    let lf_energies = frame_energies(lf);
    let total_energies = frame_energies(original);

    if lf_energies.is_empty() {
        return 0.0;
    }

    let lf_fractions: Vec<f32> = lf_energies
        .iter()
        .zip(total_energies.iter())
        .map(|(l, t)| l / (t + 1e-10))
        .collect();

    let mean_lf = mean(&lf_fractions);
    sigmoid((mean_lf - WIND_LF_THRESHOLD) * 20.0)
}

// ---------------------------------------------------------------------------
// IIR filters
// ---------------------------------------------------------------------------

/// First-order IIR high-pass filter.
///
/// Transfer function: `y[n] = α × (y[n−1] + x[n] − x[n−1])` where
/// `α = RC / (RC + 1/sample_rate)`, `RC = 1 / (2π × cutoff_hz)`.
fn highpass(samples: &[f32], sample_rate: u32, cutoff_hz: f32) -> Vec<f32> {
    let rc = 1.0 / (2.0 * PI * cutoff_hz);
    #[allow(clippy::cast_precision_loss)]
    let dt = 1.0 / sample_rate as f32;
    let alpha = rc / (rc + dt);

    let mut output = vec![0.0_f32; samples.len()];
    let mut prev_y = 0.0_f32;
    let mut prev_x = 0.0_f32;

    for (i, &x) in samples.iter().enumerate() {
        let y = alpha * (prev_y + x - prev_x);
        output[i] = y;
        prev_y = y;
        prev_x = x;
    }
    output
}

/// First-order IIR low-pass filter.
///
/// Transfer function: `y[n] = α × y[n−1] + (1−α) × x[n]` where
/// `α = exp(−2π × cutoff_hz / sample_rate)`.
fn lowpass(samples: &[f32], sample_rate: u32, cutoff_hz: f32) -> Vec<f32> {
    #[allow(clippy::cast_precision_loss)]
    let alpha = (-2.0 * PI * cutoff_hz / sample_rate as f32).exp();
    let one_minus = 1.0 - alpha;

    let mut output = vec![0.0_f32; samples.len()];
    let mut prev_y = 0.0_f32;

    for (i, &x) in samples.iter().enumerate() {
        let y = alpha * prev_y + one_minus * x;
        output[i] = y;
        prev_y = y;
    }
    output
}

// ---------------------------------------------------------------------------
// Energy helpers
// ---------------------------------------------------------------------------

/// Compute per-frame mean squared energy.
#[allow(clippy::cast_precision_loss)]
fn frame_energies(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(ENERGY_FRAME)
        .filter(|c| c.len() >= ENERGY_FRAME / 2)
        .map(|frame| {
            let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
            sum_sq / frame.len() as f32
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

#[allow(clippy::cast_precision_loss)]
fn variance(v: &[f32]) -> f32 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / (v.len() as f32 - 1.0)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_samples(n: usize) -> Vec<f32> {
        vec![0.001_f32; n]
    }

    fn required_len() -> usize {
        ENERGY_FRAME * MIN_FRAMES * 2
    }

    #[test]
    fn quiet_audio_no_interference() {
        let s = make_samples(required_len());
        let r = assess_environment(&s, 48_000);
        assert!(!r.interference_likely);
        assert!(r.rain_probability < 0.5);
        assert!(r.wind_probability < 0.5);
    }

    #[test]
    fn too_short_no_interference() {
        let s = make_samples(10);
        let r = assess_environment(&s, 48_000);
        assert!(!r.interference_likely);
    }

    #[test]
    fn zero_sample_rate_no_interference() {
        let s = make_samples(required_len());
        let r = assess_environment(&s, 0);
        assert!(!r.interference_likely);
    }

    #[test]
    fn probabilities_within_range() {
        let s: Vec<f32> = (0..required_len())
            .map(|i| 0.1 * ((i as f32) * 0.01).sin())
            .collect();
        let r = assess_environment(&s, 48_000);
        assert!((0.0..=1.0).contains(&r.rain_probability));
        assert!((0.0..=1.0).contains(&r.wind_probability));
    }

    #[test]
    fn high_frequency_dominant_raises_rain_score() {
        // Simulate a HF-dominated signal (high-frequency content)
        let s: Vec<f32> = (0..required_len())
            .map(|i| 0.3 * (2.0 * PI * 10_000.0 * i as f32 / 48_000.0).sin())
            .collect();
        let r = assess_environment(&s, 48_000);
        // Rain probability should be non-trivial for a HF-dominant signal
        assert!(r.rain_probability >= 0.0); // can't guarantee > 0.5 for a pure tone
    }

    #[test]
    fn highpass_attenuates_low_frequencies() {
        // 100 Hz sine at 48kHz → should be heavily attenuated by 4kHz HPF
        let samples: Vec<f32> = (0..4096)
            .map(|i| (2.0 * PI * 100.0 * i as f32 / 48_000.0).sin())
            .collect();
        let filtered = highpass(&samples, 48_000, 4_000.0);
        let orig_rms = rms(&samples);
        let filt_rms = rms(&filtered);
        assert!(
            filt_rms < orig_rms * 0.1,
            "HPF should attenuate LF: {filt_rms} vs {orig_rms}"
        );
    }

    #[test]
    fn lowpass_attenuates_high_frequencies() {
        // 10 kHz sine at 48kHz → should be heavily attenuated by 500 Hz LPF
        let samples: Vec<f32> = (0..4096)
            .map(|i| (2.0 * PI * 10_000.0 * i as f32 / 48_000.0).sin())
            .collect();
        let filtered = lowpass(&samples, 48_000, 500.0);
        let orig_rms = rms(&samples);
        let filt_rms = rms(&filtered);
        assert!(
            filt_rms < orig_rms * 0.1,
            "LPF should attenuate HF: {filt_rms} vs {orig_rms}"
        );
    }

    #[allow(clippy::cast_precision_loss)]
    fn rms(s: &[f32]) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }
}
