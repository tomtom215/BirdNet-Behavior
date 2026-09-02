//! Reading a clip window that reaches past the segment it was detected in.
//!
//! # The defect this exists for
//!
//! A clip is centred on its detection: with the default 6-second extraction
//! and a 3-second detection window, 1.5 seconds either side. The extractor
//! took that from **inside the segment file**, clamping at the ends — so a
//! detection near a segment boundary silently produced a shorter clip with
//! the call itself cut off.
//!
//! Measured, before the fix, on a 15-second segment at 48 kHz with a
//! 6-second extraction:
//!
//! ```text
//! detection at  0.0s -> clip 4.500s     <- the whole lead-in, gone
//! detection at  1.0s -> clip 5.500s
//! detection at  1.5s -> clip 6.000s
//! detection at  7.5s -> clip 6.000s
//! detection at 11.0s -> clip 5.500s
//! detection at 12.0s -> clip 4.500s     <- the whole tail, gone
//! ```
//!
//! At zero overlap a 15-second segment holds five 3-second windows starting
//! at 0, 3, 6, 9 and 12 — so **two of every five** were affected. The failure
//! is invisible: the clip is a valid file of a plausible length, just missing
//! its beginning or its end. These are the clips a person plays to decide
//! whether an identification is right, and the ones uploaded to BirdWeather.
//!
//! # How it is fixed
//!
//! Segments are consecutive recordings of one continuous source, so the audio
//! either side of a boundary exists — in the neighbouring file. This resolves
//! a window against the segment *and its neighbours*, reading the tail of the
//! predecessor and the head of the successor when the window reaches them.
//!
//! # The guard that matters more than the fix
//!
//! **A neighbour is only used when it actually abuts.** A station restarts, a
//! source drops for a minute, a purge removes a file: splicing across a gap
//! would produce a clip containing audio from two different times, presented
//! as one continuous recording. That is not a shorter clip, it is a fabricated
//! one — and it would be undetectable afterwards. [`Neighbour::abuts`] is the
//! check, and it is deliberately strict: when the arithmetic does not work
//! out, the window is clamped exactly as before and the clip is short.
//!
//! # The neighbours are there to be read
//!
//! Not an assumption: the transient stream directory the daemon watches is
//! drained by age, at `STREAM_RETENTION_SECS` — 600 seconds by default (see
//! `crate`'s binary, `helpers::system`). At the default 15-second segment that
//! is about forty segments resident at any moment, so a segment's predecessor
//! is on disk for ten minutes after this one is processed.
//!
//! # Why the head is reliable and the tail is opportunistic
//!
//! The predecessor is finished and closed before this segment even started.
//! The successor is *being written* when a detection in this segment is
//! processed — the watcher fires on close, and a segment closes when the next
//! one opens — so on a station keeping up with real time it holds only the
//! fraction of a second recorded since.
//!
//! Reading it anyway is safe and worth doing. WAV PCM is append-only, so a
//! partial read yields a prefix of real audio rather than anything torn, and
//! taking 0.2 seconds of genuine tail beats taking none. When the file cannot
//! be decoded at all the neighbour is simply skipped. And the case that
//! benefits most is the one that matters most: a station working through a
//! backlog — a Pi at its slowest, whose clips are least likely to be reviewed
//! promptly — has *complete* successors on disk by the time it gets to them.

use std::path::{Path, PathBuf};

use crate::audio::decode::{AudioData, DecodeError, decode_file};
use crate::detection::types::RecordingFile;

/// How far apart two segment start times may be from the predecessor's actual
/// duration and still count as contiguous, in seconds.
///
/// Segment boundaries are not sample-exact: `arecord` and `ffmpeg` cut on a
/// period boundary, and a 15-second segment is routinely 14.98 or 15.02
/// seconds long. A quarter of a second absorbs that without coming close to
/// admitting a real gap, the smallest of which is a whole segment.
pub const CONTIGUITY_TOLERANCE_SECS: f32 = 0.25;

/// A neighbouring segment that has been read and found contiguous.
struct Neighbour {
    audio: AudioData,
}

impl Neighbour {
    /// Whether `earlier` (starting `earlier_start_secs` before `later`) runs
    /// exactly up to where `later` begins.
    ///
    /// `gap_secs` is the difference between the two segments' *start*
    /// timestamps, taken from their filenames; `earlier_duration_secs` is what
    /// the earlier file actually contains. They agree when the recording was
    /// continuous.
    fn abuts(gap_secs: f32, earlier_duration_secs: f32) -> bool {
        gap_secs > 0.0 && (gap_secs - earlier_duration_secs).abs() <= CONTIGUITY_TOLERANCE_SECS
    }
}

