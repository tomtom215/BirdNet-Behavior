//! Detection daemon: orchestrates the file-watch → process → infer → report loop.
//!
//! This module provides the core detection loop that:
//! 1. Watches a directory for new audio files (via `notify`)
//! 2. Decodes, resamples, and generates mel spectrograms
//! 3. Runs inference to classify bird species
//! 4. Reports detections via a callback (database insert, WebSocket broadcast, etc.)
//!
//! The daemon is synchronous internally (all audio processing and inference is CPU-bound)
//! and designed to be spawned on a blocking thread from the async runtime.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::audio::capture::is_audio_file;
use crate::detection::pipeline::{self, PipelineConfig, PreparedChunk};
use crate::detection::privacy::PrivacyFilter;
use crate::detection::types::Detection;
use crate::inference::labels::LabelSet;
use crate::inference::model::{BirdNetModel, InferenceError, ModelConfig};
use crate::inference::species_filter::SpeciesFilter;

/// Errors from the detection daemon.
#[derive(Debug)]
pub enum DaemonError {
    /// Pipeline error (decode, resample, spectrogram).
    Pipeline(pipeline::PipelineError),
    /// Inference error.
    Inference(InferenceError),
    /// Model loading error.
    Model(String),
    /// Configuration error.
    Config(String),
    /// The daemon was stopped.
    Stopped,
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(e) => write!(f, "pipeline: {e}"),
            Self::Inference(e) => write!(f, "inference: {e}"),
            Self::Model(msg) => write!(f, "model: {msg}"),
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Stopped => write!(f, "daemon stopped"),
        }
    }
}

impl std::error::Error for DaemonError {}

impl From<pipeline::PipelineError> for DaemonError {
    fn from(e: pipeline::PipelineError) -> Self {
        Self::Pipeline(e)
    }
}

impl From<InferenceError> for DaemonError {
    fn from(e: InferenceError) -> Self {
        Self::Inference(e)
    }
}

/// Configuration for the detection daemon.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Directory to watch for new audio files.
    pub watch_dir: PathBuf,
    /// Path to the ONNX model file.
    pub model_path: PathBuf,
    /// Path to the labels file.
    pub labels_path: PathBuf,
    /// Pipeline configuration (sample rate, chunk size, etc.).
    pub pipeline: PipelineConfig,
    /// Model configuration (sensitivity, threshold, etc.).
    pub model: ModelConfig,
    /// Whether to process files already present in the watch directory on startup.
    pub process_existing: bool,
    /// Optional path to the metadata ONNX model for species filtering.
    pub metadata_model_path: Option<PathBuf>,
    /// Species filter configuration (threshold, whitelist, include/exclude).
    pub species_filter: crate::inference::species_filter::SpeciesFilterConfig,
    /// Privacy filter threshold (0.0 = disabled).
    pub privacy_threshold: f32,
    /// Station latitude (for species occurrence filtering).
    pub latitude: Option<f64>,
    /// Station longitude (for species occurrence filtering).
    pub longitude: Option<f64>,
    /// Per-species confidence threshold overrides (sci_name → threshold).
    ///
    /// Species in this map use the specified threshold instead of the global one.
    pub species_thresholds: std::collections::HashMap<String, f64>,
}

/// A detection event produced by the daemon.
#[derive(Debug, Clone)]
pub struct DetectionEvent {
    /// The detection result.
    pub detection: Detection,
    /// Source audio file path.
    pub source_file: PathBuf,
    /// Processing latency in milliseconds.
    pub latency_ms: u64,
}

/// Handle for controlling a running daemon.
pub struct DaemonHandle {
    stop_tx: mpsc::Sender<()>,
}

impl fmt::Debug for DaemonHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DaemonHandle").finish()
    }
}

impl DaemonHandle {
    /// Signal the daemon to stop.
    pub fn stop(&self) {
        let _ = self.stop_tx.send(());
    }
}

