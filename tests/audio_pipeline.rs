//! Integration tests for the full audio pipeline: decode -> resample -> spectrogram.
//!
//! Generates synthetic WAV files with known characteristics and validates
//! the entire pipeline produces correct output at each stage.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use std::path::Path;

use birdnet_core::audio::decode;
use birdnet_core::audio::resample;
use birdnet_core::audio::spectrogram::{self, MelConfig};
use birdnet_core::detection::pipeline::{self, PipelineConfig};

/// Generate a WAV file containing a sine wave at the specified frequency.
fn generate_test_wav(path: &Path, sample_rate: u32, duration_secs: f32, frequency: f32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create WAV");
    let num_samples = (sample_rate as f32 * duration_secs) as u32;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
        let amplitude = (sample * i16::MAX as f32) as i16;
        writer
            .write_sample(amplitude)
            .expect("failed to write sample");
    }

    writer.finalize().expect("failed to finalize WAV");
}

/// Generate a multi-tone WAV with several bird-like frequency components.
fn generate_birdsong_wav(path: &Path, sample_rate: u32, duration_secs: f32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create WAV");
    let num_samples = (sample_rate as f32 * duration_secs) as u32;

    // Simulate bird-like frequencies: fundamental + harmonics with frequency modulation
    let freqs = [2000.0_f32, 3500.0, 4200.0, 5000.0, 6500.0];
    let amps = [0.3_f32, 0.25, 0.2, 0.15, 0.1];

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let mut sample = 0.0_f32;

        for (freq, amp) in freqs.iter().zip(amps.iter()) {
            // Add slight frequency modulation (bird-like warble)
            let fm = 5.0 * (2.0 * std::f32::consts::PI * 8.0 * t).sin();
            sample += amp * (2.0 * std::f32::consts::PI * (freq + fm) * t).sin();
        }

        // Apply amplitude envelope (attack-sustain-release)
        let envelope = if t < 0.1 {
            t / 0.1 // attack
        } else if t > duration_secs - 0.2 {
            (duration_secs - t) / 0.2 // release
        } else {
            1.0 // sustain
        };

        sample *= envelope;
        let amplitude = (sample * i16::MAX as f32 * 0.8) as i16;
        writer
            .write_sample(amplitude)
            .expect("failed to write sample");
    }

    writer.finalize().expect("failed to finalize WAV");
}

/// Generate a stereo WAV file (for mono downmix testing).
fn generate_stereo_wav(path: &Path, sample_rate: u32, duration_secs: f32) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create WAV");
    let num_samples = (sample_rate as f32 * duration_secs) as u32;

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Left channel: 440 Hz
        let left = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        // Right channel: 880 Hz
        let right = (2.0 * std::f32::consts::PI * 880.0 * t).sin();

        writer
            .write_sample((left * i16::MAX as f32) as i16)
            .unwrap();
        writer
            .write_sample((right * i16::MAX as f32) as i16)
            .unwrap();
    }

    writer.finalize().expect("failed to finalize WAV");
}

// --- Decode tests ---

#[test]
fn decode_mono_wav_48khz() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_48k_mono.wav");
    generate_test_wav(&path, 48000, 3.0, 1000.0);

    let audio = decode::decode_file(&path).unwrap();
    assert_eq!(audio.sample_rate, 48000);

    // 3 seconds at 48kHz = 144000 samples
    let expected = 144_000;
    let tolerance = 100; // WAV encoding might add/trim a few samples
    assert!(
        (audio.samples.len() as i64 - expected as i64).unsigned_abs() < tolerance,
        "expected ~{expected} samples, got {}",
        audio.samples.len()
    );

    // Samples should be in [-1.0, 1.0]
    for &s in &audio.samples {
        assert!(s.abs() <= 1.001, "sample out of range: {s}");
    }
}

