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
    /// The configuration asks for something this call cannot give.
    Config(String),
    /// The file is not one the detection path decodes (review F11): too long
    /// for a live segment, or its header cannot be read.
    Refused(crate::audio::admission::AdmissionError),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Watch(msg) => write!(f, "file watch error: {msg}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Resample(e) => write!(f, "resample error: {e}"),
            Self::Spectrogram(e) => write!(f, "spectrogram error: {e}"),
            Self::Channel(msg) => write!(f, "channel error: {msg}"),
            Self::Config(msg) => write!(f, "pipeline configuration: {msg}"),
            Self::Refused(e) => write!(f, "not analysed: {e}"),
        }
    }
}

impl std::error::Error for PipelineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Resample(e) => Some(e),
            Self::Spectrogram(e) => Some(e),
            Self::Refused(e) => Some(e),
            Self::Watch(_) | Self::Channel(_) | Self::Config(_) => None,
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
    /// Feed raw audio samples directly to the model instead of a mel spectrogram.
    ///
    /// Set by the daemon from the loaded classifier's input spec
    /// (`InputSpec::is_waveform`). Every bundled classifier is a waveform
    /// model: BirdNET V2.4 takes `[1, 144_000]` (3 s at 48 kHz) and V3.0 and
    /// Perch take their own rate's samples, and each computes its features
    /// inside the network. The mel path is kept for a model whose input is a
    /// spectrogram.
    pub raw_audio_input: bool,
    /// Further sample rates to prepare every chunk at, for classifiers that
    /// want a rate other than [`Self::target_sample_rate`] (MMP-01).
    ///
    /// Each rate costs one more resample of the whole recording, from the
    /// decoded audio, and one more copy of every chunk's samples: the copies
    /// are in [`PreparedChunk::at_rates`]. Empty, the default, is the
    /// single-rate pipeline unchanged.
    pub extra_sample_rates: Vec<u32>,
    /// Which date the occurrence filter's week is taken from (review F13).
    /// [`OccurrenceWeek::PerFile`], the default, is BirdNET-Pi's rule.
    pub occurrence_week: OccurrenceWeek,
    /// How a multichannel recording's channels are analysed (review F12).
    /// [`ChannelAnalysis::Mean`], the default, is what the pipeline has
    /// always done.
    pub channels: ChannelAnalysis,
}

/// How a multichannel recording's channels are analysed (review F12).
///
/// This is the decoder's half of a choice the capture path already offers per
/// source (Mono, Left, Right, "Both, averaged"), for a file that reaches the
/// pipeline with its channels intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelAnalysis {
    /// The mean of every channel, as one recording: the default, and what
    /// the pipeline has always done. Antiphase content cancels in it, and the
    /// decoder warns when it does ([`decode::ChannelProvenance`]).
    #[default]
    Mean,
    /// One channel, counted from `0`, as the recording. A channel the file
    /// does not have fails the file.
    Only(u16),
    /// Every channel analysed on its own, each giving its own chunks tagged
    /// with its index ([`PreparedChunk::channel`]). Only
    /// [`process_file_channels`] gives this; a call that returns one set of
    /// chunks refuses it. Two channels of one recorder are two microphones of
    /// **one station**, heard through one clock and usually a few centimetres
    /// apart: never two stations, and never two independent observations.
    Each,
}

/// One channel's chunks, or the mix's (review F12).
#[derive(Debug, Clone)]
pub struct ChannelChunks {
    /// The channel these chunks are of, counted from `0`; `None` for the mean
    /// of every channel (or the one channel of a mono file).
    pub channel: Option<u16>,
    /// Its chunks, in time order.
    pub chunks: Vec<PreparedChunk>,
}

