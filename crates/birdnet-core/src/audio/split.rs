//! Split a long recording into station-length pieces, one packet at a time
//! (review F11, RES-05).
//!
//! The detection path decodes a file whole, which is right for a segment and
//! wrong for a night's recording from a card ([`super::admission`] refuses
//! those). This is the other way in: the recording is read packet by packet
//! and each packet's samples are handed to a [`PieceSink`] and let go, so what
//! is held at once is one packet (at most [`MAX_PACKET_FRAMES`]), never the
//! recording and never even a whole piece. [`SplitReport::peak_held_samples`]
//! says how much that was, so a test can hold it to the bound.
//!
//! # Time
//!
//! A piece is placed on the recording's own timeline: the container's
//! timestamps where it has a time base (WAV, FLAC and MP3 all do), the frames
//! decoded so far where it has none. Nothing here reads a clock; the caller
//! turns [`PieceStart::offset_frames`] into an instant from a start it was
//! given.
//!
//! # Discontinuities
//!
//! A packet the decoder refuses, or a jump in the container's timestamps, is
//! not papered over. The live decoder puts silence in place of a refused
//! packet so a segment's later detections keep their times; here the piece
//! simply ends, and the next one starts where the next audio really is
//! ([`PieceCause`] says why), so no piece is a continuous timeline that the
//! recording was not. Where the length of what was lost is unknown and the
//! container has no timestamps to recover from, every later piece's start is
//! [`Timing::Uncertain`].

use std::fmt;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use symphonia::core::io::MediaSource;

use super::decode::DecodeError;
use crate::config::validate::SEGMENT_DURATION_MAX;

/// The most frames one decoded packet may hold.
///
/// A larger one is refused ([`SplitError::PacketTooLarge`]) rather than
/// allocated: no codec this
/// station reads puts more than 65 535 frames in a packet (FLAC's ceiling;
/// symphonia reads WAV 1152 at a time), so a packet past this is a damaged or
/// hostile file, and taking it would make the bound a promise about the file
/// rather than about this code.
pub const MAX_PACKET_FRAMES: usize = 1 << 18;

/// Whether a piece's start is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    /// Placed by the container's timestamps, or by every frame before it.
    Exact,
    /// After audio of unknown length was lost in a container without
    /// timestamps: the start is late by at least what was lost.
    Uncertain,
}

/// Why a piece began where it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceCause {
    /// The first audio in the recording.
    RecordingStart,
    /// The piece before it was full.
    Continues,
    /// The decoder refused one or more packets just before it. `lost_frames`
    /// is their length where the container gave it.
    AfterLoss {
        /// Frames refused, where known.
        lost_frames: Option<u64>,
    },
    /// The container's timestamps jump: `gap_frames` of the timeline have no
    /// packet (negative: packets overlap).
    AfterGap {
        /// The jump, in frames.
        gap_frames: i64,
    },
}

/// Where a piece starts on the recording's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceStart {
    /// The piece's number in the recording, from 0.
    pub index: u64,
    /// Its first frame, counted from the recording's start.
    pub offset_frames: u64,
    /// The recording's rate, in Hz.
    pub sample_rate: u32,
    /// Whether [`Self::offset_frames`] is known.
    pub timing: Timing,
    /// Why it starts here.
    pub cause: PieceCause,
}

impl PieceStart {
    /// [`Self::offset_frames`] in whole milliseconds, rounded down.
    #[must_use]
    pub fn offset_ms(&self) -> u64 {
        let ms = u128::from(self.offset_frames) * 1000 / u128::from(self.sample_rate.max(1));
        u64::try_from(ms).unwrap_or(u64::MAX)
    }
}

/// A finished piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceEnd {
    /// Where it started.
    pub start: PieceStart,
    /// Decoded frames it holds; never more than the piece length, and never
    /// any silence this code made up.
    pub frames: u64,
}