#[test]
fn decode_stereo_to_mono() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_stereo.wav");
    generate_stereo_wav(&path, 44100, 1.0);

    let audio = decode::decode_file(&path).unwrap();
    assert_eq!(audio.sample_rate, 44100);

    // Should be mono (averaged from stereo)
    let expected = 44100;
    let tolerance = 100;
    assert!(
        (audio.samples.len() as i64 - expected as i64).unsigned_abs() < tolerance,
        "expected ~{expected} mono samples, got {}",
        audio.samples.len()
    );
}

#[test]
fn decode_16khz_wav() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_16k.wav");
    generate_test_wav(&path, 16000, 2.0, 440.0);

    let audio = decode::decode_file(&path).unwrap();
    assert_eq!(audio.sample_rate, 16000);
    assert!(audio.samples.len() > 30000);
}

// --- Resample tests ---

#[test]
fn resample_48k_to_16k() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resample_48k.wav");
    generate_test_wav(&path, 48000, 1.0, 440.0);

    let audio = decode::decode_file(&path).unwrap();
    let resampled = resample::resample(&audio.samples, 48000, 16000).unwrap();

    // Should be approximately 1/3 the number of samples
    let ratio = audio.samples.len() as f64 / resampled.len() as f64;
    assert!(
        (ratio - 3.0).abs() < 0.2,
        "expected 3:1 ratio, got {ratio:.2}"
    );
}

#[test]
fn resample_44100_to_48000() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resample_44k.wav");
    generate_test_wav(&path, 44100, 1.0, 440.0);

    let audio = decode::decode_file(&path).unwrap();
    let resampled = resample::resample(&audio.samples, 44100, 48000).unwrap();

    // Should be slightly more samples (48000/44100 ratio)
    assert!(resampled.len() > audio.samples.len());
    let ratio = resampled.len() as f64 / audio.samples.len() as f64;
    assert!(
        (ratio - 48000.0 / 44100.0).abs() < 0.1,
        "unexpected ratio: {ratio:.4}"
    );
}

#[test]
fn resample_passthrough_same_rate() {
    let samples: Vec<f32> = (0..48000).map(|i| (i as f32 / 48000.0).sin()).collect();
    let result = resample::resample(&samples, 48000, 48000).unwrap();
    assert_eq!(result.len(), samples.len());
    assert_eq!(result, samples);
}

// --- Spectrogram tests ---

#[test]
fn spectrogram_from_wav_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spec_test.wav");
    generate_test_wav(&path, 48000, 3.0, 1000.0);

    let audio = decode::decode_file(&path).unwrap();
    let config = MelConfig::default();
    let mel = spectrogram::mel_spectrogram(&audio.samples, audio.sample_rate, &config).unwrap();

    assert_eq!(mel.n_mels, 128);
    assert!(mel.n_frames > 0);
    assert_eq!(mel.data.len(), mel.n_mels * mel.n_frames);

    // All values should be finite
    for &v in &mel.data {
        assert!(v.is_finite(), "spectrogram has non-finite value: {v}");
    }
}

#[test]
fn spectrogram_birdsong_has_energy_in_high_freq_bands() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("birdsong_spec.wav");
    generate_birdsong_wav(&path, 48000, 3.0);

    let audio = decode::decode_file(&path).unwrap();
    let config = MelConfig::default();
    let mel = spectrogram::mel_spectrogram(&audio.samples, audio.sample_rate, &config).unwrap();

    // Bird frequencies (2-6.5 kHz) should show energy in mid-to-upper mel bands
    // Bottom 16 bands (< ~300 Hz) should have less energy than middle bands
    let mut low_energy = 0.0_f64;
    for band in 0..16 {
        for frame in 0..mel.n_frames {
            low_energy += f64::from(mel.get(band, frame));
        }
    }

    let mut mid_energy = 0.0_f64;
    for band in 32..80 {
        for frame in 0..mel.n_frames {
            mid_energy += f64::from(mel.get(band, frame));
        }
    }

    // Normalize by number of bands
    low_energy /= 16.0;
    mid_energy /= 48.0;

    assert!(
        mid_energy > low_energy,
        "birdsong frequencies should have more energy in mid bands: low={low_energy:.2}, mid={mid_energy:.2}"
    );
}

