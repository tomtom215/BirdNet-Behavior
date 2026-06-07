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
//!
//! This module owns the shared types and the public surface; the work is split
//! into two submodules:
//!
//! - `process` — per-file decode → pipeline → inference → filtering.
//! - `run` — the directory-watch loop that drives settled clips through it.

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, mpsc};

use crate::detection::pipeline::{self, PipelineConfig};
use crate::detection::types::Detection;
use crate::inference::model::{InferenceError, ModelConfig};

mod process;
mod run;

pub use process::{process_and_infer, process_and_infer_filtered, process_file_pipeline_only};
pub use run::run_daemon;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

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