/// Which date the species-occurrence filter's week is computed from
/// (review F13).
///
/// The geomodel is asked about one of 48 week bins (days 1-7, 8-14, 15-21 and
/// 22 to the end of each month: [`crate::civil::birdnet_week`]). A recording
/// that crosses midnight at the end of a bin holds windows from two bins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OccurrenceWeek {
    /// One week per file, from the date in the recording's filename, for every
    /// window: what BirdNET-Pi does, and the default, so a station's
    /// occurrence verdicts stay comparable with BirdNET-Pi's.
    #[default]
    PerFile,
    /// Each window's week from its own capture date (the recording's start
    /// shifted by the window's start), and the occurrence filter's verdict
    /// for that window from that week. For research that needs each window
    /// judged on the day it was heard; it departs from BirdNET-Pi only for
    /// windows past a bin boundary inside one recording.
    PerWindow,
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
            raw_audio_input: false,
            extra_sample_rates: Vec::new(),
            occurrence_week: OccurrenceWeek::PerFile,
            channels: ChannelAnalysis::Mean,
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
    /// Every sample of the chunk is the same value: digital silence (what a
    /// muted, unplugged or null source delivers) or a constant offset.
    ///
    /// There is nothing in such a chunk for a classifier to hear, and the real
    /// BirdNET+ V3.0 model answers it with NaN in all 11,560 outputs, which
    /// the station would otherwise count as a damaged classifier and tell its
    /// owner to replace the model file. [`is_constant`] says what is meant.
    pub silent: bool,
    /// The same span of audio as a waveform at each of
    /// [`PipelineConfig::extra_sample_rates`], as `(rate, samples)` (MMP-01).
    ///
    /// Cut from the recording resampled once to that rate, starting at the
    /// sample nearest [`Self::start_secs`] and as long as the chunk, padded
    /// with zeros as the primary's copy is. Empty when every classifier shares
    /// the primary's rate.
    pub at_rates: Vec<(u32, Vec<f32>)>,
    /// The channel this chunk was cut from, counted from `0`, when the
    /// pipeline analysed channels separately ([`ChannelAnalysis::Only`] and
    /// [`ChannelAnalysis::Each`]); `None` for the mean of every channel.
    pub channel: Option<u16>,
}

impl PreparedChunk {
    /// This chunk's waveform at `rate`, if the pipeline prepared one: one of
    /// [`Self::at_rates`]. The primary's own copy is
    /// [`Self::spectrogram`], which the caller reads when `rate` is the
    /// primary's.
    #[must_use]
    pub fn samples_at(&self, rate: u32) -> Option<&[f32]> {
        self.at_rates
            .iter()
            .find(|(r, _)| *r == rate)
            .map(|(_, samples)| samples.as_slice())
    }
}

/// Whether every sample is the same value, the test behind
/// [`PreparedChunk::silent`].
///
/// Exact, not a level: a threshold on loudness would have to guess where the
/// model starts to fail, and measured on the real V3.0 model it does not fail
/// at one (ten one-LSB flips in zeros answer; one flip does not, and white
/// noise changes between an RMS of 5e-7 and 7e-7). A window with no variation at all is
/// the case no capture path argues about. A NaN or infinite sample is never
/// "constant", so damaged audio still reaches the model and is counted as the
/// failure it is.
#[must_use]
pub fn is_constant(samples: &[f32]) -> bool {
    // Not a min/max fold: `f32::min` and `f32::max` skip a NaN, so one NaN
    // among zeros would read as silence and the damaged audio never reach the
    // model. `s - first` is NaN for a NaN or an infinity, and NaN is not zero.
    let Some(&first) = samples.first() else {
        return false;
    };
    samples.iter().all(|&s| s - first == 0.0)
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

/// How a recording of a given length is cut into chunks: the starts
/// [`process_file`] walks, and what that leaves unheard or padded.
///
/// Built by [`chunk_layout`] from the same config -> step -> minimum tail ->
/// starts arithmetic `process_file` runs, so a diagnostic that asks "what does
/// a 10 s segment lose?" reads the answer the pipeline acts on rather than a
/// copy of it (MSC-08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLayout {
    /// The recording's length, in samples at the target rate.
    pub len: usize,
    /// Each chunk's length, in samples at the target rate.
    pub chunk_samples: usize,
    /// How far apart consecutive chunks start, in samples.
    pub step: usize,
    /// The shortest remainder that still gets a chunk of its own, in samples.
    pub min_tail: usize,
    /// Where each chunk starts, in samples.
    pub starts: Vec<usize>,
}

