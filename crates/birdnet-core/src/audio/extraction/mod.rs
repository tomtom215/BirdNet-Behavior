//! Detection audio extraction and spectrogram generation.
//!
//! Extracts audio clips around each detection and saves them to disk.
//! Replaces BirdNET-Pi's `extract_safe()` Python function and sox usage
//! with symphonia (reading) and hound (WAV writing).

mod config;
mod convert;
mod extractor;
mod format;
mod wav;

use std::fmt;

use crate::audio::decode::DecodeError;

// Re-export public API.
pub use config::ExtractionConfig;
pub use extractor::Extractor;
pub use format::AudioFormat;
pub use wav::generate_spectrogram;

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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::audio::spectrogram::MelConfig;
    use crate::detection::types::Detection;

    use super::extractor::build_extraction_filename;
    use super::wav::write_wav_clip;
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
            ..ExtractionConfig::default()
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
