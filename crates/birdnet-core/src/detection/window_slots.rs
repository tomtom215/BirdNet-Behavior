//! What a detection count counts, and a count that does not scale with
//! `OVERLAP` (audit ACC-19, MSC-09).
//!
//! # What a stored row is
//!
//! The pipeline cuts each recording into windows of the classifier's length
//! `W` (3 s for BirdNET V2.4), starting every `W - OVERLAP` seconds
//! ([`crate::detection::pipeline::chunk_step_samples`]); a station running
//! classifiers of several lengths steps by the shortest window instead, and
//! `OVERLAP` does not move that step. Every species that clears its bar in a
//! window, and survives the filters, is one row, keyed by the recording, the
//! species and the window's offset (`chunk_offset_secs`).
//!
//! So a row count is a count of **positive windows**, and the number of
//! windows a recording has is `1 + (len - W) / (W - OVERLAP)`: with `OVERLAP`
//! 1.5 a bird singing throughout a 15 s recording is nine rows where `OVERLAP`
//! 0 gives five. A count of rows scales with the overlap the station happened
//! to run with, and two stations — or one station before and after a change —
//! cannot be compared by it. It is not a count of birds, of calls, or of
//! anything in the world: it is a count of the station's own windows.
//!
//! # The window-slot count
//!
//! [`slot_of`] places each row's window in the grid of `W`-second slots the
//! same recording would have been cut into at `OVERLAP` 0 (`floor(offset /
//! W)`), and [`count_slots`] counts distinct `(recording, source, species,
//! slot)`. Under policy [`SLOT_POLICY_VERSION`]:
//!
//! - at `OVERLAP` 0 every row is its own slot, so the count **is** the row
//!   count, and nothing an existing station reports changes;
//! - at any overlap, a species positive in every window of a recording is
//!   counted in the same slots as at `OVERLAP` 0, give or take the one slot a
//!   recording's tail can add;
//! - it never exceeds the row count and does not depend on the order rows are
//!   read in.
//!
//! What it is **not**: a count of individuals, calls or events. A call that
//! straddles a slot boundary is two slots, as it is two windows at `OVERLAP`
//! 0. And a positive window at an offset the zero-overlap grid lacks (a call
//! heard only in the window from 1.5 s to 4.5 s) is credited to the slot its
//! window starts in, so overlap still finds calls a zero-overlap station would
//! have missed: the count does not scale with overlap, but its sensitivity is
//! still the station's.
//!
//! # Read side only
//!
//! Nothing stored changes. The rule lives here so the read layer
//! (`crates/birdnet-db/src/sqlite/queries/events.rs` and the views over it) can
//! offer a count that is comparable across overlap settings beside the row
//! count, never instead of it.

use std::collections::HashSet;

/// The version of the slot rule. A response counting slots should state it.
pub const SLOT_POLICY_VERSION: &str = "slots-1";

/// Offsets are stored as rounded seconds (`f32` arithmetic upstream), so a
/// window at `3 * 1.5 = 4.5` can be read back as `4.499_999`. A slot boundary
/// is moved this far towards zero before flooring.
const BOUNDARY_SLACK_SECS: f64 = 1e-4;

/// The slot width a station's rows are counted in: the step it would run at
/// with `OVERLAP` 0, which is the shared step when one is set (several
/// classifiers) and the window length otherwise.
#[must_use]
pub fn slot_secs(chunk_duration_secs: f32, chunk_step_secs: Option<f32>) -> f64 {
    let step = chunk_step_secs.filter(|s| s.is_finite() && *s > 0.0);
    f64::from(step.unwrap_or(chunk_duration_secs))
}

/// The zero-overlap slot a window starting `offset_secs` into its recording
/// falls in, for slots `slot_secs` wide.
///
/// `None` for an offset that names no
/// place (negative, not finite) or a width that is not positive.
#[must_use]
pub fn slot_of(offset_secs: f64, slot_secs: f64) -> Option<i64> {
    if !offset_secs.is_finite() || offset_secs < 0.0 || !slot_secs.is_finite() || slot_secs <= 0.0 {
        return None;
    }
    let slot = ((offset_secs + BOUNDARY_SLACK_SECS) / slot_secs).floor();
    // A recording would have to be longer than i64::MAX slots to overflow.
    #[allow(clippy::cast_possible_truncation)]
    Some(slot as i64)
}

