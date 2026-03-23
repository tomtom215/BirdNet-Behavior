//! Criterion benchmarks for the birdnet-core audio pipeline.
//!
//! Measures performance of the hot path components:
//! - Mel spectrogram computation (FFT + mel filterbank)
//! - Audio quality assessment (SNR, spectral flatness, rain detection)
//! - Audio resampling (rubato polynomial interpolation)
//!
//! Run with:
//! ```bash
//! cargo bench -p birdnet-core
//! # HTML report: target/criterion/report/index.html
//! ```

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Shared audio generators
// ---------------------------------------------------------------------------

/// Generate a synthetic mono audio chunk containing a bird-like call.
///
/// The signal contains a fundamental at `fund_hz` plus three harmonics,
/// amplitude-modulated with a 10 Hz envelope, simulating a brief song phrase.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn synthetic_bird_call(duration_secs: f32, sample_rate: u32, fund_hz: f32) -> Vec<f32> {
    let n = (duration_secs * sample_rate as f32) as usize;
    let envelope_freq = 10.0_f32; // 10 Hz AM
    (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let envelope = (2.0 * PI * envelope_freq * t).sin().mul_add(0.5, 0.5);
            let harmonic = |h: f32| -> f32 { (1.0 / h) * (2.0 * PI * fund_hz * h * t).sin() };
            envelope * (harmonic(1.0) + harmonic(2.0) + harmonic(3.0) + harmonic(4.0)) * 0.2
        })
        .collect()
}

/// Generate white noise.
#[allow(clippy::cast_precision_loss)]
fn white_noise(n_samples: usize) -> Vec<f32> {
    // Simple deterministic pseudo-random (LCG) — no rand dependency
    let mut x: u64 = 0xDEAD_BEEF_CAFE_BABE;
    (0..n_samples)
        .map(|_| {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((x >> 33) as f32 / u32::MAX as f32).mul_add(2.0, -1.0)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Spectrogram benchmarks
// ---------------------------------------------------------------------------

fn bench_mel_spectrogram(c: &mut Criterion) {
    use birdnet_core::audio::spectrogram::{MelConfig, mel_spectrogram};

    let mel_config = MelConfig::default();
    let mut group = c.benchmark_group("mel_spectrogram");

    for duration in [3.0_f32, 9.0, 30.0] {
        let samples = synthetic_bird_call(duration, 48_000, 3_000.0);
        group.bench_with_input(
            BenchmarkId::new("bird_call", format!("{duration:.0}s")),
            &samples,
            |b, s| b.iter(|| mel_spectrogram(s, 48_000, &mel_config).unwrap()),
        );
    }

    // Worst case: white noise (high spectral flatness, stresses filterbank)
    let noise = white_noise(48_000 * 9); // 9 seconds
    group.bench_with_input(BenchmarkId::new("white_noise", "9s"), &noise, |b, s| {
        b.iter(|| mel_spectrogram(s, 48_000, &mel_config).unwrap());
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Audio quality benchmarks
// ---------------------------------------------------------------------------

fn bench_audio_quality(c: &mut Criterion) {
    use birdnet_core::audio::quality::assess_quality;

    let mut group = c.benchmark_group("audio_quality");

    // 3-second bird call
    let bird = synthetic_bird_call(3.0, 48_000, 4_000.0);
    group.bench_function("assess_quality/bird_call_3s", |b| {
        b.iter(|| assess_quality(&bird, 48_000).unwrap());
    });

    // 3-second white noise (worst case for SNR estimator)
    let noise = white_noise(48_000 * 3);
    group.bench_function("assess_quality/white_noise_3s", |b| {
        b.iter(|| assess_quality(&noise, 48_000).unwrap());
    });

    group.finish();
}

fn bench_snr_estimation(c: &mut Criterion) {
    use birdnet_core::audio::quality::snr::estimate_snr;

    let mut group = c.benchmark_group("snr_estimation");

    for n_samples in [4_096_usize, 48_000, 144_000] {
        #[allow(clippy::cast_precision_loss)]
        let samples = synthetic_bird_call(n_samples as f32 / 48_000.0, 48_000, 2_500.0);
        group.bench_with_input(
            BenchmarkId::new("estimate_snr", n_samples),
            &samples,
            |b, s| b.iter(|| estimate_snr(s)),
        );
    }

    group.finish();
}

fn bench_rain_detection(c: &mut Criterion) {
    use birdnet_core::audio::quality::rain_detector::assess_environment;

    let mut group = c.benchmark_group("rain_detection");

    // Typical 3-second analysis window at 48 kHz
    let samples = synthetic_bird_call(3.0, 48_000, 3_000.0);
    group.bench_function("assess_environment/3s", |b| {
        b.iter(|| assess_environment(&samples, 48_000));
    });

    // Noisy signal (stresses HF energy estimation)
    let noise = white_noise(48_000 * 3);
    group.bench_function("assess_environment/noise_3s", |b| {
        b.iter(|| assess_environment(&noise, 48_000));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Noise floor tracker benchmark
// ---------------------------------------------------------------------------

fn bench_noise_floor_tracker(c: &mut Criterion) {
    use birdnet_core::audio::quality::NoiseFloorTracker;

    let mut group = c.benchmark_group("noise_floor_tracker");

    let frame = synthetic_bird_call(512.0 / 48_000.0, 48_000, 2_000.0);
    group.bench_function("update/single_frame", |b| {
        let mut tracker = NoiseFloorTracker::new();
        b.iter(|| tracker.update(&frame));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion entry points
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_mel_spectrogram,
    bench_audio_quality,
    bench_snr_estimation,
    bench_rain_detection,
    bench_noise_floor_tracker,
);
criterion_main!(benches);
