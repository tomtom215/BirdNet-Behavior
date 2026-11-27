//! Audio decoding via symphonia.
//!
//! Decodes WAV, FLAC, and MP3 files into f32 sample buffers.
//! Replaces `librosa.load()` and `soundfile.read()`.

use std::fmt;
use std::path::Path;

/// Decoded audio data as mono f32 samples at a known sample rate.
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Mono audio samples normalized to [-1.0, 1.0].
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
}

/// Which of a file's channels a decode returns (review F12).
///
/// The capture path already lets each source pick Mono, Left, Right or "Both,
/// averaged" before a segment is written; this is the same choice for a file
/// that reaches the decoder with its channels intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelMix {
    /// The arithmetic mean of every channel, sample by sample: what
    /// [`decode_file`] has always returned, and the default. Antiphase
    /// content cancels in it; [`ChannelProvenance::mix_energy_ratio`] says how
    /// much.
    #[default]
    Mean,
    /// One channel alone, counted from `0` (left is `0`, right `1`). A channel
    /// the file does not have is an error, never a substitute.
    Channel(u16),
}

impl ChannelMix {
    /// The left channel of a stereo file.
    pub const LEFT: Self = Self::Channel(0);
    /// The right channel of a stereo file.
    pub const RIGHT: Self = Self::Channel(1);
}

/// What was mixed to give the samples a decode returned (review F12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedMix {
    /// The file has one channel; nothing was mixed.
    Mono,
    /// The mean of this many channels.
    Mean(u16),
    /// This one channel, of the file's [`ChannelProvenance::channels`].
    Channel(u16),
    /// Every channel, each returned separately.
    Each,
}

/// Where a decode's samples came from: the file's channels and format as the
/// decoder found them, and what was done to the channels (review F12).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelProvenance {
    /// The file's own channel count, as decoded.
    pub channels: u16,
    /// The sample rate the file was decoded at, before any resampling.
    pub sample_rate: u32,
    /// The sample format the decoder produced (`"s16"`, `"s24"`, `"f32"`,
    /// ...), else the one the container declared, when either is known.
    pub sample_format: Option<String>,
    /// Bits per sample, when the container declared it.
    pub bits_per_sample: Option<u32>,
    /// What was mixed.
    pub mix: AppliedMix,
    /// The energy of the channels' mean relative to the mean of the
    /// channels' own energies, over the whole file: `sum(mean^2) /
    /// (sum over channels of sum(x^2) / n)`.
    ///
    /// `1.0` for identical channels; `1/n` for `n` channels of equal energy
    /// that are uncorrelated, and for one live channel among `n - 1` silent
    /// ones (`0.5` in stereo); about `0.0` for antiphase stereo (`L = -R`),
    /// whose mean is silence although each channel is not. A value under
    /// `1/n` can only come from channels that are anticorrelated, which is
    /// cancellation. `None` for a one-channel file, or one with no energy.
    pub mix_energy_ratio: Option<f64>,
}

/// How far under the uncorrelated floor (`1/n`) the mix energy ratio may fall
/// before cancellation is reported.
///
/// Half of it, that is at least 3 dB of the
/// channels' energy cancelled in the mean beyond what averaging uncorrelated
/// channels costs. For stereo the warning fires below `0.25`.
pub const CANCELLATION_FLOOR_FACTOR: f64 = 0.5;

impl ChannelProvenance {
    /// Whether the mean lost enough to antiphase content that analysing it
    /// is analysing less than the microphones heard: the mix energy ratio is
    /// under [`CANCELLATION_FLOOR_FACTOR`] of `1/n`. Never for one channel.
    #[must_use]
    pub fn cancellation_suspected(&self) -> bool {
        let n = f64::from(self.channels.max(1));
        self.channels >= 2
            && self
                .mix_energy_ratio
                .is_some_and(|r| r < CANCELLATION_FLOOR_FACTOR / n)
    }
}