/// One stored row, as the slot rule reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotRow<'a> {
    /// The recording the row came from (`File_Name`).
    pub recording: &'a str,
    /// The source that recorded it, when one was recorded.
    pub source: Option<&'a str>,
    /// Scientific name.
    pub species: &'a str,
    /// The window's offset into the recording (`chunk_offset_secs`).
    pub offset_secs: f64,
}

/// What [`count_slots`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotCount {
    /// Rows read.
    pub rows: u64,
    /// Distinct `(recording, source, species, slot)`.
    pub slots: u64,
    /// Rows whose offset named no slot; counted here, in neither of the
    /// others' slots, and never guessed.
    pub unplaced: u64,
}

/// Count `rows` in slots `slot_secs` wide (see the module documentation).
#[must_use]
pub fn count_slots<'a>(rows: impl IntoIterator<Item = SlotRow<'a>>, slot_secs: f64) -> SlotCount {
    let mut seen: HashSet<(&str, Option<&str>, &str, i64)> = HashSet::new();
    let mut count = SlotCount::default();
    for row in rows {
        count.rows += 1;
        match slot_of(row.offset_secs, slot_secs) {
            Some(slot) => {
                if seen.insert((row.recording, row.source, row.species, slot)) {
                    count.slots += 1;
                }
            }
            None => count.unplaced += 1,
        }
    }
    count
}