/// Where the pieces go.
pub trait PieceSink {
    /// A piece begins.
    ///
    /// # Errors
    ///
    /// Whatever stops the sink taking it; the split stops.
    fn begin(&mut self, start: &PieceStart) -> Result<(), SplitError>;
    /// Mono samples for the open piece, in order.
    ///
    /// # Errors
    ///
    /// As [`Self::begin`].
    fn write(&mut self, mono: &[f32]) -> Result<(), SplitError>;
    /// The open piece is complete.
    ///
    /// # Errors
    ///
    /// As [`Self::begin`].
    fn end(&mut self, piece: &PieceEnd) -> Result<(), SplitError>;
}

/// What a split read, for the whole recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplitReport {
    /// The recording's rate, in Hz.
    pub sample_rate: u32,
    /// Pieces handed to the sink.
    pub pieces: u64,
    /// Frames decoded, all of which went into pieces.
    pub decoded_frames: u64,
    /// Frames made up as silence: always 0. A discontinuity ends the piece
    /// instead; the field is here so a reader of the report does not have to
    /// take that on trust from this comment.
    pub zero_filled_frames: u64,
    /// Packets the decoder refused.
    pub lost_packets: u64,
    /// Their frames, where the container gave a length.
    pub lost_frames: u64,
    /// Of [`Self::lost_packets`], those of unknown length.
    pub lost_packets_of_unknown_length: u64,
    /// Jumps in the container's timestamps.
    pub gaps: u64,
    /// The most samples held at once by the split's own buffers (their
    /// capacity, so what was allocated, not only what was used).
    pub peak_held_samples: usize,
}

/// Why a split stopped.
#[derive(Debug)]
pub enum SplitError {
    /// The recording could not be read.
    Decode(DecodeError),
    /// A packet held more than [`MAX_PACKET_FRAMES`].
    PacketTooLarge {
        /// Its frames.
        frames: usize,
    },
    /// The piece length is outside 1..=[`SEGMENT_DURATION_MAX`] seconds.
    PieceLength(u32),
    /// The sink refused a piece.
    Sink(String),
}

impl fmt::Display for SplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "cannot read the recording: {e}"),
            Self::PacketTooLarge { frames } => write!(
                f,
                "a packet holds {frames} frames, more than the {MAX_PACKET_FRAMES} any \
                 recording this station reads puts in one; refused rather than allocated"
            ),
            Self::PieceLength(secs) => write!(
                f,
                "a piece of {secs} s is outside 1..={SEGMENT_DURATION_MAX} s, the lengths a \
                 station segment may have"
            ),
            Self::Sink(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for SplitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl From<DecodeError> for SplitError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

/// Split the recording at `path` into pieces of `piece_secs` seconds.
///
/// # Errors
///
/// See [`SplitError`]. Pieces already handed to the sink stay handed.
pub fn split_file(
    path: &Path,
    piece_secs: u32,
    sink: &mut dyn PieceSink,
) -> Result<SplitReport, SplitError> {
    let file = std::fs::File::open(path).map_err(DecodeError::Io)?;
    split_source(
        Box::new(file),
        path.extension().and_then(|e| e.to_str()),
        piece_secs,
        sink,
    )
}

/// A timestamp in time-base units, as frames at `rate`, rounded.
fn ts_frames(ts: i64, (numer, denom): (u32, u32), rate: u32) -> i64 {
    let num = i128::from(ts) * i128::from(numer) * i128::from(rate);
    let den = i128::from(denom.max(1));
    i64::try_from((num + den / 2).div_euclid(den)).unwrap_or(i64::MAX)
}

/// Whether a packet at `pts` breaks the timeline that ended at `expected`
/// (both in time-base units), and by how many units.
const fn discontinuity(expected: Option<i64>, pts: i64) -> Option<i64> {
    match expected {
        Some(e) if e != pts => Some(pts.saturating_sub(e)),
        _ => None,
    }
}

/// The split's state between packets.
struct Splitter<'s> {
    sink: &'s mut dyn PieceSink,
    piece_frames: u64,
    sample_rate: u32,
    open: Option<PieceEnd>,
    next_index: u64,
    next_cause: PieceCause,
    timing: Timing,
    report: SplitReport,
}

