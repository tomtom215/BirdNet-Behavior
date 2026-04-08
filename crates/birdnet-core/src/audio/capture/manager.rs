//! `CaptureManager`: lifecycle management for audio capture processes.
//!
//! Starts the appropriate subprocess (arecord / ffmpeg), monitors it for
//! unexpected exits, and restarts it up to `max_restarts` times.

use super::process::{CaptureProcess, is_tool_available, required_tool, spawn_capture};
use super::types::{CaptureError, RecordingConfig};

/// Maximum number of automatic restarts before giving up.
const DEFAULT_MAX_RESTARTS: u32 = 10;

/// Manages the lifecycle of an audio capture process.
///
/// Call [`CaptureManager::start`] to begin recording, and rely on
/// [`CaptureManager::check_and_restart`] (called periodically from a
/// monitoring task) to keep the subprocess alive after unexpected exits.
#[derive(Debug)]
pub struct CaptureManager {
    config: RecordingConfig,
    process: Option<CaptureProcess>,
    restart_count: u32,
    max_restarts: u32,
}

impl CaptureManager {
    /// Create a new capture manager.
    ///
    /// Does not start capture — call [`start`](Self::start) to begin recording.
    pub const fn new(config: RecordingConfig) -> Self {
        Self {
            config,
            process: None,
            restart_count: 0,
            max_restarts: DEFAULT_MAX_RESTARTS,
        }
    }

    /// Override the maximum number of automatic restarts (default: 10).
    #[must_use]
    pub const fn with_max_restarts(mut self, n: u32) -> Self {
        self.max_restarts = n;
        self
    }

    /// Start the capture process.
    ///
    /// # Errors
    ///
    /// Returns `CaptureError` if the required tool (arecord/ffmpeg) is not
    /// found in `PATH`, if the output directory cannot be created, or if the
    /// subprocess cannot be spawned.
    pub fn start(&mut self) -> Result<(), CaptureError> {
        // Ensure output directory exists.
        std::fs::create_dir_all(&self.config.output_dir).map_err(CaptureError::Spawn)?;

        let tool = required_tool(&self.config.source);
        if !is_tool_available(tool) {
            return Err(CaptureError::Config(format!("{tool} not found in PATH")));
        }

        let process = spawn_capture(&self.config)?;
        self.process = Some(process);
        self.restart_count = 0;
        tracing::info!("capture started");
        Ok(())
    }

    /// Stop the capture process.
    pub fn stop(&mut self) {
        if let Some(ref mut process) = self.process
            && let Err(e) = process.stop()
        {
            tracing::warn!(error = %e, "error stopping capture process");
        }
        self.process = None;
    }

    /// Check if the capture process is still running and restart if needed.
    ///
    /// Returns `true` if the process is running (or was successfully restarted).
    /// Returns `false` if the max restart count has been exceeded.
    pub fn check_and_restart(&mut self) -> bool {
        let is_running = self
            .process
            .as_mut()
            .is_some_and(CaptureProcess::is_running);

        if is_running {
            // Process is healthy — reset the restart counter so transient
            // failures (e.g. USB microphone momentary disconnect) don't
            // permanently exhaust the restart budget over a long deployment.
            if self.restart_count > 0 {
                tracing::debug!(
                    previous_restarts = self.restart_count,
                    "capture process stable, resetting restart counter"
                );
                self.restart_count = 0;
            }
            return true;
        }

        if self.process.is_none() {
            return false;
        }

        if self.restart_count >= self.max_restarts {
            tracing::error!(
                restarts = self.restart_count,
                max = self.max_restarts,
                "capture process exceeded max restarts"
            );
            return false;
        }

        self.restart_count += 1;
        tracing::warn!(
            restart = self.restart_count,
            "capture process died, restarting"
        );
        self.process = None;

        match self.start() {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "failed to restart capture");
                false
            }
        }
    }

    /// Whether the capture process is currently running.
    pub fn is_running(&mut self) -> bool {
        self.process
            .as_mut()
            .is_some_and(CaptureProcess::is_running)
    }

    /// Get the restart count.
    pub const fn restart_count(&self) -> u32 {
        self.restart_count
    }

    /// Get the recording configuration.
    pub const fn config(&self) -> &RecordingConfig {
        &self.config
    }
}

impl Drop for CaptureManager {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::types::{AudioFormat, CaptureSource};
    use std::path::PathBuf;

    fn microphone_config() -> RecordingConfig {
        RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48_000,
                channels: 1,
            },
            output_dir: PathBuf::from("/tmp/birdnet_test_manager"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
        }
    }

    #[test]
    fn new_manager_not_running() {
        let mut mgr = CaptureManager::new(microphone_config());
        assert!(!mgr.is_running());
        assert_eq!(mgr.restart_count(), 0);
    }

    #[test]
    fn start_fails_without_arecord() {
        if is_tool_available("arecord") {
            return; // skip — arecord is present, don't actually start recording
        }
        let mut mgr = CaptureManager::new(microphone_config());
        let result = mgr.start();
        assert!(result.is_err());
    }

    #[test]
    fn check_and_restart_returns_false_when_not_started() {
        let mut mgr = CaptureManager::new(microphone_config());
        // No process → returns false without crashing.
        assert!(!mgr.check_and_restart());
    }

    #[test]
    fn start_rtsp_graceful_fail() {
        if is_tool_available("ffmpeg") {
            return; // skip if ffmpeg is available
        }
        let config = RecordingConfig {
            source: CaptureSource::Rtsp {
                url: "rtsp://example.com/stream".into(),
                stream_id: "cam1".into(),
            },
            output_dir: std::env::temp_dir().join("birdnet_rtsp_test"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
        };
        let mut mgr = CaptureManager::new(config);
        assert!(mgr.start().is_err());
    }
}
