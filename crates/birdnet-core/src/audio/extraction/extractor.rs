//! Core extractor logic: extracts audio clips around detections.

use std::path::{Path, PathBuf};

use crate::audio::decode::decode_file;
use crate::detection::types::Detection;

use super::convert::{apply_freq_shift, convert_audio_format};
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
        // 1. Calculate safe extraction boundaries (matches BirdNET-Pi logic).
        let spacer = (self.config.extraction_length - 3.0) / 2.0;
        let safe_start = (detection.start - spacer).max(0.0);
        let safe_stop = (detection.stop + spacer).min(self.config.recording_length);

        tracing::debug!(
            species = %detection.common_name,
            safe_start,
            safe_stop,
            "extracting detection clip"
        );

        // 2. Decode the source audio file.
        let audio = decode_file(source_file)?;

        // 3. Extract the relevant samples.
        let start_sample = (safe_start * audio.sample_rate as f32) as usize;
        let stop_sample =
            ((safe_stop * audio.sample_rate as f32) as usize).min(audio.samples.len());

        if start_sample >= stop_sample || start_sample >= audio.samples.len() {
            return Err(ExtractionError::Decode(format!(
                "invalid sample range: {start_sample}..{stop_sample} (total {})",
                audio.samples.len()
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
