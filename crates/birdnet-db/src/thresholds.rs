//! Suggesting a per-species confidence threshold from the station's own reviews.
//!
//! # What this is for
//!
//! Per-species thresholds already exist, and an operator sets them by guessing:
//! a species keeps producing rubbish, so they try 0.85, then 0.9. Meanwhile the
//! station holds the evidence to answer the question properly — every
//! confirmed and rejected verdict, with the confidence the model gave it.
//!
//! # It only ever suggests
//!
//! Nothing here changes what a station records. The suggestion is rendered
//! beside the species with the evidence it came from, and applying it is the
//! operator pressing a button. A tuner that moved thresholds by itself would
//! let one bad afternoon of reviewing raise a species out of the record, and
//! the operator would have no way of knowing why the bird stopped appearing.
//!
//! # Why Youden's J
//!
//! For a candidate threshold *t*, "admit when confidence ≥ *t*" is a
//! classifier, and the reviews are its labels. Youden's J —
//! `sensitivity + specificity − 1` — is the standard single-number summary for
//! picking a cut-off, weights the two error kinds equally, and needs no
//! assumption about the shape of the distribution. On the tens-to-hundreds of
//! verdicts a station accumulates, every candidate can be evaluated exactly:
//! there is no optimisation to get wrong.
//!
//! Weighting the two errors equally is a choice, and the honest one to expose:
//! a station cannot know whether the operator would rather miss a bird or
//! record a phantom. The evidence that goes with the suggestion — how many
//! confirmations it would have cost, how many rejections it would have caught —
//! is what lets them decide.

/// One reviewed detection: what the model said, and what the operator said.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewedDetection {
    /// The model's confidence, in `[0, 1]`.
    pub confidence: f64,
    /// Whether the operator confirmed it. `false` means rejected.
    pub confirmed: bool,
}

/// A suggested threshold, with the evidence behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdSuggestion {
    /// The suggested confidence threshold.
    pub threshold: f64,
    /// Confirmed detections at or above it — kept, and correctly.
    pub confirmed_kept: usize,
    /// Confirmed detections below it — the cost of the suggestion.
    pub confirmed_lost: usize,
    /// Rejected detections below it — caught, which is the point.
    pub rejected_caught: usize,
    /// Rejected detections at or above it — still admitted.
    pub rejected_kept: usize,
}

impl ThresholdSuggestion {
    /// Youden's J for this split, in `[-1, 1]`.
    ///
    /// Reported so the UI can show *how well* the threshold separates, not
    /// only where it falls. A J near zero means the reviews carry no signal
    /// about confidence and the suggestion is not worth taking.
    #[must_use]
    // Review counts are in the hundreds; `usize`-to-`f64` cannot lose a bit
    // until 2^53 verdicts, which is more than a station will ever produce.
    #[allow(clippy::cast_precision_loss)]
    pub fn youden_j(&self) -> f64 {
        let confirmed = self.confirmed_kept + self.confirmed_lost;
        let rejected = self.rejected_caught + self.rejected_kept;
        if confirmed == 0 || rejected == 0 {
            return 0.0;
        }
        let sensitivity = self.confirmed_kept as f64 / confirmed as f64;
        let specificity = self.rejected_caught as f64 / rejected as f64;
        sensitivity + specificity - 1.0
    }
}

/// Fewest verdicts before a suggestion is offered at all.
///
/// Five is not a statistical claim; it is the point below which the suggestion
/// would be visibly arbitrary to the operator reading it. The real guard is
/// [`MIN_PER_CLASS`].
pub const MIN_VERDICTS: usize = 5;

/// Fewest of *each* verdict before a suggestion is offered.
///
/// With rejections but no confirmations the best separation is always "reject
/// everything", and with confirmations but no rejections it is "admit
/// everything". Neither is a threshold; both are what a one-sided review
/// history looks like, and it is the common case early on — an operator
/// confirms the interesting birds and never touches the rest.
pub const MIN_PER_CLASS: usize = 2;

/// Suggest a threshold for one species, or `None` if the evidence is too thin.
///
/// Candidates are the observed confidences themselves: a threshold between two
/// adjacent observations behaves identically to the higher one, so nothing is
/// gained by searching a grid, and a grid would report thresholds no detection
/// ever sat on.
///
/// Ties go to the **lowest** threshold — the least aggressive of the equally
/// good ones. When several cut-offs separate the reviews equally well, the one
/// that discards fewest future detections is the one to suggest.
#[must_use]
pub fn suggest_threshold(reviews: &[ReviewedDetection]) -> Option<ThresholdSuggestion> {
    if reviews.len() < MIN_VERDICTS {
        return None;
    }
    let confirmed = reviews.iter().filter(|r| r.confirmed).count();
    let rejected = reviews.len() - confirmed;
    if confirmed < MIN_PER_CLASS || rejected < MIN_PER_CLASS {
        return None;
    }

    let mut candidates: Vec<f64> = reviews.iter().map(|r| r.confidence).collect();
    candidates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    candidates.dedup_by(|a, b| (*a - *b).abs() < f64::EPSILON);

    let mut best: Option<ThresholdSuggestion> = None;
    for &t in &candidates {
        let split = split_at(reviews, t);
        let j = split.youden_j();
        // Strictly greater, so the first (lowest) of equally good candidates
        // wins — `candidates` is sorted ascending.
        if best.as_ref().is_none_or(|b| j > b.youden_j()) {
            best = Some(split);
        }
    }
    best
}

