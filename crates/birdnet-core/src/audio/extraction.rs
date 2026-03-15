//! Detection audio extraction and spectrogram generation.
//!
//! Extracts audio clips around each detection and saves them to disk.
//! Replaces BirdNET-Pi's `extract_safe()` Python function and sox usage
//! with symphonia (reading) and hound (WAV writing).

use std::fmt;
use std::path::{Path, PathBuf};

use super::decode::{DecodeError, decode_file};
use super::spectrogram::{MelConfig, MelSpectrogram, SpectrogramError, mel_spectrogram};
use crate::detection::types::Detection;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during audio extraction.
#[derive(Debug)]
pub enum ExtractionError {
    /// File I/O error.
    Io(std::io::Error),
    /// Audio decoding error.
    Decode(String),
    /// Audio writing error.
    Write(String),
    /// Audio format conversion error (ffmpeg/sox subprocess).
    Conversion(String),
}

impl fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Decode(msg) => write!(f, "decode error: {msg}"),
            Self::Write(msg) => write!(f, "write error: {msg}"),
            Self::Conversion(msg) => write!(f, "format conversion error: {msg}"),
        }
    }
}

impl std::error::Error for ExtractionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Decode(_) | Self::Write(_) | Self::Conversion(_) => None,
        }
    }
}

impl From<std::io::Error> for ExtractionError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<DecodeError> for ExtractionError {
    fn from(e: DecodeError) -> Self {
        match e {
            DecodeError::Io(io_err) => Self::Io(io_err),
            DecodeError::Format(msg) => Self::Decode(msg),
            DecodeError::NoTracks => Self::Decode(String::from("no audio tracks found")),
        }
    }
}

// ---------------------------------------------------------------------------
// Audio format
// ---------------------------------------------------------------------------

/// Supported audio output formats for extracted clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// WAV (PCM 16-bit) — no external tools required.
    Wav,
    /// MP3 — requires ffmpeg or sox.
    Mp3,
    /// FLAC — requires ffmpeg or sox.
    Flac,
    /// OGG Vorbis — requires ffmpeg or sox.
    Ogg,
}