/// [`decode_file`], with the channels chosen by `mix`, and where the samples
/// came from (review F12).
///
/// # Errors
///
/// As [`decode_file`], and [`DecodeError::Format`] for a channel the file does
/// not have.
pub fn decode_file_with_provenance(
    path: &Path,
    mix: ChannelMix,
) -> Result<(AudioData, ChannelProvenance), DecodeError> {
    let select = match mix {
        ChannelMix::Mean => Select::Mean,
        ChannelMix::Channel(c) => Select::Channel(usize::from(c)),
    };
    let mut decoded = decode_file_inner(path, None, select)?;
    let samples = decoded.outputs.pop().unwrap_or_default();
    Ok((
        AudioData {
            samples,
            sample_rate: decoded.provenance.sample_rate,
        },
        decoded.provenance,
    ))
}

/// Every channel of a file, each decoded separately and in channel order, for
/// analysing channels independently (review F12). One element for a
/// one-channel file.
///
/// # Errors
///
/// As [`decode_file`].
pub fn decode_file_each_channel(
    path: &Path,
) -> Result<(Vec<AudioData>, ChannelProvenance), DecodeError> {
    let decoded = decode_file_inner(path, None, Select::Each)?;
    let rate = decoded.provenance.sample_rate;
    Ok((
        decoded
            .outputs
            .into_iter()
            .map(|samples| AudioData {
                samples,
                sample_rate: rate,
            })
            .collect(),
        decoded.provenance,
    ))
}

/// Errors that can occur during audio decoding.
#[derive(Debug)]
pub enum DecodeError {
    /// File not found or inaccessible.
    Io(std::io::Error),
    /// Unsupported or corrupt audio format.
    Format(String),
    /// No audio tracks in the file.
    NoTracks,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Format(msg) => write!(f, "format error: {msg}"),
            Self::NoTracks => write!(f, "no audio tracks found"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(_) | Self::NoTracks => None,
        }
    }
}

impl From<std::io::Error> for DecodeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Maximum mono samples decoded by [`decode_file_capped`] for the on-demand
/// spectrogram views.
///
/// The spectrogram is a *visual* downsampled to a fixed frame count, so showing
/// the leading portion of an unusually long recording is harmless — but
/// decoding the whole thing into an `f32` buffer is not. The public
/// `/api/v2/spectrogram` endpoint decodes on demand, so without a cap a long
/// station recording (or a misconfigured multi-minute segment) could allocate
/// hundreds of MB per request and, under the render concurrency limit, OOM a
/// small Pi. 28.8 M samples is 10 minutes at 48 kHz — far longer than any
/// normal recording — bounding the buffer to ~115 MB.
pub const SPECTROGRAM_DECODE_SAMPLE_CAP: usize = 10 * 60 * 48_000;

/// Decode an audio file to mono f32 samples.
///
/// # Errors
///
/// Returns `DecodeError` if the file cannot be read, decoded, or contains no audio.
pub fn decode_file(path: &Path) -> Result<AudioData, DecodeError> {
    decode_file_inner(path, None, Select::Mean).map(Decoded::into_mean)
}

/// Decode an audio file to mono f32 samples, stopping after at most
/// `max_samples` mono samples have been collected.
///
/// For the on-demand spectrogram views, where a visual of the leading portion
/// of an over-long recording is acceptable but decoding the whole file into an
/// unbounded buffer on a small Pi is not (see [`SPECTROGRAM_DECODE_SAMPLE_CAP`]).
/// The detection pipeline uses the uncapped [`decode_file`] so no audio is lost.
///
/// # Errors
///
/// Returns `DecodeError` if the file cannot be read, decoded, or contains no audio.
pub fn decode_file_capped(path: &Path, max_samples: usize) -> Result<AudioData, DecodeError> {
    decode_file_inner(path, Some(max_samples), Select::Mean).map(Decoded::into_mean)
}

/// Probe an audio file's duration in seconds **without decoding its samples**.
///
/// Reads only the container metadata — the default audio track's playable frame
/// count and sample rate — so it is cheap: one file open and a header parse, no
/// per-packet decode. Used by the detection daemon to persist each saved clip's
/// length for the Recordings grid, on top of the decode the pipeline already
/// did, rather than re-decoding the whole file just to measure it.
///
/// Returns `None` when the duration can't be determined honestly (unreadable
/// file, no audio track, missing sample rate, or a container that doesn't
/// record its frame count) — the caller then stores `NULL`, never a guess.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn probe_duration_secs(path: &Path) -> Option<f64> {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;

    let track = format.default_track(TrackType::Audio)?;
    let sample_rate = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .and_then(|a| a.sample_rate)?;
    let num_frames = track.num_frames?;
    if sample_rate == 0 {
        return None;
    }
    Some(num_frames as f64 / f64::from(sample_rate))
}

