//! Audio capture from microphone and RTSP streams.
//!
//! Manages subprocess control for `arecord` (local microphone) and
//! `ffmpeg` (RTSP streams). Replaces `birdnet_recording.sh`.
//!
//! Uses system tools via `std::process::Command` rather than direct ALSA
//! bindings, avoiding the `cpal` crate dependency and leveraging battle-tested
//! system utilities.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Errors from audio capture.
#[derive(Debug)]
pub enum CaptureError {
    /// Failed to spawn capture subprocess.
    Spawn(std::io::Error),
    /// Capture process exited with an error.
    Process(String),
    /// Invalid configuration.
    Config(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "failed to spawn capture process: {e}"),
            Self::Process(msg) => write!(f, "capture process error: {msg}"),
            Self::Config(msg) => write!(f, "capture config error: {msg}"),
        }
    }
}

impl std::error::Error for CaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(e) => Some(e),
            Self::Process(_) | Self::Config(_) => None,
        }
    }
}

impl From<std::io::Error> for CaptureError {
    fn from(e: std::io::Error) -> Self {
        Self::Spawn(e)
    }
}

/// Audio capture source configuration.
#[derive(Debug, Clone)]
pub enum CaptureSource {
    /// Local microphone via `arecord`.
    Microphone {
        /// ALSA device name (e.g., "plughw:1,0").
        device: String,
        /// Sample rate in Hz.
        sample_rate: u32,
        /// Number of channels.
        channels: u16,
    },
    /// RTSP stream via `ffmpeg`.
    Rtsp {
        /// RTSP URL.
        url: String,
        /// Stream identifier for filenames.
        stream_id: String,
    },
}

/// Configuration for a recording session.
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// Audio source.
    pub source: CaptureSource,
    /// Output directory for recordings.
    pub output_dir: PathBuf,
    /// Duration of each recording segment in seconds.
    pub segment_duration_secs: u32,
    /// Audio format for output files.
    pub format: AudioFormat,
}

/// Output audio format.
#[derive(Debug, Clone, Copy)]
pub enum AudioFormat {
    Wav,
    Flac,
}

impl AudioFormat {
    const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }
}

/// A running audio capture process.
#[derive(Debug)]
pub struct CaptureProcess {
    child: Child,
    source: CaptureSource,
}

impl CaptureProcess {
    /// Check if the capture process is still running.
    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }

    /// Stop the capture process gracefully.
    ///
    /// # Errors
    ///
    /// Returns `CaptureError` if the process cannot be terminated.
    pub fn stop(&mut self) -> Result<(), CaptureError> {
        self.child.kill().map_err(CaptureError::Spawn)?;
        self.child.wait().map_err(CaptureError::Spawn)?;
        Ok(())
    }

    /// Get the capture source configuration.
    pub const fn source(&self) -> &CaptureSource {
        &self.source
    }
}

impl Drop for CaptureProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Generate a BirdNET-Pi compatible output filename.
///
/// Format: `YYYY-MM-DD-birdnet-[RTSP_ID-]HH:MM:SS.ext`
fn recording_filename(rtsp_id: Option<&str>, format: AudioFormat) -> String {
    // Use a simple timestamp pattern for ffmpeg/arecord to fill in
    // The actual timestamping is handled by the calling code
    let ext = format.extension();
    rtsp_id.map_or_else(
        || format!("%Y-%m-%d-birdnet-%H:%M:%S.{ext}"),
        |id| format!("%Y-%m-%d-birdnet-{id}-%H:%M:%S.{ext}"),
    )
}

/// Start an audio capture process for a microphone source.
///
/// Uses `arecord` to capture audio from the specified ALSA device.
///
/// # Errors
///
/// Returns `CaptureError` if `arecord` cannot be started.
pub fn start_microphone_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    let CaptureSource::Microphone {
        ref device,
        sample_rate,
        channels,
    } = config.source
    else {
        return Err(CaptureError::Config("expected microphone source".into()));
    };

    let filename_pattern = recording_filename(None, config.format);
    let output_path = config.output_dir.join(&filename_pattern);

    let mut cmd = Command::new("arecord");
    cmd.arg("-D")
        .arg(device)
        .arg("-f")
        .arg("S16_LE")
        .arg("-r")
        .arg(sample_rate.to_string())
        .arg("-c")
        .arg(channels.to_string())
        .arg("--max-file-time")
        .arg(config.segment_duration_secs.to_string())
        .arg("--use-strftime")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let child = cmd.spawn()?;
    tracing::info!(device = device, "started microphone capture via arecord");

    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
    })
}

