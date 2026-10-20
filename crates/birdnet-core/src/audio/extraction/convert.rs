//! Audio format conversion and frequency shifting.
//!
//! Handles conversion from WAV to MP3, FLAC, and OGG using ffmpeg or sox,
//! and frequency shifting for accessibility (ffmpeg's `afreqshift`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{AudioFormat, ExtractionError};
use crate::process::run_with_timeout;

/// The longest a converter may take on one clip before it is killed.
///
/// A detection clip is a few seconds of audio, and even an MP3 encode of one
/// on a Pi Zero finishes in well under a minute; the frequency-shift path caps
/// its output at [`MAX_FREQ_SHIFT_OUTPUT_SECS`]. The conversion runs on the
/// event-processor thread, so a converter that hangs — on a device that
/// stopped answering, on a codec bug — used to stop every detection after it
/// until the watchdog restarted the station (PR-8).
pub const CONVERTER_TIMEOUT: Duration = Duration::from_secs(60);

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

/// The ffmpeg filter that shifts every frequency component by `shift_hz`.
///
/// `afreqshift` is a true shift in hertz (single-sideband: every component
/// moves by the same number of Hz, so an 8 kHz song at `-3000` is heard at
/// 5 kHz) and leaves the clip's length and tempo alone. A positive shift raises
/// every component and a negative one lowers it.
///
/// **For hearing loss, the useful direction is *down*.** See
/// [`ACCESSIBILITY_SHIFT_HZ`].
///
/// This used to be `asetrate=<rate + shift>,aresample=<rate>`, which the
/// comments called a shift in Hz that preserves duration. It is neither: it
/// scales every frequency by `(rate + shift) / rate` (so `-3000` at 48 kHz moved
/// 8 kHz to 7.5 kHz, still out of reach of the listener it was written for) and
/// it plays the clip back slower by the same ratio (measured with ffmpeg 6.1.1
/// on an 8 kHz tone: 7 500 Hz and 3.2 s out of 3.0 s in).
///
/// Pulled out of [`apply_freq_shift`] so the string is observable in a unit test
/// rather than only reachable through a live ffmpeg run. Needs ffmpeg 4.3 or
/// newer, which is where `afreqshift` appeared; with an older one the filter is
/// unknown, ffmpeg exits non-zero and [`apply_freq_shift`] reports failure.
fn freq_shift_filter(shift_hz: i32) -> String {
    format!("afreqshift=shift={shift_hz}")
}

/// Convert a WAV file to the target format using ffmpeg (preferred) or sox,
/// and return the path that is on disk afterwards.
///
/// On success that is `output_path` and the source WAV is removed. When both
/// converters fail the WAV is kept — under its own `.wav` name, beside where
/// the converted clip would have been (DD-36). It used to be renamed to the
/// target name, so a `.mp3` on disk was a WAV under the wrong extension and
/// the row's `File_Name` said the wrong format; the caller now records what
/// the return value names.
///
/// # Errors
///
/// Returns [`ExtractionError::Write`] if the kept WAV cannot be committed
/// under its name; a converter failure is not an error, it is the fallback.
pub(super) fn convert_audio_format(
    wav_path: &Path,
    output_path: &Path,
    format: AudioFormat,
) -> Result<PathBuf, ExtractionError> {
    // The converters write a `.part` sibling (the extension stays last, so
    // both infer the format from it), which is synced and renamed into place
    // only once the tool has exited successfully (PS-7, S-4). Try ffmpeg
    // first, fall back to sox.
    let part = crate::atomic_file::part_path(output_path);
    let result = convert_with_ffmpeg(wav_path, &part, format)
        .or_else(|_| convert_with_sox(wav_path, &part))
        .and_then(|()| {
            crate::atomic_file::commit(&part, output_path)
                .map_err(|e| ExtractionError::Write(e.to_string()))
        });

    match result {
        Ok(()) => {
            // Remove the intermediate WAV file.
            if let Err(e) = std::fs::remove_file(wav_path) {
                tracing::debug!(path = %wav_path.display(), error = %e, "failed to remove intermediate WAV");
            }
            Ok(output_path.to_path_buf())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            let kept = output_path.with_extension(AudioFormat::Wav.extension());
            tracing::warn!(
                error = %e,
                format = %format,
                kept = %kept.display(),
                "format conversion failed, keeping the WAV under its own name"
            );
            if kept != wav_path {
                crate::atomic_file::commit(wav_path, &kept)?;
            }
            Ok(kept)
        }
    }
}