/// Process a single audio file through the full pipeline (no model -- pipeline-only mode).
///
/// This is useful for testing the audio pipeline without a model,
/// or when running in "prepare only" mode.
///
/// # Errors
///
/// Returns `DaemonError` if any pipeline stage fails.
pub fn process_file_pipeline_only(
    path: &Path,
    config: &PipelineConfig,
) -> Result<Vec<PreparedChunk>, DaemonError> {
    let chunks = pipeline::process_file(path, config)?;
    Ok(chunks)
}

/// Process a single audio file and run inference.
///
/// Returns all detections found in the file, or an empty vec if
/// nothing meets the confidence threshold.
///
/// # Errors
///
/// Returns `DaemonError` if any stage fails.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn process_and_infer(
    path: &Path,
    pipeline_config: &PipelineConfig,
    model: &BirdNetModel,
) -> Result<Vec<DetectionEvent>, DaemonError> {
    let start = Instant::now();

    let chunks = pipeline::process_file(path, pipeline_config)?;
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        file = %path.display(),
        chunks = chunks.len(),
        pipeline_ms = pipeline_elapsed.as_millis(),
        "audio pipeline complete"
    );

    let mut events = Vec::new();

    for chunk in &chunks {
        let infer_start = Instant::now();

        let detections = model.predict(
            &chunk.spectrogram.data,
            &chunk.recording.date,
            &chunk.recording.time,
            chunk.start_secs,
            chunk.end_secs,
            0, // week will be computed by caller
        )?;

        let infer_elapsed = infer_start.elapsed();
        let total_ms = start.elapsed().as_millis() as u64;

        for detection in detections {
            tracing::info!(
                species = %detection.common_name,
                confidence = format!("{:.1}%", detection.confidence * 100.0),
                chunk = format!("{:.1}s-{:.1}s", chunk.start_secs, chunk.end_secs),
                infer_ms = infer_elapsed.as_millis(),
                "detection"
            );

            events.push(DetectionEvent {
                detection,
                source_file: path.to_path_buf(),
                latency_ms: total_ms,
            });
        }
    }

    let total = start.elapsed();
    tracing::info!(
        file = %path.display(),
        detections = events.len(),
        total_ms = total.as_millis(),
        "file processing complete"
    );

    Ok(events)
}

/// Process a single audio file with privacy and species occurrence filters.
///
/// After running inference, applies the privacy filter (suppressing chunks
/// with human voice) and the species occurrence filter (only keeping species
/// that are likely present at the given location and time of year).
///
/// # Errors
///
/// Returns `DaemonError` if any stage fails.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_arguments
)]
pub fn process_and_infer_filtered(
    path: &Path,
    pipeline_config: &PipelineConfig,
    model: &BirdNetModel,
    privacy_filter: &PrivacyFilter,
    species_filter: &mut SpeciesFilter,
    lat: Option<f64>,
    lon: Option<f64>,
    week: u32,
) -> Result<Vec<DetectionEvent>, DaemonError> {
    let start = Instant::now();

    let chunks = pipeline::process_file(path, pipeline_config)?;
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        file = %path.display(),
        chunks = chunks.len(),
        pipeline_ms = pipeline_elapsed.as_millis(),
        "audio pipeline complete"
    );

    // Run inference on all chunks first to collect raw predictions
    let mut all_predictions: Vec<Vec<Detection>> = Vec::with_capacity(chunks.len());

    for chunk in &chunks {
        let detections = model.predict(
            &chunk.spectrogram.data,
            &chunk.recording.date,
            &chunk.recording.time,
            chunk.start_secs,
            chunk.end_secs,
            week,
        )?;
        all_predictions.push(detections);
    }

    // Apply privacy filter
    let filtered_predictions = privacy_filter.filter_predictions(&all_predictions);

    // Build the allowed species set from the species filter
    let allowed_species = if let (Some(lat), Some(lon)) = (lat, lon) {
        Some(species_filter.filter_species(lat, lon, week, model.labels())?)
    } else {
        None
    };

    // Collect events, applying species filter
    let mut events = Vec::new();
    let total_ms = start.elapsed().as_millis() as u64;

    for (chunk, detections) in chunks.iter().zip(filtered_predictions.iter()) {
        for detection in detections {
            // Apply species filter if we have one
            if let Some(ref allowed) = allowed_species {
                if !allowed.contains(&detection.scientific_name) {
                    continue;
                }
            }

            // Apply per-species confidence threshold (checked in event_processor instead)
            // The daemon produces raw events; threshold filtering is done downstream.

            tracing::info!(
                species = %detection.common_name,
                confidence = format!("{:.1}%", detection.confidence * 100.0),
                chunk = format!("{:.1}s-{:.1}s", chunk.start_secs, chunk.end_secs),
                "detection (filtered)"
            );

            events.push(DetectionEvent {
                detection: detection.clone(),
                source_file: path.to_path_buf(),
                latency_ms: total_ms,
            });
        }
    }

    let total = start.elapsed();
    tracing::info!(
        file = %path.display(),
        detections = events.len(),
        total_ms = total.as_millis(),
        privacy = privacy_filter.is_enabled(),
        species_filter = species_filter.has_model(),
        "filtered file processing complete"
    );

    Ok(events)
}

