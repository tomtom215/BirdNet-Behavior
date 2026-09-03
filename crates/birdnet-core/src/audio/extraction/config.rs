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
    /// Positive raises the pitch, negative lowers it. For high-frequency
    /// hearing loss the useful direction is negative — see
    /// [`super::ACCESSIBILITY_SHIFT_HZ`], which records why, and what this
    /// comment used to claim.
    /// Implemented via ffmpeg `asetrate`+`aresample` filter or sox `pitch` effect.
    ///
    /// BirdNET-Pi equivalent: `FREQ_SHIFT` config option with sox/rubberband.
    pub freq_shift_hz: i32,
    /// Extra seconds of audio before the detection, on top of the symmetric
    /// lead-in that [`Self::extraction_length`] already implies.
    ///
    /// `0.0` — the default — keeps a clip centred on its detection, which is
    /// what it has always been. A positive value lengthens the clip at the
    /// front only: with a 6-second extraction and a 3-second detection window
    /// the lead-in is 1.5 s, and `pre_capture_secs = 1.0` makes it 2.5 s and
    /// the clip 7 seconds.
    ///
    /// Worth having because a call is not centred in the window that detects
    /// it. `BirdNET` scores a 3-second chunk, and a bird that started singing
    /// half a second before that chunk opened has its first notes in the
    /// *previous* one — which is audible to a person deciding whether the
    /// identification is right, and is exactly what an asymmetric lead-in
    /// recovers.
    ///
    /// This only reaches anything because clip windows span segment
    /// boundaries (`super::span`); before that, asking for more lead-in at the
    /// start of a segment produced the same clamped clip.
    pub pre_capture_secs: f32,
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
            pre_capture_secs: 0.0,
        }
    }
}