impl Splitter<'_> {
    fn finish_piece(&mut self) -> Result<(), SplitError> {
        if let Some(piece) = self.open.take() {
            self.sink.end(&piece)?;
            self.report.pieces += 1;
        }
        Ok(())
    }

    /// Hand `mono`, which starts at timeline frame `at`, to the sink.
    fn write(&mut self, mono: &[f32], at: u64) -> Result<(), SplitError> {
        let mut written = 0_usize;
        while written < mono.len() {
            if self.open.is_none() {
                let start = PieceStart {
                    index: self.next_index,
                    offset_frames: at + written as u64,
                    sample_rate: self.sample_rate,
                    timing: self.timing,
                    cause: self.next_cause,
                };
                self.sink.begin(&start)?;
                self.open = Some(PieceEnd { start, frames: 0 });
                self.next_index += 1;
                self.next_cause = PieceCause::Continues;
            }
            let Some(piece) = self.open.as_mut() else {
                break;
            };
            let room = usize::try_from(self.piece_frames - piece.frames).unwrap_or(usize::MAX);
            let take = room.min(mono.len() - written);
            self.sink.write(&mono[written..written + take])?;
            piece.frames += take as u64;
            written += take;
            if piece.frames == self.piece_frames {
                self.finish_piece()?;
            }
        }
        Ok(())
    }

    /// A discontinuity: the open piece ends, and the next starts with `cause`.
    fn break_at(&mut self, cause: PieceCause) -> Result<(), SplitError> {
        self.finish_piece()?;
        // A loss followed by a jump is reported as the loss; the first cause
        // is what a reader needs to find the damage.
        if !matches!(
            self.next_cause,
            PieceCause::AfterLoss { .. } | PieceCause::AfterGap { .. }
        ) {
            self.next_cause = cause;
        }
        Ok(())
    }
}

/// [`split_file`] for any source symphonia can read; `ext` is the extension
/// hint for the probe.
///
/// # Errors
///
/// As [`split_file`].
#[allow(
    clippy::cast_precision_loss,
    reason = "a channel count as the divisor of a mono mix"
)]
pub fn split_source(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    piece_secs: u32,
    sink: &mut dyn PieceSink,
) -> Result<SplitReport, SplitError> {
    use symphonia::core::audio::GenericAudioBufferRef;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    if piece_secs == 0 || u64::from(piece_secs) > SEGMENT_DURATION_MAX {
        return Err(SplitError::PieceLength(piece_secs));
    }
    let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::Format(e.to_string()))?;
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
        .filter(|&r| r > 0)
        .ok_or_else(|| DecodeError::Format("unknown sample rate".into()))?;
    let time_base = track.time_base.map(|tb| (tb.numer.get(), tb.denom.get()));
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| DecodeError::Format(e.to_string()))?;

    let mut s = Splitter {
        sink,
        piece_frames: u64::from(piece_secs) * u64::from(sample_rate),
        sample_rate,
        open: None,
        next_index: 0,
        next_cause: PieceCause::RecordingStart,
        timing: Timing::Exact,
        report: SplitReport {
            sample_rate,
            ..SplitReport::default()
        },
    };
    let mut interleaved: Vec<f32> = Vec::new();
    let mut mono: Vec<f32> = Vec::new();
    // The container's timeline: where the next packet should start (in its
    // time-base units), and the frame the next audio lands on.
    let mut expected_ts: Option<i64> = None;
    let mut position: u64 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            // As the live decoder: a header claiming more than the file
            // holds ends the recording at the last whole packet.
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof && s.report.decoded_frames > 0 =>
            {
                break;
            }
            Err(e) => return Err(DecodeError::Format(e.to_string()).into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let pts = packet.pts.get();
        let dur = packet.dur.get();
        if let Some(tb) = time_base {
            if let Some(jump) = discontinuity(expected_ts, pts) {
                s.report.gaps += 1;
                s.break_at(PieceCause::AfterGap {
                    gap_frames: ts_frames(jump, tb, sample_rate),
                })?;
            }
            position = u64::try_from(ts_frames(pts, tb, sample_rate)).unwrap_or(0);
            expected_ts = (dur > 0)
                .then(|| i64::try_from(dur).ok().and_then(|d| pts.checked_add(d)))
                .flatten();
        }

        match decoder.decode(&packet) {
            Ok(buf) => {
                let buf: GenericAudioBufferRef<'_> = buf;
                let channels = buf.num_planes();
                let frames = buf.frames();
                if channels == 0 || frames == 0 {
                    continue;
                }
                if frames > MAX_PACKET_FRAMES {
                    return Err(SplitError::PacketTooLarge { frames });
                }
                interleaved.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut interleaved[..]);
                mono.clear();
                mono.extend(
                    interleaved
                        .chunks_exact(channels)
                        .map(|f| f.iter().sum::<f32>() / channels as f32),
                );
                s.report.peak_held_samples = s
                    .report
                    .peak_held_samples
                    .max(interleaved.capacity() + mono.capacity());
                s.write(&mono, position)?;
                position += frames as u64;
                s.report.decoded_frames += frames as u64;
            }
            Err(SymphoniaError::DecodeError(_)) => {
                s.report.lost_packets += 1;
                let lost_frames = time_base.filter(|_| dur > 0).and_then(|tb| {
                    u64::try_from(ts_frames(i64::try_from(dur).ok()?, tb, sample_rate)).ok()
                });
                match lost_frames {
                    Some(n) => s.report.lost_frames += n,
                    None => s.report.lost_packets_of_unknown_length += 1,
                }
                // Without timestamps the next audio's place is the frames
                // counted so far, which no longer include what was lost.
                if time_base.is_none() {
                    s.timing = Timing::Uncertain;
                }
                s.break_at(PieceCause::AfterLoss { lost_frames })?;
            }
            Err(e) => return Err(DecodeError::Format(e.to_string()).into()),
        }
    }
    s.finish_piece()?;
    Ok(s.report)
}

