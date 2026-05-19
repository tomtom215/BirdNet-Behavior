//! Core extractor logic: extracts audio clips around detections.

use std::path::{Path, PathBuf};

use crate::audio::decode::decode_file;
use crate::detection::types::Detection;

use super::convert::{apply_freq_shift, convert_audio_format};
use super::metadata::{DetectionMeta, embed_wav_metadata};
use super::wav::write_wav_clip;
use super::{ExtractionConfig, ExtractionError};

/// Extracts audio clips around detections and writes them to disk.
#[derive(Debug)]
pub struct Extractor {
    config: ExtractionConfig,
}

impl Extractor {
    /// Create a new extractor with the given configuration.
    pub const fn new(config: ExtractionConfig) -> Self {
        Self { config }
    }

    /// Return a reference to the extractor configuration.
    pub const fn config(&self) -> &ExtractionConfig {
        &self.config
    }

    /// Extract an audio clip for a detection from the source recording.
    ///
    /// Returns the path to the extracted audio file.
    ///
    /// # Errors
    ///
    /// Returns [`ExtractionError`] if the source cannot be decoded, the
    /// output directory cannot be created, or writing fails.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn extract_detection(
        &self,
        source_file: &Path,
        detection: &Detection,
    ) -> Result<PathBuf, ExtractionError> {
        // 1. Decode first so we know the actual audio length. Without this,
        //    safe_stop was clamped to the configured `recording_length`
        //    (default 15 s), and any detection beyond that window produced
        //    start_sample > stop_sample → "invalid sample range" with the
        //    range inverted.
        let audio = decode_file(source_file)?;
        if audio.samples.is_empty() {
            return Err(ExtractionError::Decode(format!(
                "decoded audio is empty: {}",
                source_file.display()
            )));
        }
        let actual_duration_secs = audio.samples.len() as f32 / audio.sample_rate as f32;

        // 2. Calculate safe extraction boundaries against the file's real
        //    duration, not the operator-configured recording length.
        let spacer = (self.config.extraction_length - 3.0) / 2.0;
        let safe_start = (detection.start - spacer)
            .max(0.0)
            .min(actual_duration_secs);
        let safe_stop = (detection.stop + spacer)
            .max(safe_start)
            .min(actual_duration_secs);

        tracing::debug!(
            species = %detection.common_name,
            safe_start,
            safe_stop,
            actual_duration_secs,
            configured_recording_length = self.config.recording_length,
            "extracting detection clip"
        );

        // 3. Extract the relevant samples. The clamping above guarantees
        //    0 ≤ start ≤ stop ≤ samples.len(), but we re-check here so the
        //    invariant is enforced at the slicing boundary too.
        let start_sample = (safe_start * audio.sample_rate as f32) as usize;
        let stop_sample =
            ((safe_stop * audio.sample_rate as f32) as usize).min(audio.samples.len());

        if start_sample >= stop_sample {
            return Err(ExtractionError::Decode(format!(
                "no usable audio range for detection at {start_sample}..{stop_sample} samples \
                 (file has {} samples, detection at {:.2}s-{:.2}s)",
                audio.samples.len(),
                detection.start,
                detection.stop
            )));
        }

        let clip_samples = &audio.samples[start_sample..stop_sample];

        // 4. Build output path: output_dir/By_Date/YYYY-MM-DD/Common_Name_Safe/
        let output_dir = self
            .config
            .output_dir
            .join("By_Date")
            .join(&detection.date)
            .join(detection.common_name_safe());

        std::fs::create_dir_all(&output_dir)?;

        // 5. Build filename with target format extension.
        let ext = self.config.target_format.extension();
        let filename = build_extraction_filename(detection, ext);
        let output_path = output_dir.join(&filename);

        // 6. Write the WAV file using hound (with optional frequency shifting).
        if self.config.freq_shift_hz != 0 || self.config.target_format.needs_conversion() {
            // Write to a temporary WAV first, then apply shift and/or convert.
            let wav_path = output_path.with_extension("wav");
            write_wav_clip(clip_samples, audio.sample_rate, &wav_path)?;

            if self.config.freq_shift_hz != 0 {
                // Apply frequency shift: write shifted WAV, then convert if needed.
                let shifted_path = wav_path.with_file_name(format!(
                    "_shifted_{}",
                    wav_path.file_name().unwrap_or_default().to_string_lossy()
                ));
                let shift_ok = apply_freq_shift(
                    &wav_path,
                    &shifted_path,
                    audio.sample_rate,
                    self.config.freq_shift_hz,
                );
                if shift_ok {
                    let _ = std::fs::remove_file(&wav_path);
                    if self.config.target_format.needs_conversion() {
                        convert_audio_format(
                            &shifted_path,
                            &output_path,
                            self.config.target_format,
                        )?;
                    } else {
                        std::fs::rename(&shifted_path, &output_path)?;
                    }
                } else {
                    // Shift failed — fall back to unshifted.
                    tracing::warn!(
                        freq_shift_hz = self.config.freq_shift_hz,
                        "frequency shift failed, using original"
                    );
                    let _ = std::fs::remove_file(&shifted_path);
                    if self.config.target_format.needs_conversion() {
                        convert_audio_format(&wav_path, &output_path, self.config.target_format)?;
                    } else {
                        std::fs::rename(&wav_path, &output_path)?;
                    }
                }
            } else {
                convert_audio_format(&wav_path, &output_path, self.config.target_format)?;
            }
        } else {
            write_wav_clip(clip_samples, audio.sample_rate, &output_path)?;
        }

        // Embed RIFF INFO metadata into WAV files (best-effort, non-fatal).
        if self.config.target_format == super::format::AudioFormat::Wav {
            let meta = DetectionMeta {
                common_name: detection.common_name.clone(),
                scientific_name: detection.scientific_name.clone(),
                confidence: detection.confidence,
                date: detection.date.clone(),
                time: detection.time.clone(),
            };
            if let Err(e) = embed_wav_metadata(&output_path, &meta) {
                tracing::debug!(
                    error = %e,
                    path = %output_path.display(),
                    "WAV metadata embedding failed (non-fatal)"
                );
            }
        }

        tracing::info!(
            path = %output_path.display(),
            species = %detection.common_name,
            format = %ext,
            "extracted detection clip"
        );

        Ok(output_path)
    }
}

