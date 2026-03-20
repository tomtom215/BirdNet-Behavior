//! Configuration for audio clip extraction.

use std::path::PathBuf;

use super::AudioFormat;

/// Configuration for audio clip extraction.
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Total extraction length in seconds (default 6.0).
    pub extraction_length: f32,
    /// Base directory for extracted files (e.g., `~/BirdSongs/Extracted`).
    pub output_dir: PathBuf,
    /// Audio output format extension (e.g., "wav").
    pub audio_format: String,
    /// Target audio format for extraction output.
    pub target_format: AudioFormat,
    /// Recording segment length in seconds, used for `safe_stop` clamping.
    pub recording_length: f32,
    /// Frequency shift in Hz applied to extracted clips (0 = disabled).
    ///
    /// Shifts the audio pitch upward by the specified Hz, making high-frequency
    /// bird calls accessible to people with high-frequency hearing loss.
    /// Implemented via ffmpeg `asetrate`+`aresample` filter or sox `pitch` effect.
    ///
    /// BirdNET-Pi equivalent: `FREQ_SHIFT` config option with sox/rubberband.
    pub freq_shift_hz: i32,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            extraction_length: 6.0,
            output_dir: PathBuf::from("BirdSongs/Extracted"),
            audio_format: String::from("wav"),
            target_format: AudioFormat::Wav,
            recording_length: 15.0,
            freq_shift_hz: 0,
        }
    }
}
