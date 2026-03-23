//! Audio quality assessment pipeline.
//!
//! Evaluates each audio chunk before ML inference to filter out
//! low-quality recordings that would produce spurious detections.
//!
//! ## Pipeline
//!
//! 1. **SNR estimation** — frame-based peak-to-noise-floor ratio.
//! 2. **Spectral flatness** — tonal vs. noise-like discrimination.
//! 3. **Noise floor tracking** — adaptive minimum-statistics estimator.
//! 4. **Environmental assessment** — rain / wind detection via IIR filters.
//! 5. **Composite score** — weighted combination of all metrics.
//!
//! ## Usage
//!
//! ```rust
//! use birdnet_core::audio::quality::{assess_quality, QualityThresholds};
//!
//! let samples = vec![0.0_f32; 48_000]; // 1 second at 48 kHz
//! let score = assess_quality(&samples, 48_000).unwrap();
//! let usable = score.is_usable(&QualityThresholds::default());
//! println!("{score}");  // QualityScore { snr=0.0dB flatness=0.000 ... }
//! ```

pub mod noise_floor;
pub mod rain_detector;
pub mod snr;
pub mod types;

pub use noise_floor::NoiseFloorTracker;
pub use rain_detector::{EnvironmentalAssessment, assess_environment};
pub use snr::{estimate_snr, spectral_flatness};
pub use types::{QualityError, QualityScore, QualityThresholds};

use realfft::RealFftPlanner;

/// Minimum audio length (samples) required for quality assessment.
pub const MIN_SAMPLES: usize = 4_096;

/// Assess the quality of a mono audio chunk.
///
/// Runs the full quality pipeline (SNR, spectral flatness, rain/wind
/// detection) and returns a [`QualityScore`].
///
/// # Errors
///
/// Returns [`QualityError::TooShort`] when `samples.len() < MIN_SAMPLES`.
/// Returns [`QualityError::UnsupportedSampleRate`] for sample rates other
/// than 48 000 Hz.
pub fn assess_quality(samples: &[f32], sample_rate: u32) -> Result<QualityScore, QualityError> {
    if sample_rate != 48_000 {
        return Err(QualityError::UnsupportedSampleRate(sample_rate));
    }
    if samples.len() < MIN_SAMPLES {
        return Err(QualityError::TooShort {
            len: samples.len(),
            required: MIN_SAMPLES,
        });
    }

    // --- SNR & noise floor ---
    let (snr_db, noise_floor_dbfs) = estimate_snr(samples);

    // --- Spectral flatness via power spectrum ---
    let sf = compute_spectral_flatness(samples);

    // --- Environmental assessment ---
    let env = assess_environment(samples, sample_rate);
    let rain_detected = env.interference_likely;

    // --- Composite score ---
    let score = composite_score(snr_db, sf, rain_detected);

    Ok(QualityScore {
        snr_db,
        spectral_flatness: sf,
        noise_floor_dbfs,
        rain_detected,
        score,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute spectral flatness from a short FFT of the input.
///
/// Uses a 1024-point FFT on the first 1024 samples (or the whole buffer
/// if shorter) for a fast broadband spectral flatness estimate.
fn compute_spectral_flatness(samples: &[f32]) -> f32 {
    const FFT_SIZE: usize = 1024;
    let n = samples.len().min(FFT_SIZE);

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = samples[..n].to_vec();
    let mut output = fft.make_output_vec();

    if fft.process(&mut input, &mut output).is_err() {
        return 0.5; // fallback: neutral value
    }

    // Build power spectrum from FFT magnitudes
    let power: Vec<f32> = output.iter().map(|c| c.re.hypot(c.im).powi(2)).collect();

    spectral_flatness(&power)
}

/// Compute a composite quality score in \[0.0, 1.0\].
///
/// Weights:
/// - SNR score:             40 %
/// - Spectral flatness:     40 % (inverted — high flatness = bad)
/// - Rain penalty:          20 % (full deduction if rain detected)
fn composite_score(snr_db: f32, spectral_flatness_val: f32, rain: bool) -> f32 {
    // SNR: 0 dB → 0.0, 20 dB → 1.0 (sigmoid-like linear clamp)
    let snr_score = (snr_db / 20.0).clamp(0.0, 1.0);

    // Spectral flatness: invert so that tonal (low) = high score
    let flatness_score = 1.0 - spectral_flatness_val.clamp(0.0, 1.0);

    let rain_penalty = if rain { 0.0_f32 } else { 1.0_f32 };

    0.20_f32
        .mul_add(
            rain_penalty,
            0.40_f32.mul_add(snr_score, 0.40 * flatness_score),
        )
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine_chunk(freq_hz: f32, n_samples: usize, amplitude: f32) -> Vec<f32> {
        (0..n_samples)
            .map(|i| amplitude * (2.0 * PI * freq_hz * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn assess_quality_sine_returns_ok() {
        let samples = sine_chunk(2000.0, MIN_SAMPLES * 2, 0.5);
        let score = assess_quality(&samples, 48_000).unwrap();
        assert!(score.score >= 0.0 && score.score <= 1.0);
        assert!(!score.rain_detected);
    }

    #[test]
    fn assess_quality_too_short_errors() {
        let samples = vec![0.0_f32; 100];
        assert!(matches!(
            assess_quality(&samples, 48_000),
            Err(QualityError::TooShort { .. })
        ));
    }

    #[test]
    fn assess_quality_wrong_sample_rate_errors() {
        let samples = vec![0.0_f32; MIN_SAMPLES * 2];
        assert!(matches!(
            assess_quality(&samples, 44_100),
            Err(QualityError::UnsupportedSampleRate(44_100))
        ));
    }

    #[test]
    fn composite_score_range() {
        // All combinations of extremes must stay in [0, 1]
        for &snr in &[0.0_f32, 5.0, 20.0] {
            for &sf in &[0.0_f32, 0.5, 1.0] {
                for rain in [false, true] {
                    let s = composite_score(snr, sf, rain);
                    assert!((0.0..=1.0).contains(&s), "score {s} out of range");
                }
            }
        }
    }

    #[test]
    fn tonal_signal_higher_score_than_noise() {
        // Tonal: 2 kHz sine
        let tonal = sine_chunk(2000.0, MIN_SAMPLES * 3, 0.5);
        let tonal_score = assess_quality(&tonal, 48_000).unwrap();

        // Noise-like: deterministic pseudo-random (LCG, wrapping arithmetic)
        let mut state: u64 = 0xDEAD_C0DE_CAFE_BABE_u64;
        let noisy: Vec<f32> = (0..MIN_SAMPLES * 3)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 33) as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        let noisy_score = assess_quality(&noisy, 48_000).unwrap();

        assert!(
            tonal_score.score >= noisy_score.score,
            "tonal {:.2} should score >= noisy {:.2}",
            tonal_score.score,
            noisy_score.score
        );
    }

    #[test]
    fn quality_score_display() {
        let samples = sine_chunk(1000.0, MIN_SAMPLES * 2, 0.3);
        let score = assess_quality(&samples, 48_000).unwrap();
        let display = format!("{score}");
        assert!(display.contains("QualityScore"));
        assert!(display.contains("dB"));
    }
}
