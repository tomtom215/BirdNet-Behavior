//! WAV writing and spectrogram generation for extracted audio clips.

use std::path::Path;

use crate::audio::decode::decode_file;
use crate::audio::spectrogram::{MelConfig, MelSpectrogram, SpectrogramError, mel_spectrogram};

use super::ExtractionError;

/// Write mono f32 samples to a 16-bit PCM WAV file.
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
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|e| ExtractionError::Write(e.to_string()))?;

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
        .map_err(|e| ExtractionError::Write(e.to_string()))?;

    Ok(())
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