/// The rows that stand for their slot.
///
/// For each distinct `(recording, source,
/// species, slot)`, the index of its earliest window (the lowest offset, then
/// the earliest index). Unplaceable rows are kept, each standing for itself, so
/// a read layer that thins with this loses nothing it cannot place. Indices
/// come back in ascending order.
///
/// This is what a read layer applies before grouping rows into events, to
/// count events that do not multiply with overlap.
#[must_use]
pub fn slot_representatives(rows: &[SlotRow<'_>], slot_secs: f64) -> Vec<usize> {
    let mut best: std::collections::HashMap<(&str, Option<&str>, &str, i64), usize> =
        std::collections::HashMap::new();
    let mut kept = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let Some(slot) = slot_of(row.offset_secs, slot_secs) else {
            kept.push(i);
            continue;
        };
        best.entry((row.recording, row.source, row.species, slot))
            .and_modify(|j| {
                if row.offset_secs < rows[*j].offset_secs {
                    *j = i;
                }
            })
            .or_insert(i);
    }
    kept.extend(best.into_values());
    kept.sort_unstable();
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn row(offset: f64) -> SlotRow<'static> {
        SlotRow {
            recording: "2026-05-19-birdnet-09:00:00.wav",
            source: Some("mic"),
            species: "Turdus merula",
            offset_secs: offset,
        }
    }

    /// Window starts the pipeline cuts for a recording `len` seconds long:
    /// every `step` from 0 while a full window fits, plus a tail start when at
    /// least `min_tail` of audio remains (the shape of `chunk_starts`).
    #[allow(clippy::while_float)]
    fn starts(len: f64, window: f64, step: f64, min_tail: f64) -> Vec<f64> {
        let mut out = Vec::new();
        let mut pos = 0.0;
        while pos + min_tail <= len + 1e-9 {
            out.push(pos);
            if pos + window >= len - 1e-9 {
                break;
            }
            pos += step;
        }
        out
    }

    #[test]
    fn an_offset_that_names_no_place_is_unplaced_and_counted() {
        assert_eq!(slot_of(-1.0, 3.0), None);
        assert_eq!(slot_of(f64::NAN, 3.0), None);
        assert_eq!(slot_of(1.0, 0.0), None);
        let c = count_slots([row(0.0), row(f64::NAN)], 3.0);
        assert_eq!(
            c,
            SlotCount {
                rows: 2,
                slots: 1,
                unplaced: 1
            }
        );
    }

    #[test]
    fn a_rounded_offset_lands_in_its_own_slot() {
        // 3 * 1.5 stored as 4.499999: still slot 1 of 3 s slots, and 6.0
        // stored as 5.99999 is slot 2.
        assert_eq!(slot_of(4.499_999, 3.0), Some(1));
        assert_eq!(slot_of(5.999_99, 3.0), Some(2));
        assert_eq!(slot_of(3.0, 3.0), Some(1));
    }

    /// The defect, in numbers: a species positive in every window of a 15 s
    /// recording is five rows at OVERLAP 0 and nine at 1.5, and five slots at
    /// both.
    #[test]
    fn rows_scale_with_overlap_and_slots_do_not() {
        let rows_at = |overlap: f64| {
            starts(15.0, 3.0, 3.0 - overlap, 1.5)
                .into_iter()
                .map(row)
                .collect::<Vec<_>>()
        };
        let zero = count_slots(rows_at(0.0), 3.0);
        let lapped = count_slots(rows_at(1.5), 3.0);
        assert_eq!((zero.rows, zero.slots), (5, 5));
        assert_eq!(lapped.rows, 9, "the rows scale with the overlap");
        assert_eq!(lapped.slots, 5, "the slots do not");
    }

    #[test]
    fn the_earliest_window_of_each_slot_stands_for_it() {
        let rows = [row(4.5), row(3.0), row(1.5), row(0.0), row(f64::NAN)];
        // Slot 0: offsets 1.5 (index 2) and 0.0 (index 3) -> 3. Slot 1: 4.5
        // (0) and 3.0 (1) -> 1. The unplaced row (4) is kept.
        assert_eq!(slot_representatives(&rows, 3.0), vec![1, 3, 4]);
    }

    #[test]
    fn different_species_sources_and_recordings_never_share_a_slot() {
        let mut a = row(0.0);
        let mut b = row(0.0);
        b.species = "Pica pica";
        let mut c = row(0.0);
        c.source = Some("camera");
        let mut d = row(0.0);
        d.recording = "other.wav";
        a.offset_secs = 1.0;
        assert_eq!(count_slots([a, b, c, d], 3.0).slots, 4);
    }

    #[test]
    fn the_slot_width_is_the_shared_step_when_one_is_set() {
        assert!((slot_secs(3.0, None) - 3.0).abs() < 1e-12);
        assert!((slot_secs(5.0, Some(4.5)) - 4.5).abs() < 1e-12);
        assert!((slot_secs(3.0, Some(0.0)) - 3.0).abs() < 1e-12);
    }

    proptest! {
        /// At OVERLAP 0 the slot count is the row count: nothing an existing
        /// station reports changes.
        #[test]
        fn at_zero_overlap_every_row_is_its_own_slot(
            len in 3.0_f64..120.0,
            positive in proptest::collection::vec(any::<bool>(), 0..64),
        ) {
            let windows = starts(len, 3.0, 3.0, 1.5);
            let rows: Vec<_> = windows
                .iter()
                .zip(positive.iter().cycle())
                .filter(|(_, p)| **p)
                .map(|(s, _)| row(*s))
                .collect();
            let c = count_slots(rows.iter().copied(), 3.0);
            prop_assert_eq!(c.slots, c.rows);
        }

        /// A species positive in every window: the slots at any overlap are
        /// the zero-overlap rows, give or take the tail's one, while the rows
        /// grow with the overlap.
        #[test]
        fn continuous_presence_counts_the_same_slots_at_any_overlap(
            len in 3.0_f64..120.0,
            overlap_tenths in 0_u32..25,
        ) {
            let overlap = f64::from(overlap_tenths) / 10.0;
            let zero = count_slots(starts(len, 3.0, 3.0, 1.5).into_iter().map(row), 3.0);
            let lapped = count_slots(
                starts(len, 3.0, 3.0 - overlap, 1.5).into_iter().map(row),
                3.0,
            );
            prop_assert!(lapped.slots.abs_diff(zero.rows) <= 1,
                "overlap {overlap}: {lapped:?} against {zero:?}");
            prop_assert!(lapped.rows >= zero.rows);
        }

        /// Never more slots than rows, and the order rows are read in does
        /// not matter.
        #[test]
        fn slots_are_bounded_by_rows_and_independent_of_order(
            offsets in proptest::collection::vec(0.0_f64..60.0, 0..40),
            seed in any::<u64>(),
        ) {
            let rows: Vec<_> = offsets.iter().map(|o| row(*o)).collect();
            let mut shuffled = rows.clone();
            // A deterministic permutation from the seed.
            let n = shuffled.len();
            if n > 1 {
                let mut s = seed;
                for i in (1..n).rev() {
                    s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    #[allow(clippy::cast_possible_truncation)]
                    let j = (s >> 33) as usize % (i + 1);
                    shuffled.swap(i, j);
                }
            }
            let a = count_slots(rows.iter().copied(), 3.0);
            let b = count_slots(shuffled.iter().copied(), 3.0);
            prop_assert_eq!(a, b);
            prop_assert!(a.slots <= a.rows);
            prop_assert_eq!(slot_representatives(&rows, 3.0).len() as u64, a.slots + a.unplaced);
        }
    }
}
