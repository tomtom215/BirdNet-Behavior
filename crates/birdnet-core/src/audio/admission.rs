//! Whether a file in the watch directory is a live segment the detection path
//! may decode whole (review F11).
//!
//! The detection pipeline decodes a recording into one buffer, resamples a copy
//! and copies every window out of it. That is right for a station segment,
//! which `SEGMENT_DURATION` bounds at [`SEGMENT_DURATION_MAX`], and wrong for
//! anything else: the watch directory accepts any file whose name parses, so a
//! recording of a night dropped there by hand was decoded whole, and on a Pi
//! that is the memory. A long recording belongs to the ingestion command
//! ([`INGEST_COMMAND`]), which splits it into segments without holding more than
//! one packet.
//!
//! [`admit_for_live_analysis`] reads only the container's header, so it refuses
//! before any sample is decoded. A header that does not say how long the file
//! is (a WAV still being streamed, its length the `0xFFFFFFFF` placeholder)
//! cannot be refused by length; it is admitted with a sample cap
//! ([`Admitted::sample_cap`]) for the decode to stop at.

use std::fmt;
use std::path::Path;

use super::decode::DecodeError;
use crate::config::validate::SEGMENT_DURATION_MAX;

/// How far past [`SEGMENT_DURATION_MAX`] a live file may run, in seconds.
///
/// A segment is cut on the clock, and a writer that is late to close one (a
/// stalled tee, a slow card, `arecord`'s own rounding) leaves it a little long.
/// A minute is sixty times the longest overrun this station's writers have
/// been seen to leave, and still far from the hours a hand-dropped recording
/// holds.
pub const LIVE_ADMISSION_MARGIN_SECS: u64 = 60;

/// The longest file, in seconds, the detection path decodes whole:
/// [`SEGMENT_DURATION_MAX`] plus [`LIVE_ADMISSION_MARGIN_SECS`].
pub const LIVE_MAX_FILE_SECS: u64 = SEGMENT_DURATION_MAX + LIVE_ADMISSION_MARGIN_SECS;

/// The command that takes a recording longer than [`LIVE_MAX_FILE_SECS`].
pub const INGEST_COMMAND: &str = "birdnet-behavior --ingest-recordings";

/// A file the detection path may decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Admitted {
    /// The length its header gives, bounded by the bytes on disk for PCM.
    /// `None` when the header does not say.
    pub claimed_secs: Option<f64>,
    /// The rate the header gives, in Hz.
    pub sample_rate: u32,
}

impl Admitted {
    /// The most mono samples a decode of this file should hold:
    /// [`LIVE_MAX_FILE_SECS`] at its own rate. For
    /// [`super::decode::decode_file_capped`], so a file whose header did not
    /// say how long it is still cannot take more than a segment's memory.
    #[must_use]
    pub fn sample_cap(&self) -> usize {
        usize::try_from(u64::from(self.sample_rate).saturating_mul(LIVE_MAX_FILE_SECS))
            .unwrap_or(usize::MAX)
    }
}

/// Why a file was not admitted.
#[derive(Debug)]
pub enum AdmissionError {
    /// The header could not be read.
    Unreadable(DecodeError),
    /// The file is longer than a live segment may be.
    TooLong {
        /// The length its header gives, in seconds.
        claimed_secs: f64,
        /// [`LIVE_MAX_FILE_SECS`].
        limit_secs: u64,
    },
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "cannot read the recording's header: {e}"),
            Self::TooLong {
                claimed_secs,
                limit_secs,
            } => write!(
                f,
                "the recording is {claimed_secs:.0} s long, and the detection path analyses \
                 at most {limit_secs} s in one file (the longest SEGMENT_DURATION, \
                 {SEGMENT_DURATION_MAX} s, and a {LIVE_ADMISSION_MARGIN_SECS} s margin); \
                 nothing was decoded. Split it into station segments with `{INGEST_COMMAND} \
                 <file> --ingest-source <name> --ingest-start <time>`"
            ),
        }
    }
}

impl std::error::Error for AdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable(e) => Some(e),
            Self::TooLong { .. } => None,
        }
    }
}

