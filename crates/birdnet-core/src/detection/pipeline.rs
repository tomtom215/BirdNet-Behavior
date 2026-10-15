//! Detection pipeline: watch → decode → spectrogram → infer → report.
//!
//! Watches a directory for new audio files via `notify`, then processes each
//! through the audio pipeline (decode → resample → mel spectrogram) and
//! prepares it for inference.
//!
//! The inference step itself is pluggable -- the pipeline produces mel
//! spectrograms and accepts results back for reporting.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::audio::capture::is_audio_file;
use crate::audio::decode;
use crate::audio::resample;
use crate::audio::spectrogram::{self, MelConfig, MelSpectrogram};
use crate::detection::types::RecordingFile;

/// Errors from the detection pipeline.
#[derive(Debug)]
pub enum PipelineError {
    /// File watcher failed to start or encountered an error.
    Watch(String),
    /// Audio decoding failed.
    Decode(decode::DecodeError),
    /// Resampling failed.
    Resample(resample::ResampleError),
    /// Spectrogram computation failed.
    Spectrogram(spectrogram::SpectrogramError),
    /// Channel communication error.
    Channel(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watch(msg) => write!(f, "file watch error: {msg}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Resample(e) => write!(f, "resample error: {e}"),
            Self::Spectrogram(e) => write!(f, "spectrogram error: {e}"),
            Self::Channel(msg) => write!(f, "channel error: {msg}"),
        }
    }
}

impl std::error::Error for PipelineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Resample(e) => Some(e),
            Self::Spectrogram(e) => Some(e),
            Self::Watch(_) | Self::Channel(_) => None,
        }
    }
}

impl From<decode::DecodeError> for PipelineError {
    fn from(e: decode::DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl From<resample::ResampleError> for PipelineError {
    fn from(e: resample::ResampleError) -> Self {
        Self::Resample(e)
    }
}

impl From<spectrogram::SpectrogramError> for PipelineError {
    fn from(e: spectrogram::SpectrogramError) -> Self {
        Self::Spectrogram(e)
    }
}

/// Configuration for the detection pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Directory to watch for new audio files.
    pub watch_dir: PathBuf,
    /// Target sample rate for the ML model.
    pub target_sample_rate: u32,
    /// Mel spectrogram configuration (only used when `raw_audio_input` is false).
    pub mel_config: MelConfig,
    /// Duration of each audio chunk in seconds.
    pub chunk_duration_secs: f32,
    /// Overlap between chunks in seconds.
    pub chunk_overlap_secs: f32,
    /// How far apart consecutive chunks start, in seconds, when that is not
    /// simply `chunk_duration_secs - chunk_overlap_secs` (`G-10` Stage 4).
    ///
    /// # Why the step has to be separable from the length
    ///
    /// With more than one classifier the chunk is cut to the **longest**
    /// window any of them wants, and each takes its own window from the chunk
    /// start — `build_input_tensor` copies `min(len, expected)`, so a
    /// shorter-window model reads a prefix.
    ///
    /// If the step were then derived from that longest window, every model
    /// with a shorter one would acquire a blind spot. BirdNET+ V3.0 wants
    /// 4.5 s where Perch v2 wants 5.0 s: cut at 5.0 s and step 5.0 s, and the
    /// half-second tail of every chunk is audio BirdNET never sees — a gap it
    /// would not have had running alone, and one nothing would report.
    ///
    /// So the step is the **shortest** window instead. The shortest-window
    /// model then behaves exactly as it does alone, every longer-window model
    /// gets overlapping chunks rather than gaps, and no classifier sees less
    /// than it would by itself. The cost is that those longer-window models
    /// run `max(window) / min(window)` more inferences — about 11 % for Perch
    /// beside BirdNET — which the daemon logs at startup because on a Pi that
    /// is a real number.
    ///
    /// `None` keeps `chunk_duration_secs - chunk_overlap_secs`, which is what
    /// a single-classifier station has always done.
    ///
    /// # What a detection's end time then means
    ///
    /// A detection is stamped with the **chunk's** span, not the span its own
    /// classifier heard, so a shorter-window model's detection can carry an
    /// `end_secs` up to `max(window) - min(window)` later than the audio it
    /// actually read — half a second for BirdNET beside Perch. That is
    /// deliberate: the chunk is the unit two classifiers are judged to agree
    /// on, and stamping per-model spans would give the same merged species two
    /// different end times depending on which classifier won the confidence.
    /// The recorded span always *contains* the audio heard, so a clip cut from
    /// it contains the detection. With one classifier the two are identical.
    pub chunk_step_secs: Option<f32>,
    /// Minimum confidence threshold for reporting.
    pub confidence_threshold: f32,
    /// Feed raw audio samples directly to the model instead of a mel spectrogram.
    ///
    /// Set automatically by the daemon when a V3.0-style model is detected
    /// (input shape `[1, 96_000]`). V2.4 models expect a mel spectrogram
    /// (`[1, 144_000]` = 128 mel bands × 1125 frames); V3.0 models perform
    /// their own internal feature extraction from the raw waveform.
    pub raw_audio_input: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            watch_dir: PathBuf::from("/tmp/StreamData"),
            target_sample_rate: 48000,
            mel_config: MelConfig::default(),
            chunk_duration_secs: 3.0,
            chunk_overlap_secs: 0.0,
            chunk_step_secs: None,
            confidence_threshold: 0.25,
            raw_audio_input: false,
        }
    }
}