/// Build extraction filename following BirdNET-Pi convention.
///
/// Format: `Common_Name-ConfPct-YYYY-MM-DD-birdnet-RTSP_ID-HH:MM:SS.ext`
/// or without RTSP: `Common_Name-ConfPct-YYYY-MM-DD-birdnet-HH:MM:SS.ext`
pub(super) fn build_extraction_filename(detection: &Detection, format: &str) -> String {
    let name_safe = detection.common_name_safe();
    let conf_pct = detection.confidence_pct();
    let date = &detection.date;
    let time = &detection.time;

    // Parse the source file for RTSP ID if present in the detection's
    // extracted filename, otherwise omit it.
    let rtsp_part = detection
        .file_name_extr
        .as_deref()
        .and_then(|f| {
            // Attempt to extract RTSP ID from the source filename pattern.
            let base = f.rsplit('/').next().unwrap_or(f);
            // Pattern: YYYY-MM-DD-birdnet-RTSP_ID-HH:MM:SS.ext
            let parts: Vec<&str> = base.splitn(6, '-').collect();
            if parts.len() >= 6 {
                // parts[4] could be RTSP ID
                let candidate = parts[4];
                if !candidate.contains(':') {
                    return Some(format!("{candidate}-"));
                }
            }
            None
        })
        .unwrap_or_default();

    format!("{name_safe}-{conf_pct}-{date}-birdnet-{rtsp_part}{time}.{format}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::extraction::{AudioFormat, ExtractionConfig};
    use hound::{SampleFormat, WavSpec, WavWriter};

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

    fn det(start: f32, stop: f32) -> Detection {
        Detection {
            date: "2026-05-19".into(),
            time: "09:00:00".into(),
            scientific_name: "Pica pica".into(),
            common_name: "Eurasian Magpie".into(),
            confidence: 0.85,
            start,
            stop,
            week: 20,
            file_name_extr: None,
        }
    }

    /// Regression: a detection past the configured `recording_length` used to
    /// compute `start_sample > stop_sample` and fail with "invalid sample
    /// range". After the fix the clamp uses the *actual* decoded length.
    #[test]
    fn extraction_clamps_to_actual_audio_length_not_config() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
        write_silent_wav(&src, 30.0, 48_000);

        let cfg = ExtractionConfig {
            output_dir: tmp.path().join("out"),
            audio_format: "wav".into(),
            recording_length: 15.0, // operator-configured, deliberately < file
            extraction_length: 6.0,
            target_format: AudioFormat::Wav,
            freq_shift_hz: 0,
        };
        let extractor = Extractor::new(cfg);

        // Detection at 25s — past the 15s configured length but well inside
        // the 30s real audio. Old code: error. New code: success.
        let out = extractor
            .extract_detection(&src, &det(25.0, 28.0))
            .expect("extraction should succeed within the real file length");
        assert!(out.exists());
    }

    /// Empty-audio guard: a zero-sample WAV must produce a clear error,
    /// not a panic on `..` slicing.
    #[test]
    fn extraction_rejects_empty_audio_with_clear_message() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
        write_silent_wav(&src, 0.0, 48_000);

        let cfg = ExtractionConfig {
            output_dir: tmp.path().join("out"),
            audio_format: "wav".into(),
            recording_length: 15.0,
            extraction_length: 6.0,
            target_format: AudioFormat::Wav,
            freq_shift_hz: 0,
        };
        let extractor = Extractor::new(cfg);

        let err = extractor
            .extract_detection(&src, &det(0.0, 3.0))
            .expect_err("empty audio should fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("empty") || msg.contains("no usable audio range"),
            "error should mention empty audio, got: {msg}"
        );
    }

    /// A detection that lies entirely past the end of the file should fail
    /// with a clear message — never with "start > stop" range inversion.
    #[test]
    fn extraction_past_end_of_file_fails_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
        write_silent_wav(&src, 5.0, 48_000);

        let cfg = ExtractionConfig {
            output_dir: tmp.path().join("out"),
            audio_format: "wav".into(),
            recording_length: 15.0,
            extraction_length: 6.0,
            target_format: AudioFormat::Wav,
            freq_shift_hz: 0,
        };
        let extractor = Extractor::new(cfg);

        // Detection start past EOF: should clamp to file end, leaving an
        // empty window → clean error rather than start>stop.
        let err = extractor
            .extract_detection(&src, &det(20.0, 23.0))
            .expect_err("detection past EOF should fail");
        let msg = format!("{err}");
        assert!(
            !msg.contains("invalid sample range") || msg.contains("no usable audio range"),
            "error message changed shape; got: {msg}"
        );
    }

    /// Decode the WAV the extractor produced and compare its length to the
    /// configured extraction window. Pins the spacer-around-detection
    /// arithmetic: `(extraction_length - 3.0) / 2.0` applied symmetrically
    /// around the detection's [start, stop].
    ///
    /// Why this matters: mutation testing previously surfaced `replace / with *`
    /// and `replace - with +` mutants in this exact arithmetic that the
    /// existing tests passed unchanged. The bug pattern that emitted
    /// `start_sample > stop_sample` in PR #35 was an arithmetic-on-clamps
    /// problem of the same shape.
    #[test]
    fn extraction_clip_length_matches_extraction_window() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
        write_silent_wav(&src, 30.0, 48_000);

        // extraction_length = 6.0 → spacer = (6 - 3) / 2 = 1.5 s either side.
        // Detection at 10-13 → expected clip span 8.5-14.5 s → 6 s total.
        let cfg = ExtractionConfig {
            output_dir: tmp.path().join("out"),
            audio_format: "wav".into(),
            recording_length: 30.0,
            extraction_length: 6.0,
            target_format: AudioFormat::Wav,
            freq_shift_hz: 0,
        };
        let extractor = Extractor::new(cfg);
        let out = extractor
            .extract_detection(&src, &det(10.0, 13.0))
            .expect("extraction succeeds");

        let reader = hound::WavReader::open(&out).expect("WAV reader");
        let samples = reader.duration();
        // Spec is 48 kHz × 6 s = 288 000 samples. ±1 sample is fine.
        assert!(
            samples.abs_diff(288_000) <= 1,
            "expected ~288_000 samples (6 s @ 48 kHz), got {samples}"
        );
    }

    /// `extraction_length = 3.0` should give a clip exactly 3 s long
    /// (spacer = 0). Anchors the boundary case of the spacer formula.
    #[test]
    fn extraction_clip_length_with_zero_spacer() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");
        write_silent_wav(&src, 30.0, 48_000);

        let cfg = ExtractionConfig {
            output_dir: tmp.path().join("out"),
            audio_format: "wav".into(),
            recording_length: 30.0,
            extraction_length: 3.0, // spacer = 0
            target_format: AudioFormat::Wav,
            freq_shift_hz: 0,
        };
        let extractor = Extractor::new(cfg);
        let out = extractor
            .extract_detection(&src, &det(10.0, 13.0))
            .expect("extraction succeeds");
        let reader = hound::WavReader::open(&out).expect("WAV reader");
        let samples = reader.duration();
        // 48 kHz × 3 s = 144 000.
        assert!(
            samples.abs_diff(144_000) <= 1,
            "expected ~144_000 samples (3 s @ 48 kHz), got {samples}"
        );
    }

    /// Extracted clip starts at `detection.start - spacer`. Pin the offset
    /// arithmetic by inserting a sentinel pulse at a known sample index
    /// and confirming it ends up where the spacer arithmetic predicts.
    #[test]
    fn extraction_offset_matches_safe_start() {
        use hound::WavWriter;
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("2026-05-19-birdnet-09:00:00.wav");

        // 30 s of zeros except for a single +1 sample at exactly t = 11.5 s.
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(&src, spec).unwrap();
        let pulse_idx = 48_000 * 23 / 2; // 11.5 s × 48 000 Hz = 552_000
        let total = 48_000 * 30;
        for i in 0..total {
            let s: i16 = if i == pulse_idx { 16_384 } else { 0 };
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();

        // extraction_length = 6.0, spacer = 1.5. Detection at [10, 13]:
        // safe_start = 10 - 1.5 = 8.5 s → pulse at 11.5 - 8.5 = 3.0 s into clip.
        let cfg = ExtractionConfig {
            output_dir: tmp.path().join("out"),
            audio_format: "wav".into(),
            recording_length: 30.0,
            extraction_length: 6.0,
            target_format: AudioFormat::Wav,
            freq_shift_hz: 0,
        };
        let extractor = Extractor::new(cfg);
        let out = extractor
            .extract_detection(&src, &det(10.0, 13.0))
            .expect("extraction succeeds");

        let mut reader = hound::WavReader::open(&out).expect("WAV reader");
        let samples: Vec<i16> = reader
            .samples::<i16>()
            .collect::<Result<_, _>>()
            .expect("read samples");

        // Locate the pulse — should be at index 3 s × 48 000 = 144 000,
        // within ±1 sample of rounding noise.
        let (idx, _) = samples
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| s.unsigned_abs())
            .expect("samples non-empty");
        let expected_clip_offset = 144_000_i64;
        let drift = (idx as i64 - expected_clip_offset).abs();
        assert!(
            drift <= 1,
            "pulse drifted: expected offset ~{expected_clip_offset} in clip, found at {idx} (drift {drift})"
        );
    }

    // ─── build_extraction_filename: pure function, no audio I/O ────────

    fn det_named(scientific_name: &str, common_name: &str, confidence: f32) -> Detection {
        Detection {
            date: "2026-05-19".into(),
            time: "09:00:00".into(),
            scientific_name: scientific_name.into(),
            common_name: common_name.into(),
            confidence,
            start: 0.0,
            stop: 3.0,
            week: 20,
            file_name_extr: None,
        }
    }

    #[test]
    fn extraction_filename_canonical_no_rtsp() {
        let d = det_named("Pica pica", "Eurasian Magpie", 0.93);
        let name = build_extraction_filename(&d, "wav");
        assert_eq!(name, "Eurasian_Magpie-93-2026-05-19-birdnet-09:00:00.wav");
    }

    #[test]
    fn extraction_filename_includes_rtsp_id_when_present() {
        let mut d = det_named("Pica pica", "Eurasian Magpie", 0.93);
        d.file_name_extr = Some("/var/lib/birdnet/2026-05-19-birdnet-RTSP_2-09:00:00.wav".into());
        let name = build_extraction_filename(&d, "flac");
        assert_eq!(
            name,
            "Eurasian_Magpie-93-2026-05-19-birdnet-RTSP_2-09:00:00.flac"
        );
    }

    #[test]
    fn extraction_filename_omits_rtsp_when_source_has_no_rtsp_segment() {
        let mut d = det_named("Pica pica", "Eurasian Magpie", 0.93);
        d.file_name_extr = Some("/var/lib/birdnet/2026-05-19-birdnet-09:00:00.wav".into());
        let name = build_extraction_filename(&d, "wav");
        assert_eq!(name, "Eurasian_Magpie-93-2026-05-19-birdnet-09:00:00.wav");
    }

    #[test]
    fn extraction_filename_format_extension_overrides() {
        // Trip the format argument so a mutant swapping it for an empty
        // string fails an assertion.
        let d = det_named("Pica pica", "Eurasian Magpie", 0.93);
        assert!(build_extraction_filename(&d, "mp3").ends_with(".mp3"));
        assert!(build_extraction_filename(&d, "flac").ends_with(".flac"));
    }
}
