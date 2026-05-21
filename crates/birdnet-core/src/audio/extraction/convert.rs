//! Audio format conversion and frequency shifting.
//!
//! Handles conversion from WAV to MP3, FLAC, and OGG using ffmpeg or sox,
//! and frequency shifting for accessibility.

use std::path::Path;

use super::{AudioFormat, ExtractionError};

/// The ffmpeg `-codec:a` (and quality) arguments for a target format.
///
/// Returns `None` for [`AudioFormat::Wav`] — there is nothing to transcode,
/// so the caller short-circuits. Split out from [`convert_with_ffmpeg`] so
/// the per-format argument mapping is unit-testable without invoking ffmpeg
/// (the command-execution branches still need the real binary, but the
/// codec choice no longer does).
const fn ffmpeg_codec_args(format: AudioFormat) -> Option<&'static [&'static str]> {
    match format {
        AudioFormat::Mp3 => Some(&["-codec:a", "libmp3lame", "-q:a", "2"]),
        AudioFormat::Flac => Some(&["-codec:a", "flac"]),
        AudioFormat::Ogg => Some(&["-codec:a", "libvorbis", "-q:a", "4"]),
        AudioFormat::Wav => None,
    }
}

/// The resampling rate that shifts every frequency component by `shift_hz`.
///
/// ffmpeg's `asetrate` filter reinterprets the sample rate (speeding up or
/// slowing down the audio), which shifts pitch; `aresample` then restores
/// the original rate. A positive `shift_hz` raises pitch, making calls
/// audible to listeners with high-frequency hearing loss.
///
/// Pulled out of [`apply_freq_shift`] so the arithmetic (the `*`, `+`, `/`
/// that cargo-mutants flips) is observable in a unit test rather than only
/// reachable through a live ffmpeg run.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
fn freq_shift_resample_rate(sample_rate: u32, shift_hz: i32) -> u32 {
    (f64::from(sample_rate) * (1.0 + f64::from(shift_hz) / f64::from(sample_rate))) as u32
}

