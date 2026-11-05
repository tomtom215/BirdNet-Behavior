//! A stretch of exactly constant audio is marked `silent` by the pipeline and
//! nothing else is.
//!
//! The real BirdNET+ V3.0 model answers a window of digital silence with NaN
//! in every output; a muted or unplugged source therefore made a healthy
//! classifier look damaged (see `PreparedChunk::silent`). The unit test beside
//! `infer_chunk` proves a flagged chunk is skipped; this proves the flag is
//! set by `process_file`, from the samples it decoded, and only when it
//! should be — a skip that fired on real birdsong would hide every detection.

use std::path::{Path, PathBuf};

use birdnet_core::detection::pipeline::{self, PipelineConfig};

const RATE: u32 = 32_000;

/// A 16-bit mono WAV of `samples`, named the way the pipeline requires.
fn write_wav(dir: &Path, name: &str, samples: &[i16]) -> PathBuf {
    let data_len = u32::try_from(samples.len() * 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn config(dir: &Path) -> PipelineConfig {
    PipelineConfig {
        watch_dir: dir.to_path_buf(),
        target_sample_rate: RATE,
        chunk_duration_secs: 5.0,
        chunk_overlap_secs: 0.0,
        chunk_step_secs: Some(4.5),
        raw_audio_input: true,
        ..PipelineConfig::default()
    }
}

/// `(start_secs, silent)` for every chunk of `path`.
fn flags(dir: &Path, path: &Path) -> Vec<(f32, bool)> {
    pipeline::process_file(path, &config(dir))
        .expect("the file decodes")
        .iter()
        .map(|c| (c.start_secs, c.silent))
        .collect()
}

/// A deterministic non-constant signal: a ramp that never repeats a run.
fn tone(n: usize) -> Vec<i16> {
    (0..n)
        .map(|i| i16::try_from((i * 37) % 2000).unwrap() - 1000)
        .collect()
}

#[test]
fn digital_silence_is_flagged_and_a_signal_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let rate = RATE as usize;

    let zeros = write_wav(
        dir.path(),
        "2026-05-19-birdnet-06:30:00.wav",
        &vec![0; 30 * rate],
    );
    let zero_flags = flags(dir.path(), &zeros);
    assert!(zero_flags.len() >= 6, "{zero_flags:?}");
    assert!(
        zero_flags.iter().all(|&(_, silent)| silent),
        "{zero_flags:?}"
    );

    let signal = write_wav(
        dir.path(),
        "2026-05-19-birdnet-06:40:00.wav",
        &tone(30 * rate),
    );
    let signal_flags = flags(dir.path(), &signal);
    assert!(
        signal_flags.iter().all(|&(_, silent)| !silent),
        "{signal_flags:?}"
    );

    // Speech-then-silence: the first 12 s are signal, the rest zeros. Chunk
    // starts at 0, 4.5 and 9 s overlap the signal (9 s reaches 14 s); from
    // 13.5 s on every chunk is entirely within the zeros.
    let mut mixed_samples = tone(12 * rate);
    mixed_samples.resize(30 * rate, 0);
    let mixed = write_wav(
        dir.path(),
        "2026-05-19-birdnet-06:50:00.wav",
        &mixed_samples,
    );
    for (start, silent) in flags(dir.path(), &mixed) {
        assert_eq!(silent, start >= 12.0, "chunk at {start} s");
    }
}
