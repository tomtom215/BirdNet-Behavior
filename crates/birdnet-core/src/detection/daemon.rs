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

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
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
    /// Per-species confidence threshold overrides (`sci_name` → threshold).
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
    /// Correlation ID stamped at file-arrival time and propagated through
    /// every event the daemon emits for that file. Every log line, DB
    /// write, and notification dispatched downstream tags this ID so the
    /// operator can trace one audio file end-to-end with a single grep.
    /// Empty when the upstream call site did not set one (older API).
    pub correlation_id: String,
}

/// Handle for controlling a running daemon.
pub struct DaemonHandle {
    stop_tx: mpsc::Sender<()>,
    heartbeat: Arc<AtomicU64>,
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

    /// A shared counter the detection loop increments on every iteration.
    ///
    /// Sample it periodically (e.g. from the systemd watchdog pinger): if it
    /// stops advancing, the detection pipeline has hung and the process should
    /// be restarted rather than left silently frozen.
    #[must_use]
    pub fn heartbeat(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.heartbeat)
    }
}

/// Generate a short correlation ID for a single audio file.
///
/// Format: `e-{ms-since-epoch}-{counter:04x}`. Sortable by arrival time,
/// monotonically increasing per-process, short enough for one log line.
/// Used only as a debug aid for tracing one file through the pipeline —
/// uniqueness across processes is not required (every process scrapes
/// its own log).
#[must_use]
pub fn new_event_correlation_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("e-{ms}-{n:04x}")
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
/// `correlation_id`, if non-empty, is stamped on every event emitted for
/// this file and surfaced in every log line — see [`DetectionEvent::correlation_id`].
///
/// # Errors
///
/// Returns `DaemonError` if any stage fails.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn process_and_infer(
    path: &Path,
    pipeline_config: &PipelineConfig,
    model: &mut BirdNetModel,
    correlation_id: &str,
) -> Result<Vec<DetectionEvent>, DaemonError> {
    let start = Instant::now();

    let chunks = pipeline::process_file(path, pipeline_config)?;
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        correlation_id,
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
                correlation_id,
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
                correlation_id: correlation_id.to_owned(),
            });
        }
    }

    let total = start.elapsed();
    tracing::info!(
        correlation_id,
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
    model: &mut BirdNetModel,
    privacy_filter: &PrivacyFilter,
    species_filter: &mut SpeciesFilter,
    lat: Option<f64>,
    lon: Option<f64>,
    week: u32,
    correlation_id: &str,
) -> Result<Vec<DetectionEvent>, DaemonError> {
    let start = Instant::now();

    let chunks = pipeline::process_file(path, pipeline_config)?;
    let pipeline_elapsed = start.elapsed();

    tracing::debug!(
        correlation_id,
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
            if let Some(ref allowed) = allowed_species
                && !allowed.contains(&detection.scientific_name)
            {
                continue;
            }

            // Apply per-species confidence threshold (checked in event_processor instead)
            // The daemon produces raw events; threshold filtering is done downstream.

            tracing::info!(
                correlation_id,
                species = %detection.common_name,
                confidence = format!("{:.1}%", detection.confidence * 100.0),
                chunk = format!("{:.1}s-{:.1}s", chunk.start_secs, chunk.end_secs),
                "detection (filtered)"
            );

            events.push(DetectionEvent {
                detection: detection.clone(),
                source_file: path.to_path_buf(),
                latency_ms: total_ms,
                correlation_id: correlation_id.to_owned(),
            });
        }
    }

    let total = start.elapsed();
    tracing::info!(
        correlation_id,
        file = %path.display(),
        detections = events.len(),
        total_ms = total.as_millis(),
        privacy = privacy_filter.is_enabled(),
        species_filter = species_filter.has_model(),
        "filtered file processing complete"
    );

    Ok(events)
}

/// How long a file's size must stay unchanged before it is considered fully
/// written and safe to decode.
///
/// Capture backends don't publish clips atomically — notably ffmpeg's segment
/// muxer (used for RTSP) writes each clip *in place* over several seconds,
/// emitting a stream of create/modify events the whole time. Decoding before
/// the clip is finalized fails with "unexpected end of file", and the same
/// growing file gets reprocessed on every write. Two seconds comfortably clears
/// the inter-write gaps of a real-time PCM segment while adding latency that is
/// negligible next to a multi-second recording.
const FILE_SETTLE: Duration = Duration::from_secs(2);