/// A prepared audio chunk ready for ML inference.
#[derive(Debug, Clone)]
pub struct PreparedChunk {
    /// Mel spectrogram for this chunk.
    pub spectrogram: MelSpectrogram,
    /// Start time of this chunk within the recording (seconds).
    pub start_secs: f32,
    /// End time of this chunk within the recording (seconds).
    pub end_secs: f32,
    /// Source recording file metadata.
    pub recording: RecordingFile,
}

/// The shortest trailing remainder of a recording that still gets a chunk of
/// its own, in seconds.
///
/// BirdNET-Pi's `splitSignal` stops at the first window shorter than
/// `minlen=1.5` and zero-pads any longer one, whatever the window length
/// (`scripts/utils/analysis.py`). This project used a quarter of the window
/// (0.75 s at 3 s) until that was measured against it: a 16 s recording gave
/// a sixth chunk here (1 s of audio padded with 2 s of silence) and none
/// there, so the same file could store a detection BirdNET-Pi never made.
const MIN_TAIL_SECS: f32 = 1.5;

/// [`MIN_TAIL_SECS`] in samples, never more than half the window, so a model
/// with a window shorter than 3 s still keeps the tail it is half-way through.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn min_tail_samples(chunk_samples: usize, sample_rate: u32) -> usize {
    ((MIN_TAIL_SECS * sample_rate as f32) as usize).min(chunk_samples / 2)
}

/// Where each chunk of a `len`-sample recording starts.
///
/// The first chunk always exists, however short the recording (it is padded
/// by the caller). After each chunk the next starts `step` samples on, and
/// the walk stops when that leaves less than `min_tail` samples, or none.
fn chunk_starts(len: usize, step: usize, min_tail: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut pos = 0;
    while pos < len {
        starts.push(pos);
        pos += step;
        if pos >= len || len - pos < min_tail {
            break;
        }
    }
    starts
}

/// Process a single audio file through the pipeline.
///
/// Decodes, resamples, splits into chunks, and computes mel spectrograms.
/// Returns prepared chunks ready for inference.
///
/// # Errors
///
/// Returns `PipelineError` if any stage of the pipeline fails.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn process_file(
    path: &Path,
    config: &PipelineConfig,
) -> Result<Vec<PreparedChunk>, PipelineError> {
    let recording = RecordingFile::parse(&path.to_string_lossy()).ok_or_else(|| {
        PipelineError::Watch(format!(
            "cannot parse recording filename: {}",
            path.display()
        ))
    })?;

    // Decode audio
    let audio = decode::decode_file(path)?;

    // Resample to target rate
    let samples = resample::resample(&audio.samples, audio.sample_rate, config.target_sample_rate)?;

    // Split into chunks
    let chunk_samples = (config.chunk_duration_secs * config.target_sample_rate as f32) as usize;
    let step = chunk_step_samples(config);

    let min_tail = min_tail_samples(chunk_samples, config.target_sample_rate);
    let mut chunks = Vec::new();

    for pos in chunk_starts(samples.len(), step, min_tail) {
        let end = (pos + chunk_samples).min(samples.len());
        let mut chunk_data = samples[pos..end].to_vec();

        // Pad short chunks with zeros (matching Python behavior)
        if chunk_data.len() < chunk_samples {
            chunk_data.resize(chunk_samples, 0.0);
        }

        let start_secs = pos as f32 / config.target_sample_rate as f32;
        let end_secs = end as f32 / config.target_sample_rate as f32;

        // V3.0 models (raw_audio_input=true) take the raw waveform directly.
        // Store as a "1 × N" MelSpectrogram so the rest of the pipeline is unchanged.
        // V2.4 models compute a proper mel spectrogram.
        let spectrogram = if config.raw_audio_input {
            let n = chunk_data.len();
            MelSpectrogram {
                n_mels: 1,
                n_frames: n,
                data: chunk_data,
            }
        } else {
            spectrogram::mel_spectrogram(
                &chunk_data,
                config.target_sample_rate,
                &config.mel_config,
            )?
        };

        chunks.push(PreparedChunk {
            spectrogram,
            start_secs,
            end_secs,
            recording: recording.clone(),
        });
    }

    Ok(chunks)
}

