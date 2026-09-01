//! The duplicate-prediction interval: one continuous song is one detection.
//!
//! # The problem
//!
//! A 15-second recording is five 3-second chunks. A blackbird that sings
//! through the whole of it is detected in every one, so the station records
//! five detections of a bird that sang once. Migration 24 gave each of those
//! rows its own second — before it, all five shared the file's start time and
//! were indistinguishable — but they are still five rows for one song.
//!
//! That is not only untidy. Every count in the application is a row count: the
//! daily totals, the species-activity heat map, the dawn-chorus curve, the
//! sessionisation in `birdnet-behavioral`. A species that sings in long phrases
//! outscores one that calls in short bursts by a factor of however many chunks
//! its phrases happen to span, and nothing in the numbers says so.
//!
//! # What this does
//!
//! [`DuplicateGate`] admits a species at most once per
//! `--duplicate-interval-secs`. It is **off by default**: switching it on
//! changes how many rows a station records, and doing that silently on upgrade
//! would put a visible step in every chart the operator has been watching.
//!
//! # First-heard wins, not most-confident
//!
//! Within an interval the *first* detection is the one kept, not the loudest.
//! Keeping the highest-confidence chunk would need the whole interval buffered
//! before anything could be written, which would delay every notification by
//! the interval and leave detections unwritten if the process stopped mid-song.
//! The first chunk is also the more useful timestamp: it is when the bird
//! started singing.

use std::collections::HashMap;

/// What the gate decided about one detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DuplicateVerdict {
    /// Record it.
    Admit,
    /// The same species was admitted this recently; drop it.
    Suppress {
        /// Seconds between this detection and the one that shadowed it.
        since_last_secs: i64,
    },
}

/// Remembers when each species was last admitted.
///
/// Bounded by the model's label count (~11 500 short strings, low single-digit
/// megabytes at absolute worst, and in practice the few dozen species a site
/// actually hears), so entries are never evicted — an eviction policy would be
/// a second thing to get wrong for no memory that matters.
#[derive(Debug)]
pub(super) struct DuplicateGate {
    /// Zero disables the gate.
    interval_secs: i64,
    /// Scientific name → timestamp of the last admitted detection, in seconds.
    last_admitted: HashMap<String, i64>,
}

impl DuplicateGate {
    /// A gate suppressing repeats within `interval_secs`.
    ///
    /// Anything that is not a positive number of seconds disables it. An
    /// earlier version clamped the stored value with `.max(0)` as well; that
    /// was dead — `is_enabled` is `> 0`, which already rejects a negative, and
    /// `admit` returns before the comparison when the gate is off, so no
    /// mutation of the clamp could be made to fail a test.
    pub(super) fn new(interval_secs: i64) -> Self {
        Self {
            interval_secs,
            last_admitted: HashMap::new(),
        }
    }

    /// Whether the gate will suppress anything.
    ///
    /// `> 0` rather than `!= 0`: `DUPLICATE_INTERVAL_SECS=-30` is a typo, and
    /// a gate that treated it as a window would compare against a negative
    /// interval, never suppress, and still tell the operator at startup that
    /// the feature was on.
    pub(super) const fn is_enabled(&self) -> bool {
        self.interval_secs > 0
    }

    /// Decide on one detection, recording it if admitted.
    ///
    /// `at_secs` is the detection's own timestamp, not the wall clock: the
    /// daemon can be minutes behind on a backlog, and a run of files processed
    /// back-to-back would otherwise all look simultaneous.
    ///
    /// The comparison is on the absolute difference because files are not
    /// processed in timestamp order — `process_existing_files` walks the watch
    /// directory in whatever order the filesystem returns, so on a restart with
    /// a backlog an *earlier* recording routinely arrives after a later one. A
    /// signed `at - last < interval` test would admit every one of those.
    pub(super) fn admit(&mut self, sci_name: &str, at_secs: i64) -> DuplicateVerdict {
        if !self.is_enabled() {
            return DuplicateVerdict::Admit;
        }
        if let Some(&last) = self.last_admitted.get(sci_name) {
            let gap = at_secs.saturating_sub(last).abs();
            if gap < self.interval_secs {
                return DuplicateVerdict::Suppress {
                    since_last_secs: gap,
                };
            }
        }
        self.last_admitted.insert(sci_name.to_owned(), at_secs);
        DuplicateVerdict::Admit
    }
}