/// Seconds since midnight for an `HH:MM:SS` time, or `None` if malformed.
fn seconds_of_day(time: &str) -> Option<i64> {
    let mut parts = time.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let s: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(0..=23).contains(&h) || !(0..=59).contains(&m) {
        return None;
    }
    // 60 is a legal leap second in a wall-clock timestamp and a capture
    // process can emit one; it is not worth refusing a segment over.
    if !(0..=60).contains(&s) {
        return None;
    }
    Some(h * 3600 + m * 60 + s)
}

/// Absolute seconds for a segment, from its date and time.
///
/// Days are folded in so a segment at 00:00:02 is correctly one second after
/// one at 23:59:58 the previous night — the boundary a nocturnal station
/// crosses every time it records anything.
///
/// **`i64`, not a float.** Epoch seconds for any date this decade are around
/// 1.78e9, and `f32` carries 24 bits of mantissa — a resolution of 256 seconds
/// up there. The first version of this returned `f32`, and every segment of a
/// 15-second station collapsed onto the same value: no neighbour was ever
/// *earlier* or *later* than another, so none was ever found, and the fix
/// silently did nothing. `a_segment_time_survives_being_compared` pins it.
fn segment_epoch_secs(file: &RecordingFile) -> Option<i64> {
    let mut parts = file.date.split('-');
    let y: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    let days = crate::civil::days_from_civil(y, m, d);
    days.checked_mul(86_400)?
        .checked_add(seconds_of_day(&file.time)?)
}

/// The gap between two segment starts, in seconds, as a float.
///
/// Converted only *after* the subtraction, where the value is a handful of
/// seconds and a float is exact.
#[allow(clippy::cast_precision_loss)]
const fn gap_secs(later: i64, earlier: i64) -> f32 {
    (later - earlier) as f32
}

/// The segments beside `source` that belong to the same capture source,
/// as `(predecessor, successor)` paths.
///
/// "Same source" is the `rtsp_id` component of the filename, so a station with
/// three microphones writing into one directory never reaches across to
/// another one's audio. Both are `None` when the directory cannot be read,
/// which is the ordinary case for a segment that has already been purged.
fn neighbours(source: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let Some(dir) = source.parent() else {
        return (None, None);
    };
    let Some(here) = source.to_str().and_then(RecordingFile::parse) else {
        return (None, None);
    };
    let Some(here_at) = segment_epoch_secs(&here) else {
        return (None, None);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (None, None);
    };

    let mut before: Option<(i64, PathBuf)> = None;
    let mut after: Option<(i64, PathBuf)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path == source {
            continue;
        }
        let Some(other) = path.to_str().and_then(RecordingFile::parse) else {
            continue;
        };
        if other.rtsp_id != here.rtsp_id {
            continue;
        }
        let Some(at) = segment_epoch_secs(&other) else {
            continue;
        };
        if at < here_at {
            // Latest of the earlier ones.
            if before.as_ref().is_none_or(|(best, _)| at > *best) {
                before = Some((at, path));
            }
        } else if at > here_at {
            // Earliest of the later ones.
            if after.as_ref().is_none_or(|(best, _)| at < *best) {
                after = Some((at, path));
            }
        }
    }

    (before.map(|(_, p)| p), after.map(|(_, p)| p))
}

/// The samples for `[start_secs, stop_secs)` relative to `source`, reaching
/// into the neighbouring segments when the window extends past either end.
///
/// `audio` is the already-decoded source segment, so the common case — a
/// window wholly inside it — costs one slice and no extra I/O.
///
/// Returns the samples and the number of seconds of lead-in that were actually
/// obtained, which the caller needs because the clip's start time is no longer
/// necessarily `max(start_secs, 0)`.
#[derive(Debug, Clone)]
pub struct SpannedWindow {
    /// The samples, in source order.
    pub samples: Vec<f32>,
    /// Seconds of audio taken from before the source segment's own start.
    pub lead_in_secs: f32,
    /// Seconds of audio taken from after the source segment's own end.
    pub tail_secs: f32,
}

