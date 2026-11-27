//! Kinds of birds at an equal amount heard: rarefied richness (CHT-07).
//!
//! The number of species heard on a day rises with how much was heard, whatever is present: a day
//! with four hundred detections names more species than a day with forty from the same pool. To
//! compare days, each day's richness is taken at a common number of detections `m`: the expected
//! number of species in `m` of the day's detections drawn without replacement (Hurlbert 1971;
//! Sanders 1968; the formula is cited from general knowledge and checked in the tests against
//! cases worked by hand):
//!
//! ```text
//! E[S_m] = Σ_i ( 1 − C(N − N_i, m) / C(N, m) )
//! ```
//!
//! where `N` is the day's detections and `N_i` those of species `i`. It is exact, so there is no
//! resampling and no seed. A day with fewer than `m` detections cannot be taken down to `m`: it is
//! too few to say, never a zero. Detections are not individuals, and a bird singing all morning is
//! many detections; the measure equalises how much was *heard*, which is what the raw count is
//! mostly a picture of, and says nothing about how many birds there were.

/// The common depth for a set of days.
///
/// It is the 25th percentile (nearest rank) of their detection totals, and at least [`MIN_DEPTH`]. A quarter of the days are then too few to say, which keeps
/// the depth from being dragged to the quietest day, where every day would be read at a handful
/// of detections. `None` for no days.
#[must_use]
pub fn common_depth(totals: &[u64]) -> Option<u64> {
    let mut t: Vec<u64> = totals.iter().copied().filter(|&n| n > 0).collect();
    if t.is_empty() {
        return None;
    }
    t.sort_unstable();
    let rank = t.len().div_ceil(4); // ceil(0.25 · len), nearest rank, at least 1
    Some(t[rank.saturating_sub(1)].max(MIN_DEPTH))
}

/// The smallest depth a day is read at: below ten detections the expected richness is mostly the
/// draw.
pub const MIN_DEPTH: u64 = 10;

/// `ln C(n, k)`, by summing logs (n is a day's detections: thousands at most).
fn ln_choose(n: u64, k: u64) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    let k = k.min(n - k);
    #[allow(clippy::cast_precision_loss)]
    (0..k)
        .map(|i| ((n - i) as f64).ln() - ((i + 1) as f64).ln())
        .sum()
}

/// The expected number of species in `m` of a day's detections, `counts` being the detections of
/// each species heard that day. `None` when the day had fewer than `m` detections (too few to
/// say) or `m` is zero.
#[must_use]
pub fn rarefied(counts: &[u64], m: u64) -> Option<f64> {
    let n: u64 = counts.iter().sum();
    if m == 0 || n < m {
        return None;
    }
    let all = ln_choose(n, m);
    Some(
        counts
            .iter()
            .filter(|&&c| c > 0)
            .map(|&c| 1.0 - (ln_choose(n - c, m) - all).exp())
            .sum(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand cases: at `m = N` every species is in the draw; at `m = 1` one species is; two
    /// species of 1 and 1 at `m = 1` is 1, and of 2 and 2 at `m = 2` is 1 + 4/6 (the chance the
    /// two drawn are of two kinds is 4 of the 6 pairs).
    #[test]
    fn the_expected_richness_against_hand_values() {
        assert!((rarefied(&[5, 3, 1], 9).unwrap() - 3.0).abs() < 1e-9);
        assert!((rarefied(&[5, 3, 1], 1).unwrap() - 1.0).abs() < 1e-9);
        assert!((rarefied(&[2, 2], 2).unwrap() - (1.0 + 4.0 / 6.0)).abs() < 1e-9);
        assert_eq!(rarefied(&[2, 2], 5), None, "fewer than m is too few to say");
        assert_eq!(rarefied(&[], 3), None);
    }

    /// The acceptance gate (CHT-07): two days drawn from the same pool, one heard five times as
    /// much, name different numbers of species raw and the same number, within a tolerance, at
    /// a common depth.
    #[test]
    fn hearing_more_of_the_same_pool_does_not_raise_rarefied_richness() {
        // A pool of 12 species with uneven shares; the quiet day is the busy day's shares at a
        // fifth of the detections, so species with a small share are missed on it.
        let busy: Vec<u64> = vec![120, 80, 60, 40, 30, 20, 15, 10, 8, 5, 4, 3];
        let quiet: Vec<u64> = busy.iter().map(|c| c / 5).collect();
        let raw_busy = busy.iter().filter(|&&c| c > 0).count();
        let raw_quiet = quiet.iter().filter(|&&c| c > 0).count();
        assert!(raw_busy > raw_quiet, "{raw_busy} {raw_quiet}");
        let m = 50;
        let a = rarefied(&busy, m).unwrap();
        let b = rarefied(&quiet, m).unwrap();
        assert!((a - b).abs() < 0.6, "busy {a:.2} quiet {b:.2}");
    }

    #[test]
    fn the_depth_is_the_lower_quartile_and_never_tiny() {
        assert_eq!(common_depth(&[]), None);
        assert_eq!(common_depth(&[3, 4]), Some(MIN_DEPTH));
        assert_eq!(common_depth(&[100, 40, 300, 80, 0]), Some(40));
        assert_eq!(common_depth(&[10, 20, 30, 40, 50, 60, 70, 80]), Some(20));
    }
}