/// Count the four outcomes of admitting at `threshold`.
#[must_use]
fn split_at(reviews: &[ReviewedDetection], threshold: f64) -> ThresholdSuggestion {
    let mut s = ThresholdSuggestion {
        threshold,
        confirmed_kept: 0,
        confirmed_lost: 0,
        rejected_caught: 0,
        rejected_kept: 0,
    };
    for r in reviews {
        // `>=` matches how the threshold is applied downstream: a detection at
        // exactly the threshold is admitted. A `>` here would make the
        // suggestion disagree with the gate it is a suggestion for.
        match (r.confidence >= threshold, r.confirmed) {
            (true, true) => s.confirmed_kept += 1,
            (false, true) => s.confirmed_lost += 1,
            (false, false) => s.rejected_caught += 1,
            (true, false) => s.rejected_kept += 1,
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_PER_CLASS, MIN_VERDICTS, ReviewedDetection, ThresholdSuggestion, suggest_threshold,
    };

    /// `n` reviews at `confidence` with the given verdict.
    fn many(n: usize, confidence: f64, confirmed: bool) -> Vec<ReviewedDetection> {
        (0..n)
            .map(|_| ReviewedDetection {
                confidence,
                confirmed,
            })
            .collect()
    }

    /// Concatenate review groups.
    fn reviews(groups: &[Vec<ReviewedDetection>]) -> Vec<ReviewedDetection> {
        groups.iter().flatten().copied().collect()
    }

    // ── the suggestion ──────────────────────────────────────────────────

    #[test]
    fn a_clean_separation_is_suggested_at_the_lowest_confirmed_confidence() {
        // Rejections at 0.60–0.70, confirmations at 0.85–0.95. The threshold
        // that separates them perfectly is the lowest confirmed confidence:
        // any higher would start discarding confirmations for nothing.
        let r = reviews(&[
            many(3, 0.60, false),
            many(2, 0.70, false),
            many(3, 0.85, true),
            many(2, 0.95, true),
        ]);
        let s = suggest_threshold(&r).expect("a suggestion");
        assert!((s.threshold - 0.85).abs() < f64::EPSILON, "{}", s.threshold);
        assert_eq!(s.confirmed_kept, 5);
        assert_eq!(s.confirmed_lost, 0);
        assert_eq!(s.rejected_caught, 5);
        assert_eq!(s.rejected_kept, 0);
        assert!(
            (s.youden_j() - 1.0).abs() < f64::EPSILON,
            "a clean split is J=1"
        );
    }

    #[test]
    fn the_threshold_is_inclusive_matching_the_gate_it_advises() {
        // A detection at exactly the threshold is admitted downstream. A `>`
        // here would make the suggestion disagree with the gate it exists to
        // configure — the operator would apply 0.85, and the 0.85 detections
        // the suggestion counted as kept would be quarantined.
        let r = reviews(&[many(3, 0.5, false), many(3, 0.85, true)]);
        let s = suggest_threshold(&r).expect("a suggestion");
        assert!((s.threshold - 0.85).abs() < f64::EPSILON);
        assert_eq!(
            s.confirmed_kept, 3,
            "detections at exactly the threshold were counted as lost"
        );
    }

    #[test]
    fn ties_go_to_the_least_aggressive_threshold() {
        // Rejections all at 0.5, confirmations all at 0.9. Every candidate in
        // (0.5, 0.9] separates perfectly; suggesting 0.9 rather than the
        // lowest such value would needlessly discard any future detection
        // between them.
        let r = reviews(&[many(3, 0.50, false), many(3, 0.90, true)]);
        let s = suggest_threshold(&r).expect("a suggestion");
        assert!(
            (s.threshold - 0.90).abs() < f64::EPSILON,
            "expected the lowest perfectly-separating observed value, got {}",
            s.threshold
        );
    }

    #[test]
    fn an_overlapping_distribution_trades_the_two_errors() {
        // The realistic case: the classes overlap, so no threshold is perfect
        // and the answer is a trade. Six rejections at 0.6/0.8, six
        // confirmations at 0.7/0.9.
        let r = reviews(&[
            many(3, 0.60, false),
            many(3, 0.80, false),
            many(3, 0.70, true),
            many(3, 0.90, true),
        ]);
        let s = suggest_threshold(&r).expect("a suggestion");
        // J at 0.70: sens 6/6=1.0, spec 3/6=0.5 → 0.5
        // J at 0.80: sens 3/6=0.5, spec 3/6=0.5 → 0.0
        // J at 0.90: sens 3/6=0.5, spec 6/6=1.0 → 0.5
        // Tie between 0.70 and 0.90; the lower wins.
        assert!((s.threshold - 0.70).abs() < f64::EPSILON, "{}", s.threshold);
        assert!((s.youden_j() - 0.5).abs() < 1e-9, "{}", s.youden_j());
    }

    #[test]
    fn the_evidence_adds_up_to_every_review() {
        // The four counts are what the operator decides on. A miscount would
        // make the suggestion look better or worse than it is.
        let r = reviews(&[
            many(4, 0.60, false),
            many(2, 0.95, false),
            many(5, 0.90, true),
            many(1, 0.40, true),
        ]);
        let s = suggest_threshold(&r).expect("a suggestion");
        assert_eq!(
            s.confirmed_kept + s.confirmed_lost + s.rejected_caught + s.rejected_kept,
            12
        );
        assert_eq!(s.confirmed_kept + s.confirmed_lost, 6, "confirmations");
        assert_eq!(s.rejected_caught + s.rejected_kept, 6, "rejections");
    }

    // ── when not to suggest ─────────────────────────────────────────────

    #[test]
    fn too_few_verdicts_produces_no_suggestion() {
        let r = reviews(&[many(2, 0.5, false), many(2, 0.9, true)]);
        assert_eq!(r.len(), MIN_VERDICTS - 1);
        assert_eq!(suggest_threshold(&r), None);
    }

    #[test]
    fn enough_verdicts_produces_one() {
        // Counterpart, with the literal count rather than the constant: a test
        // written as `MIN_VERDICTS` on both sides moves with any mutation of
        // it and can never fail.
        assert_eq!(MIN_VERDICTS, 5, "the counts below assume this minimum");
        let r = reviews(&[many(3, 0.5, false), many(2, 0.9, true)]);
        assert_eq!(r.len(), 5);
        assert!(suggest_threshold(&r).is_some());
    }

    #[test]
    fn a_one_sided_review_history_produces_no_suggestion() {
        // The common case early on: an operator confirms the interesting birds
        // and never rejects anything. The best separation is then "admit
        // everything", which is not a threshold — and suggesting one anyway
        // would put a number in front of the operator with nothing behind it.
        assert_eq!(suggest_threshold(&many(20, 0.9, true)), None);
        assert_eq!(suggest_threshold(&many(20, 0.5, false)), None);

        // One of a class is still not enough.
        assert_eq!(MIN_PER_CLASS, 2, "the counts below assume this minimum");
        let barely = reviews(&[many(9, 0.9, true), many(1, 0.5, false)]);
        assert_eq!(suggest_threshold(&barely), None);
        let enough = reviews(&[many(9, 0.9, true), many(2, 0.5, false)]);
        assert!(suggest_threshold(&enough).is_some());
    }

    // ── the quality measure ─────────────────────────────────────────────

    #[test]
    fn youden_j_is_zero_when_confidence_carries_no_signal() {
        // Identical distributions: every threshold splits both classes the
        // same way, so the suggestion is worthless and must say so rather than
        // presenting a number as if it separated something.
        let r = reviews(&[
            many(3, 0.6, false),
            many(3, 0.9, false),
            many(3, 0.6, true),
            many(3, 0.9, true),
        ]);
        let s = suggest_threshold(&r).expect("a suggestion");
        assert!(s.youden_j().abs() < 1e-9, "J was {}", s.youden_j());
    }

    #[test]
    fn youden_j_is_one_for_a_perfect_split() {
        // Counterpart to the gate above: a J that was always zero would pass
        // it, and the UI could never tell a good suggestion from a useless one.
        let r = reviews(&[many(3, 0.5, false), many(3, 0.9, true)]);
        assert!((suggest_threshold(&r).unwrap().youden_j() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn youden_j_of_a_one_sided_split_is_zero_rather_than_a_division_by_zero() {
        // Reachable through the public struct even though `suggest_threshold`
        // will not produce it, and `0/0` is NaN — which compares false against
        // everything and would sort such a row to an arbitrary place.
        let s = ThresholdSuggestion {
            threshold: 0.9,
            confirmed_kept: 3,
            confirmed_lost: 0,
            rejected_caught: 0,
            rejected_kept: 0,
        };
        assert!(s.youden_j().abs() < f64::EPSILON, "J was {}", s.youden_j());
    }
}
