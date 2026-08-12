//! Debouncing filesystem events until a captured clip is finished being written.
//!
//! Capture backends do not publish clips atomically. `arecord --max-file-time`
//! creates a segment and then writes into it for the whole segment duration;
//! ffmpeg's segment muxer (used for RTSP) does the same in place, emitting a
//! stream of create/modify events throughout. Anything that decodes on the
//! *creation* event therefore reads a file holding little more than a WAV
//! header, and fails with "unexpected end of file".
//!
//! This module owns the one rule that avoids that — wait until a file's size
//! has stopped changing — so that every consumer of the capture directory
//! applies the same rule. It previously lived privately inside the detection
//! daemon, and the live-spectrogram producer, unable to see it, substituted
//! `thread::sleep(100ms)` under a comment claiming it ensured the file was
//! "fully written". Against a 15-second segment that decoded roughly a second
//! in, so *every* frame failed and the dashboard's live spectrogram never drew
//! anything on a working station.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a file's size must stay unchanged before it is considered fully
/// written and safe to decode.
///
/// Two seconds comfortably clears the inter-write gaps of a real-time PCM
/// segment while adding latency that is negligible next to a multi-second
/// recording.
pub const FILE_SETTLE: Duration = Duration::from_secs(2);

/// Debounces filesystem events so each captured file is decoded once, and only
/// after it has finished being written.
///
/// A capture backend emits a burst of create/modify events while it streams a
/// clip to disk. [`PendingFiles`] holds each path until its size has been
/// stable for [`FILE_SETTLE`], then yields it exactly once. File size (polled
/// via the injected sizer) is the source of truth, so a clip still settles
/// after its final watcher event and a dropped event cannot strand it.
#[derive(Default)]
pub struct PendingFiles {
    /// path -> (last observed size, the instant that size last changed)
    seen: HashMap<PathBuf, (u64, Instant)>,
}

impl PendingFiles {
    /// A tracker with nothing pending.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record watcher activity on `path`. Repeated calls for a path already
    /// tracked are no-ops: the size poll in [`Self::drain_settled`] drives the
    /// settle timer, not the (bursty, backend-specific) event rate.
    pub fn note(&mut self, path: PathBuf, now: Instant) {
        // `u64::MAX` is a "size not yet observed" sentinel that the first sweep
        // always treats as a change, establishing the real baseline.
        self.seen.entry(path).or_insert((u64::MAX, now));
    }

    /// Return the tracked files whose size has been unchanged for at least
    /// `settle`, removing them so each is yielded exactly once. `sizer` returns
    /// a file's current size, or `None` if it has vanished (which drops it).
    pub fn drain_settled<F>(&mut self, now: Instant, settle: Duration, sizer: F) -> Vec<PathBuf>
    where
        F: Fn(&Path) -> Option<u64>,
    {
        let mut ready = Vec::new();
        self.seen
            .retain(|path, (last_size, last_change)| match sizer(path) {
                None => false,
                Some(current) if current != *last_size => {
                    *last_size = current;
                    *last_change = now;
                    true
                }
                Some(current) if current > 0 && now.duration_since(*last_change) >= settle => {
                    ready.push(path.clone());
                    false
                }
                Some(_) => true,
            });
        ready
    }
}

/// Current size of `path` on disk, or `None` if it cannot be stat'd.
///
/// The default sizer for real filesystems; tests inject their own.
pub fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn pending_files_yields_only_after_size_is_stable() {
        // Models a segment growing in place, then finalizing.
        let mut pending = PendingFiles::new();
        let clip = PathBuf::from("/tmp/birdnet-stream/clip.wav");
        let t0 = Instant::now();
        pending.note(clip.clone(), t0);

        // Fresh closure per call (each reads the current `size`) so the sizer is
        // passed by value — `&closure` would trip clippy::needless_borrows.
        let size = Cell::new(100u64);
        let sizer = || |_: &Path| Some(size.get());

        // Baseline observed -> not ready.
        assert!(pending.drain_settled(t0, FILE_SETTLE, sizer()).is_empty());
        // Still growing -> the settle timer resets, still not ready.
        size.set(200);
        assert!(
            pending
                .drain_settled(t0 + Duration::from_millis(500), FILE_SETTLE, sizer())
                .is_empty()
        );
        // Size now stable, but the settle window has not elapsed yet.
        assert!(
            pending
                .drain_settled(t0 + Duration::from_millis(700), FILE_SETTLE, sizer())
                .is_empty()
        );
        // Stable for >= FILE_SETTLE since the last change -> yielded once.
        let ready = pending.drain_settled(
            t0 + Duration::from_millis(500) + FILE_SETTLE,
            FILE_SETTLE,
            sizer(),
        );
        assert_eq!(
            ready,
            vec![clip],
            "a settled clip must be processed exactly once"
        );
        // ...and never again (it was removed when processed).
        assert!(
            pending
                .drain_settled(t0 + Duration::from_secs(60), FILE_SETTLE, sizer())
                .is_empty(),
            "a processed clip must not be reprocessed"
        );
    }

    #[test]
    fn pending_files_drops_vanished_and_never_yields_empty() {
        let mut pending = PendingFiles::new();
        let gone = PathBuf::from("/tmp/birdnet-stream/gone.wav");
        let empty = PathBuf::from("/tmp/birdnet-stream/empty.wav");
        let t0 = Instant::now();
        pending.note(gone.clone(), t0);
        pending.note(empty, t0);

        let sizer = || {
            |p: &Path| {
                if p == gone.as_path() {
                    None
                } else {
                    Some(0u64)
                }
            }
        };

        // A vanished file is dropped; a zero-byte file is never "stable enough"
        // to decode no matter how long it sits.
        assert!(
            pending
                .drain_settled(t0 + Duration::from_secs(60), FILE_SETTLE, sizer())
                .is_empty()
        );
    }

    /// The regression this module exists for: a segment written over 15s is not
    /// decodable a fraction of a second after it appears, which is exactly what
    /// the live spectrogram used to do with a fixed 100 ms sleep.
    #[test]
    fn a_still_growing_segment_is_never_yielded_early() {
        let mut pending = PendingFiles::new();
        let clip = PathBuf::from("/tmp/birdnet-stream/segment.wav");
        let t0 = Instant::now();
        pending.note(clip, t0);

        // arecord writes for 15s; poll every 100 ms and grow the file each time.
        let size = Cell::new(0u64);
        let sizer = || |_: &Path| Some(size.get());
        for tick in 1_u64..=150 {
            size.set(tick * 1024);
            let elapsed = Duration::from_millis(100 * tick);
            assert!(
                pending
                    .drain_settled(t0 + elapsed, FILE_SETTLE, sizer())
                    .is_empty(),
                "a growing segment must never be yielded (tick {tick})"
            );
        }
        // Writing stops; after the settle window it is finally offered once.
        let done = t0 + Duration::from_millis(15_000) + FILE_SETTLE;
        assert_eq!(pending.drain_settled(done, FILE_SETTLE, sizer()).len(), 1);
    }
}
