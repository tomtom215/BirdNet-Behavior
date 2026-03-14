//! Audio capture subprocess management.
//!
//! Wraps `arecord` (local microphone) and `ffmpeg` (RTSP) child processes.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use super::types::{CaptureError, CaptureSource, RecordingConfig, recording_filename};

/// A running audio capture process.
#[derive(Debug)]
pub struct CaptureProcess {
    pub(super) child: Child,
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

/// Check if a path has a supported audio extension (.wav / .flac / .mp3).
pub(crate) fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ext.eq_ignore_ascii_case("wav")
                || ext.eq_ignore_ascii_case("flac")
                || ext.eq_ignore_ascii_case("mp3")
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

/// Start an audio capture process for a microphone source via `arecord`.
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

/// Start an audio capture process for an RTSP stream via `ffmpeg`.
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

    let child = Command::new("ffmpeg")
        .arg("-rtsp_transport")
        .arg("tcp")
        .arg("-i")
        .arg(url)
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("48000")
        .arg("-ac")
        .arg("1")
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(config.segment_duration_secs.to_string())
        .arg("-strftime")
        .arg("1")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

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

/// Spawn the appropriate capture process based on the source type.
///
/// # Errors
///
/// Returns `CaptureError` if the process cannot be started.
pub fn spawn_capture(config: &RecordingConfig) -> Result<CaptureProcess, CaptureError> {
    match &config.source {
        CaptureSource::Microphone { .. } => start_microphone_capture(config),
        CaptureSource::Rtsp { .. } => start_rtsp_capture(config),
    }
}

/// Return the system tool name required for the given source.
pub fn required_tool(source: &CaptureSource) -> &'static str {
    match source {
        CaptureSource::Microphone { .. } => "arecord",
        CaptureSource::Rtsp { .. } => "ffmpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::AudioFormat;
    use super::*;

    #[test]
    fn is_audio_file_wav() {
        assert!(is_audio_file(Path::new("test.wav")));
        assert!(is_audio_file(Path::new("test.WAV")));
    }

    #[test]
    fn is_audio_file_flac() {
        assert!(is_audio_file(Path::new("test.flac")));
    }

    #[test]
    fn is_audio_file_mp3() {
        assert!(is_audio_file(Path::new("test.mp3")));
    }

    #[test]
    fn is_audio_file_rejects_txt() {
        assert!(!is_audio_file(Path::new("BirdDB.txt")));
    }

    #[test]
    fn required_tool_microphone() {
        let src = CaptureSource::Microphone {
            device: "plughw:1,0".into(),
            sample_rate: 48_000,
            channels: 1,
        };
        assert_eq!(required_tool(&src), "arecord");
    }

    #[test]
    fn required_tool_rtsp() {
        let src = CaptureSource::Rtsp {
            url: "rtsp://cam.local/stream".into(),
            stream_id: "cam1".into(),
        };
        assert_eq!(required_tool(&src), "ffmpeg");
    }

    #[test]
    fn start_microphone_missing_tool() {
        // This test verifies graceful failure when arecord is absent.
        // On CI arecord may not exist — that's fine.
        if is_tool_available("arecord") {
            return; // skip if arecord is present (would actually start recording)
        }
        use std::path::PathBuf;
        let config = RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48_000,
                channels: 1,
            },
            output_dir: PathBuf::from("/tmp"),
            segment_duration_secs: 15,
            format: AudioFormat::Wav,
        };
        assert!(start_microphone_capture(&config).is_err());
    }
}
