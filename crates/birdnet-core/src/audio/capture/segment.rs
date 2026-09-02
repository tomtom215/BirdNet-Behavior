//! Rotating WAV segment writer for the in-process capture tee.
//!
//! When capture stops asking `arecord` to segment for us (see
//! [`super::tee`] for why), this module takes over the job `arecord
//! --max-file-time --use-strftime` used to do: split a continuous PCM stream
//! into fixed-length WAV files whose names carry the local wall-clock time the
//! segment began.
//!
//! # The filenames are load-bearing
//!
//! Everything downstream keys off the filename, not off file metadata:
//!
//! * [`crate::detection::types::RecordingFile::parse`] reads the detection's `Date`
//!   and `Time` out of it — the DB timestamps are the filename's timestamps;
//! * the source label the metrics gauge and the per-source pipeline filter use
//!   is recovered from the `-birdnet-<id>-` segment of the name;
//! * the supervisor's silent-stall probe finds "this source's newest segment"
//!   by matching the name ([`super::types::filename_matches_stream`]).
//!
//! So the names this module produces are not cosmetic and are not merely
//! "compatible": they are byte-for-byte what the `strftime` pattern in
//! [`super::types::recording_filename`] expands to, in **local** time, which is
//! what `arecord --use-strftime` produced. A test pins the two against each
//! other so the writer and the pattern cannot drift apart.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::PathBuf;

use super::live::PcmSpec;
use super::types::{AudioFormat, LocalOffset, recording_filename_at};
use crate::civil::civil_from_unix_secs;

/// Bytes in the canonical 44-byte PCM WAV header this module writes.
const WAV_HEADER_BYTES: usize = 44;

/// Byte offset of the RIFF chunk's size field within the header.
const RIFF_SIZE_OFFSET: u64 = 4;

/// Byte offset of the `data` chunk's size field within the header.
const DATA_SIZE_OFFSET: u64 = 40;

