//! Audio capture configuration types.
//!
//! Defines `CaptureSource`, `RecordingConfig`, and `AudioFormat`.

use std::path::PathBuf;

/// Audio capture source configuration.
#[derive(Debug, Clone)]
pub enum CaptureSource {
    /// Local microphone via `arecord` (ALSA).
    Microphone {
        /// ALSA device name (e.g., "plughw:1,0").
        device: String,
        /// Sample rate in Hz.
        sample_rate: u32,
        /// Number of channels.
        channels: u16,
        /// Stream identifier for filenames and metrics when more than one
        /// local microphone is configured (e.g. `MIC_1`, `MIC_2`). `None` for
        /// a lone local mic, which keeps the historical id-less filename and
        /// the `local` metrics label.
        stream_id: Option<String>,
    },
    /// `PipeWire` or `PulseAudio` microphone via `ffmpeg -f pulse`.
    ///
    /// Works with both native `PulseAudio` and `PipeWire` (via `pipewire-pulse` compatibility layer).
    /// Use an empty string for the system default device.
    PipeWire {
        /// PulseAudio/PipeWire source name (empty = system default).
        device: String,
        /// Sample rate in Hz.
        sample_rate: u32,
        /// Number of channels.
        channels: u16,
        /// Stream identifier for filenames and metrics when more than one
        /// local microphone is configured. `None` for a lone local mic.
        stream_id: Option<String>,
    },
    /// RTSP stream via `ffmpeg`.
    Rtsp {
        /// RTSP URL.
        url: String,
        /// Stream identifier for filenames.
        stream_id: String,
        /// Transport ffmpeg should negotiate with the camera.
        transport: RtspTransport,
    },
}

/// RTSP transport preference passed to ffmpeg's `-rtsp_transport`.
///
/// Mirrors the `rtsp_transport` column the admin UI exposes per audio source.
/// Before this was threaded through, the transport was hard-coded to TCP and
/// the UI control had no effect — a camera that only speaks UDP could never be
/// captured. `Auto` resolves to TCP (the most NAT-/firewall-robust choice, and
/// the behaviour every prior release shipped); `Tcp` / `Udp` force the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RtspTransport {
    /// Let the station pick the robust default (TCP).
    #[default]
    Auto,
    /// Force RTP-over-RTSP (TCP interleaved).
    Tcp,
    /// Force RTP-over-UDP.
    Udp,
}

impl RtspTransport {
    /// The value to pass to ffmpeg's `-rtsp_transport`. ffmpeg has no literal
    /// "auto", so `Auto` resolves to the TCP default here.
    #[must_use]
    pub const fn ffmpeg_arg(self) -> &'static str {
        match self {
            Self::Auto | Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
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
    /// Software capture gain in decibels, applied via ffmpeg's `volume` audio
    /// filter. `0.0` is unity gain (no boost/cut) and keeps local-microphone
    /// capture on the lighter-weight `arecord` path; a non-zero value routes
    /// microphone capture through ffmpeg so the gain can actually be applied.
    /// Mirrors the per-source `gain_db` the admin UI exposes for each source.
    pub gain_db: f32,
}

/// Output audio format.
#[derive(Debug, Clone, Copy)]
pub enum AudioFormat {
    /// Uncompressed PCM WAV (`.wav`).
    Wav,
    /// Losslessly compressed FLAC (`.flac`).
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

    #[test]
    fn rtsp_transport_ffmpeg_arg() {
        // Auto resolves to TCP (ffmpeg has no literal "auto"); explicit choices
        // pass through. This is what makes the per-source UDP/TCP control work.
        assert_eq!(RtspTransport::Auto.ffmpeg_arg(), "tcp");
        assert_eq!(RtspTransport::Tcp.ffmpeg_arg(), "tcp");
        assert_eq!(RtspTransport::Udp.ffmpeg_arg(), "udp");
        // The default is Auto.
        assert_eq!(RtspTransport::default(), RtspTransport::Auto);
    }
}