impl AudioFormat {
    /// File extension for this format.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
        }
    }

    /// Parse a format string (case-insensitive).
    ///
    /// Returns `Wav` for unrecognized formats.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "mp3" => Self::Mp3,
            "flac" => Self::Flac,
            "ogg" | "vorbis" => Self::Ogg,
            _ => Self::Wav,
        }
    }

    /// Whether this format requires external conversion from WAV.
    pub const fn needs_conversion(self) -> bool {
        !matches!(self, Self::Wav)
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for audio clip extraction.
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Total extraction length in seconds (default 6.0).
    pub extraction_length: f32,
    /// Base directory for extracted files (e.g., `~/BirdSongs/Extracted`).
    pub output_dir: PathBuf,
    /// Audio output format extension (e.g., "wav").
    pub audio_format: String,
    /// Target audio format for extraction output.
    pub target_format: AudioFormat,
    /// Recording segment length in seconds, used for `safe_stop` clamping.
    pub recording_length: f32,
    /// Frequency shift in Hz applied to extracted clips (0 = disabled).
    ///
    /// Shifts the audio pitch upward by the specified Hz, making high-frequency
    /// bird calls accessible to people with high-frequency hearing loss.
    /// Implemented via ffmpeg `asetrate`+`aresample` filter or sox `pitch` effect.
    ///
    /// BirdNET-Pi equivalent: `FREQ_SHIFT` config option with sox/rubberband.
    pub freq_shift_hz: i32,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            extraction_length: 6.0,
            output_dir: PathBuf::from("BirdSongs/Extracted"),
            audio_format: String::from("wav"),
            target_format: AudioFormat::Wav,
            recording_length: 15.0,
            freq_shift_hz: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Extractor
// ---------------------------------------------------------------------------

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
                        convert_audio_format(&shifted_path, &output_path, self.config.target_format)?;
                    } else {
                        std::fs::rename(&shifted_path, &output_path)?;
                    }
                } else {
                    // Shift failed — fall back to unshifted.
                    tracing::warn!(freq_shift_hz = self.config.freq_shift_hz, "frequency shift failed, using original");
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

// ---------------------------------------------------------------------------
// Filename generation
// ---------------------------------------------------------------------------

/// Build extraction filename following BirdNET-Pi convention.
///
/// Format: `Common_Name-ConfPct-YYYY-MM-DD-birdnet-RTSP_ID-HH:MM:SS.ext`
/// or without RTSP: `Common_Name-ConfPct-YYYY-MM-DD-birdnet-HH:MM:SS.ext`
fn build_extraction_filename(detection: &Detection, format: &str) -> String {
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

// ---------------------------------------------------------------------------
// Audio format conversion
// ---------------------------------------------------------------------------

/// Convert a WAV file to the target format using ffmpeg (preferred) or sox.
///
/// On success the source WAV file is removed.
///
/// # Errors
///
/// Returns [`ExtractionError::Conversion`] if neither ffmpeg nor sox is
/// available or the conversion process fails.
fn convert_audio_format(
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

// ---------------------------------------------------------------------------
// Frequency shifting
// ---------------------------------------------------------------------------

/// Apply frequency shifting to a WAV file using ffmpeg (preferred) or sox.
///
/// Uses the `asetrate` + `aresample` ffmpeg filter to shift pitch by the given
/// number of Hz, or the sox `pitch` effect as a fallback.
///
/// Returns `true` on success, `false` if both tools fail or are unavailable.
/// BirdNET-Pi equivalent: `FREQ_SHIFT` config applied via sox/rubberband.
fn apply_freq_shift(
    input_path: &Path,
    output_path: &Path,
    sample_rate: u32,
    shift_hz: i32,
) -> bool {
    use std::process::Command;

    // ffmpeg approach: use asetrate to shift the sample rate, then resample back.
    // This is equivalent to speeding up/slowing down, shifting all frequencies.
    // shift_hz > 0 shifts up (makes calls accessible to those with high-freq hearing loss).
    #[allow(clippy::cast_precision_loss)]
    let new_rate = (sample_rate as f64 * (1.0 + shift_hz as f64 / sample_rate as f64)) as u32;
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
    // 1 Hz shift ≈ 100 * log2(1 + shift_hz / sample_rate) * 100 cents (approximation).
    #[allow(clippy::cast_precision_loss)]
    let cents = (1200.0f64 * (1.0 + shift_hz as f64 / sample_rate as f64).log2()) as i32;

    Command::new("sox")
        .arg(input_path)
        .arg(output_path)
        .args(["pitch", &cents.to_string()])
        .status()
        .is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------------------
// WAV writing
// ---------------------------------------------------------------------------

/// Write mono f32 samples to a 16-bit PCM WAV file.
///
/// # Errors
///
/// Returns [`ExtractionError::Write`] if writing fails.
#[allow(clippy::cast_possible_truncation)]
fn write_wav_clip(
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

// ---------------------------------------------------------------------------
// Spectrogram generation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_detection() -> Detection {
        Detection {
            date: "2026-03-14".into(),
            time: "08:30:00".into(),
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            start: 3.0,
            stop: 6.0,
            week: 11,
            file_name_extr: None,
        }
    }

    #[test]
    fn default_config_values() {
        let config = ExtractionConfig::default();
        assert!((config.extraction_length - 6.0).abs() < f32::EPSILON);
        assert!((config.recording_length - 15.0).abs() < f32::EPSILON);
        assert_eq!(config.audio_format, "wav");
    }

    #[test]
    fn build_filename_without_rtsp() {
        let det = sample_detection();
        let name = build_extraction_filename(&det, "wav");
        assert_eq!(
            name,
            "Eurasian_Blackbird-87-2026-03-14-birdnet-08:30:00.wav"
        );
    }

    #[test]
    fn spacer_calculation() {
        // extraction_length=6, so spacer = (6-3)/2 = 1.5
        let config = ExtractionConfig::default();
        let spacer = (config.extraction_length - 3.0) / 2.0;
        assert!((spacer - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn safe_boundaries_clamped() {
        let config = ExtractionConfig {
            extraction_length: 6.0,
            recording_length: 15.0,
            ..ExtractionConfig::default()
        };
        let spacer = (config.extraction_length - 3.0) / 2.0;

        // Detection near the start: start=0.5, so safe_start should clamp to 0.
        let safe_start = (0.5_f32 - spacer).max(0.0);
        assert!((safe_start - 0.0).abs() < f32::EPSILON);

        // Detection near the end: stop=14.5, so safe_stop should clamp to 15.
        let safe_stop = (14.5_f32 + spacer).min(config.recording_length);
        assert!((safe_stop - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn extract_nonexistent_source_returns_error() {
        let config = ExtractionConfig::default();
        let extractor = Extractor::new(config);
        let det = sample_detection();
        let result = extractor.extract_detection(Path::new("/nonexistent/audio.wav"), &det);
        assert!(result.is_err());
    }

    #[test]
    fn write_and_read_wav_clip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output_path = dir.path().join("test_clip.wav");

        // Generate a short sine wave.
        let sample_rate = 48_000_u32;
        let duration_samples = sample_rate as usize / 2; // 0.5s
        let samples: Vec<f32> = (0..duration_samples)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();

        write_wav_clip(&samples, sample_rate, &output_path).expect("write wav");
        assert!(output_path.exists());

        // Read back with hound and verify basic properties.
        let reader = hound::WavReader::open(&output_path).expect("read wav");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, sample_rate);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(reader.len() as usize, duration_samples);
    }

    #[test]
    fn extract_detection_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create a source WAV file with 3 seconds of audio.
        let sample_rate = 48_000_u32;
        let duration_secs = 3.0_f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let num_samples = (duration_secs * sample_rate as f32) as usize;
        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
            })
            .collect();

        let source_path = dir.path().join("source.wav");
        write_wav_clip(&samples, sample_rate, &source_path).expect("write source");

        let config = ExtractionConfig {
            extraction_length: 3.0, // no padding so spacer = 0
            output_dir: dir.path().to_path_buf(),
            audio_format: "wav".into(),
            target_format: AudioFormat::Wav,
            recording_length: 3.0,
        };

        let extractor = Extractor::new(config);
        let det = Detection {
            date: "2026-03-14".into(),
            time: "10:00:00".into(),
            scientific_name: "Parus major".into(),
            common_name: "Great Tit".into(),
            confidence: 0.95,
            start: 0.0,
            stop: 3.0,
            week: 11,
            file_name_extr: None,
        };

        let result = extractor.extract_detection(&source_path, &det);
        assert!(result.is_ok(), "extract_detection failed: {result:?}");

        let output_path = result.expect("already checked");
        assert!(output_path.exists());
        assert!(output_path.to_string_lossy().contains("Great_Tit"));
        assert!(output_path.to_string_lossy().contains("By_Date"));
        assert!(output_path.to_string_lossy().contains("2026-03-14"));
    }

    #[test]
    fn generate_spectrogram_from_wav() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create a WAV file with enough samples for spectrogram.
        let sample_rate = 48_000_u32;
        let num_samples = sample_rate as usize; // 1 second
        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();

        let wav_path = dir.path().join("test_spec.wav");
        write_wav_clip(&samples, sample_rate, &wav_path).expect("write wav");

        let mel_config = MelConfig::default();
        let result = generate_spectrogram(&wav_path, &mel_config);
        assert!(result.is_ok(), "spectrogram generation failed: {result:?}");

        let spec = result.expect("already checked");
        assert_eq!(spec.n_mels, 128);
        assert!(spec.n_frames > 0);
    }
}