/// How many mono frames of silence stand in for a packet the decoder refused.
///
/// `dur` is the packet's duration in its track's time base
/// (`time_base = numer / denom` seconds per unit); the answer is that span in
/// frames at `sample_rate`, rounded to the nearest frame. With no time base the
/// container's units cannot be trusted to be frames, so the answer is `None`
/// and the caller counts the packet as lost rather than guessing its length.
/// A zero duration is `None` too: the container did not say how long it was.
#[must_use]
fn lost_packet_frames(dur: u64, time_base: Option<(u32, u32)>, sample_rate: u32) -> Option<usize> {
    let (numer, denom) = time_base?;
    if dur == 0 || numer == 0 || denom == 0 {
        return None;
    }
    let num = u128::from(dur) * u128::from(numer) * u128::from(sample_rate);
    let den = u128::from(denom);
    usize::try_from((num + den / 2) / den)
        .ok()
        .filter(|&n| n > 0)
}

/// Which channels [`decode_file_inner`] collects.
#[derive(Debug, Clone, Copy)]
enum Select {
    /// The mean of every channel, into one output.
    Mean,
    /// One channel, into one output.
    Channel(usize),
    /// Every channel, one output each.
    Each,
}

/// What [`decode_file_inner`] collected.
struct Decoded {
    /// One output for [`Select::Mean`] and [`Select::Channel`], one per channel
    /// for [`Select::Each`].
    outputs: Vec<Vec<f32>>,
    /// Where they came from.
    provenance: ChannelProvenance,
}

impl Decoded {
    /// The one output of a [`Select::Mean`] decode, as [`AudioData`].
    fn into_mean(mut self) -> AudioData {
        AudioData {
            samples: self.outputs.pop().unwrap_or_default(),
            sample_rate: self.provenance.sample_rate,
        }
    }
}

/// The codec's sample format, as a short lowercase name.
const fn sample_format_name(format: symphonia::core::audio::sample::SampleFormat) -> &'static str {
    use symphonia::core::audio::sample::SampleFormat as F;
    match format {
        F::U8 => "u8",
        F::U16 => "u16",
        F::U24 => "u24",
        F::U32 => "u32",
        F::S8 => "s8",
        F::S16 => "s16",
        F::S24 => "s24",
        F::S32 => "s32",
        F::F32 => "f32",
        F::F64 => "f64",
    }
}

/// The sample format of a decoded buffer, as [`sample_format_name`] names it.
const fn buffer_format_name(
    buffer: &symphonia::core::audio::GenericAudioBufferRef<'_>,
) -> &'static str {
    use symphonia::core::audio::GenericAudioBufferRef as B;
    use symphonia::core::audio::sample::SampleFormat as F;
    sample_format_name(match buffer {
        B::U8(_) => F::U8,
        B::U16(_) => F::U16,
        B::U24(_) => F::U24,
        B::U32(_) => F::U32,
        B::S8(_) => F::S8,
        B::S16(_) => F::S16,
        B::S24(_) => F::S24,
        B::S32(_) => F::S32,
        B::F32(_) => F::F32,
        B::F64(_) => F::F64,
    })
}

/// How often a file whose channels cancel in the mean is named in the log, in
/// seconds: a source with an inverted channel fills every recording.
const CANCELLATION_LOG_INTERVAL_SECS: u64 = 600;

/// When cancellation was last logged, in seconds since the Unix epoch (`0`:
/// never).
static CANCELLATION_LOGGED_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Warn, at most once per [`CANCELLATION_LOG_INTERVAL_SECS`], that a file's
/// channels cancelled in their mean.
fn report_cancellation(path: &Path, provenance: &ChannelProvenance) {
    use std::sync::atomic::Ordering;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let last = CANCELLATION_LOGGED_AT.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < CANCELLATION_LOG_INTERVAL_SECS {
        return;
    }
    CANCELLATION_LOGGED_AT.store(now.max(1), Ordering::Relaxed);
    tracing::warn!(
        path = %path.display(),
        channels = provenance.channels,
        mix_energy_ratio = provenance.mix_energy_ratio,
        repeat_suppressed_secs = CANCELLATION_LOG_INTERVAL_SECS,
        "this recording's channels cancel when averaged to mono (one is inverted, or the \
         microphones are out of phase): the mean holds much less than either channel. Pick \
         one channel for this source rather than \"Both, averaged\""
    );
}

