//! Signal-to-noise ratio estimation.
//!
//! Estimates SNR by dividing the audio buffer into short frames,
//! computing the RMS energy per frame, and comparing the peak frame
//! energy to the low-percentile (noise) frame energy.
//!
//! This approach is robust to brief loud events (bird calls) in an
//! otherwise quiet recording; the noise floor is estimated from the
//! quietest frames rather than the whole-buffer average.

/// Frame size in samples for short-time RMS computation.
const FRAME_SAMPLES: usize = 512;

/// Percentile of frame energies to use as the noise floor estimate.
/// 0.20 = bottom 20% of frames.
const NOISE_PERCENTILE: f32 = 0.20;

/// Minimum number of frames required for a reliable SNR estimate.
const MIN_FRAMES: usize = 4;

/// Estimate the SNR and noise floor of an audio chunk.
///
/// # Algorithm
///
/// 1. Divide `samples` into non-overlapping frames of `FRAME_SAMPLES`.
/// 2. Compute per-frame RMS amplitude.
/// 3. The noise floor is the `NOISE_PERCENTILE` percentile of frame RMSes.
/// 4. The signal peak is the maximum frame RMS.
/// 5. `SNR_dB = 20 × log₁₀(peak_rms / noise_rms)`.
///
/// # Returns
///
/// `(snr_db, noise_floor_dbfs)` — both are non-negative for SNR;
/// noise floor is in dBFS (negative values relative to full scale).
///
/// Returns `(0.0, −96.0)` for empty or very short input.
pub fn estimate_snr(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, -96.0);
    }

    let frame_rms_values = compute_frame_rms(samples);
    if frame_rms_values.len() < MIN_FRAMES {
        let whole = rms(samples);
        return (0.0, rms_to_dbfs(whole));
    }

    let mut sorted = frame_rms_values.clone();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let noise_idx = ((sorted.len() as f32 * NOISE_PERCENTILE) as usize).clamp(1, sorted.len());
    // Use the median of the lower percentile for stability
    let noise_rms = sorted[..noise_idx]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max)
        .max(1e-10_f32);

    let signal_rms = frame_rms_values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max)
        .max(1e-10_f32);

    let snr_db = (20.0_f32 * (signal_rms / noise_rms).log10()).max(0.0);
    let noise_floor_dbfs = rms_to_dbfs(noise_rms);

    (snr_db, noise_floor_dbfs)
}

/// Compute spectral flatness from a power spectrum.
///
/// Spectral flatness (Wiener entropy) is the ratio of the geometric mean
/// to the arithmetic mean of the power spectrum.
///
/// - **0.0** = perfectly tonal (single frequency component).
/// - **1.0** = flat (white noise).
///
/// Bird vocalisations typically produce values in the range 0.05–0.5.
/// Values above 0.85 suggest white-noise or broadband interference.
///
/// The input is the **linear** power spectrum (squared magnitudes).
#[allow(clippy::cast_precision_loss)]
pub fn spectral_flatness(power_spectrum: &[f32]) -> f32 {
    if power_spectrum.is_empty() {
        return 0.0;
    }

    let n = power_spectrum.len() as f32;
    let epsilon = 1e-10_f32;

    // Geometric mean = exp(mean(log(x)))
    let log_sum: f32 = power_spectrum.iter().map(|&p| (p + epsilon).ln()).sum();
    let geometric_mean = (log_sum / n).exp();

    // Arithmetic mean
    let arithmetic_mean = power_spectrum.iter().sum::<f32>() / n;

    if arithmetic_mean < epsilon {
        return 0.0;
    }

    (geometric_mean / arithmetic_mean).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute per-frame RMS for non-overlapping frames.
fn compute_frame_rms(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(FRAME_SAMPLES)
        .filter(|c| c.len() >= FRAME_SAMPLES / 2)
        .map(rms)
        .collect()
}

/// Root mean square of a slice.
#[allow(clippy::cast_precision_loss)]
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Convert linear RMS amplitude to dBFS.
pub(crate) fn rms_to_dbfs(rms_val: f32) -> f32 {
    if rms_val < 1e-10 {
        return -96.0;
    }
    20.0 * rms_val.log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    fn sine_wave(freq_hz: f32, amplitude: f32, n_samples: usize, sample_rate: u32) -> Vec<f32> {
        (0..n_samples)
            .map(|i| amplitude * (2.0 * PI * freq_hz * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn empty_input_returns_defaults() {
        let (snr, floor) = estimate_snr(&[]);
        assert_eq!(snr, 0.0);
        assert_eq!(floor, -96.0);
    }

    #[test]
    fn uniform_signal_has_low_snr() {
        // Constant amplitude — all frames equal, so SNR ≈ 0
        let samples = vec![0.01_f32; FRAME_SAMPLES * 20];
        let (snr, _) = estimate_snr(&samples);
        assert!(
            snr < 5.0,
            "uniform signal should have near-zero SNR, got {snr}"
        );
    }

    #[test]
    fn tonal_burst_produces_high_snr() {
        // Quiet noise with a loud tone burst in the middle
        let mut samples = vec![0.001_f32; FRAME_SAMPLES * 20];
        let tone = sine_wave(3000.0, 0.5, FRAME_SAMPLES * 4, 48_000);
        samples[FRAME_SAMPLES * 8..FRAME_SAMPLES * 12].copy_from_slice(&tone);
        let (snr, _) = estimate_snr(&samples);
        assert!(snr > 10.0, "tonal burst should produce high SNR, got {snr}");
    }

    #[test]
    fn noise_floor_negative_dbfs() {
        let samples = vec![0.01_f32; FRAME_SAMPLES * 10];
        let (_, floor) = estimate_snr(&samples);
        assert!(
            floor < 0.0,
            "noise floor should be negative dBFS, got {floor}"
        );
    }

    #[test]
    fn spectral_flatness_pure_tone_is_low() {
        // A single-frequency power spectrum: one bin with power, rest zero
        let mut power = vec![0.0_f32; 256];
        power[50] = 1.0;
        let sf = spectral_flatness(&power);
        assert!(
            sf < 0.1,
            "pure tone should have low spectral flatness, got {sf}"
        );
    }

    #[test]
    fn spectral_flatness_white_noise_approaches_one() {
        // Flat power spectrum approximates white noise
        let power = vec![1.0_f32; 256];
        let sf = spectral_flatness(&power);
        assert!(
            sf > 0.95,
            "flat power spectrum should have high spectral flatness, got {sf}"
        );
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn spectral_flatness_empty_returns_zero() {
        assert_eq!(spectral_flatness(&[]), 0.0);
    }

    #[test]
    fn rms_to_dbfs_full_scale() {
        // RMS of 1.0 should be 0 dBFS
        assert!((rms_to_dbfs(1.0) - 0.0).abs() < 0.01);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn rms_to_dbfs_near_zero() {
        assert_eq!(rms_to_dbfs(0.0), -96.0);
    }
}