/// A detection's timestamp as seconds, for gap arithmetic only.
///
/// The zone offset is deliberately not applied: every detection in a station's
/// stream carries the same convention, so it cancels in the subtraction, and
/// asking for the offset here would mean threading a database read into the
/// hot path to compute a difference that does not need it.
///
/// `None` for a pair that names no civil time. `Date` and `Time` are free-form
/// `TEXT`, and a detection whose timestamp cannot be read must not be silently
/// gated — see the caller.
#[must_use]
pub(super) fn detection_secs(date: &str, time: &str) -> Option<i64> {
    birdnet_core::civil::parse_civil(date, time)
        .map(|c| birdnet_core::civil::unix_secs_from_civil(&c))
}

#[cfg(test)]
mod tests {
    use super::{DuplicateGate, DuplicateVerdict, detection_secs};

    /// Seconds for a time on 2026-03-14.
    fn t(hms: &str) -> i64 {
        detection_secs("2026-03-14", hms).expect("a well-formed timestamp")
    }

    /// Whether the gate admitted this.
    fn admitted(g: &mut DuplicateGate, sci: &str, hms: &str) -> bool {
        g.admit(sci, t(hms)) == DuplicateVerdict::Admit
    }

    // ── the interval ────────────────────────────────────────────────────

    #[test]
    fn one_continuous_song_becomes_one_detection() {
        // Five chunks of a fifteen-second recording, three seconds apart —
        // exactly what the pipeline produces for a bird that sings throughout.
        let mut g = DuplicateGate::new(30);
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        for hms in ["05:00:03", "05:00:06", "05:00:09", "05:00:12"] {
            assert!(
                !admitted(&mut g, "Turdus merula", hms),
                "chunk at {hms} was recorded as a separate detection"
            );
        }
    }

    #[test]
    fn the_bird_is_admitted_again_once_the_interval_has_passed() {
        // Counterpart, and the reason this is an interval and not a mute: a
        // gate that never re-admitted would record each species once and then
        // go silent for the rest of the station's life.
        let mut g = DuplicateGate::new(30);
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        assert!(!admitted(&mut g, "Turdus merula", "05:00:29"));
        assert!(admitted(&mut g, "Turdus merula", "05:00:30"));
    }

    #[test]
    fn the_interval_boundary_is_exclusive() {
        // Pinned in both directions: at exactly the interval the bird is
        // admitted, one second inside it is not.
        let mut g = DuplicateGate::new(10);
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        assert!(!admitted(&mut g, "Turdus merula", "05:00:09"));
        assert!(admitted(&mut g, "Turdus merula", "05:00:10"));
    }

    #[test]
    fn the_interval_restarts_from_the_admitted_detection() {
        // A suppressed chunk must not extend the window, or a species singing
        // continuously would be shut out indefinitely rather than recorded
        // once per interval.
        let mut g = DuplicateGate::new(10);
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        assert!(!admitted(&mut g, "Turdus merula", "05:00:05"));
        assert!(!admitted(&mut g, "Turdus merula", "05:00:09"));
        assert!(
            admitted(&mut g, "Turdus merula", "05:00:10"),
            "a suppressed chunk pushed the window forward"
        );
    }

    // ── scope ───────────────────────────────────────────────────────────

    #[test]
    fn the_interval_is_per_species() {
        // A dawn chorus is many species at once. A global interval would
        // record one bird and discard the chorus.
        let mut g = DuplicateGate::new(60);
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        assert!(admitted(&mut g, "Parus major", "05:00:01"));
        assert!(admitted(&mut g, "Erithacus rubecula", "05:00:02"));
        assert!(!admitted(&mut g, "Turdus merula", "05:00:03"));
    }