/// Read a clip window, spanning segment boundaries where it can.
///
/// # Errors
///
/// Never fails on a neighbour: a neighbour that cannot be read, cannot be
/// decoded, has a different sample rate, or does not abut is simply not used,
/// and the window clamps as it did before. The `Result` is present so a caller
/// can propagate a future hard failure; today only the caller's own decode of
/// the source segment can fail.
pub fn read_window(
    source: &Path,
    audio: &AudioData,
    start_secs: f32,
    stop_secs: f32,
) -> Result<SpannedWindow, DecodeError> {
    #[allow(clippy::cast_precision_loss)]
    let duration = audio.samples.len() as f32 / audio.sample_rate as f32;
    let needs_head = start_secs < 0.0;
    let needs_tail = stop_secs > duration;

    // The overwhelmingly common case: the window is inside the segment.
    if !needs_head && !needs_tail {
        return Ok(SpannedWindow {
            samples: slice_secs(audio, start_secs.max(0.0), stop_secs.min(duration)),
            lead_in_secs: 0.0,
            tail_secs: 0.0,
        });
    }

    let (predecessor, successor) = neighbours(source);
    let here_at = source
        .to_str()
        .and_then(RecordingFile::parse)
        .and_then(|f| segment_epoch_secs(&f));

    let mut head: Vec<f32> = Vec::new();
    let mut lead_in_secs = 0.0_f32;
    if needs_head
        && let Some(path) = predecessor
        && let (Some(prev), Some(here_at)) = (usable_neighbour(&path, audio.sample_rate), here_at)
        && let Some(prev_at) = path
            .to_str()
            .and_then(RecordingFile::parse)
            .and_then(|f| segment_epoch_secs(&f))
    {
        #[allow(clippy::cast_precision_loss)]
        let prev_duration = prev.audio.samples.len() as f32 / prev.audio.sample_rate as f32;
        if Neighbour::abuts(gap_secs(here_at, prev_at), prev_duration) {
            let wanted = -start_secs;
            let take = wanted.min(prev_duration);
            head = slice_secs(&prev.audio, prev_duration - take, prev_duration);
            lead_in_secs = take;
        }
    }

    let mut tail: Vec<f32> = Vec::new();
    let mut tail_secs = 0.0_f32;
    if needs_tail
        && let Some(path) = successor
        && let (Some(next), Some(here_at)) = (usable_neighbour(&path, audio.sample_rate), here_at)
        && let Some(next_at) = path
            .to_str()
            .and_then(RecordingFile::parse)
            .and_then(|f| segment_epoch_secs(&f))
    {
        // The *source* is the earlier segment here, so it is this segment's
        // duration that must match the gap.
        if Neighbour::abuts(gap_secs(next_at, here_at), duration) {
            #[allow(clippy::cast_precision_loss)]
            let next_duration = next.audio.samples.len() as f32 / next.audio.sample_rate as f32;
            let wanted = stop_secs - duration;
            let take = wanted.min(next_duration);
            tail = slice_secs(&next.audio, 0.0, take);
            tail_secs = take;
        }
    }

    let body = slice_secs(audio, start_secs.max(0.0), stop_secs.min(duration));
    let mut samples = Vec::with_capacity(head.len() + body.len() + tail.len());
    samples.extend_from_slice(&head);
    samples.extend_from_slice(&body);
    samples.extend_from_slice(&tail);

    Ok(SpannedWindow {
        samples,
        lead_in_secs,
        tail_secs,
    })
}

/// Decode a neighbour, rejecting one that cannot contribute.
///
/// A different sample rate means a different device or a reconfigured one, and
/// concatenating them without resampling would splice audio that plays at the
/// wrong speed. Resampling here would be worse: it would hide the fact that
/// the capture configuration changed mid-recording, which is something an
/// operator needs to know about rather than have smoothed over.
fn usable_neighbour(path: &Path, sample_rate: u32) -> Option<Neighbour> {
    let audio = decode_file(path).ok()?;
    if audio.sample_rate != sample_rate || audio.samples.is_empty() {
        return None;
    }
    Some(Neighbour { audio })
}

/// The samples of `audio` between two times, clamped to what exists.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn slice_secs(audio: &AudioData, from: f32, to: f32) -> Vec<f32> {
    let rate = audio.sample_rate as f32;
    let start = (from.max(0.0) * rate) as usize;
    let stop = (to.max(0.0) * rate) as usize;
    let start = start.min(audio.samples.len());
    let stop = stop.min(audio.samples.len()).max(start);
    audio.samples[start..stop].to_vec()
}

#[cfg(test)]
mod tests;
