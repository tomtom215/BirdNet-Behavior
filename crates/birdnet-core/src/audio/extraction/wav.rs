//! WAV writing and spectrogram generation for extracted audio clips.

use std::path::Path;

use crate::audio::decode::decode_file;
use crate::audio::spectrogram::{MelConfig, MelSpectrogram, SpectrogramError, mel_spectrogram};

use super::ExtractionError;

/// Write mono f32 samples to a 16-bit PCM WAV file.
///
/// The file appears under `output_path` whole or not at all (PS-7, S-4): it
/// is written as a `.part` sibling, synced, renamed into place, and the
/// directory synced. A crash or a power cut mid-write leaves the `.part`
/// behind and nothing under the final name, so no database row can point at
/// a truncated clip. On any error the `.part` is removed.
///
/// # Errors
///
/// Returns [`ExtractionError::Write`] if writing fails.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn write_wav_clip(
    samples: &[f32],
    sample_rate: u32,
    output_path: &Path,
) -> Result<(), ExtractionError> {
    let part = crate::atomic_file::part_path(output_path);
    let written = write_wav_to(samples, sample_rate, &part).and_then(|()| {
        crate::atomic_file::commit(&part, output_path)
            .map_err(|e| ExtractionError::Write(e.to_string()))
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

#[allow(clippy::cast_possible_truncation)]
fn write_wav_to(samples: &[f32], sample_rate: u32, path: &Path) -> Result<(), ExtractionError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| ExtractionError::Write(e.to_string()))?;

    for &sample in samples {
        // Clamp to [-1.0, 1.0] then scale to i16 range.
        let clamped = sample.clamp(-1.0, 1.0);
        let scaled = (clamped * f32::from(i16::MAX)) as i16;
        writer
            .write_sample(scaled)
            .map_err(|e| ExtractionError::Write(e.to_string()))?;
    }

    writer
        .finalize()
        .map_err(|e| ExtractionError::Write(e.to_string()))
}

/// Generate a mel spectrogram for an extracted audio clip.
///
/// Returns the computed [`MelSpectrogram`] in dB scale, suitable for PNG
/// rendering. Uses the same mel configuration as the `BirdNET` model pipeline.
///
/// # Errors
///
/// Returns [`ExtractionError`] if the file cannot be decoded or the
/// spectrogram computation fails.
pub fn generate_spectrogram(
    audio_path: &Path,
    mel_config: &MelConfig,
) -> Result<MelSpectrogram, ExtractionError> {
    let audio = decode_file(audio_path)?;

    let mel =
        mel_spectrogram(&audio.samples, audio.sample_rate, mel_config).map_err(|e| match e {
            SpectrogramError::InputTooShort { samples, n_fft } => ExtractionError::Decode(format!(
                "audio too short for spectrogram: {samples} samples < {n_fft} n_fft"
            )),
            SpectrogramError::InvalidConfig(msg) | SpectrogramError::Fft(msg) => {
                ExtractionError::Decode(msg)
            }
        })?;

    // Convert to dB scale for visual rendering.
    Ok(mel.to_db(1.0, 80.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the worker below writes: handed over in an environment variable
    /// because a `#[ignore]`d test takes no arguments.
    const WORKER_TARGET: &str = "BNB_WAV_WORKER_TARGET";

    /// The clip the worker writes: 600 s at 48 kHz, ~58 MB of PCM, so the
    /// write takes long enough to be interrupted.
    const WORKER_SAMPLES: u32 = 48_000 * 600;

    /// The worker: write a clip large enough that a kill can land inside the
    /// write. Run only by the gate below, with the target path in the
    /// environment; a bare `cargo test` skips it.
    #[test]
    #[ignore = "worker for a_kill_mid_write_leaves_nothing_under_the_final_name"]
    fn write_a_large_clip_worker() {
        let Ok(target) = std::env::var(WORKER_TARGET) else {
            return;
        };
        let samples: Vec<f32> = (0..WORKER_SAMPLES)
            .map(|i| f32::from(u8::try_from(i % 97).unwrap_or(0)) / 97.0)
            .collect();
        write_wav_clip(&samples, 48_000, Path::new(&target)).unwrap();
    }

    /// PS-7 / S-4: a clip is whole or absent under its final name.
    ///
    /// The worker is killed the moment a file appears in the directory —
    /// that is, once the write has begun and long before it ends — and
    /// whatever is left is asserted: either nothing under the final name (a
    /// `.part` may remain) or, if the kill somehow came late, a complete WAV
    /// that decodes to every sample. A clip written straight to its final
    /// name leaves a truncated file there, which fails the second arm.
    ///
    /// Waiting for the file rather than sleeping a fixed time is what makes
    /// this a discriminator: a kill that lands before the worker has opened
    /// anything leaves nothing under either design.
    #[test]
    fn a_kill_mid_write_leaves_nothing_under_the_final_name() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("clip.wav");
        let exe = std::env::current_exe().unwrap();
        let mut interrupted = 0;
        for _attempt in 0..3 {
            for entry in std::fs::read_dir(dir.path()).unwrap() {
                let _ = std::fs::remove_file(entry.unwrap().path());
            }
            let mut child = std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "audio::extraction::wav::tests::write_a_large_clip_worker",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env(WORKER_TARGET, &final_path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn the worker");
            // Kill as soon as the worker has created a file, i.e. while it is
            // writing; give up waiting after ten seconds.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut finished = false;
            loop {
                if child.try_wait().unwrap().is_some() {
                    finished = true;
                    break;
                }
                if std::fs::read_dir(dir.path()).unwrap().next().is_some() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the worker never started writing"
                );
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
            if !finished {
                child.kill().unwrap();
                let _ = child.wait();
                interrupted += 1;
            }
            if let Ok(meta) = std::fs::metadata(&final_path) {
                    // Present means complete: it decodes to every sample.
                    let reader = hound::WavReader::open(&final_path).unwrap_or_else(|e| {
                        panic!(
                            "a file under the final name must be a whole WAV ({} bytes): {e}",
                            meta.len()
                        )
                    });
                    assert_eq!(
                        reader.len(),
                        WORKER_SAMPLES,
                        "a file under the final name must be the whole clip ({} bytes)",
                        meta.len()
                    );
            }
        }
        assert!(
            interrupted > 0,
            "no attempt was interrupted; every worker finished first"
        );
    }
}