/// One piece written as a mono 32-bit float WAV at the recording's rate.
///
/// Float, so the samples are the mono mix the live decoder would make of the
/// recording, unrounded. Written under a name ending in [`WRITING_SUFFIX`],
/// which no watcher takes for audio, and renamed to its own on
/// [`Self::commit`], so a watch directory never sees it half-written.
pub struct WavPieceFile {
    writer: Option<hound::WavWriter<BufWriter<std::fs::File>>>,
    part: PathBuf,
    dest: PathBuf,
}

/// What [`WavPieceFile`] appends to a piece's name while it is written.
///
/// Not [`crate::atomic_file::part_path`], which keeps `.wav` last for tools that
/// read the extension, and so would be taken for a finished segment.
pub const WRITING_SUFFIX: &str = ".writing";

impl WavPieceFile {
    /// Start writing `dest`.
    ///
    /// # Errors
    ///
    /// The file cannot be created.
    pub fn create(dest: &Path, sample_rate: u32) -> std::io::Result<Self> {
        let mut name = dest.as_os_str().to_owned();
        name.push(WRITING_SUFFIX);
        let part = PathBuf::from(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let file = std::fs::File::create(&part)?;
        let writer = hound::WavWriter::new(BufWriter::new(file), spec).map_err(hound_io)?;
        Ok(Self {
            writer: Some(writer),
            part,
            dest: dest.to_path_buf(),
        })
    }

    /// Append samples.
    ///
    /// # Errors
    ///
    /// The write fails.
    pub fn write(&mut self, mono: &[f32]) -> std::io::Result<()> {
        let Some(w) = self.writer.as_mut() else {
            return Err(std::io::Error::other("piece already committed"));
        };
        for &s in mono {
            w.write_sample(s).map_err(hound_io)?;
        }
        Ok(())
    }

    /// Finish the file, make it durable, and give it its name.
    ///
    /// # Errors
    ///
    /// Any step fails; the partial file is then removed.
    pub fn commit(mut self) -> std::io::Result<PathBuf> {
        if let Some(w) = self.writer.take() {
            w.finalize().map_err(hound_io)?;
        }
        crate::atomic_file::commit(&self.part, &self.dest)?;
        Ok(self.dest.clone())
    }
}

impl fmt::Debug for WavPieceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WavPieceFile")
            .field("part", &self.part)
            .field("dest", &self.dest)
            .field("committed", &self.writer.is_none())
            .finish()
    }
}