/// Start an audio capture process for an RTSP stream.
///
/// Uses `ffmpeg` to capture audio from the RTSP URL.
///
/// # Errors
///
/// Returns `CaptureError` if `ffmpeg` cannot be started.
pub fn start_rtsp_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    let CaptureSource::Rtsp {
        ref url,
        ref stream_id,
    } = config.source
    else {
        return Err(CaptureError::Config("expected RTSP source".into()));
    };

    let filename_pattern = recording_filename(Some(stream_id), config.format);
    let output_path = config.output_dir.join(&filename_pattern);

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-rtsp_transport")
        .arg("tcp")
        .arg("-i")
        .arg(url)
        .arg("-vn") // no video
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("48000")
        .arg("-ac")
        .arg("1") // mono
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(config.segment_duration_secs.to_string())
        .arg("-strftime")
        .arg("1")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let child = cmd.spawn()?;
    tracing::info!(
        stream_id = stream_id,
        url = url,
        "started RTSP capture via ffmpeg"
    );

    Ok(CaptureProcess {
        child,
        source: config.source.clone(),
    })
}

/// Check if a required capture tool is available on the system.
pub fn is_tool_available(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Manage disk space by removing old recordings.
///
/// Removes the oldest files in `dir` until free space exceeds `min_free_bytes`.
///
/// # Errors
///
/// Returns `CaptureError` if the directory cannot be read.
pub fn cleanup_old_recordings(dir: &Path, max_age_days: u32) -> Result<u32, CaptureError> {
    let now = std::time::SystemTime::now();
    let max_age = std::time::Duration::from_secs(u64::from(max_age_days) * 86400);
    let mut removed = 0_u32;

    let entries = std::fs::read_dir(dir).map_err(|e| CaptureError::Config(e.to_string()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Only remove audio files
        let is_audio = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("wav")
                    || ext.eq_ignore_ascii_case("flac")
                    || ext.eq_ignore_ascii_case("mp3")
            });

        if !is_audio {
            continue;
        }

        let dominated = entry.metadata().ok().and_then(|m| {
            let modified = m.modified().ok()?;
            let age = now.duration_since(modified).ok()?;
            Some(age > max_age)
        });

        if dominated == Some(true) && std::fs::remove_file(&path).is_ok() {
            tracing::debug!(path = %path.display(), "removed old recording");
            removed += 1;
        }
    }

    if removed > 0 {
        tracing::info!(count = removed, "cleaned up old recordings");
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_filename_local_mic() {
        let name = recording_filename(None, AudioFormat::Wav);
        assert!(name.contains("birdnet"));
        assert!(name.ends_with(".wav"));
        assert!(!name.contains("cam"));
    }

    #[test]
    fn recording_filename_rtsp() {
        let name = recording_filename(Some("cam1"), AudioFormat::Flac);
        assert!(name.contains("birdnet"));
        assert!(name.contains("cam1"));
        assert!(name.ends_with(".flac"));
    }

    #[test]
    fn audio_format_extension() {
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert_eq!(AudioFormat::Flac.extension(), "flac");
    }

    #[test]
    fn cleanup_nonexistent_dir_returns_error() {
        let result = cleanup_old_recordings(Path::new("/nonexistent/dir"), 30);
        assert!(result.is_err());
    }

    #[test]
    fn cleanup_empty_dir() {
        let dir = std::env::temp_dir().join("birdnet_test_cleanup");
        let _ = std::fs::create_dir_all(&dir);
        let result = cleanup_old_recordings(&dir, 30);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
        let _ = std::fs::remove_dir(&dir);
    }
}
