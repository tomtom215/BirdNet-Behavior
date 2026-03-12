//! Detection pipeline: watch → decode → spectrogram → infer → report.
//!
//! Watches a directory for new audio files via `notify`, then processes each
//! through the audio pipeline (decode → resample → mel spectrogram) and
//! prepares it for inference.
//!
//! The inference step itself is pluggable -- the pipeline produces mel
//! spectrograms and accepts results back for reporting.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::audio::decode;
use crate::audio::resample;
use crate::audio::spectrogram::{self, MelConfig, MelSpectrogram};
use crate::detection::types::RecordingFile;

/// Errors from the detection pipeline.
#[derive(Debug)]
pub enum PipelineError {
    /// File watcher failed to start or encountered an error.
    Watch(String),
    /// Audio decoding failed.
    Decode(decode::DecodeError),
    /// Resampling failed.
    Resample(resample::ResampleError),
    /// Spectrogram computation failed.
    Spectrogram(spectrogram::SpectrogramError),
    /// Channel communication error.
    Channel(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watch(msg) => write!(f, "file watch error: {msg}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Resample(e) => write!(f, "resample error: {e}"),
            Self::Spectrogram(e) => write!(f, "spectrogram error: {e}"),
            Self::Channel(msg) => write!(f, "channel error: {msg}"),
        }
    }
}

impl std::error::Error for PipelineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Resample(e) => Some(e),
            Self::Spectrogram(e) => Some(e),
            Self::Watch(_) | Self::Channel(_) => None,
        }
    }
}

impl From<decode::DecodeError> for PipelineError {
    fn from(e: decode::DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl From<resample::ResampleError> for PipelineError {
    fn from(e: resample::ResampleError) -> Self {
        Self::Resample(e)
    }
}

impl From<spectrogram::SpectrogramError> for PipelineError {
    fn from(e: spectrogram::SpectrogramError) -> Self {
        Self::Spectrogram(e)
    }
}

/// Configuration for the detection pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Directory to watch for new audio files.
    pub watch_dir: PathBuf,
    /// Target sample rate for the ML model.
    pub target_sample_rate: u32,
    /// Mel spectrogram configuration.
    pub mel_config: MelConfig,
    /// Duration of each audio chunk in seconds.
    pub chunk_duration_secs: f32,
    /// Overlap between chunks in seconds.
    pub chunk_overlap_secs: f32,
    /// Minimum confidence threshold for reporting.
    pub confidence_threshold: f32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            watch_dir: PathBuf::from("/tmp/StreamData"),
            target_sample_rate: 48000,
            mel_config: MelConfig::default(),
            chunk_duration_secs: 3.0,
            chunk_overlap_secs: 0.0,
            confidence_threshold: 0.25,
        }
    }
}

/// A prepared audio chunk ready for ML inference.
#[derive(Debug, Clone)]
pub struct PreparedChunk {
    /// Mel spectrogram for this chunk.
    pub spectrogram: MelSpectrogram,
    /// Start time of this chunk within the recording (seconds).
    pub start_secs: f32,
    /// End time of this chunk within the recording (seconds).
    pub end_secs: f32,
    /// Source recording file metadata.
    pub recording: RecordingFile,
}

/// Process a single audio file through the pipeline.
///
/// Decodes, resamples, splits into chunks, and computes mel spectrograms.
/// Returns prepared chunks ready for inference.
///
/// # Errors
///
/// Returns `PipelineError` if any stage of the pipeline fails.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss, clippy::cast_sign_loss)]
pub fn process_file(
    path: &Path,
    config: &PipelineConfig,
) -> Result<Vec<PreparedChunk>, PipelineError> {
    let recording = RecordingFile::parse(&path.to_string_lossy())
        .ok_or_else(|| PipelineError::Watch(format!(
            "cannot parse recording filename: {}",
            path.display()
        )))?;

    // Decode audio
    let audio = decode::decode_file(path)?;

    // Resample to target rate
    let samples = resample::resample(
        &audio.samples,
        audio.sample_rate,
        config.target_sample_rate,
    )?;

    // Split into chunks
    let chunk_samples = (config.chunk_duration_secs * config.target_sample_rate as f32) as usize;
    let overlap_samples = (config.chunk_overlap_secs * config.target_sample_rate as f32) as usize;
    let step = chunk_samples.saturating_sub(overlap_samples).max(1);

    let mut chunks = Vec::new();
    let mut pos = 0;

    while pos < samples.len() {
        let end = (pos + chunk_samples).min(samples.len());
        let mut chunk_data = samples[pos..end].to_vec();

        // Pad short chunks with zeros (matching Python behavior)
        if chunk_data.len() < chunk_samples {
            chunk_data.resize(chunk_samples, 0.0);
        }

        let mel = spectrogram::mel_spectrogram(
            &chunk_data,
            config.target_sample_rate,
            &config.mel_config,
        )?;

        let start_secs = pos as f32 / config.target_sample_rate as f32;
        let end_secs = end as f32 / config.target_sample_rate as f32;

        chunks.push(PreparedChunk {
            spectrogram: mel,
            start_secs,
            end_secs,
            recording: recording.clone(),
        });

        pos += step;

        // Don't create a chunk if remaining audio is too short
        if samples.len() - pos < chunk_samples / 4 && pos < samples.len() {
            break;
        }
    }

    Ok(chunks)
}

/// Create a file watcher for a directory.
///
/// Returns a receiver that yields paths to newly created/modified audio files.
/// The watcher must be kept alive (not dropped) for events to continue.
///
/// # Errors
///
/// Returns `PipelineError::Watch` if the watcher cannot be created.
pub fn watch_directory(
    dir: &Path,
) -> Result<(RecommendedWatcher, mpsc::Receiver<PathBuf>), PipelineError> {
    let (tx, rx) = mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            if let Ok(event) = result {
                if matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_)
                ) {
                    for path in event.paths {
                        if is_audio_file(&path) {
                            let _ = tx.send(path);
                        }
                    }
                }
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| PipelineError::Watch(e.to_string()))?;

    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .map_err(|e| PipelineError::Watch(e.to_string()))?;

    Ok((watcher, rx))
}

/// Check if a path has a supported audio extension.
fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ext.eq_ignore_ascii_case("wav")
                || ext.eq_ignore_ascii_case("flac")
                || ext.eq_ignore_ascii_case("mp3")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_audio_file_accepts_supported_extensions() {
        assert!(is_audio_file(Path::new("recording.wav")));
        assert!(is_audio_file(Path::new("recording.WAV")));
        assert!(is_audio_file(Path::new("recording.flac")));
        assert!(is_audio_file(Path::new("recording.mp3")));
        assert!(is_audio_file(Path::new("/data/StreamData/2026-03-11-birdnet-08:30:00.wav")));
    }

    #[test]
    fn is_audio_file_rejects_non_audio() {
        assert!(!is_audio_file(Path::new("data.txt")));
        assert!(!is_audio_file(Path::new("image.png")));
        assert!(!is_audio_file(Path::new("noext")));
    }

    #[test]
    fn default_pipeline_config() {
        let config = PipelineConfig::default();
        assert_eq!(config.target_sample_rate, 48000);
        assert!((config.chunk_duration_secs - 3.0).abs() < f32::EPSILON);
        assert!((config.confidence_threshold - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn process_nonexistent_file_returns_error() {
        let config = PipelineConfig::default();
        let result = process_file(Path::new("/nonexistent/2026-03-11-birdnet-08:30:00.wav"), &config);
        assert!(result.is_err());
    }
}