/// Debounces filesystem events so each captured file is decoded once, and only
/// after it has finished being written.
///
/// A capture backend emits a burst of create/modify events while it streams a
/// clip to disk. [`PendingFiles`] holds each path until its size has been
/// stable for [`FILE_SETTLE`], then yields it exactly once. File size (polled
/// via the injected sizer) is the source of truth, so a clip still settles
/// after its final watcher event and a dropped event cannot strand it.
#[derive(Default)]
struct PendingFiles {
    /// path -> (last observed size, the instant that size last changed)
    seen: HashMap<PathBuf, (u64, Instant)>,
}

impl PendingFiles {
    fn new() -> Self {
        Self::default()
    }

    /// Record watcher activity on `path`. Repeated calls for a path already
    /// tracked are no-ops: the size poll in [`Self::drain_settled`] drives the
    /// settle timer, not the (bursty, backend-specific) event rate.
    fn note(&mut self, path: PathBuf, now: Instant) {
        // `u64::MAX` is a "size not yet observed" sentinel that the first sweep
        // always treats as a change, establishing the real baseline.
        self.seen.entry(path).or_insert((u64::MAX, now));
    }

    /// Return the tracked files whose size has been unchanged for at least
    /// `settle`, removing them so each is yielded exactly once. `sizer` returns
    /// a file's current size, or `None` if it has vanished (which drops it).
    fn drain_settled<F>(&mut self, now: Instant, settle: Duration, sizer: F) -> Vec<PathBuf>
    where
        F: Fn(&Path) -> Option<u64>,
    {
        let mut ready = Vec::new();
        self.seen
            .retain(|path, (last_size, last_change)| match sizer(path) {
                None => false,
                Some(current) if current != *last_size => {
                    *last_size = current;
                    *last_change = now;
                    true
                }
                Some(current) if current > 0 && now.duration_since(*last_change) >= settle => {
                    ready.push(path.clone());
                    false
                }
                Some(_) => true,
            });
        ready
    }
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
#[allow(clippy::too_many_lines)]
pub fn run_daemon(
    config: &DaemonConfig,
    event_tx: mpsc::SyncSender<DetectionEvent>,
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
    let mut model = BirdNetModel::load(&config.model_path, labels, config.model.clone())?;

    // Auto-detect the sample rate the model expects from its input shape.
    // V2.4 → [1, 144_000] = 48 kHz × 3 s; V3.0 → [1, 96_000] = 32 kHz × 3 s.
    let model_sample_rate = model.infer_sample_rate();

    tracing::info!(
        model_path = %config.model_path.display(),
        input_shape = ?model.input_shape(),
        sample_rate = model_sample_rate,
        "model loaded, starting daemon"
    );

    // Build pipeline config, overriding sample rate and input mode to match the model.
    let mut pipeline_config = config.pipeline.clone();
    if pipeline_config.target_sample_rate != model_sample_rate {
        tracing::info!(
            configured = pipeline_config.target_sample_rate,
            model = model_sample_rate,
            "adjusting pipeline sample rate to match model"
        );
        pipeline_config.target_sample_rate = model_sample_rate;
    }
    // V3.0 models expect raw audio; V2.4 models expect a mel spectrogram.
    let raw_mode = model.infer_sample_rate() == 32_000;
    if raw_mode != pipeline_config.raw_audio_input {
        tracing::info!(
            raw_audio_input = raw_mode,
            "adjusting pipeline input mode to match model"
        );
        pipeline_config.raw_audio_input = raw_mode;
    }

    // Adopt the model's recommended chunk length when it differs from the
    // pipeline default. This matters most for V3.0 preview3 (dynamic input
    // shape): with 3.0 s × 32 kHz = 96 000 samples the Magpie reference
    // confidence on the bundled WAV is ~0.52, but at 4.5 s × 32 kHz =
    // 144 000 samples it rises to ~0.72. The model accepts variable length
    // so this is purely a per-chunk accuracy tuning. Fixed-shape V2.4 keeps
    // its trained 3.0 s window.
    let model_chunk_secs = model.recommended_chunk_secs();
    let configured_chunk_secs = pipeline_config.chunk_duration_secs;
    if (model_chunk_secs - configured_chunk_secs).abs() > 0.01 {
        tracing::info!(
            configured_chunk_secs = configured_chunk_secs,
            model_chunk_secs,
            "adjusting pipeline chunk duration to match model recommendation"
        );
        pipeline_config.chunk_duration_secs = model_chunk_secs;
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

    // Liveness heartbeat: the loop bumps this once per iteration so an external
    // watchdog can distinguish a hung pipeline (no progress) from an idle one.
    let heartbeat = Arc::new(AtomicU64::new(0));
    let heartbeat_loop = Arc::clone(&heartbeat);

    // Start file watcher. The `RecommendedWatcher` MUST live for the
    // lifetime of the spawned thread — dropping it stops delivery and
    // closes the channel. Bound to a name that the closure captures so
    // `move ||` takes ownership and keeps it alive.
    let (file_watcher, file_rx) =
        pipeline::watch_directory(&config.watch_dir).map_err(DaemonError::Pipeline)?;

    // Snapshot the backlog settings to move into the loop thread; `config` is
    // a borrow and cannot outlive this call on the spawned 'static thread.
    let process_existing = config.process_existing;
    let watch_dir = config.watch_dir.clone();

    // Main daemon loop -- runs on its own thread
    std::thread::spawn(move || {
        // Keep the watcher alive for the lifetime of this thread.
        // Without this the `RecommendedWatcher` gets dropped when
        // `start_detection_daemon` returns, the underlying `notify`
        // backend stops, and `file_rx` immediately reports
        // `Disconnected` — silently breaking the watch path.
        let _watcher = file_watcher;
        tracing::info!("detection daemon started");

        // Process any pre-existing backlog here, on the loop thread, rather
        // than before signalling readiness. The event consumer is already
        // draining by now, so a large backlog cannot block startup past the
        // systemd TimeoutStartSec, and with a bounded event channel it applies
        // backpressure instead of dead-locking an undrained queue.
        if process_existing {
            process_existing_files(
                &watch_dir,
                &pipeline_config,
                &mut model,
                &privacy_filter,
                &mut species_filter,
                lat,
                lon,
                &event_tx,
            );
        }

        // Debounce watcher events: a clip is decoded only once its size has
        // been stable for FILE_SETTLE (see PendingFiles), so an in-progress
        // ffmpeg/RTSP segment isn't decoded mid-write (which fails with
        // "unexpected end of file" and reprocesses the same growing file).
        let mut pending = PendingFiles::new();

        loop {
            // Heartbeat: record that the loop is still cycling so a watchdog
            // can tell a hung pipeline from an idle one.
            heartbeat_loop.fetch_add(1, Ordering::Relaxed);

            // Check for stop signal (non-blocking)
            if stop_rx.try_recv().is_ok() {
                tracing::info!("detection daemon stopping");
                break;
            }

            // Collect every watcher event currently available (blocking briefly
            // for the first) into the pending set. A burst of modify events for
            // a file still being written collapses into a single entry.
            match file_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(path) => {
                    pending.note(path, Instant::now());
                    while let Ok(path) = file_rx.try_recv() {
                        pending.note(path, Instant::now());
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("file watcher disconnected, stopping daemon");
                    break;
                }
            }

            // Process whichever files have finished being written. Polling the
            // size each sweep means a clip settles even after its last watcher
            // event, so the final segment is never stranded.
            for path in pending.drain_settled(Instant::now(), FILE_SETTLE, |p| {
                std::fs::metadata(p).map(|m| m.len()).ok()
            }) {
                // Keep the watchdog fed if a single sweep processes several files.
                heartbeat_loop.fetch_add(1, Ordering::Relaxed);

                // Stamp a correlation ID on every event we publish for this
                // file so the operator can trace one file through the entire
                // pipeline by grepping a single string.
                let correlation_id = new_event_correlation_id();
                tracing::info!(
                    correlation_id = %correlation_id,
                    file = %path.display(),
                    "begin processing file"
                );

                match process_and_infer_filtered(
                    &path,
                    &pipeline_config,
                    &mut model,
                    &privacy_filter,
                    &mut species_filter,
                    lat,
                    lon,
                    0, // week will be computed by caller
                    &correlation_id,
                ) {
                    Ok(events) => {
                        for event in events {
                            if event_tx.send(event).is_err() {
                                tracing::warn!(
                                    correlation_id = %correlation_id,
                                    "event receiver dropped, stopping daemon"
                                );
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            correlation_id = %correlation_id,
                            file = %path.display(),
                            error = %e,
                            "failed to process file"
                        );
                    }
                }
            }
        }

        tracing::info!("detection daemon stopped");
    });

    Ok(DaemonHandle { stop_tx, heartbeat })
}

/// Process any audio files already present in the watch directory.
#[allow(clippy::too_many_arguments)]
fn process_existing_files(
    dir: &Path,
    pipeline_config: &PipelineConfig,
    model: &mut BirdNetModel,
    privacy_filter: &PrivacyFilter,
    species_filter: &mut SpeciesFilter,
    lat: Option<f64>,
    lon: Option<f64>,
    event_tx: &mpsc::SyncSender<DetectionEvent>,
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

        let correlation_id = new_event_correlation_id();
        match process_and_infer_filtered(
            &path,
            pipeline_config,
            model,
            privacy_filter,
            species_filter,
            lat,
            lon,
            0,
            &correlation_id,
        ) {
            Ok(events) => {
                for event in events {
                    // Surface a closed receiver instead of swallowing it: with
                    // the prior `let _ =` a consumer that dropped mid-backlog
                    // left this loop spinning through the rest of the watch
                    // directory pointlessly (each `send` errored, was ignored,
                    // and we processed the next file anyway). The main runtime
                    // loop treats a closed receiver as fatal — match that here.
                    if event_tx.send(event).is_err() {
                        tracing::debug!(
                            "existing-file backlog stopping: event receiver closed"
                        );
                        return;
                    }
                }
                count += 1;
            }
            Err(e) => {
                tracing::debug!(
                    correlation_id = %correlation_id,
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
    use std::cell::Cell;

    #[test]
    fn pending_files_yields_only_after_size_is_stable() {
        // Models an ffmpeg/RTSP segment growing in place, then finalizing.
        let mut pending = PendingFiles::new();
        let clip = PathBuf::from("/tmp/birdnet-stream/clip.wav");
        let t0 = Instant::now();
        pending.note(clip.clone(), t0);

        // Fresh closure per call (each reads the current `size`) so the sizer is
        // passed by value — `&closure` would trip clippy::needless_borrows.
        let size = Cell::new(100u64);
        let sizer = || |_: &Path| Some(size.get());

        // Baseline observed -> not ready.
        assert!(pending.drain_settled(t0, FILE_SETTLE, sizer()).is_empty());
        // Still growing -> the settle timer resets, still not ready.
        size.set(200);
        assert!(
            pending
                .drain_settled(t0 + Duration::from_millis(500), FILE_SETTLE, sizer())
                .is_empty()
        );
        // Size now stable, but the settle window has not elapsed yet.
        assert!(
            pending
                .drain_settled(t0 + Duration::from_millis(700), FILE_SETTLE, sizer())
                .is_empty()
        );
        // Stable for >= FILE_SETTLE since the last change -> yielded once.
        let ready = pending.drain_settled(
            t0 + Duration::from_millis(500) + FILE_SETTLE,
            FILE_SETTLE,
            sizer(),
        );
        assert_eq!(
            ready,
            vec![clip],
            "a settled clip must be processed exactly once"
        );
        // ...and never again (it was removed when processed).
        assert!(
            pending
                .drain_settled(t0 + Duration::from_secs(60), FILE_SETTLE, sizer())
                .is_empty(),
            "a processed clip must not be reprocessed"
        );
    }

    #[test]
    fn pending_files_drops_vanished_and_never_yields_empty() {
        let mut pending = PendingFiles::new();
        let gone = PathBuf::from("/tmp/birdnet-stream/gone.wav");
        let empty = PathBuf::from("/tmp/birdnet-stream/empty.wav");
        let t0 = Instant::now();
        pending.note(gone.clone(), t0); // keep: gone is reused by the sizer below
        pending.note(empty, t0);

        // Fresh closure per call (passed by value): gone vanished (None), empty
        // is zero bytes.
        let sizer = || {
            |p: &Path| {
                if p == gone.as_path() {
                    None
                } else {
                    Some(0u64)
                }
            }
        };

        // A vanished file is dropped; a zero-byte file is never "stable enough"
        // to decode no matter how long it sits.
        assert!(
            pending
                .drain_settled(t0 + Duration::from_secs(10), FILE_SETTLE, sizer())
                .is_empty()
        );
        assert!(
            pending
                .drain_settled(t0 + Duration::from_secs(20), FILE_SETTLE, sizer())
                .is_empty()
        );
    }

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
        let handle = DaemonHandle {
            stop_tx,
            heartbeat: Arc::new(AtomicU64::new(0)),
        };
        handle.stop(); // Should not panic even if receiver is alive
    }

    #[test]
    fn daemon_handle_exposes_shared_heartbeat() {
        let (stop_tx, _stop_rx) = mpsc::channel();
        let heartbeat = Arc::new(AtomicU64::new(7));
        let handle = DaemonHandle {
            stop_tx,
            heartbeat: Arc::clone(&heartbeat),
        };
        // The accessor returns a handle onto the *same* counter, so a watchdog
        // observes the loop's increments.
        assert_eq!(handle.heartbeat().load(Ordering::Relaxed), 7);
        heartbeat.fetch_add(1, Ordering::Relaxed);
        assert_eq!(handle.heartbeat().load(Ordering::Relaxed), 8);
    }

    #[test]
    fn run_daemon_loop_advances_heartbeat() {
        // A healthy detection loop must keep advancing its heartbeat, so the
        // watchdog never mistakes a *running* daemon for a hung one and
        // needlessly restarts a healthy field station. Stand the real loop up
        // against the tiny bundled model and assert the counter climbs.
        const TINY_V24: &[u8] = include_bytes!("../testdata/tiny_v24_test.onnx");

        let tmp = tempfile::tempdir().unwrap();
        let watch_dir = tmp.path().join("recs");
        std::fs::create_dir_all(&watch_dir).unwrap();
        let model_path = tmp.path().join("model.onnx");
        std::fs::write(&model_path, TINY_V24).unwrap();
        let labels_path = tmp.path().join("labels.txt");
        let labels = (0..11)
            .map(|i| format!("Species{i}_Bird {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&labels_path, labels).unwrap();

        let config = DaemonConfig {
            watch_dir,
            model_path,
            labels_path,
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

        let (event_tx, _event_rx) = mpsc::sync_channel(64);
        let handle = run_daemon(&config, event_tx).expect("daemon starts with the tiny model");

        let hb = handle.heartbeat();
        let start = hb.load(Ordering::Relaxed);
        // The loop polls on a 500 ms timeout, so ~1.2 s is at least two cycles.
        std::thread::sleep(Duration::from_millis(1_200));
        let after = hb.load(Ordering::Relaxed);
        handle.stop();

        assert!(
            after > start,
            "detection loop heartbeat must advance while running (start={start}, after={after})"
        );
    }

    // ─── Correlation-ID generator ─────────────────────────────────────────
    //
    // Operators rely on the correlation_id to trace one audio file through
    // decode → infer → notify → DB write with a single `grep` over the log.
    // The contract is "unique enough that two files arriving in the same
    // millisecond still get distinct IDs."

    #[test]
    fn correlation_id_has_recognisable_shape() {
        let id = new_event_correlation_id();
        // `e-{ms}-{counter:04x}`
        assert!(id.starts_with("e-"), "expected prefix 'e-', got: {id}");
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 3, "expected 3 hyphen-segments in: {id}");
        assert!(parts[1].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn correlation_id_is_unique_across_rapid_calls() {
        // Generate a bunch quickly — the per-process counter must keep
        // them distinct even when SystemTime returns the same millisecond.
        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            ids.insert(new_event_correlation_id());
        }
        assert_eq!(ids.len(), 1000, "correlation IDs are not unique");
    }

    #[test]
    fn detection_event_carries_correlation_id() {
        // Pin the contract: DetectionEvent has the field and round-trips
        // through clone without losing it.
        use crate::detection::types::Detection;
        let event = DetectionEvent {
            detection: Detection {
                date: "2026-05-19".to_owned(),
                time: "09:00:00".to_owned(),
                scientific_name: "Pica pica".to_owned(),
                common_name: "Eurasian Magpie".to_owned(),
                confidence: 0.93,
                start: 0.0,
                stop: 4.5,
                week: 20,
                file_name_extr: None,
            },
            source_file: PathBuf::from("/tmp/x.wav"),
            latency_ms: 42,
            correlation_id: "e-12345-0001".to_owned(),
        };
        // Use the clone path even though we drop the original immediately —
        // the clippy::redundant_clone flag will fire, but pinning Clone is
        // exactly the point of this test. Bind both so the second move
        // through Clone is observable.
        let cloned = event.clone();
        drop(event);
        assert_eq!(cloned.correlation_id, "e-12345-0001");
    }
}