/// Run the detection daemon loop.
///
/// Watches `watch_dir` for new audio files and processes them through
/// the full pipeline. Detections are sent to `event_tx`.
///
/// Returns a `DaemonHandle` for stopping the daemon, and spawns the
/// watch loop on the current thread (blocking).
///
/// # Errors
///
/// Returns `DaemonError` if the model cannot be loaded or the watcher fails.
pub fn run_daemon(
    config: &DaemonConfig,
    event_tx: mpsc::Sender<DetectionEvent>,
) -> Result<DaemonHandle, DaemonError> {
    // Load labels
    let labels = LabelSet::load(&config.labels_path)
        .map_err(|e| DaemonError::Model(format!("labels: {e}")))?;

    tracing::info!(
        species_count = labels.len(),
        labels_path = %config.labels_path.display(),
        "labels loaded"
    );

    // Load model
    let model = BirdNetModel::load(&config.model_path, labels, config.model.clone())?;

    // Auto-detect the sample rate the model expects from its input shape.
    // V2.4 → [1, 144_000] = 48 kHz × 3 s; V3.0 → [1, 96_000] = 32 kHz × 3 s.
    let model_sample_rate = model.infer_sample_rate();

    tracing::info!(
        model_path = %config.model_path.display(),
        input_shape = ?model.input_shape(),
        sample_rate = model_sample_rate,
        "model loaded, starting daemon"
    );

    // Build pipeline config, overriding sample rate to match the model.
    let mut pipeline_config = config.pipeline.clone();
    if pipeline_config.target_sample_rate != model_sample_rate {
        tracing::info!(
            configured = pipeline_config.target_sample_rate,
            model = model_sample_rate,
            "adjusting pipeline sample rate to match model"
        );
        pipeline_config.target_sample_rate = model_sample_rate;
    }

    // Load species filter (metadata model)
    let mut species_filter = config.metadata_model_path.as_ref().map_or_else(
        || SpeciesFilter::new_passthrough(config.species_filter.clone()),
        |mdata_path| match SpeciesFilter::load(mdata_path, config.species_filter.clone()) {
            Ok(sf) => sf,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to load metadata model, falling back to passthrough"
                );
                SpeciesFilter::new_passthrough(config.species_filter.clone())
            }
        },
    );

    // Create privacy filter
    let privacy_filter = PrivacyFilter::new(config.privacy_threshold);

    if privacy_filter.is_enabled() {
        tracing::info!(
            threshold = config.privacy_threshold,
            "privacy filter enabled"
        );
    }

    let lat = config.latitude;
    let lon = config.longitude;

    // Create stop channel
    let (stop_tx, stop_rx) = mpsc::channel();

    // Start file watcher
    let (_watcher, file_rx) =
        pipeline::watch_directory(&config.watch_dir).map_err(DaemonError::Pipeline)?;

    // Process existing files if requested
    if config.process_existing {
        process_existing_files(
            &config.watch_dir,
            &pipeline_config,
            &model,
            &privacy_filter,
            &mut species_filter,
            lat,
            lon,
            &event_tx,
        );
    }

    // Main daemon loop -- runs on current thread
    std::thread::spawn(move || {
        tracing::info!("detection daemon started");

        loop {
            // Check for stop signal (non-blocking)
            if stop_rx.try_recv().is_ok() {
                tracing::info!("detection daemon stopping");
                break;
            }

            // Wait for new file with timeout
            match file_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(path) => {
                    // Small delay to let the file finish writing
                    std::thread::sleep(Duration::from_millis(200));

                    match process_and_infer_filtered(
                        &path,
                        &pipeline_config,
                        &model,
                        &privacy_filter,
                        &mut species_filter,
                        lat,
                        lon,
                        0, // week will be computed by caller
                    ) {
                        Ok(events) => {
                            for event in events {
                                if event_tx.send(event).is_err() {
                                    tracing::warn!("event receiver dropped, stopping daemon");
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                file = %path.display(),
                                error = %e,
                                "failed to process file"
                            );
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("file watcher disconnected, stopping daemon");
                    break;
                }
            }
        }

        tracing::info!("detection daemon stopped");
    });

    Ok(DaemonHandle { stop_tx })
}

/// Process any audio files already present in the watch directory.
#[allow(clippy::too_many_arguments)]
fn process_existing_files(
    dir: &Path,
    pipeline_config: &PipelineConfig,
    model: &BirdNetModel,
    privacy_filter: &PrivacyFilter,
    species_filter: &mut SpeciesFilter,
    lat: Option<f64>,
    lon: Option<f64>,
    event_tx: &mpsc::Sender<DetectionEvent>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                dir = %dir.display(),
                error = %e,
                "cannot read watch directory for existing files"
            );
            return;
        }
    };

    let mut count = 0_u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        if !is_audio_file(&path) {
            continue;
        }

        match process_and_infer_filtered(
            &path,
            pipeline_config,
            model,
            privacy_filter,
            species_filter,
            lat,
            lon,
            0,
        ) {
            Ok(events) => {
                for event in events {
                    let _ = event_tx.send(event);
                }
                count += 1;
            }
            Err(e) => {
                tracing::debug!(
                    file = %path.display(),
                    error = %e,
                    "skipping existing file"
                );
            }
        }
    }

    if count > 0 {
        tracing::info!(count, "processed existing audio files");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_defaults() {
        let config = DaemonConfig {
            watch_dir: PathBuf::from("/tmp/StreamData"),
            model_path: PathBuf::from("/opt/birdnet/model.onnx"),
            labels_path: PathBuf::from("/opt/birdnet/labels.txt"),
            pipeline: PipelineConfig::default(),
            model: ModelConfig::default(),
            process_existing: false,
            metadata_model_path: None,
            species_filter: crate::inference::species_filter::SpeciesFilterConfig::default(),
            privacy_threshold: 0.0,
            latitude: None,
            longitude: None,
            species_thresholds: std::collections::HashMap::new(),
        };
        assert_eq!(config.watch_dir, PathBuf::from("/tmp/StreamData"));
        assert!(!config.process_existing);
        assert!(config.metadata_model_path.is_none());
        assert!((config.privacy_threshold).abs() < f32::EPSILON);
        assert!(config.species_thresholds.is_empty());
    }

    #[test]
    fn process_nonexistent_file_returns_error() {
        let config = PipelineConfig::default();
        let result = process_file_pipeline_only(
            Path::new("/nonexistent/2026-03-11-birdnet-08:30:00.wav"),
            &config,
        );
        assert!(result.is_err());
    }

    #[test]
    fn daemon_handle_stop_does_not_panic() {
        let (stop_tx, _stop_rx) = mpsc::channel();
        let handle = DaemonHandle { stop_tx };
        handle.stop(); // Should not panic even if receiver is alive
    }
}