impl Drop for WavPieceFile {
    fn drop(&mut self) {
        // Not committed (or the commit failed before the rename): no
        // half-written piece is left behind.
        if self.part.exists() {
            drop(self.writer.take());
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

fn hound_io(e: hound::Error) -> std::io::Error {
    match e {
        hound::Error::IoError(e) => e,
        other => std::io::Error::other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decode::tests::{MP3_FRAME_SAMPLES, write_silent_mp3};
    use std::io::{Read, Seek, SeekFrom};

    /// Records what it is handed and keeps no samples, so a split of hours
    /// costs nothing here; it checks every piece stays within its length.
    #[derive(Default)]
    struct Recorder {
        piece_frames: u64,
        pieces: Vec<PieceEnd>,
        open_frames: u64,
        sum: f64,
    }

    impl PieceSink for Recorder {
        fn begin(&mut self, _: &PieceStart) -> Result<(), SplitError> {
            assert_eq!(
                self.open_frames, 0,
                "a piece begins only after the last ended"
            );
            Ok(())
        }
        fn write(&mut self, mono: &[f32]) -> Result<(), SplitError> {
            self.open_frames += mono.len() as u64;
            assert!(
                self.open_frames <= self.piece_frames,
                "a piece past its length"
            );
            self.sum += mono.iter().map(|&s| f64::from(s)).sum::<f64>();
            Ok(())
        }
        fn end(&mut self, piece: &PieceEnd) -> Result<(), SplitError> {
            assert_eq!(piece.frames, self.open_frames);
            self.open_frames = 0;
            self.pieces.push(*piece);
            Ok(())
        }
    }

    /// A 16-bit mono WAV generated as it is read: `frames` of a ramp, behind
    /// a header that says so. Hours of it occupy nothing until read, and are
    /// never all in memory unless the reader puts them there.
    struct LazyWav {
        header: [u8; 44],
        data_len: u64,
        pos: u64,
    }

    impl LazyWav {
        fn new(rate: u32, frames: u64) -> Self {
            let data_len = frames * 2;
            let data32 = u32::try_from(data_len).unwrap();
            let mut h = [0_u8; 44];
            h[0..4].copy_from_slice(b"RIFF");
            h[4..8].copy_from_slice(&(data32 + 36).to_le_bytes());
            h[8..16].copy_from_slice(b"WAVEfmt ");
            h[16..20].copy_from_slice(&16_u32.to_le_bytes());
            h[20..22].copy_from_slice(&1_u16.to_le_bytes()); // PCM
            h[22..24].copy_from_slice(&1_u16.to_le_bytes()); // mono
            h[24..28].copy_from_slice(&rate.to_le_bytes());
            h[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
            h[32..34].copy_from_slice(&2_u16.to_le_bytes());
            h[34..36].copy_from_slice(&16_u16.to_le_bytes());
            h[36..40].copy_from_slice(b"data");
            h[40..44].copy_from_slice(&data32.to_le_bytes());
            Self {
                header: h,
                data_len,
                pos: 0,
            }
        }
        fn len(&self) -> u64 {
            44 + self.data_len
        }
    }

    impl Read for LazyWav {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut n = 0;
            while n < buf.len() && self.pos < self.len() {
                buf[n] = if self.pos < 44 {
                    self.header[usize::try_from(self.pos).unwrap()]
                } else {
                    let i = (self.pos - 44) / 2;
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_possible_wrap,
                        reason = "a ramp below 200 as a test sample"
                    )]
                    let sample = ((i % 200) as i16 - 100) * 100;
                    sample.to_le_bytes()[usize::try_from((self.pos - 44) % 2).unwrap()]
                };
                n += 1;
                self.pos += 1;
            }
            Ok(n)
        }
    }

    impl Seek for LazyWav {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            let target = match to {
                SeekFrom::Start(p) => i128::from(p),
                SeekFrom::Current(d) => i128::from(self.pos) + i128::from(d),
                SeekFrom::End(d) => i128::from(self.len()) + i128::from(d),
            };
            self.pos = u64::try_from(target).map_err(|_| std::io::Error::other("before start"))?;
            Ok(self.pos)
        }
    }

    impl MediaSource for LazyWav {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.len())
        }
    }

    /// Two hours at 4 kHz, 28.8 M frames, split into 15 s pieces: every frame
    /// arrives, every piece is whole and consecutive, and what the split held
    /// at once is a packet's worth, not the recording.
    #[test]
    fn a_two_hour_recording_is_split_holding_one_packet_at_a_time() {
        const RATE: u32 = 4_000;
        const FRAMES: u64 = 2 * 3600 * 4_000;
        let mut rec = Recorder {
            piece_frames: 15 * u64::from(RATE),
            ..Recorder::default()
        };
        let report = split_source(
            Box::new(LazyWav::new(RATE, FRAMES)),
            Some("wav"),
            15,
            &mut rec,
        )
        .expect("splits");
        assert_eq!(report.decoded_frames, FRAMES);
        assert_eq!(report.pieces, 480);
        assert_eq!(rec.pieces.len(), 480);
        for (i, p) in rec.pieces.iter().enumerate() {
            let i = i as u64;
            assert_eq!(p.start.index, i);
            assert_eq!(p.start.offset_frames, i * 60_000, "consecutive");
            assert_eq!(p.frames, 60_000);
            assert_eq!(p.start.timing, Timing::Exact);
        }
        assert_eq!(rec.pieces[0].start.cause, PieceCause::RecordingStart);
        assert_eq!(rec.pieces[1].start.cause, PieceCause::Continues);
        assert!(
            report.peak_held_samples <= 4 * 1152,
            "held {} samples at once, against {FRAMES} in the recording",
            report.peak_held_samples
        );
        assert_eq!(report.zero_filled_frames, 0);
    }

    /// Review F11: a packet the decoder refuses ends the piece, and the next
    /// piece starts where the next audio really is. The live decoder's
    /// silence would have made one continuous piece the recording was not.
    #[test]
    fn a_lost_packet_starts_a_new_piece_at_the_next_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("damaged.mp3");
        write_silent_mp3(&path, 40, Some(20));
        let mut rec = Recorder {
            piece_frames: 600 * 48_000,
            ..Recorder::default()
        };
        let report = split_file(&path, 600, &mut rec).expect("splits");
        let f = MP3_FRAME_SAMPLES as u64;
        assert_eq!(rec.pieces.len(), 2, "{:?}", rec.pieces);
        assert_eq!(rec.pieces[0].start.offset_frames, 0);
        assert_eq!(rec.pieces[0].frames, 20 * f);
        assert_eq!(
            rec.pieces[1].start.offset_frames,
            21 * f,
            "after the lost packet"
        );
        assert_eq!(rec.pieces[1].frames, 19 * f);
        assert_eq!(
            rec.pieces[1].start.cause,
            PieceCause::AfterLoss {
                lost_frames: Some(f)
            }
        );
        assert_eq!(rec.pieces[1].start.timing, Timing::Exact);
        assert_eq!(report.lost_packets, 1);
        assert_eq!(report.lost_frames, f);
        assert_eq!(report.decoded_frames, 39 * f);
        assert_eq!(report.zero_filled_frames, 0);
    }

    /// The counterpart: the same stream undamaged is one piece of every frame.
    #[test]
    fn an_undamaged_recording_is_not_broken() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clean.mp3");
        write_silent_mp3(&path, 40, None);
        let mut rec = Recorder {
            piece_frames: 600 * 48_000,
            ..Recorder::default()
        };
        let report = split_file(&path, 600, &mut rec).unwrap();
        assert_eq!(rec.pieces.len(), 1);
        assert_eq!(rec.pieces[0].frames, 40 * MP3_FRAME_SAMPLES as u64);
        assert_eq!((report.lost_packets, report.gaps), (0, 0));
    }

    struct ToDisk<'a> {
        dir: &'a Path,
        open: Option<WavPieceFile>,
        written: Vec<(PieceStart, PathBuf)>,
    }
    impl PieceSink for ToDisk<'_> {
        fn begin(&mut self, start: &PieceStart) -> Result<(), SplitError> {
            let dest = self.dir.join(format!("piece-{}.wav", start.index));
            self.open = Some(
                WavPieceFile::create(&dest, start.sample_rate)
                    .map_err(|e| SplitError::Sink(e.to_string()))?,
            );
            assert!(!dest.exists(), "not under its own name until committed");
            Ok(())
        }
        fn write(&mut self, mono: &[f32]) -> Result<(), SplitError> {
            self.open
                .as_mut()
                .unwrap()
                .write(mono)
                .map_err(|e| SplitError::Sink(e.to_string()))
        }
        fn end(&mut self, piece: &PieceEnd) -> Result<(), SplitError> {
            let path = self
                .open
                .take()
                .unwrap()
                .commit()
                .map_err(|e| SplitError::Sink(e.to_string()))?;
            self.written.push((piece.start, path));
            Ok(())
        }
    }

    /// A WAV written as pieces reads back, piece after piece, as the samples
    /// the live decoder makes of the whole recording, and a piece boundary
    /// that falls inside a packet splits it.
    #[test]
    fn pieces_written_to_disk_read_back_as_the_recording() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 1_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&src, spec).unwrap();
        for i in 0..10_000_i32 {
            #[allow(clippy::cast_possible_truncation, reason = "test samples within i16")]
            {
                w.write_sample(((i % 300) * 50) as i16).unwrap();
                w.write_sample((-(i % 70) * 90) as i16).unwrap();
            }
        }
        w.finalize().unwrap();

        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let mut sink = ToDisk {
            dir: &out,
            open: None,
            written: Vec::new(),
        };
        let report = split_file(&src, 3, &mut sink).unwrap();
        assert_eq!(report.pieces, 4);
        let offsets: Vec<u64> = sink.written.iter().map(|(s, _)| s.offset_frames).collect();
        assert_eq!(offsets, [0, 3_000, 6_000, 9_000]);
        assert_eq!(sink.written[2].0.offset_ms(), 6_000);

        let whole = crate::audio::decode::decode_file(&src).unwrap().samples;
        let mut joined = Vec::new();
        for (_, p) in &sink.written {
            let piece = crate::audio::decode::decode_file(p).unwrap();
            assert_eq!(piece.sample_rate, 1_000);
            joined.extend(piece.samples);
        }
        assert_eq!(joined, whole, "the pieces are the recording, unrounded");
        let leftovers: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .filter_map(|e| {
                let n = e.ok()?.file_name().to_string_lossy().into_owned();
                n.ends_with(WRITING_SUFFIX).then_some(n)
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// A streaming placeholder header says nothing about length; the split
    /// takes the audio that is there.
    #[test]
    fn a_placeholder_length_header_splits_what_is_there() {
        let mut wav = LazyWav::new(8_000, 8_000);
        wav.header[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        wav.header[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut rec = Recorder {
            piece_frames: 15 * 8_000,
            ..Recorder::default()
        };
        let report = split_source(Box::new(wav), Some("wav"), 15, &mut rec).unwrap();
        assert_eq!(report.decoded_frames, 8_000);
        assert_eq!(rec.pieces.len(), 1);
    }

    #[test]
    fn a_piece_longer_than_a_segment_may_be_is_refused() {
        for secs in [0, 601] {
            let mut rec = Recorder::default();
            assert!(matches!(
                split_source(Box::new(LazyWav::new(8_000, 10)), Some("wav"), secs, &mut rec),
                Err(SplitError::PieceLength(s)) if s == secs
            ));
        }
    }

    /// The timeline test the splitter breaks on: any difference between where
    /// a packet should start and where it does, either way.
    #[test]
    fn a_timestamp_jump_either_way_is_a_discontinuity() {
        assert_eq!(discontinuity(None, 500), None, "the first packet");
        assert_eq!(discontinuity(Some(1152), 1152), None);
        assert_eq!(discontinuity(Some(1152), 2304), Some(1152));
        assert_eq!(discontinuity(Some(2304), 1152), Some(-1152));
        assert_eq!(ts_frames(24, (1, 1000), 48_000), 1152);
        assert_eq!(ts_frames(-1152, (1, 48_000), 48_000), -1152);
    }
}