#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn decode_file_inner(
    path: &Path,
    max_samples: Option<usize>,
    select: Select,
) -> Result<Decoded, DecodeError> {
    use symphonia::core::audio::GenericAudioBufferRef;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    // Symphonia 0.6: `probe` takes options by value and returns the
    // `FormatReader` directly (no more `ProbeResult` wrapper).
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    // 0.6 splits tracks by `TrackType`; we only want audio.
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoTracks)?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| DecodeError::Format("track has no audio codec params".into()))?;
    let sample_rate = audio_params
        .sample_rate
        .ok_or_else(|| DecodeError::Format("unknown sample rate".into()))?;
    let time_base = track.time_base.map(|tb| (tb.numer.get(), tb.denom.get()));
    let sample_format = audio_params
        .sample_format
        .map(|f| sample_format_name(f).to_owned());
    let bits_per_sample = audio_params.bits_per_sample;
    let declared_channels = audio_params
        .channels
        .as_ref()
        .map(symphonia::core::audio::Channels::count);

    // `make_audio_decoder` now takes the audio-specific codec params
    // (previously `make` accepted the whole CodecParameters struct).
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    // One output, or one per channel once the count is known.
    let mut outputs: Vec<Vec<f32>> = match select {
        Select::Mean | Select::Channel(_) => vec![Vec::new()],
        Select::Each => declared_channels.map_or_else(Vec::new, |n| vec![Vec::new(); n]),
    };
    // The file's channel count, from the first buffer decoded.
    let mut native: Option<usize> = None;
    // The sample format the decoder produced, from the first buffer.
    let mut decoded_format: Option<String> = None;
    // Frames of silence owed to outputs not allocated yet (a damaged packet
    // before the first decoded one, in a file that declared no channels).
    let mut owed_frames = 0_usize;
    // Frames placed in the output, decoded or filled: what the old single
    // buffer's length was.
    let mut frames_out = 0_usize;
    // For the mix energy ratio: the sum of the mean's squares, and of the mean
    // over channels of each channel's squares.
    let mut mean_energy = 0.0_f64;
    let mut channel_energy = 0.0_f64;
    let mut interleaved: Vec<f32> = Vec::new();
    // Packets the decoder refused: those replaced by silence of their own
    // length, and those whose length the container did not give (dropped,
    // so everything after them is early by an unknown amount). Reported in
    // one warning per file after the loop.
    let mut filled_packets = 0_usize;
    let mut filled_frames = 0_usize;
    let mut dropped_packets = 0_usize;

    // 0.6: `next_packet` returns `Result<Option<Packet>>` (EOF is now `None`
    // rather than a sentinel `UnexpectedEof` error).
    //
    // A header that claims more data than the file holds still surfaces as an
    // `UnexpectedEof` I/O error, though. That is exactly what a crash leaves
    // behind: `SegmentWriter` declares the segment's expected length up front
    // and only corrects it on close, and ffmpeg's `0xFFFFFFFF` placeholder is
    // never back-patched if it is killed. Failing the file there discarded
    // every sample already decoded — the whole recording — so once at least
    // one packet has decoded, a short read ends the stream instead.
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof && frames_out > 0 =>
            {
                tracing::warn!(
                    path = %path.display(),
                    decoded_samples = frames_out,
                    "audio file is shorter than its header claims (truncated write?); \
                     using the samples that exist"
                );
                break;
            }
            Err(e) => return Err(DecodeError::Format(e.to_string())),
        };
        // `track_id` is a struct field now, not a method call.
        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let audio_buf: GenericAudioBufferRef<'_> = audio_buf;
                let num_channels = audio_buf.num_planes();
                let num_frames = audio_buf.frames();
                if num_channels == 0 || num_frames == 0 {
                    continue;
                }

                interleaved.resize(audio_buf.samples_interleaved(), 0.0_f32);
                audio_buf.copy_to_slice_interleaved(&mut interleaved[..]);
                native.get_or_insert(num_channels);
                if decoded_format.is_none() {
                    decoded_format = Some(buffer_format_name(&audio_buf).to_owned());
                }
                if let Select::Channel(c) = select
                    && c >= num_channels
                {
                    return Err(DecodeError::Format(format!(
                        "channel {c} was asked for and the file has {num_channels} \
                         (channels count from 0)"
                    )));
                }
                if matches!(select, Select::Each) {
                    if outputs.is_empty() {
                        outputs = vec![vec![0.0; owed_frames]; num_channels];
                        owed_frames = 0;
                    }
                    if outputs.len() != num_channels {
                        return Err(DecodeError::Format(format!(
                            "the channel count changed from {} to {num_channels} inside the \
                             file, so its channels cannot be told apart",
                            outputs.len()
                        )));
                    }
                }

                for frame in 0..num_frames {
                    let row = &interleaved[frame * num_channels..(frame + 1) * num_channels];
                    // Mix to mono by averaging channels: the same sum, in the
                    // same order, as this has always been.
                    let mut sum = 0.0_f32;
                    for &x in row {
                        sum += x;
                    }
                    let mean = sum / num_channels as f32;
                    if num_channels >= 2 {
                        let m = f64::from(mean);
                        mean_energy += m * m;
                        channel_energy += row
                            .iter()
                            .map(|&x| f64::from(x) * f64::from(x))
                            .sum::<f64>()
                            / num_channels as f64;
                    }
                    match select {
                        Select::Mean => outputs[0].push(mean),
                        Select::Channel(c) => outputs[0].push(row[c]),
                        Select::Each => {
                            for (out, &x) in outputs.iter_mut().zip(row) {
                                out.push(x);
                            }
                        }
                    }
                }
                frames_out += num_frames;
            }
            // A damaged packet. Skipping it would move every later sample
            // earlier, so detections after it would be stamped, clipped and
            // corroborated a packet too early: put silence of the packet's
            // own length in its place when the container says how long it
            // is, and count it as dropped when it does not.
            Err(SymphoniaError::DecodeError(_)) => {
                let Some(frames) = lost_packet_frames(packet.dur.get(), time_base, sample_rate)
                else {
                    dropped_packets += 1;
                    continue;
                };
                filled_packets += 1;
                filled_frames += frames;
                frames_out += frames;
                if outputs.is_empty() {
                    owed_frames += frames;
                }
                for out in &mut outputs {
                    out.resize(out.len() + frames, 0.0);
                }
            }
            Err(e) => return Err(DecodeError::Format(e.to_string())),
        }

        // Stop once the optional sample cap is reached. The spectrogram views
        // pass a cap so a pathologically long recording can't allocate an
        // unbounded buffer on a Pi; the detection path passes `None` and keeps
        // every sample. Peak overshoot is one packet's worth of frames before
        // the truncate, which symphonia bounds per packet.
        if let Some(cap) = max_samples
            && frames_out >= cap
        {
            for out in &mut outputs {
                out.truncate(cap);
            }
            tracing::warn!(
                path = %path.display(),
                cap,
                "audio decode reached the sample cap; using the leading portion only"
            );
            break;
        }
    }

    if filled_packets > 0 || dropped_packets > 0 {
        tracing::warn!(
            path = %path.display(),
            filled_packets,
            filled_frames,
            dropped_packets,
            "audio file has damaged packets the decoder could not read; \
             each of known length is replaced by silence so later timestamps \
             stay true, and any of unknown length is dropped, so audio after \
             it is early by that packet's length"
        );
    }

    let channels = native.or(declared_channels).unwrap_or(1);
    let provenance = ChannelProvenance {
        channels: u16::try_from(channels).unwrap_or(u16::MAX),
        sample_rate,
        sample_format: decoded_format.or(sample_format),
        bits_per_sample,
        mix: match select {
            _ if channels < 2 && !matches!(select, Select::Each) => AppliedMix::Mono,
            Select::Mean => AppliedMix::Mean(u16::try_from(channels).unwrap_or(u16::MAX)),
            Select::Channel(c) => AppliedMix::Channel(u16::try_from(c).unwrap_or(u16::MAX)),
            Select::Each => AppliedMix::Each,
        },
        mix_energy_ratio: (channels >= 2 && channel_energy > 0.0)
            .then(|| mean_energy / channel_energy),
    };
    if provenance.cancellation_suspected() {
        report_cancellation(path, &provenance);
    }
    Ok(Decoded {
        outputs,
        provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A 16-bit WAV of `channels` channels, frame `i`'s channel `c` being
    /// `gain[c] * tone(i)`: a 1 kHz tone at 48 kHz, one second long.
    fn write_multichannel(path: &Path, gain: &[f32]) {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let spec = WavSpec {
            channels: u16::try_from(gain.len()).unwrap(),
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        for i in 0..48_000 {
            #[allow(clippy::cast_precision_loss)]
            let tone = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin() * 0.5;
            for g in gain {
                #[allow(clippy::cast_possible_truncation)]
                w.write_sample((g * tone * 32_767.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    fn ratio_of(gain: &[f32]) -> (Option<f64>, bool, ChannelProvenance) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.wav");
        write_multichannel(&path, gain);
        let (_, p) = decode_file_with_provenance(&path, ChannelMix::Mean).unwrap();
        (p.mix_energy_ratio, p.cancellation_suspected(), p)
    }

    /// Review F12: the mean of antiphase stereo is silence though neither
    /// channel is, and the decode says so; identical channels lose nothing;
    /// one silent channel halves the energy, which is averaging and not
    /// cancellation; three channels of which one is inverted cancel most of
    /// the tone, and one live channel of three does not.
    #[test]
    fn the_mix_energy_ratio_measures_what_averaging_cancelled() {
        let near = |r: Option<f64>, want: f64| r.is_some_and(|r| (r - want).abs() < 1e-3);
        let (r, warned, p) = ratio_of(&[1.0, -1.0]);
        assert!(near(r, 0.0), "antiphase: {p:?}");
        assert!(warned, "antiphase stereo is cancellation");
        assert_eq!((p.channels, p.mix), (2, AppliedMix::Mean(2)));
        assert_eq!(p.sample_format.as_deref(), Some("s16"));
        assert_eq!(p.bits_per_sample, Some(16));

        let (r, warned, _) = ratio_of(&[1.0, 1.0]);
        assert!(near(r, 1.0) && !warned, "identical: {r:?}");
        let (r, warned, _) = ratio_of(&[1.0, 0.0]);
        assert!(near(r, 0.5) && !warned, "one silent channel: {r:?}");
        let (r, warned, p) = ratio_of(&[1.0, 1.0, -1.0]);
        assert!(near(r, 1.0 / 9.0) && warned, "three, one inverted: {r:?}");
        assert_eq!(p.mix, AppliedMix::Mean(3));
        let (r, warned, _) = ratio_of(&[1.0, 0.0, 0.0]);
        assert!(near(r, 1.0 / 3.0) && !warned, "one live of three: {r:?}");
    }

    /// A one-channel file mixes nothing and has no ratio to report.
    #[test]
    fn a_mono_file_reports_one_channel_and_no_mix() {
        let (r, warned, p) = ratio_of(&[1.0]);
        assert_eq!(
            (p.channels, p.mix, r, warned),
            (1, AppliedMix::Mono, None, false)
        );
    }

    /// One channel, or each, can be decoded on its own; a channel the file does
    /// not have is an error. Counterpart: the default mean is bit for bit
    /// what `decode_file` has always returned.
    #[test]
    fn a_chosen_channel_or_each_channel_is_decoded_on_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lr.wav");
        write_multichannel(&path, &[1.0, -0.5]);
        let mean = decode_file(&path).unwrap();
        let (left, pl) = decode_file_with_provenance(&path, ChannelMix::LEFT).unwrap();
        let (right, pr) = decode_file_with_provenance(&path, ChannelMix::RIGHT).unwrap();
        assert_eq!(
            (pl.mix, pr.mix),
            (AppliedMix::Channel(0), AppliedMix::Channel(1))
        );
        let peak = |a: &AudioData| a.samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((peak(&left) - 0.5).abs() < 1e-3, "left is the full tone");
        assert!(
            (peak(&right) - 0.25).abs() < 1e-3,
            "right is half of it, inverted"
        );
        assert!(left.samples[12] * right.samples[12] < 0.0, "in antiphase");
        assert!((peak(&mean) - 0.125).abs() < 1e-3, "the mean of the two");
        assert!(decode_file_with_provenance(&path, ChannelMix::Channel(2)).is_err());

        let (each, pe) = decode_file_each_channel(&path).unwrap();
        assert_eq!(pe.mix, AppliedMix::Each);
        assert_eq!(each.len(), 2);
        assert_eq!(each[0].samples, left.samples);
        assert_eq!(each[1].samples, right.samples);

        let (by_default, _) = decode_file_with_provenance(&path, ChannelMix::default()).unwrap();
        assert_eq!(by_default.samples.len(), mean.samples.len());
        assert!(
            by_default
                .samples
                .iter()
                .zip(&mean.samples)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "the default mix is decode_file's"
        );
    }

    #[test]
    fn decode_nonexistent_file_returns_error() {
        let result = decode_file(&PathBuf::from("/nonexistent/file.wav"));
        assert!(result.is_err());
    }

    #[test]
    fn decode_file_capped_bounds_sample_count() {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        for _ in 0..10_000 {
            writer.write_sample(0_i16).unwrap();
        }
        writer.finalize().unwrap();

        // Uncapped: every sample is decoded.
        assert_eq!(decode_file(&path).unwrap().samples.len(), 10_000);
        // Capped: decoding stops at exactly the cap.
        assert_eq!(
            decode_file_capped(&path, 1_000).unwrap().samples.len(),
            1_000
        );
        // A cap above the clip length is a no-op (no truncation, no panic).
        assert_eq!(
            decode_file_capped(&path, 50_000).unwrap().samples.len(),
            10_000
        );
    }

    /// Write a 1 s, 48 kHz mono WAV and then overwrite the `data` chunk's
    /// length field (and the RIFF size to match) with `claimed_len`, as a
    /// crash-truncated segment (header
    /// written with the expected size up front) or an ffmpeg stream killed
    /// before it could back-patch its `0xFFFFFFFF` placeholder would look.
    fn write_wav_with_claimed_data_len(path: &Path, claimed_len: u32) {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for i in 0..48_000_i32 {
            #[allow(clippy::cast_possible_truncation)]
            writer.write_sample(((i % 200) - 100) as i16).unwrap();
        }
        writer.finalize().unwrap();
        let mut bytes = std::fs::read(path).unwrap();
        let pos = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("data chunk present");
        let actual = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
        assert_eq!(actual, 96_000, "fixture: 48 000 16-bit samples");
        bytes[pos + 4..pos + 8].copy_from_slice(&claimed_len.to_le_bytes());
        // Keep the RIFF size consistent with the claim, as both writers do:
        // `SegmentWriter` writes `data + 36`, ffmpeg writes `-1` to both.
        let riff = if claimed_len == u32::MAX {
            u32::MAX
        } else {
            claimed_len + 36
        };
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn decode_wav_claiming_more_data_than_exists_keeps_its_samples() {
        let dir = tempfile::tempdir().unwrap();
        // Header claims 15 s of 16-bit mono at 48 kHz; only 1 s is on disk.
        let fifteen = dir.path().join("claims_15s.wav");
        write_wav_with_claimed_data_len(&fifteen, 15 * 48_000 * 2);
        let audio = decode_file(&fifteen).expect("truncated WAV decodes what exists");
        assert_eq!(audio.samples.len(), 48_000);
        assert_eq!(audio.sample_rate, 48_000);
    }

    #[test]
    fn decode_wav_with_unpatched_placeholder_length_keeps_its_samples() {
        let dir = tempfile::tempdir().unwrap();
        // ffmpeg's streaming placeholder, never back-patched.
        let placeholder = dir.path().join("placeholder.wav");
        write_wav_with_claimed_data_len(&placeholder, 0xFFFF_FFFF);
        let audio = decode_file(&placeholder).expect("placeholder-length WAV decodes");
        assert_eq!(audio.samples.len(), 48_000);
    }

    /// The counterpart: a file that ends before *any* audio is still an error,
    /// not an empty recording analysed as silence.
    #[test]
    fn decode_wav_with_header_but_no_audio_is_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("header_only.wav");
        write_wav_with_claimed_data_len(&path, 15 * 48_000 * 2);
        let bytes = std::fs::read(&path).unwrap();
        let pos = bytes.windows(4).position(|w| w == b"data").unwrap();
        std::fs::write(&path, &bytes[..pos + 8]).unwrap();
        assert!(decode_file(&path).is_err());
    }

    /// Samples per MPEG-1 Layer III frame.
    const MP3_FRAME_SAMPLES: usize = 1152;

    /// Write `frames` silent MPEG-1 Layer III frames (128 kbit/s, 48 kHz,
    /// mono: 384 bytes each, all-zero side information and main data). When
    /// `corrupt` names a frame, its first granule's `big_values` is set to
    /// 511, which no valid stream carries (the limit is 288): the demuxer
    /// still reads the packet, and the decoder refuses it with a
    /// `DecodeError`, as a damaged frame in an uploaded MP3 would be.
    fn write_silent_mp3(path: &Path, frames: usize, corrupt: Option<usize>) {
        const FRAME_LEN: usize = 384;
        let mut bytes = Vec::with_capacity(frames * FRAME_LEN);
        for i in 0..frames {
            let mut frame = [0_u8; FRAME_LEN];
            frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x94, 0xC0]);
            if corrupt == Some(i) {
                // Side information starts after the 4-byte header; for mono
                // MPEG-1, `big_values` is bits 30..=38 of it.
                frame[4 + 3] |= 0x03;
                frame[4 + 4] |= 0xFE;
            }
            bytes.extend_from_slice(&frame);
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// The counterpart: the clean fixture decodes to every frame, so the
    /// corrupted case below is measured against a length the decoder really
    /// produces for this stream.
    #[test]
    fn decode_clean_mp3_fixture_yields_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clean.mp3");
        write_silent_mp3(&path, 40, None);
        let audio = decode_file(&path).expect("clean fixture decodes");
        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(audio.samples.len(), 40 * MP3_FRAME_SAMPLES);
    }

    /// ACC-13: a packet the decoder refuses must still occupy its time.
    /// Dropping it moves every later sample earlier, so detections after the
    /// damage are stamped, clipped and corroborated a packet too early.
    #[test]
    fn decode_mp3_with_a_corrupt_mid_file_frame_keeps_its_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("damaged.mp3");
        write_silent_mp3(&path, 40, Some(20));
        let audio = decode_file(&path).expect("one bad frame does not fail the file");
        assert_eq!(
            audio.samples.len(),
            40 * MP3_FRAME_SAMPLES,
            "a refused packet must be replaced by silence of its own length, \
             not dropped"
        );
    }

    /// The seam for containers that do not say how long a packet is: no time
    /// base or a zero duration is `None` (counted as dropped, never guessed),
    /// and a known duration converts through the time base to frames.
    #[test]
    fn lost_packet_frames_converts_known_durations_and_refuses_unknown_ones() {
        assert_eq!(
            lost_packet_frames(1152, Some((1, 48_000)), 48_000),
            Some(1152)
        );
        // A millisecond time base: 24 ms at 48 kHz is 1152 frames.
        assert_eq!(lost_packet_frames(24, Some((1, 1000)), 48_000), Some(1152));
        assert_eq!(lost_packet_frames(1152, None, 48_000), None);
        assert_eq!(lost_packet_frames(0, Some((1, 48_000)), 48_000), None);
    }

    #[test]
    fn probe_duration_secs_reads_length_without_decoding() {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        // 96_000 frames at 48 kHz = exactly 2.0 seconds.
        for _ in 0..96_000 {
            writer.write_sample(0_i16).unwrap();
        }
        writer.finalize().unwrap();

        let secs = probe_duration_secs(&path).expect("WAV duration is known");
        assert!((secs - 2.0).abs() < 1e-6, "expected 2.0s, got {secs}");
    }

    #[test]
    fn probe_duration_secs_none_for_unreadable_or_non_audio() {
        // Missing file → None (honest: never a guessed duration).
        assert!(probe_duration_secs(&PathBuf::from("/nonexistent/clip.wav")).is_none());
        // A file that isn't decodable audio → None, not a panic.
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("notaudio.wav");
        std::fs::write(&junk, b"not a real wav file").unwrap();
        assert!(probe_duration_secs(&junk).is_none());
    }
}
