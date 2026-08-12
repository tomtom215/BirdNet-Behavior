//! Audio capture configuration types.
//!
//! Defines `CaptureSource`, `RecordingConfig`, and `AudioFormat`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use super::live::LiveAudioHubHandle;
use crate::civil::CivilTime;

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

impl CaptureSource {
    /// The label that identifies this source everywhere it is named: the
    /// `birdnet_audio_source_up{source}` gauge, the published capture status,
    /// the live-audio tap key, and the `-birdnet-<id>-` field of every segment
    /// filename.
    ///
    /// RTSP streams keep their `stream_id`; a local microphone uses its
    /// `stream_id` when it has one and otherwise collapses to `local`, which is
    /// what the detection side recovers from an id-less filename. Defined here,
    /// beside the filename helpers, because the label and the filename are the
    /// same identity viewed from two directions — the supervisor's gauge label
    /// and the detection pipeline's per-source filter have to agree or every
    /// frame from the source is silently discarded.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Rtsp { stream_id, .. } => stream_id.clone(),
            Self::Microphone { stream_id, .. } | Self::PipeWire { stream_id, .. } => {
                stream_id.clone().unwrap_or_else(|| "local".to_owned())
            }
        }
    }
}

/// The station's UTC offset in seconds (east-positive; CEST is `7200`), shared
/// live between the capture supervisor that maintains it and the segment writer
/// that stamps filenames with it.
///
/// # Why this is shared mutable state rather than a plain number
///
/// Segment filenames carry **local** civil time, because that is what
/// `arecord --use-strftime` wrote and what every downstream consumer — the
/// detection `Date`/`Time` columns above all — has always assumed. A snapshot
/// taken when capture starts would be wrong for the rest of a deployment the
/// moment daylight saving changed: a station started in February would still be
/// naming files in CET in August. The supervisor refreshes this on every tick,
/// so the offset the writer stamps is never more than one tick stale.
///
/// Learning the offset is deliberately *not* this type's job. The workspace has
/// no date/time crate and forbids `unsafe`, so the value is obtained where a
/// SQLite connection is available and pushed in here.
#[derive(Debug, Clone)]
pub struct LocalOffset(Arc<AtomicI64>);

/// Widest real-world UTC offset, in seconds (UTC+14, Line Islands). Values
/// outside ±this are clamped: a garbage offset would silently mis-stamp every
/// recording, and no plausible source of one is worth honouring.
const MAX_UTC_OFFSET_SECS: i64 = 14 * 3600;

impl LocalOffset {
    /// A shared offset seeded to `secs`.
    #[must_use]
    pub fn new(secs: i64) -> Self {
        let offset = Self(Arc::new(AtomicI64::new(0)));
        offset.set(secs);
        offset
    }

    /// A shared offset seeded to UTC.
    ///
    /// The right seed for tests and for tooling that never writes segments; the
    /// binary replaces it with the real offset before capture starts and keeps
    /// it current thereafter.
    #[must_use]
    pub fn utc() -> Self {
        Self::new(0)
    }

    /// Publish a new offset, clamped to a real-world range.
    pub fn set(&self, secs: i64) {
        self.0.store(
            secs.clamp(-MAX_UTC_OFFSET_SECS, MAX_UTC_OFFSET_SECS),
            Ordering::Relaxed,
        );
    }

    /// The current offset in seconds.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for LocalOffset {
    fn default() -> Self {
        Self::utc()
    }
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
    /// The station's live UTC offset, used to stamp segment filenames with
    /// local civil time. See [`LocalOffset`] for why it is shared rather than
    /// copied.
    pub local_offset: LocalOffset,
    /// Where a teed microphone source publishes its live PCM, so `/stream` can
    /// serve live audio without opening the (exclusive) capture device a second
    /// time. `None` disables the tap — capture still records normally; there is
    /// simply nothing for `/stream` to subscribe to. That is the right setting
    /// for tooling and tests, not for the daemon.
    pub live_audio: Option<LiveAudioHubHandle>,
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

/// Expand `recording_filename`'s `strftime` pattern for a concrete **local**
/// civil time.
///
/// The in-process segment writer names its own files, so this is the third
/// member of a set that must never disagree: `recording_filename` is the
/// `strftime` pattern `arecord`/`ffmpeg` are handed, `filename_matches_stream`
/// is the matcher the stall probe uses, and this is the expansion the tee
/// writes.
/// A test substitutes a timestamp into that pattern and asserts it equals this
/// function's output, so "byte-identical to what `arecord --use-strftime`
/// produced" is a checked property rather than a claim.
///
/// Public because the invariant it anchors is cross-crate: the binary's
/// `derive_source_label` and the detection pipeline both parse these names, and
/// they should be able to prove it against the real formatter rather than
/// against a string someone typed into a test.
pub fn recording_filename_at(
    stream_id: Option<&str>,
    format: AudioFormat,
    at: CivilTime,
) -> String {
    let ext = format.extension();
    let CivilTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    } = at;
    let date = format!("{year:04}-{month:02}-{day:02}");
    let time = format!("{hour:02}:{minute:02}:{second:02}");
    stream_id.map_or_else(
        || format!("{date}-birdnet-{time}.{ext}"),
        |id| format!("{date}-birdnet-{id}-{time}.{ext}"),
    )
}