/// The sox `pitch` shift, in cents, equivalent to [`freq_shift_resample_rate`].
///
/// 1200 cents = one octave, so a doubling of frequency is `+1200`. Same
/// extraction rationale as [`freq_shift_resample_rate`]: the `1200.0 * log2`
/// arithmetic is unit-tested directly.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
fn freq_shift_cents(sample_rate: u32, shift_hz: i32) -> i32 {
    (1200.0_f64 * (1.0 + f64::from(shift_hz) / f64::from(sample_rate)).log2()) as i32
}

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
            if let Err(e) = std::fs::remove_file(wav_path) {
                tracing::debug!(path = %wav_path.display(), error = %e, "failed to remove intermediate WAV");
            }
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

    // WAV needs no transcode — short-circuit before spawning ffmpeg.
    let Some(codec_args) = ffmpeg_codec_args(format) else {
        return Ok(());
    };

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y") // overwrite
        .arg("-i")
        .arg(wav_path)
        .arg("-loglevel")
        .arg("error");
    cmd.args(codec_args);
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
    let new_rate = freq_shift_resample_rate(sample_rate, shift_hz);
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
    let cents = freq_shift_cents(sample_rate, shift_hz);

    Command::new("sox")
        .arg(input_path)
        .arg(output_path)
        .args(["pitch", &cents.to_string()])
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::{
        AudioFormat, apply_freq_shift, convert_audio_format, ffmpeg_codec_args, freq_shift_cents,
        freq_shift_resample_rate,
    };
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::path::Path;

    /// True when ffmpeg is on `PATH`. The conversion / freq-shift branches
    /// that actually spawn a tool are gated on this so a stripped CI runner
    /// without ffmpeg doesn't false-fail. The main CI image (and the
    /// mutation-testing job) install ffmpeg, so those branches are exercised
    /// there. Tests that don't need a real tool (the arithmetic helpers, the
    /// codec-arg mapping, the rename fallback) run unconditionally.
    fn ffmpeg_available() -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join("ffmpeg").is_file()))
    }

    fn write_silent_wav(path: &Path, secs: f32, sample_rate: u32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).expect("create wav");
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let n = (secs * sample_rate as f32) as u32;
        for _ in 0..n {
            writer.write_sample(0_i16).expect("write");
        }
        writer.finalize().expect("finalize");
    }

    // ── ffmpeg_codec_args ────────────────────────────────────────────────

    #[test]
    fn ffmpeg_codec_args_maps_each_format() {
        assert_eq!(
            ffmpeg_codec_args(AudioFormat::Mp3),
            Some(["-codec:a", "libmp3lame", "-q:a", "2"].as_slice())
        );
        assert_eq!(
            ffmpeg_codec_args(AudioFormat::Flac),
            Some(["-codec:a", "flac"].as_slice())
        );
        assert_eq!(
            ffmpeg_codec_args(AudioFormat::Ogg),
            Some(["-codec:a", "libvorbis", "-q:a", "4"].as_slice())
        );
        // WAV needs no transcode → no codec args.
        assert_eq!(ffmpeg_codec_args(AudioFormat::Wav), None);
    }

    // ── freq_shift_resample_rate ─────────────────────────────────────────

    #[test]
    fn freq_shift_resample_rate_zero_is_identity() {
        assert_eq!(freq_shift_resample_rate(48_000, 0), 48_000);
    }

    #[test]
    fn freq_shift_resample_rate_positive_raises() {
        // 48000 * (1 + 500/48000) = 48000 + 500 = 48500. Catches `*`→`/`,
        // `+`→`-`, and the `1.0`→`0.0`/`2.0` literal mutants.
        assert_eq!(freq_shift_resample_rate(48_000, 500), 48_500);
    }

    #[test]
    fn freq_shift_resample_rate_negative_lowers() {
        assert_eq!(freq_shift_resample_rate(48_000, -500), 47_500);
    }

    // ── freq_shift_cents ─────────────────────────────────────────────────

    #[test]
    fn freq_shift_cents_zero_is_zero() {
        assert_eq!(freq_shift_cents(48_000, 0), 0);
    }

    #[test]
    fn freq_shift_cents_octave_up_is_1200() {
        // shift == sample_rate doubles the rate → log2(2) = 1 → 1200 cents.
        // Catches the `1200.0`→`0.0`/`2400.0` literal mutants.
        assert_eq!(freq_shift_cents(48_000, 48_000), 1_200);
    }

    #[test]
    fn freq_shift_cents_fifth_distinguishes_mul_from_div() {
        // 1200 * log2(1.5) ≈ 701. `*`→`/` gives ≈ 2051, `*`→`+` gives ≈ 1201;
        // both differ from 701, so this single value pins the operator.
        assert_eq!(freq_shift_cents(48_000, 24_000), 701);
    }

    // ── convert_audio_format ─────────────────────────────────────────────

    #[test]
    fn convert_audio_format_flac_succeeds_and_removes_wav() {
        if !ffmpeg_available() {
            eprintln!("SKIP: ffmpeg not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("clip.wav");
        let out = tmp.path().join("clip.flac");
        write_silent_wav(&wav, 1.0, 48_000);

        convert_audio_format(&wav, &out, AudioFormat::Flac).expect("conversion ok");

        assert!(out.exists(), "FLAC output should be written");
        assert!(
            !wav.exists(),
            "the intermediate WAV must be removed on the success path"
        );
    }

    #[test]
    fn convert_audio_format_falls_back_to_rename_on_failure() {
        // Garbage bytes can't be decoded, so ffmpeg fails and (with sox
        // absent) sox fails too. The data must be preserved by renaming the
        // source to the target path rather than silently lost. This runs
        // unconditionally: with or without ffmpeg the conversion fails, so
        // the rename fallback is the observable behaviour either way.
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("notaudio.wav");
        let out = tmp.path().join("notaudio.flac");
        std::fs::write(&wav, b"this is not a WAV file").unwrap();

        convert_audio_format(&wav, &out, AudioFormat::Flac).expect("rename fallback returns Ok");

        assert!(
            out.exists(),
            "source must be renamed to the target on failure"
        );
        assert!(!wav.exists(), "source WAV must not survive the rename");
        assert_eq!(
            std::fs::read(&out).unwrap(),
            b"this is not a WAV file",
            "renamed file must carry the original bytes"
        );
    }

    #[test]
    fn convert_audio_format_wav_target_is_noop_and_removes_source() {
        // AudioFormat::Wav yields no codec args, so convert_with_ffmpeg
        // returns Ok(()) without spawning ffmpeg; the success arm then
        // removes the (already-correct) source. No tool needed.
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("clip.wav");
        let out = tmp.path().join("dest.wav");
        write_silent_wav(&wav, 0.1, 48_000);

        convert_audio_format(&wav, &out, AudioFormat::Wav).expect("wav no-op ok");

        assert!(
            !wav.exists(),
            "the Ok path removes the source even when nothing is transcoded"
        );
    }

    // ── apply_freq_shift ─────────────────────────────────────────────────

    #[test]
    fn apply_freq_shift_succeeds_with_real_tool() {
        if !ffmpeg_available() {
            eprintln!("SKIP: ffmpeg not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("in.wav");
        let output = tmp.path().join("out.wav");
        // Deliberately tiny (5 ms). ffmpeg's `asetrate=N` reinterprets the
        // sample rate, so the output length scales with `orig_rate / N`. Under
        // cargo-mutants a `freq_shift_resample_rate -> 1` body mutant sets
        // `asetrate=1`, expanding the clip by 48000x — on a 1 s input that is
        // ~48000 s (~4.6 GB) of output and blows past the 120 s mutation
        // timeout (a timeout fails the gate just like a missed mutant). A 5 ms
        // input caps that worst case at ~0.4 s so the mutant is caught fast.
        write_silent_wav(&input, 0.005, 48_000);

        assert!(
            apply_freq_shift(&input, &output, 48_000, 500),
            "freq shift must succeed via ffmpeg's early-return path"
        );
        assert!(output.exists(), "the shifted output should be written");
    }

    #[test]
    fn apply_freq_shift_returns_false_when_no_tool_can_process_input() {
        // A non-existent input makes ffmpeg exit non-zero (or fail to spawn);
        // with sox unavailable the function returns false rather than
        // panicking. Covers the sox-fallback branch and the final
        // `is_ok_and(success)` returning false. Runs unconditionally.
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("does-not-exist.wav");
        let output = tmp.path().join("out.wav");

        assert!(
            !apply_freq_shift(&input, &output, 48_000, 500),
            "freq shift must report failure when no tool can process the input"
        );
    }
}