#[test]
fn spectrogram_to_db_range() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db_test.wav");
    generate_test_wav(&path, 48000, 1.0, 440.0);

    let audio = decode::decode_file(&path).unwrap();
    let config = MelConfig::default();
    let mel = spectrogram::mel_spectrogram(&audio.samples, audio.sample_rate, &config).unwrap();
    let db = mel.to_db(1.0, 80.0);

    // dB values should all be finite
    for &v in &db.data {
        assert!(v.is_finite(), "dB value not finite: {v}");
    }

    let max_db = db
        .data
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, |a, b| a.max(f64::from(b)));
    let min_db = db
        .data
        .iter()
        .copied()
        .fold(f64::INFINITY, |a, b| a.min(f64::from(b)));

    // Dynamic range should be at most top_db (80 dB)
    assert!(
        max_db - min_db <= 80.1,
        "dynamic range exceeds top_db: {:.1} dB",
        max_db - min_db
    );
}

// --- Full pipeline tests ---

#[test]
fn full_pipeline_process_file() {
    let dir = tempfile::tempdir().unwrap();

    // Create a file with BirdNET-Pi naming convention
    let path = dir.path().join("2026-03-12-birdnet-06:30:00.wav");
    generate_birdsong_wav(&path, 48000, 6.0); // 6 seconds = 2 chunks of 3s

    let config = PipelineConfig {
        watch_dir: dir.path().to_path_buf(),
        ..PipelineConfig::default()
    };

    let chunks = pipeline::process_file(&path, &config).unwrap();

    // 6 seconds / 3 second chunks = 2 chunks
    assert_eq!(chunks.len(), 2, "expected 2 chunks for 6s audio");

    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.spectrogram.n_mels, 128);
        assert!(chunk.spectrogram.n_frames > 0);
        assert_eq!(chunk.recording.date, "2026-03-12");
        assert_eq!(chunk.recording.time, "06:30:00");

        // Verify chunk timing
        if i == 0 {
            assert!((chunk.start_secs - 0.0).abs() < 0.01);
            assert!((chunk.end_secs - 3.0).abs() < 0.01);
        } else {
            assert!((chunk.start_secs - 3.0).abs() < 0.01);
            assert!((chunk.end_secs - 6.0).abs() < 0.01);
        }
    }
}

#[test]
fn full_pipeline_short_file_pads_to_chunk_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-03-12-birdnet-07:00:00.wav");
    generate_test_wav(&path, 48000, 1.5, 440.0); // Only 1.5 seconds

    let config = PipelineConfig::default();
    let chunks = pipeline::process_file(&path, &config).unwrap();

    // Should produce 1 zero-padded chunk
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].spectrogram.n_mels, 128);
}

#[test]
fn full_pipeline_rtsp_filename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-03-12-birdnet-cam1-08:00:00.wav");
    generate_test_wav(&path, 48000, 3.0, 1000.0);

    let config = PipelineConfig::default();
    let chunks = pipeline::process_file(&path, &config).unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].recording.rtsp_id.as_deref(), Some("cam1"));
}

// --- File watcher tests ---

#[test]
fn file_watcher_detects_new_wav() {
    let dir = tempfile::tempdir().unwrap();

    let (_watcher, rx) = pipeline::watch_directory(dir.path()).unwrap();

    // Create a WAV file after the watcher is started
    std::thread::sleep(std::time::Duration::from_millis(100));
    let path = dir.path().join("2026-03-12-birdnet-09:00:00.wav");
    generate_test_wav(&path, 48000, 1.0, 440.0);

    // Wait for the event
    let received = rx.recv_timeout(std::time::Duration::from_secs(5));
    assert!(received.is_ok(), "watcher should detect new WAV file");

    let received_path = received.unwrap();
    assert_eq!(
        received_path.file_name().unwrap().to_str().unwrap(),
        "2026-03-12-birdnet-09:00:00.wav"
    );
}

