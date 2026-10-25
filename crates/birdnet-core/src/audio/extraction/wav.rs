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
        // Scale by 32768, ROUND, and saturate to the i16 range. 32768 because
        // that is what the decoder divided by (`i16 / 32768`), so a 16-bit
        // recording comes back exactly: scaling by 32767 changed about half of
        // all values by one step and could not write -32768. Rounding because
        // a bare `as` cast truncates toward zero, which doubles the
        // quantisation noise (0.58 LSB RMS against 0.29) in every clip a person
        // listens to. Full scale +1.0 saturates at 32767.
        let scaled = (sample * 32_768.0)
            .round()
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
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

    /// A clip's samples are rounded to the nearest 16-bit value, not cut toward
    /// zero: 2.6 LSB is stored as 3, and -2.6 LSB as -3. Full scale stays full
    /// scale, and anything beyond it is clamped.
    #[test]
    fn clip_samples_are_rounded_not_truncated() {
        let lsb = 1.0 / 32_768.0;
        let samples = [
            2.6 * lsb,
            -2.6 * lsb,
            2.4 * lsb,
            -2.4 * lsb,
            1.0,
            -1.0,
            1.7,
            -1.7,
        ];
        let dir = std::env::temp_dir().join(format!("bnb-wav-round-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("round.wav");
        write_wav_to(&samples, 48_000, &path).unwrap();
        let got: Vec<i16> = hound::WavReader::open(&path)
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            got,
            vec![3, -3, 2, -2, i16::MAX, i16::MIN, i16::MAX, i16::MIN]
        );
    }

    /// A 16-bit recording exported as a clip comes back as the same 16-bit
    /// samples. The decoder reads `i16 / 32768`; the writer scaled by 32767, so
    /// every sample past about a quarter of full scale came back one step
    /// smaller in magnitude, and `-32768` could not be written at all. Every one
    /// of the 65 536 values goes through the real decoder and the real writer.
    #[test]
    fn a_sixteen_bit_recording_exports_bit_for_bit() {
        let dir = std::env::temp_dir().join(format!("bnb-wav-exact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.wav");
        let exported = dir.join("exported.wav");

        let all: Vec<i16> = (i16::MIN..=i16::MAX).collect();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&source, spec).unwrap();
        for &v in &all {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();

        let decoded = decode_file(&source).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(
            decoded.samples.len(),
            all.len(),
            "precondition: nothing resampled or dropped"
        );
        write_wav_to(&decoded.samples, 48_000, &exported).unwrap();
        let back: Vec<i16> = hound::WavReader::open(&exported)
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();

        let changed: Vec<(i16, i16)> = all
            .iter()
            .zip(&back)
            .filter(|(a, b)| a != b)
            .map(|(a, b)| (*a, *b))
            .collect();
        assert!(
            changed.is_empty(),
            "{} of 65536 samples changed on export; first: {:?}",
            changed.len(),
            changed.first()
        );
    }

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
