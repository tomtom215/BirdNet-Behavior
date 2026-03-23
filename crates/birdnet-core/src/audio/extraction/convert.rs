//! Audio format conversion and frequency shifting.
//!
//! Handles conversion from WAV to MP3, FLAC, and OGG using ffmpeg or sox,
//! and frequency shifting for accessibility.

use std::path::Path;

use super::{AudioFormat, ExtractionError};

/// Convert a WAV file to the target format using ffmpeg (preferred) or sox.
///
/// On success the source WAV file is removed.
///
/// # Errors
///
/// Returns [`ExtractionError::Conversion`] if neither ffmpeg nor sox is
/// available or the conversion process fails.
pub(super) fn convert_audio_format(
    wav_path: &Path,
    output_path: &Path,
    format: AudioFormat,
) -> Result<(), ExtractionError> {
    // Try ffmpeg first, fall back to sox.
    let result = convert_with_ffmpeg(wav_path, output_path, format)
        .or_else(|_| convert_with_sox(wav_path, output_path));

    match result {
        Ok(()) => {
            // Remove the intermediate WAV file.
            let _ = std::fs::remove_file(wav_path);
            Ok(())
        }
        Err(e) => {
            // Clean up the intermediate WAV (rename it to the target as fallback).
            tracing::warn!(
                error = %e,
                format = %format,
                "format conversion failed, keeping WAV"
            );
            std::fs::rename(wav_path, output_path)?;
            Ok(())
        }
    }
}

/// Convert WAV to target format using ffmpeg.
fn convert_with_ffmpeg(
    wav_path: &Path,
    output_path: &Path,
    format: AudioFormat,
) -> Result<(), ExtractionError> {
    use std::process::Command;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y") // overwrite
        .arg("-i")
        .arg(wav_path)
        .arg("-loglevel")
        .arg("error");

    // Format-specific encoding options.
    match format {
        AudioFormat::Mp3 => {
            cmd.args(["-codec:a", "libmp3lame", "-q:a", "2"]);
        }
        AudioFormat::Flac => {
            cmd.args(["-codec:a", "flac"]);
        }
        AudioFormat::Ogg => {
            cmd.args(["-codec:a", "libvorbis", "-q:a", "4"]);
        }
        AudioFormat::Wav => return Ok(()),
    }

    cmd.arg(output_path);

    let output = cmd
        .output()
        .map_err(|e| ExtractionError::Conversion(format!("ffmpeg: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ExtractionError::Conversion(format!(
            "ffmpeg exited {}: {}",
            output.status,
            stderr.trim()
        )))
    }
}

/// Convert WAV to target format using sox.
fn convert_with_sox(wav_path: &Path, output_path: &Path) -> Result<(), ExtractionError> {
    use std::process::Command;

    let output = Command::new("sox")
        .arg(wav_path)
        .arg(output_path)
        .output()
        .map_err(|e| ExtractionError::Conversion(format!("sox: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(ExtractionError::Conversion(format!(
            "sox exited {}: {}",
            output.status,
            stderr.trim()
        )))
    }
}

/// Apply frequency shifting to a WAV file using ffmpeg (preferred) or sox.
///
/// Uses the `asetrate` + `aresample` ffmpeg filter to shift pitch by the given
/// number of Hz, or the sox `pitch` effect as a fallback.
///
/// Returns `true` on success, `false` if both tools fail or are unavailable.
/// BirdNET-Pi equivalent: `FREQ_SHIFT` config applied via sox/rubberband.
pub(super) fn apply_freq_shift(
    input_path: &Path,
    output_path: &Path,
    sample_rate: u32,
    shift_hz: i32,
) -> bool {
    use std::process::Command;

    // ffmpeg approach: use asetrate to shift the sample rate, then resample back.
    // This is equivalent to speeding up/slowing down, shifting all frequencies.
    // shift_hz > 0 shifts up (makes calls accessible to those with high-freq hearing loss).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let new_rate =
        (f64::from(sample_rate) * (1.0 + f64::from(shift_hz) / f64::from(sample_rate))) as u32;
    let filter = format!("asetrate={new_rate},aresample={sample_rate}");

    let ffmpeg_ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &input_path.to_string_lossy(),
            "-af",
            &filter,
            "-loglevel",
            "error",
            &output_path.to_string_lossy(),
        ])
        .status()
        .is_ok_and(|s| s.success());

    if ffmpeg_ok {
        return true;
    }

    // sox fallback: use pitch effect (shift in cents, ~100 cents = 1 semitone).
    // 1 Hz shift ~ 100 * log2(1 + shift_hz / sample_rate) * 100 cents (approximation).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let cents = (1200.0f64 * (1.0 + f64::from(shift_hz) / f64::from(sample_rate)).log2()) as i32;

    Command::new("sox")
        .arg(input_path)
        .arg(output_path)
        .args(["pitch", &cents.to_string()])
        .status()
        .is_ok_and(|s| s.success())
}
