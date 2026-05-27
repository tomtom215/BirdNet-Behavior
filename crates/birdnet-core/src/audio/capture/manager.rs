//! `CaptureManager`: lifecycle control for a single audio-capture process.
//!
//! Starts and stops the appropriate subprocess (`arecord` / `ffmpeg`) and
//! reports whether it is still alive. Deciding *when* to (re)start it —
//! restart-on-death with backoff, schedule-driven pause/resume — is the job
//! of the capture supervisor in the binary crate, which polls
//! [`CaptureManager::is_running`] and drives [`CaptureManager::start`] /
//! [`CaptureManager::stop`] accordingly.

use super::process::{CaptureProcess, is_tool_available, required_tool, spawn_capture};
use super::types::{CaptureError, RecordingConfig};

/// Manages the lifecycle of an audio capture process.
///
/// A thin, restart-policy-free wrapper: call [`start`](Self::start) to begin
/// recording, [`stop`](Self::stop) to end it, and [`is_running`](Self::is_running)
/// to check liveness. The supervisor that keeps the process alive over a long
/// deployment lives in `crate::capture::supervisor` (binary crate).
#[derive(Debug)]
pub struct CaptureManager {
    config: RecordingConfig,
    process: Option<CaptureProcess>,
}

impl CaptureManager {
    /// Create a new capture manager.
    ///
    /// Does not start capture — call [`start`](Self::start) to begin recording.
    pub const fn new(config: RecordingConfig) -> Self {
        Self {
            config,
            process: None,
        }
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

    /// Whether the capture process is currently running.
    pub fn is_running(&mut self) -> bool {
        self.process
            .as_mut()
            .is_some_and(CaptureProcess::is_running)
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
                stream_id: None,
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