/// How far apart consecutive chunks start, in samples.
///
/// [`PipelineConfig::chunk_step_secs`] when it is set — see there for why the
/// step is the shortest classifier window rather than the chunk length — and
/// otherwise the single-classifier rule this has always used.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn chunk_step_samples(config: &PipelineConfig) -> usize {
    let rate = config.target_sample_rate as f32;
    if let Some(step_secs) = config.chunk_step_secs
        && step_secs > 0.0
    {
        return ((step_secs * rate) as usize).max(1);
    }
    let chunk_samples = (config.chunk_duration_secs * rate) as usize;
    let overlap_samples = (config.chunk_overlap_secs * rate) as usize;
    chunk_samples.saturating_sub(overlap_samples).max(1)
}

/// Create a file watcher for a directory.
///
/// Returns a receiver that yields paths to newly created/modified audio files.
/// The watcher must be kept alive (not dropped) for events to continue.
///
/// # Errors
///
/// Returns `PipelineError::Watch` if the watcher cannot be created.
pub fn watch_directory(
    dir: &Path,
) -> Result<(RecommendedWatcher, mpsc::Receiver<PathBuf>), PipelineError> {
    let (tx, rx) = mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            if let Ok(event) = result
                && matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
            {
                for path in event.paths {
                    if is_audio_file(&path) {
                        let _ = tx.send(path);
                    }
                }
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| PipelineError::Watch(e.to_string()))?;

    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .map_err(|e| PipelineError::Watch(e.to_string()))?;

    Ok((watcher, rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BirdNET-Pi's `splitSignal` (`scripts/utils/analysis.py`), transcribed:
    /// the start offsets of the windows it keeps.
    fn birdnet_pi_split_starts(
        len: usize,
        rate: usize,
        overlap: f64,
        seconds: f64,
        minlen: f64,
    ) -> Vec<usize> {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_sign_loss
        )]
        let (step, window, min) = (
            ((seconds - overlap) * rate as f64) as usize,
            (seconds * rate as f64) as usize,
            (minlen * rate as f64) as usize,
        );
        let mut starts = Vec::new();
        let mut i = 0;
        while i < len {
            if (len - i).min(window) < min {
                break;
            }
            starts.push(i);
            i += step;
        }
        starts
    }

    /// Our chunk starts for the same recording, through the code
    /// `process_file` runs (config -> step -> minimum tail -> starts).
    fn our_chunk_starts(len: usize, overlap: f32, seconds: f32) -> Vec<usize> {
        let config = PipelineConfig {
            chunk_duration_secs: seconds,
            chunk_overlap_secs: overlap,
            ..PipelineConfig::default()
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_sign_loss
        )]
        let chunk = (seconds * config.target_sample_rate as f32) as usize;
        chunk_starts(
            len,
            chunk_step_samples(&config),
            min_tail_samples(chunk, config.target_sample_rate),
        )
    }

    /// Differential gate against BirdNET-Pi's splitting: every recording
    /// length from 1.5 s to 40 s in quarter-second steps, at every overlap the
    /// settings page offers a representative of. A recording shorter than
    /// `minlen` is the one case that differs on purpose (we always keep one
    /// chunk), so it starts at `minlen`.
    #[test]
    fn chunk_starts_match_birdnet_pis_split_signal() {
        const RATE: usize = 48_000;
        let mut checked = 0;
        for overlap in [0.0_f32, 0.5, 1.0, 1.5, 2.0, 2.5] {
            for quarter_secs in 6..=160 {
                let len = quarter_secs * RATE / 4;
                let theirs = birdnet_pi_split_starts(len, RATE, f64::from(overlap), 3.0, 1.5);
                let ours = our_chunk_starts(len, overlap, 3.0);
                assert_eq!(
                    ours,
                    theirs,
                    "{} s at overlap {overlap}",
                    f64::from(u32::try_from(quarter_secs).unwrap()) / 4.0
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 6 * 155, "the sweep must actually have run");
    }

    /// The case the sweep above exists for: a 16 s recording is five windows,
    /// with the last second dropped, as in BirdNET-Pi. At the old quarter-window
    /// minimum it was six, the sixth being one second of audio and two of silence.
    #[test]
    fn a_one_second_tail_is_dropped_as_birdnet_pi_drops_it() {
        let starts = our_chunk_starts(16 * 48_000, 0.0, 3.0);
        assert_eq!(starts.len(), 5, "{starts:?}");
        assert_eq!(starts.last().copied(), Some(12 * 48_000));
        // And a tail of exactly the minimum is kept, one sample under is not.
        assert_eq!(chunk_starts(144_000 + 72_000, 144_000, 72_000).len(), 2);
        assert_eq!(chunk_starts(144_000 + 71_999, 144_000, 72_000).len(), 1);
    }

    /// A recording shorter than the minimum still gets its one (padded) chunk.
    #[test]
    fn a_recording_shorter_than_the_minimum_still_gets_one_chunk() {
        assert_eq!(our_chunk_starts(48_000, 0.0, 3.0), vec![0]);
        assert!(our_chunk_starts(0, 0.0, 3.0).is_empty());
    }

    #[test]
    fn is_audio_file_accepts_supported_extensions() {
        assert!(is_audio_file(Path::new("recording.wav")));
        assert!(is_audio_file(Path::new("recording.WAV")));
        assert!(is_audio_file(Path::new("recording.flac")));
        assert!(is_audio_file(Path::new("recording.mp3")));
        assert!(is_audio_file(Path::new(
            "/data/StreamData/2026-03-11-birdnet-08:30:00.wav"
        )));
    }

    #[test]
    fn is_audio_file_rejects_non_audio() {
        assert!(!is_audio_file(Path::new("data.txt")));
        assert!(!is_audio_file(Path::new("image.png")));
        assert!(!is_audio_file(Path::new("noext")));
    }

    #[test]
    fn default_pipeline_config() {
        let config = PipelineConfig::default();
        assert_eq!(config.target_sample_rate, 48000);
        assert!((config.chunk_duration_secs - 3.0).abs() < f32::EPSILON);
        assert!((config.confidence_threshold - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn process_nonexistent_file_returns_error() {
        let config = PipelineConfig::default();
        let result = process_file(
            Path::new("/nonexistent/2026-03-11-birdnet-08:30:00.wav"),
            &config,
        );
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod chunk_step_tests {
    use super::{PipelineConfig, chunk_step_samples};

    /// Which spans of audio a classifier with `window` samples actually sees,
    /// given a chunk grid — the thing the whole step decision is about.
    fn covered(total: usize, step: usize, window: usize) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut pos = 0;
        while pos < total {
            spans.push((pos, (pos + window).min(total)));
            pos += step;
        }
        spans
    }

    /// True when `spans` leave no audio unheard between the first and last.
    fn has_no_gap(spans: &[(usize, usize)]) -> bool {
        spans.windows(2).all(|w| w[1].0 <= w[0].1)
    }

    /// A single classifier keeps exactly the arithmetic it always had.
    #[test]
    fn one_classifier_steps_by_chunk_minus_overlap() {
        let config = PipelineConfig {
            target_sample_rate: 32_000,
            chunk_duration_secs: 4.5,
            chunk_overlap_secs: 0.0,
            chunk_step_secs: None,
            ..PipelineConfig::default()
        };
        assert_eq!(chunk_step_samples(&config), 144_000);

        let overlapped = PipelineConfig {
            chunk_overlap_secs: 1.5,
            ..config
        };
        assert_eq!(chunk_step_samples(&overlapped), 96_000);
    }

    /// **The gap this design exists to prevent.** Chunk at the longest window
    /// and step by it, and the shorter-window classifier never hears the tail
    /// of any chunk — a blind spot it would not have had alone, and one
    /// nothing reports.
    ///
    /// This asserts the *problem*, so the fix below has something to be
    /// measured against.
    #[test]
    fn stepping_by_the_longest_window_would_blind_the_shorter_classifier() {
        // Perch 160 000, BirdNET 144 000, at 32 kHz.
        let naive = covered(1_600_000, 160_000, 144_000);
        assert!(
            !has_no_gap(&naive),
            "stepping by the longest window must leave the shorter one a gap, or this test \
             is not describing the problem"
        );
        // The specific hole: 16 000 samples (half a second) per chunk.
        assert_eq!(naive[0].1, 144_000);
        assert_eq!(naive[1].0, 160_000);
    }

    /// **The fix.** Step by the shortest window and every classifier's
    /// coverage is contiguous — the shorter one exactly as if it ran alone,
    /// the longer one overlapping.
    ///
    /// Observed failing with `chunk_step_secs` ignored (the `if let` arm
    /// removed from `chunk_step_samples`): the step fell back to the chunk
    /// length and the gap assertion went red.
    #[test]
    fn stepping_by_the_shortest_window_leaves_no_classifier_a_gap() {
        let config = PipelineConfig {
            target_sample_rate: 32_000,
            chunk_duration_secs: 5.0, // the longest window
            chunk_overlap_secs: 0.0,
            chunk_step_secs: Some(4.5), // the shortest
            ..PipelineConfig::default()
        };
        let step = chunk_step_samples(&config);
        assert_eq!(step, 144_000, "the step is the shortest window");

        for window in [144_000_usize, 160_000] {
            let spans = covered(1_600_000, step, window);
            assert!(
                has_no_gap(&spans),
                "a {window}-sample classifier must have contiguous coverage"
            );
        }
    }

    /// The shortest-window classifier is not merely gap-free but **identical**
    /// to running alone: same starts, same length. Anything less would be a
    /// regression for the station's existing model.
    #[test]
    fn the_shortest_window_classifier_is_unchanged_from_running_alone() {
        let alone = PipelineConfig {
            target_sample_rate: 32_000,
            chunk_duration_secs: 4.5,
            chunk_overlap_secs: 0.0,
            chunk_step_secs: None,
            ..PipelineConfig::default()
        };
        let beside_perch = PipelineConfig {
            chunk_duration_secs: 5.0,
            chunk_step_secs: Some(4.5),
            ..alone.clone()
        };
        assert_eq!(
            chunk_step_samples(&alone),
            chunk_step_samples(&beside_perch),
            "adding a longer-window classifier must not move the existing one's chunks"
        );
        assert_eq!(
            covered(1_600_000, chunk_step_samples(&alone), 144_000),
            covered(1_600_000, chunk_step_samples(&beside_perch), 144_000)
        );
    }

    /// The longer-window classifier gains coverage rather than losing it, and
    /// pays for it in inferences — the cost the daemon logs.
    #[test]
    fn the_longer_window_classifier_gains_coverage_and_pays_in_inferences() {
        let alone = covered(1_600_000, 160_000, 160_000);
        let beside = covered(1_600_000, 144_000, 160_000);
        assert!(
            beside.len() > alone.len(),
            "more chunks: {} vs {}",
            beside.len(),
            alone.len()
        );
        // 160 000 / 144 000 ≈ 1.11, the ~11 % the daemon reports.
        #[allow(clippy::cast_precision_loss)]
        let ratio = beside.len() as f64 / alone.len() as f64;
        assert!((1.05..=1.2).contains(&ratio), "ratio was {ratio}");
        assert!(has_no_gap(&beside));
    }

    /// A step of zero or a negative one would loop forever or step backwards;
    /// both fall back rather than wedging the daemon on a bad number.
    #[test]
    fn a_nonsense_step_falls_back_instead_of_wedging() {
        for bad in [0.0_f32, -1.0] {
            let config = PipelineConfig {
                target_sample_rate: 32_000,
                chunk_duration_secs: 3.0,
                chunk_overlap_secs: 0.0,
                chunk_step_secs: Some(bad),
                ..PipelineConfig::default()
            };
            assert_eq!(
                chunk_step_samples(&config),
                96_000,
                "a {bad} step must fall back to the chunk arithmetic"
            );
        }
    }

    /// An overlap at or beyond the chunk length must still advance, or the
    /// chunker never reaches the end of a recording.
    #[test]
    fn the_step_is_never_zero() {
        let config = PipelineConfig {
            target_sample_rate: 32_000,
            chunk_duration_secs: 3.0,
            chunk_overlap_secs: 10.0,
            chunk_step_secs: None,
            ..PipelineConfig::default()
        };
        assert!(chunk_step_samples(&config) >= 1);
    }
}