/// Convert WAV to target format using ffmpeg.
fn convert_with_ffmpeg(
    wav_path: &Path,
    output_path: &Path,
    format: AudioFormat,
) -> Result<(), ExtractionError> {
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

    let output = run_with_timeout(&mut cmd, CONVERTER_TIMEOUT)
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
    let output = run_with_timeout(
        Command::new("sox").arg(wav_path).arg(output_path),
        CONVERTER_TIMEOUT,
    )
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

/// The shift that makes birdsong audible to a listener with age-related
/// high-frequency hearing loss: **down**, by 3 kHz.
///
/// # This project documented the opposite, in five places
///
/// Five doc comments said a *positive* (upward) shift "makes calls accessible
/// to people with high-frequency hearing loss", including the `--freq-shift-hz`
/// CLI help an operator reads before choosing a value. That is backwards, and
/// following it moves the song further out of the listener's hearing rather
/// than into it.
///
/// Presbycusis attenuates the *top* of the range first. A warbler or kinglet at
/// 8 kHz is already above it; shifting up moves it further away. What restores
/// it is moving it *down*, into a band that still works.
///
/// The upstream this feature was ported from agrees, and is the primary source
/// checked before changing these comments — `BirdNET-Pi`'s
/// `scripts/install_config.sh` ships:
///
/// ```text
/// # Configuration of the frequency shifting feature, useful for earing impaired people.
/// # If the tool is ffmpeg, you have to define a freq. shift from HI to LO:
/// FREQSHIFT_HI=6000
/// FREQSHIFT_LO=3000
/// # If the tool is sox, you have to define the pitch shift (amount of 100ths of semintone)
/// FREQSHIFT_PITCH=-1500
/// ```
///
/// `livestream.sh` builds `rubberband=pitch=${FREQSHIFT_LO}/${FREQSHIFT_HI}`,
/// so the ratio is 3000/6000 = 0.5 — down one octave. The sox pitch is
/// negative for the same reason. Both directions agree, from two independent
/// settings.
///
/// −3000 Hz rather than upstream's halving because this project's shift is
/// linear (`afreqshift`, see [`freq_shift_filter`]) rather than a ratio: it
/// moves an 8 kHz song to 5 kHz — measured, on an 8 kHz tone, in
/// `apply_freq_shift_moves_the_pitch_by_that_many_hertz_and_keeps_the_length` —
/// while leaving a 2 kHz blackbird at a strange but still recognisable pitch.
/// (The implementation this replaced was documented in these words and moved
/// 8 kHz to 7.5 kHz: it was a ratio all along.) It is a starting point offered
/// in the UI, not a limit — any value is accepted.
pub const ACCESSIBILITY_SHIFT_HZ: i32 = -3000;

/// The sign of [`ACCESSIBILITY_SHIFT_HZ`], enforced at compile time.
///
/// A test would be the weaker guard here: the value is a constant, so this
/// fails the build rather than a run, and it cannot be skipped by a filtered
/// `cargo test`. The sign is the whole content of the constant — five doc
/// comments in this project had it backwards — so it is worth a hard stop.
const _: () = assert!(
    ACCESSIBILITY_SHIFT_HZ < 0,
    "the accessibility shift must be downward: hearing loss takes the top of \
     the range, so high song is restored by moving it down, not up"
);

/// Hard cap (in seconds) on freq-shift output length.
///
/// A frequency shift does not change a clip's length, so the shift of an
/// extracted clip (bounded by the ~6 s `extraction_length`) is never longer
/// than its input. Passing ffmpeg `-t` with this cap is **field-hardening**: a
/// degenerate filter — whether from a misconfiguration in the field or, in CI, a
/// cargo-mutants body mutant — could otherwise stretch the clip into a runaway,
/// disk-filling file. 30 s is generous headroom over any real clip while keeping
/// even the pathological case a sub-second ffmpeg run.
const MAX_FREQ_SHIFT_OUTPUT_SECS: u32 = 30;

/// Apply frequency shifting to a WAV file using ffmpeg.
///
/// Shifts every component by `shift_hz` ([`freq_shift_filter`]). The output is
/// capped at [`MAX_FREQ_SHIFT_OUTPUT_SECS`] so a bad filter can never produce a
/// runaway file, and carries no metadata of its own: ffmpeg otherwise writes a
/// `LIST`/`INFO` chunk naming its encoder, and a WAV that already has one is a
/// WAV [`super::metadata::embed_wav_metadata`] leaves alone, so a shifted clip
/// came out with no species, confidence or date inside it.
///
/// There is no sox fallback. sox's `pitch` effect scales frequencies by a ratio
/// and cannot shift by an amount of hertz, so it would have written a different
/// clip from the one the setting describes; a failed shift is reported and the
/// caller keeps the unshifted clip, and says so.
///
/// Returns `true` on success, `false` if ffmpeg is unavailable or fails.
/// BirdNET-Pi equivalent: `FREQSHIFT_*` applied through ffmpeg's `rubberband`
/// (a ratio) or sox's `pitch`; this project's shift is in hertz.
pub(super) fn apply_freq_shift(input_path: &Path, output_path: &Path, shift_hz: i32) -> bool {
    let filter = freq_shift_filter(shift_hz);
    let max_secs = MAX_FREQ_SHIFT_OUTPUT_SECS.to_string();

    run_with_timeout(
        Command::new("ffmpeg").args([
            "-y",
            "-i",
            &input_path.to_string_lossy(),
            "-af",
            &filter,
            "-t",
            &max_secs,
            "-map_metadata",
            "-1",
            "-fflags",
            "+bitexact",
            "-flags:a",
            "+bitexact",
            "-loglevel",
            "error",
            &output_path.to_string_lossy(),
        ]),
        CONVERTER_TIMEOUT,
    )
    .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::{
        AudioFormat, ExtractionError, apply_freq_shift, convert_audio_format, convert_with_sox,
        ffmpeg_codec_args, freq_shift_filter,
    };
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::path::Path;

    /// True when ffmpeg is on `PATH`. The conversion / freq-shift branches
    /// that actually spawn a tool are gated on this so a stripped CI runner
    /// without ffmpeg doesn't false-fail. The main CI image (and the
    /// mutation-testing job) install ffmpeg, so those branches are exercised
    /// there. Tests that don't need a real tool (the arithmetic helpers, the
    /// codec-arg mapping, the rename fallback) run unconditionally.
    ///
    /// With `BNB_REQUIRE_FFMPEG` set (CI sets it), a missing ffmpeg fails the
    /// test instead: a skip prints only into captured output, so without this
    /// a runner without ffmpeg would report these tests green having run none
    /// of them.
    fn ffmpeg_available() -> bool {
        let found = std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join("ffmpeg").is_file()));
        assert!(
            found || std::env::var_os("BNB_REQUIRE_FFMPEG").is_none(),
            "BNB_REQUIRE_FFMPEG is set and ffmpeg is not on PATH"
        );
        found
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

    /// Read the duration (seconds) of a WAV file by its frame count.
    fn wav_duration_secs(path: &Path) -> f64 {
        let reader = hound::WavReader::open(path).expect("open wav");
        let rate = reader.spec().sample_rate;
        f64::from(reader.duration()) / f64::from(rate)
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

    // ── freq_shift_filter ────────────────────────────────────────────────

    /// The filter is a shift in hertz, not a sample-rate reinterpretation.
    ///
    /// The string alone cannot show that (the next tests measure the audio), but
    /// it pins the two things that were wrong together: the sign carries
    /// through, and no `asetrate` is in it, because `asetrate` followed by
    /// `aresample` scales frequencies by a ratio and slows the clip.
    #[test]
    fn freq_shift_filter_is_a_shift_in_hertz() {
        assert_eq!(freq_shift_filter(-3_000), "afreqshift=shift=-3000");
        assert_eq!(freq_shift_filter(500), "afreqshift=shift=500");
        assert!(!freq_shift_filter(-3_000).contains("asetrate"));
    }

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

    /// The gate for DD-36. Garbage bytes cannot be decoded, so ffmpeg fails
    /// and (with sox absent) sox fails too; this runs with or without the
    /// tools. The WAV must survive under its *own* name and the return value
    /// must name it: renamed to `notaudio.flac`, as it used to be, the file on
    /// disk was a WAV under the wrong extension and the row said `flac`.
    #[test]
    fn a_failed_conversion_keeps_the_wav_under_its_own_name_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("notaudio.wav");
        let out = tmp.path().join("notaudio.flac");
        std::fs::write(&wav, b"this is not a WAV file").unwrap();

        let kept = convert_audio_format(&wav, &out, AudioFormat::Flac)
            .expect("a failed conversion keeps the data");

        assert_eq!(
            kept, wav,
            "the caller must be told the WAV is what is on disk"
        );
        assert!(wav.exists(), "the WAV must keep its .wav name");
        assert!(
            !out.exists(),
            "nothing may sit under the target name that is not the target format"
        );
        assert_eq!(std::fs::read(&wav).unwrap(), b"this is not a WAV file");
    }
    /// A converter that ran nothing must not say it converted.
    ///
    /// `convert_with_sox` is the last fallback before the WAV is kept under
    /// its own name (DD-36), and a version of it returning `Ok(())` without
    /// producing a file left every test here green: the caller then commits a
    /// `.part` that does not exist, which fails, which falls back, which is
    /// what the DD-36 gate asserts. cargo-mutants reported exactly that
    /// (`replace convert_with_sox -> Ok(())` missed). The input does not
    /// exist, so this holds with sox installed (non-zero exit) and without
    /// it (spawn failure); both are the conversion error the chain expects.
    #[test]
    fn a_converter_that_ran_nothing_does_not_report_success() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist.wav");
        let out = tmp.path().join("out.flac.part");

        let err = convert_with_sox(&missing, &out).expect_err("no input, no success");

        assert!(matches!(err, ExtractionError::Conversion(_)), "{err:?}");
        assert!(
            !out.exists(),
            "nothing may be written for an input that does not exist"
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

    /// Write a mono 16-bit tone, loud enough that zero crossings are clean.
    fn write_tone_wav(path: &Path, secs: u32, sample_rate: u32, hz: f64) {
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).expect("create wav");
        for i in 0..secs * sample_rate {
            let t = f64::from(i) / f64::from(sample_rate);
            #[allow(clippy::cast_possible_truncation)]
            let sample = (12_000.0 * (2.0 * std::f64::consts::PI * hz * t).sin()) as i16;
            writer.write_sample(sample).expect("write");
        }
        writer.finalize().expect("finalize");
    }

    /// The pitch of a mono 16-bit WAV in Hz, from its zero crossings.
    #[allow(clippy::cast_precision_loss)]
    fn tone_hz(path: &Path) -> f64 {
        let mut reader = hound::WavReader::open(path).expect("open wav");
        let rate = f64::from(reader.spec().sample_rate);
        let samples: Vec<i16> = reader
            .samples::<i16>()
            .map(|s| s.expect("sample"))
            .collect();
        let crossings = samples
            .windows(2)
            .filter(|w| (w[0] < 0) != (w[1] < 0))
            .count();
        crossings as f64 / 2.0 / (samples.len() as f64 / rate)
    }

    /// The ids of the RIFF chunks in a WAV file, in order.
    fn riff_chunk_ids(path: &Path) -> Vec<String> {
        let bytes = std::fs::read(path).expect("read wav");
        let mut ids = Vec::new();
        let mut at = 12;
        while at + 8 <= bytes.len() {
            ids.push(String::from_utf8_lossy(&bytes[at..at + 4]).into_owned());
            let size =
                u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]);
            at += 8 + size as usize + (size as usize & 1);
        }
        ids
    }

    #[test]
    fn apply_freq_shift_succeeds_and_preserves_duration() {
        if !ffmpeg_available() {
            eprintln!("SKIP: ffmpeg not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("in.wav");
        let output = tmp.path().join("out.wav");
        // 2 s input: a frequency shift leaves the length alone, so the output
        // must come back 2 s, to within a frame or two. Asserting that pins
        // MAX_FREQ_SHIFT_OUTPUT_SECS (a mutant shrinking the cap to 1 s
        // truncates the output) and rules out the sample-rate trick this used
        // to be, which returned 2.13 s for a 500 Hz shift at 48 kHz.
        write_silent_wav(&input, 2.0, 48_000);

        assert!(
            apply_freq_shift(&input, &output, 500),
            "freq shift must succeed through ffmpeg"
        );
        assert!(output.exists(), "the shifted output should be written");
        let dur = wav_duration_secs(&output);
        assert!(
            (dur - 2.0).abs() < 0.02,
            "a frequency shift keeps the clip's length: ~2 s expected, got {dur} s"
        );
    }

    /// The setting does what its name says: every component moves by that many
    /// hertz, in the stated direction, and nothing else about the clip moves.
    ///
    /// Measured on audio, not read off the filter string. An 8 kHz song shifted
    /// by -3000 Hz (the accessibility preset) has to be heard at 5 kHz; the
    /// `asetrate`/`aresample` pair this replaced gave 7.5 kHz and a clip 6.7 %
    /// longer, which is a pitch change a listener with high-frequency hearing
    /// loss cannot use. The counterpart shifts up, so a filter that only ever
    /// lowers cannot pass.
    #[test]
    fn apply_freq_shift_moves_the_pitch_by_that_many_hertz_and_keeps_the_length() {
        if !ffmpeg_available() {
            eprintln!("SKIP: ffmpeg not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        for (tone, shift, want) in [(8_000.0, -3_000, 5_000.0), (4_000.0, 1_000, 5_000.0)] {
            let input = tmp.path().join(format!("in_{tone}.wav"));
            let output = tmp.path().join(format!("out_{tone}.wav"));
            write_tone_wav(&input, 2, 48_000, tone);
            assert!(apply_freq_shift(&input, &output, shift), "shift {shift}");
            let heard = tone_hz(&output);
            assert!(
                (heard - want).abs() < 100.0,
                "{tone} Hz shifted by {shift} Hz should be heard at {want} Hz, got {heard:.0} Hz"
            );
            let dur = wav_duration_secs(&output);
            assert!(
                (dur - 2.0).abs() < 0.02,
                "{tone} Hz shifted by {shift} Hz changed the length: {dur} s"
            );
        }
    }

    /// ffmpeg names itself in a `LIST`/`INFO` chunk unless told not to, and a
    /// WAV that has one is left alone by the tag writer, so a shifted clip lost
    /// its species, confidence and date. The shifted file is the audio and nothing
    /// else; the extractor adds the tags afterwards.
    #[test]
    fn a_shifted_wav_carries_no_chunk_of_ffmpegs_own() {
        if !ffmpeg_available() {
            eprintln!("SKIP: ffmpeg not available");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("in.wav");
        let output = tmp.path().join("out.wav");
        write_tone_wav(&input, 1, 48_000, 8_000.0);
        assert!(apply_freq_shift(&input, &output, -3_000));
        assert_eq!(
            riff_chunk_ids(&output),
            ["fmt ", "data"],
            "only the format and the samples: no LIST chunk written by the encoder"
        );
    }

    #[test]
    fn apply_freq_shift_returns_false_when_ffmpeg_cannot_process_input() {
        // A non-existent input makes ffmpeg exit non-zero (or fail to spawn);
        // the function returns false rather than panicking, and the caller
        // keeps the unshifted clip. Runs unconditionally.
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("does-not-exist.wav");
        let output = tmp.path().join("out.wav");

        assert!(
            !apply_freq_shift(&input, &output, 500),
            "freq shift must report failure when ffmpeg cannot process the input"
        );
    }
}