/// Build the canonical 44-byte little-endian PCM WAV header.
///
/// `data_bytes` is the length of the PCM that follows. Pure, so the exact bytes
/// are unit-testable without touching a filesystem.
fn wav_header(spec: PcmSpec, data_bytes: u32) -> [u8; WAV_HEADER_BYTES] {
    let channels = spec.channels;
    let bits_per_sample = u16::try_from(super::live::BYTES_PER_SAMPLE * 8).unwrap_or(16);
    let block_align = u16::try_from(spec.bytes_per_frame()).unwrap_or(2);
    let byte_rate = u32::try_from(spec.bytes_per_second()).unwrap_or(u32::MAX);

    let mut h = [0u8; WAV_HEADER_BYTES];
    h[0..4].copy_from_slice(b"RIFF");
    // Everything after this field: 4 ("WAVE") + 24 (fmt chunk) + 8 (data
    // header) + the PCM itself.
    h[4..8].copy_from_slice(&data_bytes.saturating_add(36).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&spec.sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&bits_per_sample.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// The wall clock a [`SegmentWriter`] stamps filenames from.
///
/// Injected so rotation tests can pin timestamps instead of racing the real
/// clock (and so a test can prove the *local* offset is actually applied).
#[derive(Debug)]
pub enum SegmentClock {
    /// The system clock.
    System,
    /// A test clock that reads a shared timestamp and then advances it by one
    /// second. Advancing matters: filenames have one-second resolution, so a
    /// truly frozen clock would give consecutive segments the same name and
    /// each would overwrite the last — an artefact of the fixture, not of the
    /// writer. (In the field a 15-second segment cannot collide with its
    /// predecessor.)
    #[cfg(test)]
    Ticking(std::sync::Arc<std::sync::atomic::AtomicI64>),
}

impl SegmentClock {
    /// Current UTC Unix timestamp in seconds.
    fn now_unix(&self) -> i64 {
        match self {
            Self::System => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
            #[cfg(test)]
            Self::Ticking(t) => t.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// The segment currently being written.
#[derive(Debug)]
struct OpenSegment {
    file: File,
    path: PathBuf,
    /// PCM bytes written so far, excluding the header.
    data_bytes: u64,
}

/// Splits a continuous PCM stream into rotating, correctly-headered WAV files.
///
/// Rotation is driven by the **byte count**, not by the wall clock: a segment
/// ends after exactly `segment_duration_secs` worth of samples. That is what
/// makes "no sample is lost at a rotation" a property the writer can be held to
/// — every byte handed to [`Self::write`] lands in exactly one segment, and the
/// segments concatenate back to the original stream.
#[derive(Debug)]
pub struct SegmentWriter {
    output_dir: PathBuf,
    stream_id: Option<String>,
    format: AudioFormat,
    spec: PcmSpec,
    /// PCM bytes per segment. Always at least one frame, so rotation always
    /// makes progress.
    bytes_per_segment: u64,
    local_offset: LocalOffset,
    clock: SegmentClock,
    current: Option<OpenSegment>,
}

impl SegmentWriter {
    /// A writer that will drop `segment_duration_secs`-long WAV files into
    /// `output_dir`.
    pub fn new(
        output_dir: PathBuf,
        stream_id: Option<String>,
        format: AudioFormat,
        spec: PcmSpec,
        segment_duration_secs: u32,
        local_offset: LocalOffset,
        clock: SegmentClock,
    ) -> Self {
        // A zero-length segment would rotate forever without consuming input,
        // so the floor is one frame. `segment_duration_secs` is operator-facing
        // configuration and is not otherwise bounded here.
        let bytes_per_segment = (u64::from(segment_duration_secs)
            .saturating_mul(spec.bytes_per_second() as u64))
        .max(spec.bytes_per_frame() as u64)
        .max(1);
        Self {
            output_dir,
            stream_id,
            format,
            spec,
            bytes_per_segment,
            local_offset,
            clock,
            current: None,
        }
    }

    /// The filename this writer would open right now.
    ///
    /// Local civil time, from the UTC clock plus the supervisor-maintained
    /// offset — the same lens `arecord --use-strftime` used (it called
    /// `strftime` on `localtime()`) and the same lens the rest of the station
    /// stores detection dates in.
    fn next_filename(&self) -> String {
        let local = self
            .clock
            .now_unix()
            .saturating_add(self.local_offset.get());
        recording_filename_at(
            self.stream_id.as_deref(),
            self.format,
            civil_from_unix_secs(local),
        )
    }

    /// Open the next segment, writing its header.
    fn open(&mut self) -> io::Result<()> {
        let path = self.output_dir.join(self.next_filename());
        // A missing output directory is a real field condition, not a
        // programming error: the tmpfs the recordings live on can be
        // re-mounted, and the disk manager prunes inside it. Recreate it once
        // rather than losing the source until the next supervisor restart.
        let mut file = match File::create(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir_all(&self.output_dir)?;
                File::create(&path)?
            }
            Err(e) => return Err(e),
        };
        // Declare the size the segment is *expected* to reach, then correct it
        // on close. This is what `arecord` does with a length it knows up
        // front, and it means a segment killed mid-write still carries a
        // plausible header instead of one claiming two gigabytes of audio.
        let expected = u32::try_from(self.bytes_per_segment).unwrap_or(u32::MAX);
        file.write_all(&wav_header(self.spec, expected))?;
        tracing::debug!(path = %path.display(), "capture segment opened");
        self.current = Some(OpenSegment {
            file,
            path,
            data_bytes: 0,
        });
        Ok(())
    }

    /// Finish the open segment: patch its header to the bytes actually written.
    ///
    /// Errors are logged, not propagated — a segment whose header could not be
    /// corrected is still a file full of audio, and failing capture over it
    /// would trade a slightly-wrong WAV for a dead station.
    fn close_current(&mut self) {
        let Some(mut seg) = self.current.take() else {
            return;
        };
        let data = u32::try_from(seg.data_bytes).unwrap_or(u32::MAX);
        let patched = seg
            .file
            .seek(SeekFrom::Start(RIFF_SIZE_OFFSET))
            .and_then(|_| seg.file.write_all(&data.saturating_add(36).to_le_bytes()))
            .and_then(|()| seg.file.seek(SeekFrom::Start(DATA_SIZE_OFFSET)))
            .and_then(|_| seg.file.write_all(&data.to_le_bytes()))
            .and_then(|()| seg.file.flush());
        match patched {
            Ok(()) => tracing::debug!(
                path = %seg.path.display(),
                data_bytes = seg.data_bytes,
                "capture segment closed"
            ),
            Err(e) => tracing::warn!(
                path = %seg.path.display(),
                error = %e,
                "could not finalise capture segment header; the audio is intact"
            ),
        }
    }

    /// Append `pcm` to the current segment, rotating as many times as the input
    /// spans.
    ///
    /// Every byte lands in exactly one segment. An error leaves the writer
    /// usable: the caller decides whether to keep going or tear the source down.
    pub fn write(&mut self, pcm: &[u8]) -> io::Result<()> {
        let mut rest = pcm;
        while !rest.is_empty() {
            if self.current.is_none() {
                self.open()?;
            }
            let written = {
                // Not `expect`: a loop that can only make progress through an
                // open segment must not be able to spin forever if one somehow
                // isn't there. Erroring surfaces as a source fault the
                // supervisor restarts; spinning would peg a core and record
                // nothing, silently, for as long as the station stayed up.
                let Some(seg) = self.current.as_mut() else {
                    return Err(io::Error::other(
                        "capture segment writer has no open segment after opening one",
                    ));
                };
                let room = self.bytes_per_segment.saturating_sub(seg.data_bytes);
                let take = usize::try_from(room).unwrap_or(usize::MAX).min(rest.len());
                seg.file.write_all(&rest[..take])?;
                seg.data_bytes += take as u64;
                take
            };
            rest = &rest[written..];
            if self
                .current
                .as_ref()
                .is_some_and(|s| s.data_bytes >= self.bytes_per_segment)
            {
                self.close_current();
            }
        }
        Ok(())
    }

    /// Close the segment in progress, if any. Idempotent.
    pub fn finish(&mut self) {
        self.close_current();
    }

    /// Path of the segment currently open, for assertions.
    #[cfg(test)]
    fn current_path(&self) -> Option<&std::path::Path> {
        self.current.as_ref().map(|s| s.path.as_path())
    }
}

impl Drop for SegmentWriter {
    /// A writer dropped without an explicit [`Self::finish`] — a panicking
    /// reader thread, say — still leaves a correctly-sized WAV behind.
    fn drop(&mut self) {
        self.close_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicI64;

    const MONO_48K: PcmSpec = PcmSpec {
        sample_rate: 48_000,
        channels: 1,
    };

    /// A tiny spec so segment sizes stay readable in assertions:
    /// 10 bytes/second, 2 bytes/frame.
    const TINY: PcmSpec = PcmSpec {
        sample_rate: 5,
        channels: 1,
    };

    /// 2026-08-12 12:03:15 UTC.
    const T0: i64 = 1_786_536_195;

    struct Fixture {
        dir: tempfile::TempDir,
        clock: Arc<AtomicI64>,
        offset: LocalOffset,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().expect("tempdir"),
                clock: Arc::new(AtomicI64::new(T0)),
                offset: LocalOffset::utc(),
            }
        }

        fn writer(&self, spec: PcmSpec, secs: u32) -> SegmentWriter {
            SegmentWriter::new(
                self.dir.path().to_path_buf(),
                Some("src_seed_1".to_owned()),
                AudioFormat::Wav,
                spec,
                secs,
                self.offset.clone(),
                SegmentClock::Ticking(Arc::clone(&self.clock)),
            )
        }

        /// Every segment file in the directory, sorted by name (which sorts
        /// chronologically, the timestamps being zero-padded).
        fn segments(&self) -> Vec<PathBuf> {
            let mut files: Vec<PathBuf> = std::fs::read_dir(self.dir.path())
                .expect("read_dir")
                .flatten()
                .map(|e| e.path())
                .collect();
            files.sort();
            files
        }
    }

    // ---- header ------------------------------------------------------------

    #[test]
    fn wav_header_is_the_canonical_44_bytes() {
        let h = wav_header(MONO_48K, 96_000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([h[4], h[5], h[6], h[7]]), 96_000 + 36);
        assert_eq!(&h[8..12], b"WAVE");
        assert_eq!(&h[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes([h[16], h[17], h[18], h[19]]), 16);
        assert_eq!(u16::from_le_bytes([h[20], h[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 1, "channels");
        assert_eq!(u32::from_le_bytes([h[24], h[25], h[26], h[27]]), 48_000);
        assert_eq!(
            u32::from_le_bytes([h[28], h[29], h[30], h[31]]),
            96_000,
            "byte rate"
        );
        assert_eq!(u16::from_le_bytes([h[32], h[33]]), 2, "block align");
        assert_eq!(u16::from_le_bytes([h[34], h[35]]), 16, "bits per sample");
        assert_eq!(&h[36..40], b"data");
        assert_eq!(u32::from_le_bytes([h[40], h[41], h[42], h[43]]), 96_000);
    }

    #[test]
    fn wav_header_tracks_channel_count() {
        let stereo = PcmSpec {
            sample_rate: 44_100,
            channels: 2,
        };
        let h = wav_header(stereo, 0);
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 2);
        assert_eq!(u16::from_le_bytes([h[32], h[33]]), 4, "block align");
        assert_eq!(
            u32::from_le_bytes([h[28], h[29], h[30], h[31]]),
            176_400,
            "byte rate"
        );
    }

    // ---- rotation ----------------------------------------------------------

    /// THE guarantee: what goes in comes out. Concatenating every segment's PCM
    /// payload must reproduce the input stream byte-for-byte, with no gap and
    /// no duplication at a rotation boundary.
    #[test]
    fn no_sample_is_lost_or_duplicated_across_rotations() {
        let fx = Fixture::new();
        // 10 bytes/second × 3 s = 30 bytes per segment.
        let mut w = fx.writer(TINY, 3);
        // A stream that is not a multiple of the segment size, fed in chunks
        // that are not multiples of it either — so writes straddle boundaries.
        let stream: Vec<u8> = (0..=250u8).collect();
        for chunk in stream.chunks(7) {
            w.write(chunk).expect("write");
        }
        w.finish();

        let mut recovered = Vec::new();
        for path in fx.segments() {
            let bytes = std::fs::read(&path).expect("read segment");
            assert!(
                bytes.len() >= WAV_HEADER_BYTES,
                "{} is shorter than a header",
                path.display()
            );
            recovered.extend_from_slice(&bytes[WAV_HEADER_BYTES..]);
        }
        assert_eq!(
            recovered, stream,
            "the segments must concatenate back to the captured stream"
        );
    }

    #[test]
    fn full_segments_are_exactly_one_period_long() {
        let fx = Fixture::new();
        let mut w = fx.writer(TINY, 3); // 30 bytes/segment
        for _ in 0..9 {
            w.write(&[b'x'; 10]).expect("write");
        }
        w.finish();
        let segments = fx.segments();
        assert_eq!(segments.len(), 3, "90 bytes / 30 = 3 segments");
        for path in &segments {
            let len = std::fs::metadata(path).expect("stat").len();
            assert_eq!(
                len,
                WAV_HEADER_BYTES as u64 + 30,
                "{} should hold exactly one segment of PCM",
                path.display()
            );
        }
    }

    #[test]
    fn header_is_patched_to_the_real_length_on_close() {
        let fx = Fixture::new();
        let mut w = fx.writer(TINY, 3); // expects 30 bytes
        w.write(&[1u8; 7]).expect("write"); // …but only 7 arrive
        w.finish();
        let path = &fx.segments()[0];
        let bytes = std::fs::read(path).expect("read");
        assert_eq!(bytes.len(), WAV_HEADER_BYTES + 7);
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            7,
            "the data chunk must describe what is actually there"
        );
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            7 + 36,
            "the RIFF size must be patched too"
        );
    }

    /// Counter-test for the patch: an *unpatched* header would still claim the
    /// full expected segment. Read the file before `finish()` to prove the
    /// declared size really is wrong until the writer corrects it — i.e. that
    /// the patch is doing work rather than rewriting the same bytes.
    #[test]
    fn an_unfinished_segment_declares_the_expected_length_not_the_real_one() {
        let fx = Fixture::new();
        let mut w = fx.writer(TINY, 3); // 30 bytes expected
        w.write(&[1u8; 7]).expect("write");
        let path = w.current_path().expect("a segment is open").to_path_buf();
        let bytes = std::fs::read(&path).expect("read while open");
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            30,
            "while writing, the header declares the expected segment length"
        );
        w.finish();
        let bytes = std::fs::read(&path).expect("read after close");
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            7
        );
    }

    #[test]
    fn dropping_the_writer_finalises_the_open_segment() {
        let fx = Fixture::new();
        let path = {
            let mut w = fx.writer(TINY, 3);
            w.write(&[9u8; 11]).expect("write");
            w.current_path().expect("open").to_path_buf()
        }; // dropped here, without finish()
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            11,
            "Drop must patch the header too — a panicking reader thread \
             must not leave a wrongly-sized WAV behind"
        );
    }

    #[test]
    fn writing_nothing_opens_no_file() {
        let fx = Fixture::new();
        let mut w = fx.writer(TINY, 3);
        w.write(&[]).expect("write");
        w.finish();
        assert!(fx.segments().is_empty());
    }

    // ---- filenames ---------------------------------------------------------

    #[test]
    fn segment_names_carry_the_stream_id_and_local_time() {
        let fx = Fixture::new();
        let mut w = fx.writer(TINY, 3);
        w.write(&[0u8; 4]).expect("write");
        w.finish();
        let name = fx.segments()[0]
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        // T0 is 2026-08-12 12:03:15 UTC and the fixture offset is UTC.
        assert_eq!(name, "2026-08-12-birdnet-src_seed_1-12:03:15.wav");
    }

    /// The offset must actually be applied — a station in CEST (UTC+2) names
    /// its files 14:03, not 12:03. This is the bug class that made the day
    /// strip and the sunrise markers disagree with the detections beside them.
    #[test]
    fn local_offset_moves_the_filename_stamp() {
        let fx = Fixture::new();
        fx.offset.set(2 * 3600); // CEST
        let mut w = fx.writer(TINY, 3);
        w.write(&[0u8; 4]).expect("write");
        w.finish();
        let name = fx.segments()[0]
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "2026-08-12-birdnet-src_seed_1-14:03:15.wav");
    }

    #[test]
    fn a_lone_microphone_keeps_the_id_less_name() {
        let fx = Fixture::new();
        let mut w = SegmentWriter::new(
            fx.dir.path().to_path_buf(),
            None,
            AudioFormat::Wav,
            TINY,
            3,
            fx.offset.clone(),
            SegmentClock::Ticking(Arc::clone(&fx.clock)),
        );
        w.write(&[0u8; 4]).expect("write");
        w.finish();
        let name = fx.segments()[0]
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "2026-08-12-birdnet-12:03:15.wav");
    }

    #[test]
    fn a_vanished_output_directory_is_recreated() {
        let fx = Fixture::new();
        let missing = fx.dir.path().join("gone");
        let mut w = SegmentWriter::new(
            missing.clone(),
            None,
            AudioFormat::Wav,
            TINY,
            3,
            fx.offset.clone(),
            SegmentClock::Ticking(Arc::clone(&fx.clock)),
        );
        // The directory does not exist yet — a re-mounted tmpfs, or the disk
        // manager having pruned it. Capture must recover, not die.
        w.write(&[0u8; 4]).expect("write into a missing directory");
        w.finish();
        assert_eq!(
            std::fs::read_dir(&missing).expect("recreated").count(),
            1,
            "the writer must recreate its output directory"
        );
    }

    // ---- the produced files really are WAVs --------------------------------

    /// Decode a produced segment with `hound` — an independent WAV reader — to
    /// prove the header this module hand-writes is not merely self-consistent.
    #[test]
    fn produced_segments_decode_as_wav() {
        let fx = Fixture::new();
        let mut w = fx.writer(MONO_48K, 1); // 96 000 bytes = 48 000 samples
        // Two full seconds of a recognisable ramp.
        let samples: Vec<i16> = (0..96_000i32).map(|i| (i % 3000) as i16).collect();
        let mut pcm = Vec::with_capacity(samples.len() * 2);
        for s in &samples {
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        w.write(&pcm).expect("write");
        w.finish();

        let segments = fx.segments();
        assert_eq!(segments.len(), 2, "two one-second segments");
        let mut decoded: Vec<i16> = Vec::new();
        for path in &segments {
            let mut reader = hound::WavReader::open(path).expect("hound opens the segment");
            let spec = reader.spec();
            assert_eq!(spec.sample_rate, 48_000);
            assert_eq!(spec.channels, 1);
            assert_eq!(spec.bits_per_sample, 16);
            assert_eq!(spec.sample_format, hound::SampleFormat::Int);
            let these: Vec<i16> = reader
                .samples::<i16>()
                .map(|s| s.expect("sample"))
                .collect();
            assert_eq!(these.len(), 48_000, "{} holds one second", path.display());
            decoded.extend_from_slice(&these);
        }
        assert_eq!(
            decoded, samples,
            "the samples must survive the round trip unchanged"
        );
    }

    /// …and through **the station's own** decode path, which is what the
    /// detection pipeline and the live spectrogram actually call. `hound`
    /// proves the header is well-formed; this proves symphonia — the decoder
    /// that has to read every segment for the rest of the deployment — agrees.
    #[test]
    fn produced_segments_decode_through_the_detection_pipeline_decoder() {
        let fx = Fixture::new();
        let mut w = fx.writer(MONO_48K, 1);
        // A 440 Hz-ish ramp; the exact waveform is irrelevant, the point is a
        // full second of non-silent, non-constant samples.
        let mut pcm = Vec::with_capacity(96_000);
        for i in 0..48_000i32 {
            let s = i16::try_from((i % 400) * 50).unwrap_or(i16::MAX);
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        w.write(&pcm).expect("write");
        w.finish();

        let path = &fx.segments()[0];
        let audio = crate::audio::decode::decode_file(path).expect("the station decoder reads it");
        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(
            audio.samples.len(),
            48_000,
            "one second of mono audio must decode to one second of samples"
        );
        assert!(
            audio.samples.iter().any(|s| s.abs() > 0.01),
            "the decoded audio must not be silence"
        );
        assert!(
            audio.samples.iter().all(|s| s.is_finite()),
            "no sample may decode to NaN/Inf"
        );
    }

    /// The supervisor's silent-stall probe finds a source's newest segment by
    /// name and mtime. It has to keep working now that this writer, rather than
    /// `arecord`, creates the files — otherwise a healthy station would look
    /// stalled and be restart-looped forever.
    #[test]
    fn segments_are_visible_to_the_supervisors_stall_probe() {
        use crate::audio::capture::{CaptureManager, CaptureSource, RecordingConfig};

        let fx = Fixture::new();
        let mut w = fx.writer(TINY, 3);
        w.write(&[7u8; 12]).expect("write");
        w.finish();

        let manager = CaptureManager::new(RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:1,0".into(),
                sample_rate: 48_000,
                channels: 1,
                channel_pick: None,
                stream_id: Some("src_seed_1".into()),
            },
            output_dir: fx.dir.path().to_path_buf(),
            segment_duration_secs: 3,
            format: AudioFormat::Wav,
            gain_db: 0.0,
            pipeline: crate::audio::capture::types::AudioPipeline::none(),
            eq_chain: crate::audio::eq::EqChain::default(),
            local_offset: LocalOffset::utc(),
            live_audio: None,
        });
        let age = manager
            .latest_output_age()
            .expect("the probe must see the segment this writer produced");
        assert!(
            age < std::time::Duration::from_secs(30),
            "a segment written just now must read as fresh, not stalled: {age:?}"
        );

        // …and it must be scoped to this source: another stream's segment is
        // invisible, which is what keeps one source's stall from masking
        // another's health.
        let other = CaptureManager::new(RecordingConfig {
            source: CaptureSource::Microphone {
                device: "plughw:2,0".into(),
                sample_rate: 48_000,
                channels: 1,
                channel_pick: None,
                stream_id: Some("src_seed_2".into()),
            },
            output_dir: fx.dir.path().to_path_buf(),
            segment_duration_secs: 3,
            format: AudioFormat::Wav,
            gain_db: 0.0,
            pipeline: crate::audio::capture::types::AudioPipeline::none(),
            eq_chain: crate::audio::eq::EqChain::default(),
            local_offset: LocalOffset::utc(),
            live_audio: None,
        });
        assert_eq!(
            other.latest_output_age(),
            None,
            "src_seed_1's segments must not make src_seed_2 look alive"
        );
    }
}