    #[test]
    fn a_zero_interval_disables_the_gate() {
        let mut g = DuplicateGate::new(0);
        assert!(!g.is_enabled());
        for _ in 0..5 {
            assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        }
    }

    #[test]
    fn a_negative_interval_is_disabled_and_says_so() {
        // `DUPLICATE_INTERVAL_SECS=-30` is a typo, not an instruction. What
        // makes this true is `is_enabled` being `> 0` rather than `!= 0`: with
        // `!= 0` the gate reports itself as on at startup and then never
        // suppresses anything, which is the worst of both — the operator is
        // told a feature is running that is not.
        let mut g = DuplicateGate::new(-30);
        assert!(!g.is_enabled(), "a negative interval reported as enabled");
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        assert!(admitted(&mut g, "Turdus merula", "05:00:01"));
    }

    // ── out-of-order arrival ────────────────────────────────────────────

    #[test]
    fn a_backlog_arriving_out_of_order_is_still_deduplicated() {
        // `process_existing_files` walks the watch directory in whatever order
        // the filesystem returns, so after a restart an *earlier* recording
        // routinely arrives after a later one. A signed `at - last < interval`
        // test admits every one of those, which is the case this gate exists
        // for — a restart with an hour of backlog is exactly when the
        // duplicates pile up.
        let mut g = DuplicateGate::new(30);
        assert!(admitted(&mut g, "Turdus merula", "05:00:12"));
        for hms in ["05:00:09", "05:00:06", "05:00:03", "05:00:00"] {
            assert!(
                !admitted(&mut g, "Turdus merula", hms),
                "an out-of-order chunk at {hms} was recorded as a separate detection"
            );
        }
    }

    #[test]
    fn a_genuinely_earlier_detection_outside_the_interval_is_still_admitted() {
        // Counterpart: absolute-value comparison must not become "suppress
        // anything earlier". A backlog spanning a morning holds real, separate
        // detections and they all belong in the history.
        let mut g = DuplicateGate::new(30);
        assert!(admitted(&mut g, "Turdus merula", "09:00:00"));
        assert!(admitted(&mut g, "Turdus merula", "05:00:00"));
        assert!(admitted(&mut g, "Turdus merula", "07:00:00"));
    }

    // ── timestamps ──────────────────────────────────────────────────────

    #[test]
    fn a_timestamp_that_names_no_civil_time_is_reported_as_such() {
        // The caller admits these rather than gating them: dropping a real
        // detection because its filename was odd is the worse failure.
        assert!(detection_secs("", "").is_none());
        assert!(detection_secs("not-a-date", "05:00:00").is_none());
        assert!(detection_secs("2026-03-14", "5:00:00").is_none());
        assert!(detection_secs("2026-03-14", "05:00:00").is_some());
    }

    #[test]
    fn timestamps_are_ordered_across_a_date_boundary() {
        // Seconds, not "seconds within the day": a nightjar calling either
        // side of midnight is two detections a minute apart, and a
        // time-of-day comparison would read that gap as 23 hours 59 minutes.
        let before = detection_secs("2026-03-14", "23:59:50").unwrap();
        let after = detection_secs("2026-03-15", "00:00:10").unwrap();
        assert_eq!(after - before, 20);

        let mut g = DuplicateGate::new(30);
        assert_eq!(
            g.admit("Caprimulgus europaeus", before),
            DuplicateVerdict::Admit
        );
        assert!(
            matches!(
                g.admit("Caprimulgus europaeus", after),
                DuplicateVerdict::Suppress {
                    since_last_secs: 20
                }
            ),
            "the midnight boundary was read as a long gap"
        );
    }

    #[test]
    fn the_gap_is_reported_so_the_log_says_how_close_it_was() {
        let mut g = DuplicateGate::new(30);
        g.admit("Turdus merula", t("05:00:00"));
        assert_eq!(
            g.admit("Turdus merula", t("05:00:07")),
            DuplicateVerdict::Suppress { since_last_secs: 7 }
        );
    }
}