/// Whether a watch-directory filename was written by the capture stream
/// identified by `stream_id` (`None` = the id-less single local mic).
///
/// The inverse of [`recording_filename`] — kept beside it so the writer
/// pattern and the matcher cannot drift apart. Used by the supervisor's
/// silent-stall probe to find the newest segment belonging to one source in
/// a directory shared by several.
pub(crate) fn filename_matches_stream(name: &str, stream_id: Option<&str>, ext: &str) -> bool {
    let Some(stem) = name.strip_suffix(ext).and_then(|s| s.strip_suffix('.')) else {
        return false;
    };
    stream_id.map_or_else(
        // "…-birdnet-HH:MM:SS": what follows the marker must be a bare time,
        // otherwise this is some other stream's id-carrying file.
        || {
            stem.rfind("-birdnet-").is_some_and(|idx| {
                let rest = &stem.as_bytes()[idx + "-birdnet-".len()..];
                rest.len() == 8 && rest[2] == b':' && rest[5] == b':'
            })
        },
        // "…-birdnet-{id}-HH:MM:SS"
        |id| stem.contains(&format!("-birdnet-{id}-")),
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

    #[test]
    fn filename_matcher_identifies_idless_local_mic() {
        // The id-less single-mic file matches only the `None` stream.
        let name = "2026-06-10-birdnet-07:15:00.wav";
        assert!(filename_matches_stream(name, None, "wav"));
        assert!(!filename_matches_stream(name, Some("RTSP_1"), "wav"));
        // Wrong extension never matches.
        assert!(!filename_matches_stream(name, None, "flac"));
    }

    #[test]
    fn filename_matcher_identifies_stream_ids() {
        let cam1 = "2026-06-10-birdnet-RTSP_1-07:15:00.wav";
        assert!(filename_matches_stream(cam1, Some("RTSP_1"), "wav"));
        // An id'd file must NOT match the id-less stream nor a sibling id.
        assert!(!filename_matches_stream(cam1, None, "wav"));
        assert!(!filename_matches_stream(cam1, Some("RTSP_2"), "wav"));
        // DB-row ids (admin-UI sources) work the same way.
        let row = "2026-06-10-birdnet-back-garden-cam-07:15:00.wav";
        assert!(filename_matches_stream(row, Some("back-garden-cam"), "wav"));
        assert!(!filename_matches_stream(row, None, "wav"));
    }

    #[test]
    fn filename_matcher_round_trips_with_recording_filename() {
        // Substitute a concrete timestamp into the strftime pattern and the
        // matcher must accept exactly its own stream.
        let pattern = recording_filename(Some("MIC_2"), AudioFormat::Wav);
        let concrete = pattern
            .replace("%Y-%m-%d", "2026-06-10")
            .replace("%H:%M:%S", "12:34:56");
        assert!(filename_matches_stream(&concrete, Some("MIC_2"), "wav"));
        assert!(!filename_matches_stream(&concrete, None, "wav"));

        let idless = recording_filename(None, AudioFormat::Wav)
            .replace("%Y-%m-%d", "2026-06-10")
            .replace("%H:%M:%S", "12:34:56");
        assert!(filename_matches_stream(&idless, None, "wav"));
        assert!(!filename_matches_stream(&idless, Some("MIC_2"), "wav"));
    }

    #[test]
    fn filename_matcher_rejects_non_recordings() {
        assert!(!filename_matches_stream("notes.txt", None, "wav"));
        assert!(!filename_matches_stream(".wav", None, "wav"));
        assert!(!filename_matches_stream("2026-06-10.wav", None, "wav"));
    }

    // ---- recording_filename_at (the tee's own naming) ----------------------

    /// 2026-08-12 14:03:15, the local time a CEST station would stamp at
    /// 12:03:15 UTC.
    const SAMPLE: CivilTime = CivilTime {
        year: 2026,
        month: 8,
        day: 12,
        hour: 14,
        minute: 3,
        second: 15,
    };

    /// Substitute a concrete timestamp into the `strftime` pattern the capture
    /// subprocesses are handed, exactly as `strftime` would.
    fn expand_pattern(pattern: &str, at: CivilTime) -> String {
        pattern
            .replace(
                "%Y-%m-%d",
                &format!("{:04}-{:02}-{:02}", at.year, at.month, at.day),
            )
            .replace(
                "%H:%M:%S",
                &format!("{:02}:{:02}:{:02}", at.hour, at.minute, at.second),
            )
    }

    /// The load-bearing equivalence: the tee's own filenames are byte-identical
    /// to the expansion of the pattern `arecord --use-strftime` was given. If
    /// these two ever drift, `RecordingFile::parse` and the stall probe start
    /// disagreeing with the writer and detections silently stop landing.
    #[test]
    fn formatter_matches_the_strftime_pattern_it_replaces() {
        for (id, format) in [
            (None, AudioFormat::Wav),
            (Some("src_seed_1"), AudioFormat::Wav),
            (Some("RTSP_2"), AudioFormat::Flac),
            (Some("back-garden-cam"), AudioFormat::Wav),
        ] {
            let pattern = recording_filename(id, format);
            assert_eq!(
                recording_filename_at(id, format, SAMPLE),
                expand_pattern(&pattern, SAMPLE),
                "formatter and pattern disagree for {id:?}"
            );
        }
    }

    #[test]
    fn formatter_output_parses_back_and_matches_its_own_stream() {
        let name = recording_filename_at(Some("src_seed_1"), AudioFormat::Wav, SAMPLE);
        assert_eq!(name, "2026-08-12-birdnet-src_seed_1-14:03:15.wav");
        // The stall probe must find it…
        assert!(filename_matches_stream(&name, Some("src_seed_1"), "wav"));
        assert!(!filename_matches_stream(&name, None, "wav"));
        // …and the detection side must parse the same date/time back out.
        let parsed = crate::detection::types::RecordingFile::parse(&name).expect("parses");
        assert_eq!(parsed.date, "2026-08-12");
        assert_eq!(parsed.time, "14:03:15");
        assert_eq!(parsed.rtsp_id.as_deref(), Some("src_seed_1"));
    }

    #[test]
    fn formatter_zero_pads_every_field() {
        let early = CivilTime {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        };
        assert_eq!(
            recording_filename_at(None, AudioFormat::Wav, early),
            "2026-01-02-birdnet-03:04:05.wav"
        );
        // Unpadded fields would break the fixed-width assumptions in
        // `filename_matches_stream` and `RecordingFile::parse`.
        assert!(filename_matches_stream(
            &recording_filename_at(None, AudioFormat::Wav, early),
            None,
            "wav"
        ));
    }

    // ---- CaptureSource::label ---------------------------------------------

    #[test]
    fn label_matches_the_filename_identity() {
        let mic = CaptureSource::Microphone {
            device: "plughw:1,0".into(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: None,
        };
        assert_eq!(mic.label(), "local");

        let mic_id = CaptureSource::Microphone {
            device: "plughw:1,0".into(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: Some("src_seed_1".into()),
        };
        assert_eq!(mic_id.label(), "src_seed_1");

        let pw = CaptureSource::PipeWire {
            device: String::new(),
            sample_rate: 48_000,
            channels: 1,
            stream_id: Some("src_pw".into()),
        };
        assert_eq!(pw.label(), "src_pw");

        let rtsp = CaptureSource::Rtsp {
            url: "rtsp://cam/feed".into(),
            stream_id: "RTSP_1".into(),
            transport: RtspTransport::Auto,
        };
        assert_eq!(rtsp.label(), "RTSP_1");
    }

    // ---- LocalOffset -------------------------------------------------------

    #[test]
    fn local_offset_is_shared_between_clones() {
        // The supervisor writes; the segment writer reads. They must see the
        // same cell, or a DST change would never reach the filenames.
        let a = LocalOffset::utc();
        let b = a.clone();
        assert_eq!(b.get(), 0);
        a.set(2 * 3600);
        assert_eq!(b.get(), 7200, "a clone must observe the publisher's update");
    }

    #[test]
    fn local_offset_clamps_absurd_values() {
        let o = LocalOffset::new(i64::MAX);
        assert_eq!(o.get(), MAX_UTC_OFFSET_SECS);
        o.set(i64::MIN);
        assert_eq!(o.get(), -MAX_UTC_OFFSET_SECS);
        // Real offsets pass through untouched, including the extremes.
        o.set(-12 * 3600);
        assert_eq!(o.get(), -43_200);
        o.set(14 * 3600);
        assert_eq!(o.get(), 50_400);
        // Half-hour and quarter-hour zones (India, Nepal, Chatham) survive.
        o.set(5 * 3600 + 45 * 60);
        assert_eq!(o.get(), 20_700);
    }

    #[test]
    fn local_offset_defaults_to_utc() {
        assert_eq!(LocalOffset::default().get(), 0);
    }
}