impl ChunkLayout {
    /// Samples at the end of the recording that no chunk contains: the tail
    /// the walk stopped short of because it was under [`Self::min_tail`].
    #[must_use]
    pub fn unanalysed_tail(&self) -> usize {
        self.starts.last().map_or(self.len, |&last| {
            self.len
                .saturating_sub(last.saturating_add(self.chunk_samples))
        })
    }

    /// Samples of silence the last chunk is padded with because the recording
    /// ends inside it; `0` when it ends on or after the chunk's end.
    #[must_use]
    pub fn padded_tail(&self) -> usize {
        self.starts.last().map_or(0, |&last| {
            last.saturating_add(self.chunk_samples)
                .saturating_sub(self.len)
        })
    }
}

/// How [`process_file`] cuts a recording of `len` samples (at
/// `config.target_sample_rate`) under `config`.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn chunk_layout(config: &PipelineConfig, len: usize) -> ChunkLayout {
    let chunk_samples = (config.chunk_duration_secs * config.target_sample_rate as f32) as usize;
    let step = chunk_step_samples(config);
    let min_tail = min_tail_samples(chunk_samples, config.target_sample_rate);
    ChunkLayout {
        len,
        chunk_samples,
        step,
        min_tail,
        starts: chunk_starts(len, step, min_tail),
    }
}

/// Process a single audio file through the pipeline.
///
/// Decodes, resamples, splits into chunks, and computes mel spectrograms.
/// Returns prepared chunks ready for inference.
///
/// # Errors
///
/// Returns `PipelineError` if any stage of the pipeline fails.
pub fn process_file(
    path: &Path,
    config: &PipelineConfig,
) -> Result<Vec<PreparedChunk>, PipelineError> {
    process_file_with_source_rate(path, config).map(|(chunks, _)| chunks)
}

/// [`process_file`], and the rate the recording was decoded at.
///
/// That rate is the one before any resampling: the band the source actually
/// carried, which the daemon checks against the classifiers'
/// ([`crate::audio::band`], ACC-14).
///
/// # Errors
///
/// As [`process_file`].
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn process_file_with_source_rate(
    path: &Path,
    config: &PipelineConfig,
) -> Result<(Vec<PreparedChunk>, u32), PipelineError> {
    if config.channels == ChannelAnalysis::Each {
        return Err(PipelineError::Config(
            "each channel analysed on its own gives one set of chunks per channel; \
             call process_file_channels"
                .to_owned(),
        ));
    }
    let (mut groups, rate, _) = process_file_channels(path, config)?;
    Ok((groups.pop().map_or_else(Vec::new, |g| g.chunks), rate))
}