#[test]
fn file_watcher_ignores_non_audio_files() {
    let dir = tempfile::tempdir().unwrap();

    let (_watcher, rx) = pipeline::watch_directory(dir.path()).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Create a non-audio file
    std::fs::write(dir.path().join("readme.txt"), "not audio").unwrap();

    // Should timeout (no event for .txt files)
    let received = rx.recv_timeout(std::time::Duration::from_secs(1));
    assert!(received.is_err(), "watcher should ignore non-audio files");
}

// --- Benchmark-style timing tests ---

#[test]
fn pipeline_latency_under_5_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-03-12-birdnet-10:00:00.wav");
    generate_birdsong_wav(&path, 48000, 3.0);

    let config = PipelineConfig::default();

    let start = std::time::Instant::now();
    let chunks = pipeline::process_file(&path, &config).unwrap();
    let elapsed = start.elapsed();

    assert!(!chunks.is_empty());
    assert!(
        elapsed.as_secs() < 5,
        "pipeline took too long: {elapsed:?} (should be <5s for 3s audio)"
    );

    // Log for benchmarking
    eprintln!(
        "Pipeline latency for 3s audio: {:.1}ms ({} chunks, {} mel frames)",
        elapsed.as_secs_f64() * 1000.0,
        chunks.len(),
        chunks[0].spectrogram.n_frames
    );
}

// --- Real audio tests using Pica pica (Eurasian Magpie) detection from BirdNET-Pi ---

/// Path to the real test WAV from the BirdNET-Pi test suite.
const PICA_PICA_WAV: &str = "tests/testdata/Pica_pica_30s.wav";

#[test]
fn real_audio_decode_pica_pica() {
    let path = std::path::Path::new(PICA_PICA_WAV);
    if !path.exists() {
        eprintln!("skipping: {PICA_PICA_WAV} not found");
        return;
    }

    let audio = decode::decode_file(path).unwrap();
    assert_eq!(audio.sample_rate, 48000, "Pica pica should be 48kHz");

    // 30 seconds at 48kHz = 1_440_000 samples
    let expected = 1_440_000;
    let tolerance = 1000;
    assert!(
        (audio.samples.len() as i64 - expected as i64).unsigned_abs() < tolerance,
        "expected ~{expected} samples, got {}",
        audio.samples.len()
    );

    // Verify audio is normalized to [-1.0, 1.0]
    let max_abs = audio
        .samples
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(
        max_abs <= 1.001,
        "max absolute sample {max_abs} exceeds 1.0"
    );
    assert!(max_abs > 0.01, "audio appears silent (max abs {max_abs})");

    eprintln!(
        "Pica pica: {} samples, {:.1}s, max_abs={max_abs:.4}",
        audio.samples.len(),
        audio.samples.len() as f64 / 48000.0
    );
}

#[test]
fn real_audio_resample_pica_pica_to_16k() {
    let path = std::path::Path::new(PICA_PICA_WAV);
    if !path.exists() {
        eprintln!("skipping: {PICA_PICA_WAV} not found");
        return;
    }

    let audio = decode::decode_file(path).unwrap();
    let resampled = resample::resample(&audio.samples, 48000, 16000).unwrap();

    // 30s at 16kHz = 480_000 samples
    let expected = 480_000;
    let tolerance = 1000;
    assert!(
        (resampled.len() as i64 - expected as i64).unsigned_abs() < tolerance,
        "expected ~{expected} resampled samples, got {}",
        resampled.len()
    );

    // Resampled audio should still be in valid range
    let max_abs = resampled
        .iter()
        .copied()
        .fold(0.0_f32, |a, b| a.max(b.abs()));
    assert!(max_abs <= 1.5, "resampled max_abs {max_abs} is too large");

    eprintln!(
        "Pica pica resampled: {} -> {} samples (ratio {:.3})",
        audio.samples.len(),
        resampled.len(),
        audio.samples.len() as f64 / resampled.len() as f64
    );
}

