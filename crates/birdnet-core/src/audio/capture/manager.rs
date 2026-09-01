//! `CaptureManager`: lifecycle control for a single audio-capture process.
//!
//! Starts and stops the appropriate subprocess (`arecord` / `ffmpeg`) and
//! reports whether it is still alive. Deciding *when* to (re)start it —
//! restart-on-death with backoff, schedule-driven pause/resume — is the job
//! of the capture supervisor in the binary crate, which polls
//! [`CaptureManager::is_running`] and drives [`CaptureManager::start`] /
//! [`CaptureManager::stop`] accordingly.

use std::time::Duration;

use super::process::{CaptureProcess, is_tool_available, required_tool, spawn_capture};
use super::types::{CaptureError, CaptureSource, RecordingConfig, filename_matches_stream};

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

    /// Age of the newest segment file this source has written into its
    /// output directory, or `None` when no matching file is visible.
    ///
    /// The silent-stall signal for the capture supervisor: a subprocess that
    /// is alive but delivering no audio (wedged RTSP session, hung
    /// `arecord` after a USB re-enumeration) writes no new segments, which
    /// `is_running` alone can never reveal. Errors (missing directory,
    /// unreadable entries) degrade to `None` — the supervisor treats that as
    /// "no evidence", never as proof of a stall.
    ///
    /// A modification time in the future (clock stepped backwards after the
    /// file was written) clamps to age zero, which reads as "fresh" — the
    /// fail-open choice, consistent with how the schedule treats clock skew.
    pub fn latest_output_age(&self) -> Option<Duration> {
        let stream_id: Option<&str> = match &self.config.source {
            CaptureSource::Rtsp { stream_id, .. } => Some(stream_id),
            CaptureSource::Microphone { stream_id, .. }
            | CaptureSource::PipeWire { stream_id, .. } => stream_id.as_deref(),
        };
        let ext = self.config.format.extension();

        let entries = std::fs::read_dir(&self.config.output_dir).ok()?;
        let mut newest: Option<std::time::SystemTime> = None;
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !filename_matches_stream(name, stream_id, ext) {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            newest = Some(newest.map_or(modified, |n| n.max(modified)));
        }
        newest.map(|m| {
            std::time::SystemTime::now()
                .duration_since(m)
                .unwrap_or(Duration::ZERO)
        })
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
    use crate::audio::capture::types::{AudioFormat, CaptureSource, LocalOffset, RtspTransport};
    use std::path::PathBuf;

    fn microphone_config() -> RecordingConfig {
        RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48_000,
                channels: 1,
                channel_pick: None,
                stream_id: None,
            },
            output_dir: PathBuf::from("/tmp/birdnet_test_manager"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
            gain_db: 0.0,
            pipeline: crate::audio::capture::types::AudioPipeline::none(),
            local_offset: LocalOffset::utc(),
            live_audio: None,
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
                transport: RtspTransport::Auto,
            },
            output_dir: std::env::temp_dir().join("birdnet_rtsp_test"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
            gain_db: 0.0,
            pipeline: crate::audio::capture::types::AudioPipeline::none(),
            local_offset: LocalOffset::utc(),
            live_audio: None,
        };
        let mut mgr = CaptureManager::new(config);
        assert!(mgr.start().is_err());
    }

    /// The output-age probe must see only THIS source's segments: the newest
    /// matching file wins, other streams' files and non-recordings are
    /// invisible, and an empty/missing directory yields `None`.
    #[test]
    fn latest_output_age_scopes_to_own_stream() {
        let dir = tempfile::tempdir().unwrap();
        let config = RecordingConfig {
            source: CaptureSource::Rtsp {
                url: "rtsp://example.com/stream".into(),
                stream_id: "RTSP_1".into(),
                transport: RtspTransport::Auto,
            },
            output_dir: dir.path().to_path_buf(),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
            gain_db: 0.0,
            pipeline: crate::audio::capture::types::AudioPipeline::none(),
            local_offset: LocalOffset::utc(),
            live_audio: None,
        };
        let mgr = CaptureManager::new(config);

        // Empty directory: no evidence either way.
        assert_eq!(mgr.latest_output_age(), None);

        // Another stream's segment and a stray file are invisible to RTSP_1.
        std::fs::write(
            dir.path().join("2026-06-10-birdnet-RTSP_2-07:00:00.wav"),
            b"x",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        assert_eq!(mgr.latest_output_age(), None);

        // Its own segment is found, and the age is the file's freshness.
        std::fs::write(
            dir.path().join("2026-06-10-birdnet-RTSP_1-07:00:15.wav"),
            b"x",
        )
        .unwrap();
        let age = mgr.latest_output_age().expect("own segment visible");
        assert!(
            age < Duration::from_secs(30),
            "fresh file reads as fresh: {age:?}"
        );
    }

    /// A missing output directory degrades to `None`, never an error/panic.
    #[test]
    fn latest_output_age_tolerates_missing_dir() {
        let mut config = microphone_config();
        config.output_dir = PathBuf::from("/nonexistent/birdnet-stall-probe");
        let mgr = CaptureManager::new(config);
        assert_eq!(mgr.latest_output_age(), None);
    }
}
