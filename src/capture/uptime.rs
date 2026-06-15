//! Rolling 24-hour per-source uptime, bucketed into half-hour segments.
//!
//! The supervisor records one sample per reconcile tick into the half-hour
//! bucket for the current wall clock; [`UptimeRing::segments`] then renders the
//! last 24 hours oldest→newest for the Station Health uptime strip. The ring is
//! a fixed 48-entry array — bounded memory regardless of how long the station
//! runs — and self-expires stale buckets, so a half hour the source was down
//! reads `Down`, and a half hour it was never observed (or paused) reads `Out`.

use birdnet_core::audio::capture::{SourceState, UPTIME_SEGMENTS, UptimeSegment};

/// Seconds per uptime bucket (half an hour).
const BUCKET_SECS: u64 = 1800;

/// One half-hour bucket: which wall-clock half-hour it covers, and how many
/// ticks within it were up vs down (paused ticks count as neither).
#[derive(Debug, Clone, Copy)]
struct Bucket {
    /// `now_unix / BUCKET_SECS` for the period this bucket holds; `u64::MAX`
    /// marks an as-yet-unwritten bucket (so half-hour 0, the epoch, still reads
    /// as real data rather than "never observed").
    half_hour: u64,
    up: u32,
    down: u32,
}

impl Bucket {
    const EMPTY: Self = Self {
        half_hour: u64::MAX,
        up: 0,
        down: 0,
    };
}

/// A source's rolling 24-hour uptime accumulator.
#[derive(Debug, Clone)]
pub(super) struct UptimeRing {
    buckets: [Bucket; UPTIME_SEGMENTS],
}

impl Default for UptimeRing {
    fn default() -> Self {
        Self::new()
    }
}

impl UptimeRing {
    /// An empty ring (every half hour "not yet observed").
    pub(super) const fn new() -> Self {
        Self {
            buckets: [Bucket::EMPTY; UPTIME_SEGMENTS],
        }
    }

    /// Record one tick's outcome for the half-hour containing `now_unix`.
    ///
    /// Connected counts as up; stalled / backing-off as down; paused as neither
    /// (so an intentionally paused half hour reads `Out`, not a fault).
    pub(super) const fn record(&mut self, now_unix: u64, state: SourceState) {
        let half_hour = now_unix / BUCKET_SECS;
        let bucket = &mut self.buckets[Self::index(half_hour)];
        if bucket.half_hour != half_hour {
            *bucket = Bucket {
                half_hour,
                up: 0,
                down: 0,
            };
        }
        match state {
            SourceState::Connected => bucket.up = bucket.up.saturating_add(1),
            SourceState::Stalled | SourceState::BackingOff => {
                bucket.down = bucket.down.saturating_add(1);
            }
            SourceState::Paused => {}
        }
    }

    /// The last [`UPTIME_SEGMENTS`] half hours up to `now_unix`, oldest → newest.
    ///
    /// A half hour with no recorded ticks (never observed, or fully paused)
    /// reads `Out`; otherwise it reads `Up` when the source was connected for at
    /// least half the recorded ticks, else `Down`.
    pub(super) fn segments(&self, now_unix: u64) -> Vec<UptimeSegment> {
        let current = now_unix / BUCKET_SECS;
        let span = UPTIME_SEGMENTS as u64 - 1;
        (0..UPTIME_SEGMENTS as u64)
            .map(|offset| {
                // offset 0 = oldest (current - 47) … offset 47 = newest (current).
                let target = current.saturating_sub(span - offset);
                let bucket = &self.buckets[Self::index(target)];
                if bucket.half_hour != target || (bucket.up == 0 && bucket.down == 0) {
                    UptimeSegment::Out
                } else if bucket.up >= bucket.down {
                    UptimeSegment::Up
                } else {
                    UptimeSegment::Down
                }
            })
            .collect()
    }

    /// Ring index for a wall-clock half-hour. Forty-eight consecutive half hours
    /// map to forty-eight distinct slots, so the last 24 h never collide.
    #[allow(clippy::cast_possible_truncation)]
    const fn index(half_hour: u64) -> usize {
        (half_hour % UPTIME_SEGMENTS as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HH: u64 = BUCKET_SECS; // one half-hour of seconds

    #[test]
    fn empty_ring_is_all_out() {
        let ring = UptimeRing::new();
        let segs = ring.segments(1_000 * HH);
        assert_eq!(segs.len(), UPTIME_SEGMENTS);
        assert!(segs.iter().all(|s| *s == UptimeSegment::Out));
    }

    #[test]
    fn connected_then_down_reads_up_then_down() {
        let mut ring = UptimeRing::new();
        let base = 1_000 * HH;
        // Half-hour N: connected; half-hour N+1: backing off.
        ring.record(base, SourceState::Connected);
        ring.record(base + HH, SourceState::BackingOff);
        let segs = ring.segments(base + HH);
        // newest = current (N+1) = Down; previous (N) = Up; rest Out.
        assert_eq!(segs[UPTIME_SEGMENTS - 1], UptimeSegment::Down);
        assert_eq!(segs[UPTIME_SEGMENTS - 2], UptimeSegment::Up);
        assert_eq!(segs[UPTIME_SEGMENTS - 3], UptimeSegment::Out);
    }

    #[test]
    fn majority_up_reads_up() {
        let mut ring = UptimeRing::new();
        let base = 500 * HH;
        for _ in 0..9 {
            ring.record(base, SourceState::Connected);
        }
        ring.record(base, SourceState::BackingOff);
        // 9 up vs 1 down in the same half hour → Up.
        assert_eq!(ring.segments(base)[UPTIME_SEGMENTS - 1], UptimeSegment::Up);
    }

    #[test]
    fn paused_only_reads_out() {
        let mut ring = UptimeRing::new();
        let base = 500 * HH;
        ring.record(base, SourceState::Paused);
        ring.record(base, SourceState::Paused);
        // A fully-paused half hour is intentional, not a fault → Out.
        assert_eq!(ring.segments(base)[UPTIME_SEGMENTS - 1], UptimeSegment::Out);
    }

    #[test]
    fn ring_expires_after_full_wrap() {
        let mut ring = UptimeRing::new();
        let base = 1_000 * HH;
        ring.record(base, SourceState::Connected);
        // 48 half-hours later, the same slot is reused for a new period; the old
        // reading must not bleed through as the "current" segment.
        let later = base + UPTIME_SEGMENTS as u64 * HH;
        let segs = ring.segments(later);
        // The slot the old Up sample sat in now represents `later`, unwritten.
        assert!(segs.iter().all(|s| *s == UptimeSegment::Out));
    }
}
