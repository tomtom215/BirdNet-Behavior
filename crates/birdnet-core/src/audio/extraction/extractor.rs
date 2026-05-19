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
}
