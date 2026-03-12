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

/// Disk space information for a filesystem.
#[derive(Debug, Clone, Copy)]
pub struct DiskUsage {
    /// Total space in bytes.
    pub total_bytes: u64,
    /// Used space in bytes.
    pub used_bytes: u64,
    /// Available space in bytes.
    pub available_bytes: u64,
}

impl DiskUsage {
    /// Percentage of disk used (0.0 - 100.0).
    #[allow(clippy::cast_precision_loss)]
    pub fn used_percent(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.used_bytes as f64 / self.total_bytes as f64 * 100.0
    }

    /// Whether the disk is critically low (less than 5% available).
    pub const fn is_critical(&self) -> bool {
        self.available_bytes < self.total_bytes / 20
    }

    /// Whether the disk is getting low (less than 10% available).
    pub const fn is_low(&self) -> bool {
        self.available_bytes < self.total_bytes / 10
    }
}

/// Get disk usage information for the filesystem containing `path`.
///
/// Uses the `df` command to query filesystem statistics, avoiding
/// `unsafe` code and platform-specific `libc` bindings.
///
/// # Errors
///
/// Returns `CaptureError` if `df` is not available or the path doesn't exist.
pub fn disk_usage(path: &Path) -> Result<DiskUsage, CaptureError> {
    let output = Command::new("df")
        .arg("--output=size,used,avail")
        .arg("-B1") // bytes
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(CaptureError::Spawn)?;

    if !output.status.success() {
        return Err(CaptureError::Config(format!(
            "df failed for {}",
            path.display()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data_line = stdout
        .lines()
        .nth(1)
        .ok_or_else(|| CaptureError::Config("unexpected df output".into()))?;

    let values: Vec<u64> = data_line
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();

    if values.len() < 3 {
        return Err(CaptureError::Config("unexpected df output format".into()));
    }

    Ok(DiskUsage {
        total_bytes: values[0],
        used_bytes: values[1],
        available_bytes: values[2],
    })
}

/// Count audio files in a directory and their total size.
///
/// Returns `(file_count, total_size_bytes)`.
///
/// # Errors
///
/// Returns `CaptureError` if the directory cannot be read.
pub fn recording_stats(dir: &Path) -> Result<(u32, u64), CaptureError> {
    let entries = std::fs::read_dir(dir).map_err(|e| CaptureError::Config(e.to_string()))?;

    let mut count = 0_u32;
    let mut total_size = 0_u64;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let is_audio = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("wav")
                    || ext.eq_ignore_ascii_case("flac")
                    || ext.eq_ignore_ascii_case("mp3")
            });

        if is_audio {
            count += 1;
            total_size += entry.metadata().map_or(0, |m| m.len());
        }
    }

    Ok((count, total_size))
}

/// Manages the lifecycle of an audio capture process.
///
/// Starts the appropriate capture subprocess (arecord or ffmpeg),
/// monitors it, and restarts it if it crashes. Includes a restart
/// backoff to avoid tight restart loops.
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
    /// Does not start capture -- call `start()` to begin recording.
    pub const fn new(config: RecordingConfig) -> Self {
        Self {
            config,
            process: None,
            restart_count: 0,
            max_restarts: 10,
        }
    }

    /// Start the capture process.
    ///
    /// # Errors
    ///
    /// Returns `CaptureError` if the process cannot be started or the
    /// required tool (arecord/ffmpeg) is not available.
    pub fn start(&mut self) -> Result<(), CaptureError> {
        // Ensure output directory exists
        std::fs::create_dir_all(&self.config.output_dir).map_err(CaptureError::Spawn)?;

        let tool = match &self.config.source {
            CaptureSource::Microphone { .. } => "arecord",
            CaptureSource::Rtsp { .. } => "ffmpeg",
        };

        if !is_tool_available(tool) {
            return Err(CaptureError::Config(format!("{tool} not found in PATH")));
        }

        let process = match &self.config.source {
            CaptureSource::Microphone { .. } => start_microphone_capture(&self.config)?,
            CaptureSource::Rtsp { .. } => start_rtsp_capture(&self.config)?,
        };

        self.process = Some(process);
        self.restart_count = 0;
        tracing::info!("capture started");

        Ok(())
    }

    /// Stop the capture process.
    pub fn stop(&mut self) {
        if let Some(ref mut process) = self.process {
            if let Err(e) = process.stop() {
                tracing::warn!(error = %e, "error stopping capture process");
            }
        }
        self.process = None;
    }

    /// Check if the capture process is still running and restart if needed.
    ///
    /// Returns `true` if the process is running (or was successfully restarted).
    /// Returns `false` if the maximum restart count has been exceeded.
    pub fn check_and_restart(&mut self) -> bool {
        let is_running = self
            .process
            .as_mut()
            .is_some_and(CaptureProcess::is_running);

        if is_running {
            return true;
        }

        if self.process.is_none() {
            return false;
        }

        // Process died -- attempt restart
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

        // Drop the dead process
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

    #[test]
    fn recording_filename_local_mic() {
        let name = recording_filename(None, AudioFormat::Wav);
        assert!(name.contains("birdnet"));
        assert!(
            std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        );
        assert!(!name.contains("cam"));
    }

    #[test]
    fn recording_filename_rtsp() {
        let name = recording_filename(Some("cam1"), AudioFormat::Flac);
        assert!(name.contains("birdnet"));
        assert!(name.contains("cam1"));
        assert!(
            std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("flac"))
        );
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

    #[test]
    fn disk_usage_percent() {
        let usage = DiskUsage {
            total_bytes: 1_000_000,
            used_bytes: 750_000,
            available_bytes: 250_000,
        };
        assert!((usage.used_percent() - 75.0).abs() < 0.01);
        assert!(!usage.is_critical());
        assert!(!usage.is_low());
    }

    #[test]
    fn disk_usage_critical() {
        let usage = DiskUsage {
            total_bytes: 1_000_000,
            used_bytes: 960_000,
            available_bytes: 40_000,
        };
        assert!(usage.is_critical());
        assert!(usage.is_low());
    }

    #[test]
    fn disk_usage_low() {
        let usage = DiskUsage {
            total_bytes: 1_000_000,
            used_bytes: 920_000,
            available_bytes: 80_000,
        };
        assert!(!usage.is_critical());
        assert!(usage.is_low());
    }

    #[test]
    fn disk_usage_empty_total() {
        let usage = DiskUsage {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
        };
        assert!((usage.used_percent()).abs() < 0.01);
    }

    #[test]
    fn disk_usage_from_df() {
        // Test actual disk usage query for the temp directory
        let result = disk_usage(Path::new("/tmp"));
        assert!(result.is_ok());
        let usage = result.unwrap();
        assert!(usage.total_bytes > 0);
        assert!(usage.available_bytes <= usage.total_bytes);
    }

    #[test]
    fn recording_stats_empty_dir() {
        let dir = std::env::temp_dir().join("birdnet_test_recording_stats");
        let _ = std::fs::create_dir_all(&dir);
        let result = recording_stats(&dir);
        assert!(result.is_ok());
        let (count, size) = result.unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn recording_stats_nonexistent_dir() {
        let result = recording_stats(Path::new("/nonexistent/dir"));
        assert!(result.is_err());
    }

    #[test]
    fn capture_manager_new() {
        let config = RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48000,
                channels: 1,
            },
            output_dir: PathBuf::from("/tmp/StreamData"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
        };

        let manager = CaptureManager::new(config);
        assert_eq!(manager.restart_count(), 0);
    }

    #[test]
    fn capture_manager_start_missing_tool() {
        let config = RecordingConfig {
            source: CaptureSource::Rtsp {
                url: "rtsp://example.com/stream".into(),
                stream_id: "cam1".into(),
            },
            output_dir: std::env::temp_dir().join("birdnet_test_capture_mgr"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
        };

        let mut manager = CaptureManager::new(config);
        // This test depends on ffmpeg availability -- it should fail gracefully
        // if ffmpeg is missing or succeed if it is available
        let result = manager.start();
        if !is_tool_available("ffmpeg") {
            assert!(result.is_err());
        }
    }

    #[test]
    fn capture_manager_not_running_initially() {
        let config = RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48000,
                channels: 1,
            },
            output_dir: PathBuf::from("/tmp/StreamData"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
        };

        let mut manager = CaptureManager::new(config);
        assert!(!manager.is_running());
    }
}
