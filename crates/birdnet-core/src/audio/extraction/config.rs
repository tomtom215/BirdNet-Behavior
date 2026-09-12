//! Configuration for audio clip extraction.

use std::path::PathBuf;

use super::AudioFormat;

/// Default sample-peak ceiling for normalised clips, in dBFS.
///
/// −1 dBFS rather than 0: the clip is written as 16-bit integers, so a sample
/// at exactly full scale has nowhere to round to, and inter-sample peaks in
/// ordinary material run up to about a decibel above the sample peak this
/// measures. A decibel of headroom covers both without audibly costing level.
pub const DEFAULT_PEAK_CEILING_DBFS: f64 = -1.0;

/// The loudness an operator gets by asking for normalisation without saying
/// where, in LUFS.
///
/// EBU R128's −23 LUFS is the broadcast reference, and it is too quiet here:
/// these clips are listened to on a phone, in a browser tab, next to whatever
/// else is playing, and −23 sends the listener back to the volume control this
/// feature exists to spare them. −18 is the streaming-era convention and is
/// what a station gets by default.
pub const DEFAULT_TARGET_LUFS: f64 = -18.0;

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
    /// Loudness to normalise exported clips to, in LUFS, or `None` to write
    /// them at capture level.
    ///
    /// Off by default, because a clip is an archival record as well as
    /// something to listen to and changing what is in it should be the
    /// operator's decision. Turned on, it fixes the thing that makes a gallery
    /// of clips unusable: without it the listener rides the volume control
    /// between every one, and a quiet clip at the end of a playlist is missed.
    ///
    /// Applied to the **export only**. The samples the mel spectrogram and the
    /// classifier see are untouched — a per-clip gain there would move every
    /// confidence score.
    pub target_lufs: Option<f64>,
    /// Sample peak, in dBFS, that normalisation may not push a sample past.
    ///
    /// A **sample** peak, not an ITU true peak; see
    /// [`super::loudness`]. Negative.
    pub peak_ceiling_dbfs: f64,
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
            target_lufs: None,
            peak_ceiling_dbfs: DEFAULT_PEAK_CEILING_DBFS,
        }
    }
}