/// [`process_file_with_source_rate`] under every [`ChannelAnalysis`].
///
/// One [`ChannelChunks`] for the mean or the one chosen channel, one per channel
/// for [`ChannelAnalysis::Each`], and the decoder's account of the file's
/// channels (review F12).
///
/// # Errors
///
/// As [`process_file`], and a decode error for a channel the file lacks.
pub fn process_file_channels(
    path: &Path,
    config: &PipelineConfig,
) -> Result<(Vec<ChannelChunks>, u32, decode::ChannelProvenance), PipelineError> {
    let recording = RecordingFile::parse(&path.to_string_lossy()).ok_or_else(|| {
        PipelineError::Watch(format!(
            "cannot parse recording filename: {}",
            path.display()
        ))
    })?;
    // Judged from the header before anything is decoded (review F11), and
    // decoded under the cap that judgement gives: a file past a live
    // segment's length is refused, and one whose header understates its
    // length still cannot grow the buffer past it.
    let cap = crate::audio::admission::admit_for_live_analysis(path)
        .map_err(PipelineError::Refused)?
        .sample_cap();
    let (audios, provenance): (Vec<(Option<u16>, decode::AudioData)>, _) = match config.channels {
        ChannelAnalysis::Mean => {
            let (audio, p) =
                decode::decode_file_with_provenance(path, decode::ChannelMix::Mean, Some(cap))?;
            (vec![(None, audio)], p)
        }
        ChannelAnalysis::Only(c) => {
            let (audio, p) = decode::decode_file_with_provenance(
                path,
                decode::ChannelMix::Channel(c),
                Some(cap),
            )?;
            (vec![(Some(c), audio)], p)
        }
        ChannelAnalysis::Each => {
            let (each, p) = decode::decode_file_each_channel(path, Some(cap))?;
            (
                each.into_iter()
                    .enumerate()
                    .map(|(i, a)| (Some(u16::try_from(i).unwrap_or(u16::MAX)), a))
                    .collect(),
                p,
            )
        }
    };
    let rate = provenance.sample_rate;
    let groups = audios
        .into_iter()
        .map(|(channel, audio)| {
            Ok(ChannelChunks {
                channel,
                chunks: chunks_of(&recording, &audio, config, channel)?,
            })
        })
        .collect::<Result<_, PipelineError>>()?;
    Ok((groups, rate, provenance))
}

/// Cut one decoded stream into prepared chunks.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "sample counts and second offsets of one recording, far inside f32's exact range"
)]
fn chunks_of(
    recording: &RecordingFile,
    audio: &decode::AudioData,
    config: &PipelineConfig,
    channel: Option<u16>,
) -> Result<Vec<PreparedChunk>, PipelineError> {
    // Resample to target rate
    let samples = resample::resample(&audio.samples, audio.sample_rate, config.target_sample_rate)?;
    // And once more per rate another classifier wants, from the decoded audio
    // rather than from `samples`, so that classifier is fed what it would be
    // fed as the only one (MMP-01).
    let extra: Vec<(u32, Vec<f32>)> = config
        .extra_sample_rates
        .iter()
        .filter(|&&rate| rate != config.target_sample_rate)
        .map(|&rate| {
            Ok((
                rate,
                resample::resample(&audio.samples, audio.sample_rate, rate)?,
            ))
        })
        .collect::<Result<_, PipelineError>>()?;

    // Split into chunks
    let layout = chunk_layout(config, samples.len());
    let chunk_samples = layout.chunk_samples;
    let mut chunks = Vec::new();

    for pos in layout.starts {
        let end = (pos + chunk_samples).min(samples.len());
        let mut chunk_data = samples[pos..end].to_vec();

        // Pad short chunks with zeros (matching Python behavior)
        if chunk_data.len() < chunk_samples {
            chunk_data.resize(chunk_samples, 0.0);
        }

        let start_secs = pos as f32 / config.target_sample_rate as f32;
        let end_secs = end as f32 / config.target_sample_rate as f32;
        // Judged on the padded chunk, which is what a model would be handed.
        let silent = is_constant(&chunk_data);

        // A waveform model (raw_audio_input=true, every bundled classifier)
        // takes the samples directly; they are stored as a "1 × N"
        // MelSpectrogram so the rest of the pipeline is unchanged. A model
        // whose input is a spectrogram is given one computed here.
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

        let at_rates = extra
            .iter()
            .map(|(rate, samples)| {
                (
                    *rate,
                    cut_at_rate(
                        samples,
                        pos,
                        chunk_samples,
                        config.target_sample_rate,
                        *rate,
                    ),
                )
            })
            .collect();

        chunks.push(PreparedChunk {
            spectrogram,
            start_secs,
            end_secs,
            recording: recording.clone(),
            silent,
            at_rates,
            channel,
        });
    }

    Ok(chunks)
}

