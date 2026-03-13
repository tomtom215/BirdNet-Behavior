//! Audio capture configuration types.
//!
//! Defines `CaptureSource`, `RecordingConfig`, and `AudioFormat`.

use std::path::PathBuf;

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
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }
}

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

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// Generate a BirdNET-Pi compatible output filename pattern.
///
/// Format: `YYYY-MM-DD-birdnet-[RTSP_ID-]HH:MM:SS.ext`
pub(crate) fn recording_filename(rtsp_id: Option<&str>, format: AudioFormat) -> String {
    let ext = format.extension();
    rtsp_id.map_or_else(
        || format!("%Y-%m-%d-birdnet-%H:%M:%S.{ext}"),
        |id| format!("%Y-%m-%d-birdnet-{id}-%H:%M:%S.{ext}"),
    )
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
}