#[test]
fn real_audio_spectrogram_pica_pica() {
    let path = std::path::Path::new(PICA_PICA_WAV);
    if !path.exists() {
        eprintln!("skipping: {PICA_PICA_WAV} not found");
        return;
    }

    let audio = decode::decode_file(path).unwrap();
    let config = MelConfig::default();
    let mel = spectrogram::mel_spectrogram(&audio.samples, audio.sample_rate, &config).unwrap();

    assert_eq!(mel.n_mels, 128);
    assert!(
        mel.n_frames > 100,
        "30s audio should produce many frames, got {}",
        mel.n_frames
    );

    // All values should be finite
    let non_finite = mel.data.iter().filter(|v| !v.is_finite()).count();
    assert_eq!(
        non_finite, 0,
        "spectrogram has {non_finite} non-finite values"
    );

    // Compute energy distribution across mel bands for diagnostic output
    let total_energy: f64 = mel.data.iter().map(|&v| f64::from(v)).sum();
    assert!(
        total_energy > 0.0,
        "spectrogram should have non-zero energy"
    );

    // Verify the dB-scale spectrogram has reasonable dynamic range
    let db = mel.to_db(1.0, 80.0);
    let max_db = db
        .data
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, |a, b| a.max(f64::from(b)));
    let min_db = db
        .data
        .iter()
        .copied()
        .fold(f64::INFINITY, |a, b| a.min(f64::from(b)));

    eprintln!(
        "Pica pica spectrogram: {}x{} (mels x frames), total_energy={total_energy:.1}, dB range=[{min_db:.1}, {max_db:.1}]",
        mel.n_mels, mel.n_frames
    );

    assert!(
        max_db - min_db > 10.0,
        "spectrogram should have >10 dB dynamic range, got {:.1} dB",
        max_db - min_db
    );
}

#[test]
fn real_audio_full_pipeline_pica_pica() {
    let pica_path = std::path::Path::new(PICA_PICA_WAV);
    if !pica_path.exists() {
        eprintln!("skipping: {PICA_PICA_WAV} not found");
        return;
    }

    // Copy to a temp dir with BirdNET-Pi filename convention
    let dir = tempfile::tempdir().unwrap();
    let pipeline_path = dir.path().join("2026-03-12-birdnet-06:30:00.wav");
    std::fs::copy(pica_path, &pipeline_path).unwrap();

    let config = PipelineConfig::default();

    let start = std::time::Instant::now();
    let chunks = pipeline::process_file(&pipeline_path, &config).unwrap();
    let elapsed = start.elapsed();

    // 30 seconds / 3 second chunks = 10 chunks
    assert!(
        chunks.len() >= 9 && chunks.len() <= 10,
        "expected 9-10 chunks for 30s audio, got {}",
        chunks.len()
    );

    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.spectrogram.n_mels, 128);
        assert!(chunk.spectrogram.n_frames > 0);
        assert_eq!(chunk.recording.date, "2026-03-12");
        assert_eq!(chunk.recording.time, "06:30:00");

        // All spectrogram values must be finite
        let non_finite = chunk
            .spectrogram
            .data
            .iter()
            .filter(|v| !v.is_finite())
            .count();
        assert_eq!(
            non_finite, 0,
            "chunk {i} has {non_finite} non-finite spectrogram values"
        );
    }

    assert!(
        elapsed.as_secs() < 15,
        "pipeline took too long for 30s audio: {elapsed:?} (should be <15s)"
    );

    eprintln!(
        "Pica pica full pipeline: {:.1}ms for 30s audio ({} chunks, {:.1}ms/chunk)",
        elapsed.as_secs_f64() * 1000.0,
        chunks.len(),
        elapsed.as_secs_f64() * 1000.0 / chunks.len() as f64
    );
}