/// Whether the detection path may decode `path` whole, judged from its header
/// alone.
///
/// # Errors
///
/// [`AdmissionError::TooLong`] for a file longer than [`LIVE_MAX_FILE_SECS`],
/// and [`AdmissionError::Unreadable`] when the header cannot be read.
#[allow(clippy::cast_precision_loss)]
pub fn admit_for_live_analysis(path: &Path) -> Result<Admitted, AdmissionError> {
    use symphonia::core::codecs::audio::well_known::{CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S32LE};
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::meta::MetadataOptions;

    let unreadable = |e: String| AdmissionError::Unreadable(DecodeError::Format(e));
    let file =
        std::fs::File::open(path).map_err(|e| AdmissionError::Unreadable(DecodeError::Io(e)))?;
    let file_len = file.metadata().ok().map(|m| m.len());
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
        .map_err(|e| unreadable(e.to_string()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(AdmissionError::Unreadable(DecodeError::NoTracks))?;
    let audio = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| unreadable("track has no audio codec params".into()))?;
    let sample_rate = audio
        .sample_rate
        .filter(|&r| r > 0)
        .ok_or_else(|| unreadable("unknown sample rate".into()))?;

    // For PCM, the bytes on disk bound the frames: a header a crash left
    // claiming its expected length, or more, is not believed past them.
    let pcm_bound = if (CODEC_ID_PCM_S32LE..=CODEC_ID_PCM_MULAW).contains(&audio.codec) {
        let bytes_per_frame = audio
            .bits_per_coded_sample
            .or(audio.bits_per_sample)
            .zip(
                audio
                    .channels
                    .as_ref()
                    .map(symphonia::core::audio::Channels::count),
            )
            .map(|(bits, ch)| u64::from(bits.div_ceil(8)) * ch as u64)
            .filter(|&b| b > 0);
        bytes_per_frame.zip(file_len).map(|(b, len)| len / b)
    } else {
        None
    };
    let frames = match (track.num_frames, pcm_bound) {
        (Some(claimed), Some(bound)) => Some(claimed.min(bound)),
        (claimed, None) => claimed,
        (None, Some(_)) => None,
    };
    let claimed_secs = frames.map(|n| n as f64 / f64::from(sample_rate));
    if let Some(secs) = claimed_secs
        && secs > LIVE_MAX_FILE_SECS as f64
    {
        return Err(AdmissionError::TooLong {
            claimed_secs: secs,
            limit_secs: LIVE_MAX_FILE_SECS,
        });
    }
    Ok(Admitted {
        claimed_secs,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    /// A silent 16-bit mono WAV of `secs` seconds at a low rate, so a
    /// recording past the limit costs about a megabyte.
    fn silent_wav(path: &Path, rate: u32, secs: u64) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        let mut frames = w.get_i16_writer(u32::try_from(u64::from(rate) * secs).unwrap());
        for _ in 0..u64::from(rate) * secs {
            frames.write_sample(0);
        }
        frames.flush().unwrap();
        w.finalize().unwrap();
    }

    /// Overwrite the `data` length (and the RIFF size) with `claimed`.
    fn claim_data_len(path: &Path, claimed: u32) {
        let mut bytes = std::fs::read(path).unwrap();
        let pos = bytes.windows(4).position(|w| w == b"data").unwrap();
        bytes[pos + 4..pos + 8].copy_from_slice(&claimed.to_le_bytes());
        let riff = if claimed == u32::MAX {
            u32::MAX
        } else {
            claimed + 36
        };
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_recording_longer_than_a_segment_may_be_is_refused_and_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-05-19-birdnet-04:00:00.wav");
        silent_wav(&path, 1_000, LIVE_MAX_FILE_SECS + 1);
        let err = admit_for_live_analysis(&path).expect_err("past the limit is refused");
        let AdmissionError::TooLong {
            claimed_secs,
            limit_secs,
        } = err
        else {
            panic!("refused for its length, not {err}");
        };
        assert_eq!(limit_secs, 660);
        assert!((claimed_secs - 661.0).abs() < 1e-6, "{claimed_secs}");
        let text = err_text(&path);
        assert!(text.contains(INGEST_COMMAND), "names the command: {text}");
        assert!(text.contains("nothing was decoded"), "{text}");
    }

    fn err_text(path: &Path) -> String {
        admit_for_live_analysis(path).unwrap_err().to_string()
    }

    /// The counterpart: the longest segment the station writes, and the limit
    /// itself, are admitted, with the length the header gives.
    #[test]
    fn a_segment_up_to_the_limit_is_admitted() {
        let dir = tempfile::tempdir().unwrap();
        for secs in [15, SEGMENT_DURATION_MAX, LIVE_MAX_FILE_SECS] {
            let path = dir.path().join(format!("s{secs}.wav"));
            silent_wav(&path, 1_000, secs);
            let a = admit_for_live_analysis(&path).expect("admitted");
            assert_eq!(a.sample_rate, 1_000);
            #[allow(clippy::cast_precision_loss)]
            let want = secs as f64;
            assert!(
                a.claimed_secs.is_some_and(|c| (c - want).abs() < 1e-6),
                "{secs}: {a:?}"
            );
            assert_eq!(a.sample_cap(), 660_000);
        }
    }

    /// A segment a crash cut short keeps the length its writer declared up
    /// front (hours, for a header that was never patched with the real one).
    /// The bytes on disk bound a PCM file's length, so it is admitted, and
    /// the decode keeps what is there.
    #[test]
    fn a_pcm_header_claiming_more_than_the_file_holds_is_bounded_by_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.wav");
        silent_wav(&path, 48_000, 1);
        claim_data_len(&path, 3 * 3600 * 48_000 * 2);
        let a = admit_for_live_analysis(&path).expect("admitted: one second is on disk");
        assert!(
            a.claimed_secs.is_some_and(|c| c < 2.0),
            "bounded by its bytes: {a:?}"
        );
    }

    /// A streaming placeholder says nothing about length: admitted, with the
    /// cap a decode of it stops at.
    #[test]
    fn a_placeholder_length_is_admitted_with_a_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("streaming.wav");
        silent_wav(&path, 48_000, 1);
        claim_data_len(&path, u32::MAX);
        let a = admit_for_live_analysis(&path).expect("admitted");
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.sample_cap(), 48_000 * 660);
    }

    #[test]
    fn a_file_that_is_not_audio_is_unreadable_not_admitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.wav");
        std::fs::write(&path, b"not a wav").unwrap();
        assert!(matches!(
            admit_for_live_analysis(&path),
            Err(AdmissionError::Unreadable(_))
        ));
    }
}