/// The span `[pos, pos + len)` of a recording at `from` hertz, cut from the
/// same recording resampled to `to` hertz and zero-padded to the span's length
/// at that rate.
///
/// Start and length are scaled in integer arithmetic, rounded to the nearest
/// sample: exact for every chunk start and length the catalogue's models give
/// (3 s, 4.5 s and 5 s steps at 32 and 48 kHz).
fn cut_at_rate(samples: &[f32], pos: usize, len: usize, from: u32, to: u32) -> Vec<f32> {
    let scale = |n: usize| {
        let from = u64::from(from).max(1);
        let n = (n as u64 * u64::from(to) + from / 2) / from;
        usize::try_from(n).unwrap_or(usize::MAX)
    };
    let start = scale(pos).min(samples.len());
    let want = scale(len);
    let end = start.saturating_add(want).min(samples.len());
    let mut out = samples[start..end].to_vec();
    out.resize(want, 0.0);
    out
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

    /// [`chunk_layout`] for a `secs`-second segment at 48 kHz.
    fn layout(secs: f32, window: f32, overlap: f32) -> ChunkLayout {
        let config = PipelineConfig {
            chunk_duration_secs: window,
            chunk_overlap_secs: overlap,
            ..PipelineConfig::default()
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        chunk_layout(&config, (secs * 48_000.0) as usize)
    }

    /// The layout's starts are the walk's, and what it reports lost or
    /// padded is the arithmetic MSC-08 worked by hand: the default 15 s
    /// segment is five whole windows with nothing lost or padded; 10 s and
    /// 16 s drop their last second; overlap 1.5 adds a window at 13.5 s that
    /// is half silence; a 4.5 s window at 15 s ends on 1.5 s of audio and
    /// 3 s of silence.
    #[test]
    fn the_layout_reports_what_a_segment_length_loses_or_pads() {
        const S: usize = 48_000;
        let default = layout(15.0, 3.0, 0.0);
        assert_eq!(default.starts, vec![0, 3 * S, 6 * S, 9 * S, 12 * S]);
        assert_eq!((default.unanalysed_tail(), default.padded_tail()), (0, 0));
        assert_eq!(default.starts, our_chunk_starts(15 * S, 0.0, 3.0));

        for secs in [10.0, 16.0] {
            let l = layout(secs, 3.0, 0.0);
            assert_eq!((l.unanalysed_tail(), l.padded_tail()), (S, 0), "{secs} s");
        }

        let overlap = layout(15.0, 3.0, 1.5);
        assert_eq!(overlap.starts.last(), Some(&(27 * S / 2)));
        assert_eq!(
            (overlap.unanalysed_tail(), overlap.padded_tail()),
            (0, 3 * S / 2)
        );

        let wide = layout(15.0, 4.5, 0.0);
        assert_eq!(wide.starts, vec![0, 9 * S / 2, 9 * S, 27 * S / 2]);
        assert_eq!((wide.unanalysed_tail(), wide.padded_tail()), (0, 3 * S));

        // An 11 s segment keeps its 2 s tail as a padded window rather than
        // dropping it: padded, not lost.
        let eleven = layout(11.0, 3.0, 0.0);
        assert_eq!((eleven.unanalysed_tail(), eleven.padded_tail()), (0, S));
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
    }

    /// `is_constant` is exact: one sample that differs is enough to give the
    /// model the chunk, and damaged samples are never "constant".
    #[test]
    fn is_constant_is_exact() {
        assert!(is_constant(&[0.0; 1000]));
        assert!(
            is_constant(&[0.25; 1000]),
            "a constant offset has nothing to hear either"
        );
        assert!(is_constant(&[0.0, -0.0, 0.0]), "negative zero is zero");
        let mut one_lsb = vec![0.0_f32; 144_000];
        one_lsb[72_000] = 1.0 / 32_768.0;
        assert!(!is_constant(&one_lsb), "a single flipped bit is a signal");
        assert!(
            !is_constant(&[0.0, f32::NAN, 0.0]),
            "damaged audio must reach the model"
        );
        assert!(!is_constant(&[f32::INFINITY; 4]), "inf - inf is not zero");
        assert!(!is_constant(&[]), "no samples is not a window of silence");
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

    /// ACC-14's seam: the rate reported is the recording's own, not the rate
    /// it was resampled to, or a 16 kHz source would look like a 48 kHz one.
    #[test]
    fn the_source_rate_is_the_recording_s_not_the_target_s() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("2026-05-19-birdnet-09:00:00.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..16_000 * 3 {
            w.write_sample(i16::try_from(i % 200).unwrap()).unwrap();
        }
        w.finalize().unwrap();
        let config = PipelineConfig {
            raw_audio_input: true,
            target_sample_rate: 48_000,
            chunk_duration_secs: 3.0,
            ..PipelineConfig::default()
        };
        let (chunks, rate) = super::process_file_with_source_rate(&wav, &config).unwrap();
        assert_eq!(rate, 16_000);
        assert!(!chunks.is_empty());
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

#[cfg(test)]
pub(crate) mod channel_tests {
    use super::{
        ChannelAnalysis, PipelineConfig, process_file_channels, process_file_with_source_rate,
    };

    /// A 6 s, 48 kHz, 16-bit WAV whose channel `c` at frame `i` is
    /// `gain[c] * noise(i)`, named as the pipeline expects.
    pub fn write_channels(dir: &std::path::Path, gain: &[i16]) -> std::path::PathBuf {
        let path = dir.join("2026-05-19-birdnet-09:00:00.wav");
        let spec = hound::WavSpec {
            channels: u16::try_from(gain.len()).unwrap(),
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        let mut x: u32 = 1;
        for _ in 0..48_000 * 6 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            #[allow(
                clippy::cast_possible_wrap,
                reason = "a test noise generator, wrapped on purpose"
            )]
            let n = ((x >> 16) as i16) / 8;
            for &g in gain {
                w.write_sample(n.saturating_mul(g)).unwrap();
            }
        }
        w.finalize().unwrap();
        path
    }

    fn config(channels: ChannelAnalysis) -> PipelineConfig {
        PipelineConfig {
            raw_audio_input: true,
            target_sample_rate: 48_000,
            chunk_duration_secs: 3.0,
            channels,
            ..PipelineConfig::default()
        }
    }

    /// A WAV of `secs` seconds of quiet noise at 8 kHz, named as the pipeline
    /// expects (8 kHz keeps a twelve-minute file near 11 MB).
    fn write_long(dir: &std::path::Path, secs: u32) -> std::path::PathBuf {
        let path = dir.join("2026-05-19-birdnet-09:00:00.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..8_000 * secs {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_possible_wrap,
                reason = "i % 97 fits an i16"
            )]
            w.write_sample(((i % 97) as i16) - 48).unwrap();
        }
        w.finalize().unwrap();
        path
    }

    /// Review F11: a file longer than a live segment may be is refused before
    /// it is decoded, and the refusal names the command that takes it; the
    /// detection path no longer decodes a recording of any length whole.
    #[test]
    fn a_recording_longer_than_a_segment_is_refused_and_pointed_at_the_ingest() {
        let tmp = tempfile::tempdir().unwrap();
        let secs = u32::try_from(crate::audio::admission::LIVE_MAX_FILE_SECS).unwrap() + 1;
        let wav = write_long(tmp.path(), secs);
        let mut cfg = config(ChannelAnalysis::Mean);
        cfg.target_sample_rate = 8_000;
        let text = match process_file_channels(&wav, &cfg) {
            Ok((groups, _, _)) => panic!(
                "a recording past the live limit was decoded whole: {} chunks",
                groups[0].chunks.len()
            ),
            Err(e) => e.to_string(),
        };
        assert!(
            text.contains(crate::audio::admission::INGEST_COMMAND),
            "the refusal names the ingest command: {text}"
        );
    }

    /// The counterpart: a recording exactly at the limit is analysed, every
    /// sample of it.
    #[test]
    fn a_recording_at_the_live_limit_is_analysed_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let secs = u32::try_from(crate::audio::admission::LIVE_MAX_FILE_SECS).unwrap();
        let wav = write_long(tmp.path(), secs);
        let mut cfg = config(ChannelAnalysis::Mean);
        cfg.target_sample_rate = 8_000;
        let (groups, rate, _) = process_file_channels(&wav, &cfg).expect("admitted");
        assert_eq!(rate, 8_000);
        let chunks = &groups[0].chunks;
        let last = chunks.last().expect("chunks");
        assert!(
            (last.end_secs - f32::from(u16::try_from(secs).unwrap())).abs() < 0.01,
            "the last chunk ends at the recording's end: {}",
            last.end_secs
        );
    }

    /// Review F12: antiphase stereo (`R = -L`) averages to exact silence, so
    /// the default pipeline marks every chunk silent and classifies nothing.
    /// Analysed channel by channel, each is heard, and each chunk says which
    /// channel it came from; one channel can also be chosen alone.
    #[test]
    fn antiphase_stereo_is_silent_mixed_and_heard_channel_by_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let wav = write_channels(tmp.path(), &[1, -1]);

        let (mean, _, provenance) =
            process_file_channels(&wav, &config(ChannelAnalysis::default())).unwrap();
        assert_eq!(mean.len(), 1);
        assert_eq!(mean[0].channel, None);
        assert!(
            mean[0]
                .chunks
                .iter()
                .all(|c| c.silent && c.channel.is_none()),
            "the mean is silence"
        );
        assert!(provenance.cancellation_suspected(), "{provenance:?}");

        let (each, _, _) = process_file_channels(&wav, &config(ChannelAnalysis::Each)).unwrap();
        let channels: Vec<Option<u16>> = each.iter().map(|g| g.channel).collect();
        assert_eq!(channels, [Some(0), Some(1)]);
        for g in &each {
            assert_eq!(g.chunks.len(), 2);
            assert!(
                g.chunks.iter().all(|c| !c.silent && c.channel == g.channel),
                "channel {:?}",
                g.channel
            );
        }
        // The two channels are the same audio inverted.
        let (l, r) = (
            &each[0].chunks[0].spectrogram.data,
            &each[1].chunks[0].spectrogram.data,
        );
        assert!(l.iter().zip(r).all(|(a, b)| (a + b).abs() < 1e-6));

        let (right, _, _) = process_file_channels(&wav, &config(ChannelAnalysis::Only(1))).unwrap();
        assert_eq!(right.len(), 1);
        assert_eq!(right[0].channel, Some(1));
        assert_eq!(right[0].chunks[0].spectrogram.data, *r);
        assert!(
            process_file_channels(&wav, &config(ChannelAnalysis::Only(2))).is_err(),
            "no third channel"
        );
    }

    /// A live channel beside a silent one: the mean is heard (half the level),
    /// the live channel is heard, the silent one is digital silence and is
    /// marked so. Counterpart: the default is chunk for chunk what
    /// `process_file_with_source_rate` gives, and that refuses `Each`.
    #[test]
    fn one_silent_channel_is_marked_silent_and_the_default_is_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let wav = write_channels(tmp.path(), &[1, 0]);
        let (each, _, provenance) =
            process_file_channels(&wav, &config(ChannelAnalysis::Each)).unwrap();
        assert_eq!(each.len(), 2, "one set of chunks per channel");
        assert!(each[0].chunks.iter().all(|c| !c.silent));
        assert!(each[1].chunks.iter().all(|c| c.silent));
        assert!(
            !provenance.cancellation_suspected(),
            "a silent channel is not cancellation"
        );

        let (mean, rate) =
            process_file_with_source_rate(&wav, &config(ChannelAnalysis::Mean)).unwrap();
        assert_eq!(rate, 48_000);
        let (groups, _, _) = process_file_channels(&wav, &config(ChannelAnalysis::Mean)).unwrap();
        assert_eq!(groups[0].chunks.len(), mean.len());
        for (a, b) in groups[0].chunks.iter().zip(&mean) {
            assert_eq!(a.spectrogram.data, b.spectrogram.data);
            assert!(!a.silent);
        }
        assert!(process_file_with_source_rate(&wav, &config(ChannelAnalysis::Each)).is_err());
    }
}
