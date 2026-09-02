//! Which species look like model artefacts rather than birds.
//!
//! # The problem
//!
//! Every station accumulates a few species it has never actually had. A dog
//! barks, a fence creaks, a neighbour's wind chime rings, and the classifier
//! reaches for the nearest bird — the same nearest bird, every time, because
//! the sound is the same. After a season the species has a hundred detections
//! and appears in the life list, the species richness count and the
//! co-occurrence matrix as though it were a resident.
//!
//! They are hard to spot by eye precisely because they look like everything
//! else in a list of names and counts. What separates them is the *shape* of
//! their detections, and that is computable.
//!
//! # It reports; it does not act
//!
//! This is a heuristic. It is wrong about a genuinely scarce visitor that
//! happened to be recorded three times in one afternoon, and it will stay
//! wrong about that, because nothing in the data distinguishes that bird from
//! an artefact. So the output is a list with its reasons attached and a button
//! that adds the species to the exclusion list the operator already has —
//! never an automatic filter. A species removed from a station's record by a
//! heuristic that nobody agreed to is worse than the phantom it removed.
//!
//! # The signals
//!
//! Each is a separate, nameable observation rather than a term in an opaque
//! score, because the operator has to judge the verdict and cannot judge a
//! number. [`PhantomSignal`] is what the UI prints.

/// One reason a species looks like an artefact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhantomSignal {
    /// Every review of this species has been a rejection.
    ///
    /// The strongest signal available, because it is the operator's own
    /// judgement rather than an inference about it.
    NeverConfirmed,
    /// The species has never been detected confidently.
    ///
    /// A bird that is really present eventually sings close to the microphone
    /// on a still morning. One that never clears the floor by a margin is
    /// something the model is guessing at.
    NeverConfident,
    /// Its confidences occupy a very narrow band.
    ///
    /// Real detections of a real species vary — different distances, different
    /// calls, different weather. A repeated non-bird sound produces the same
    /// score every time because it is the same sound.
    NoConfidenceSpread,
    /// Many detections, on very few days.
    ///
    /// The signature of one noisy afternoon: building work, a visiting dog, a
    /// tractor. A resident is heard across many days.
    ConfinedToFewDays,
}

impl PhantomSignal {
    /// A short label for the UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NeverConfirmed => "every review rejected",
            Self::NeverConfident => "never detected confidently",
            Self::NoConfidenceSpread => "confidence never varies",
            Self::ConfinedToFewDays => "many detections, few days",
        }
    }
}

/// What is known about one species' detections.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesShape {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Total detections.
    pub total: i64,
    /// Distinct calendar dates it was detected on.
    pub distinct_days: i64,
    /// Lowest confidence recorded.
    pub min_confidence: f64,
    /// Highest confidence recorded.
    pub max_confidence: f64,
    /// Reviews that confirmed it.
    pub confirmed: i64,
    /// Reviews that rejected it.
    pub rejected: i64,
}

/// A species flagged as a possible phantom, with its reasons.
#[derive(Debug, Clone, PartialEq)]
pub struct PhantomReport {
    /// The species and its shape, so the UI can show the numbers behind the
    /// verdict rather than only the verdict.
    pub shape: SpeciesShape,
    /// Why it was flagged. Never empty.
    pub signals: Vec<PhantomSignal>,
}

/// Detections below which a species is never flagged.
///
/// A species with three detections has no shape to read. Flagging it would
/// mean flagging every genuine scarce visitor on the day it arrived, which is
/// the one thing this must not do.
pub const MIN_DETECTIONS: i64 = 10;

/// Reviews below which [`PhantomSignal::NeverConfirmed`] is not claimed.
pub const MIN_REVIEWS: i64 = 3;

/// How far above its lowest confidence a species must reach, once, to escape
/// [`PhantomSignal::NeverConfident`].
pub const CONFIDENT_MARGIN: f64 = 0.10;

/// Confidence range below which [`PhantomSignal::NoConfidenceSpread`] fires.
pub const FLAT_SPREAD: f64 = 0.05;

/// Days at or below which [`PhantomSignal::ConfinedToFewDays`] fires.
pub const FEW_DAYS: i64 = 2;

/// Signals required before a species is reported at all.
///
/// One signal on its own is too easily an accident of a short history — a
/// scarce migrant heard twice in one morning trips `ConfinedToFewDays` and
/// nothing else. Two independent signals is where the shape stops looking like
/// a bird.
pub const MIN_SIGNALS: usize = 2;

/// Which signals fire for one species.
///
/// Order is fixed strongest-first so the UI's first reason is its best one.
#[must_use]
pub fn signals_for(shape: &SpeciesShape) -> Vec<PhantomSignal> {
    let mut signals = Vec::new();
    if shape.total < MIN_DETECTIONS {
        return signals;
    }

    let reviews = shape.confirmed + shape.rejected;
    if reviews >= MIN_REVIEWS && shape.confirmed == 0 {
        signals.push(PhantomSignal::NeverConfirmed);
    }
    // Relative to its own floor, not to a global constant: what counts as
    // "confident" depends on the species and on where the operator set the
    // threshold, and a fixed 0.9 would flag every species at a station running
    // a low global threshold.
    if shape.max_confidence - shape.min_confidence < CONFIDENT_MARGIN && shape.max_confidence < 1.0
    {
        signals.push(PhantomSignal::NeverConfident);
    }
    if shape.max_confidence - shape.min_confidence < FLAT_SPREAD {
        signals.push(PhantomSignal::NoConfidenceSpread);
    }
    if shape.distinct_days <= FEW_DAYS {
        signals.push(PhantomSignal::ConfinedToFewDays);
    }
    signals
}

/// Report the species whose detections look like artefacts.
///
/// Ordered by signal count descending, then by detection count descending — the
/// species doing the most damage to the record first.
#[must_use]
pub fn report(shapes: &[SpeciesShape]) -> Vec<PhantomReport> {
    let mut out: Vec<PhantomReport> = shapes
        .iter()
        .filter_map(|shape| {
            let signals = signals_for(shape);
            (signals.len() >= MIN_SIGNALS).then(|| PhantomReport {
                shape: shape.clone(),
                signals,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.signals
            .len()
            .cmp(&a.signals.len())
            .then(b.shape.total.cmp(&a.shape.total))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIDENT_MARGIN, FEW_DAYS, FLAT_SPREAD, MIN_DETECTIONS, MIN_REVIEWS, MIN_SIGNALS,
        PhantomSignal, SpeciesShape, report, signals_for,
    };

    /// A healthy resident: many detections, many days, a wide confidence
    /// spread, confirmed on review. Nothing about it should fire.
    fn resident() -> SpeciesShape {
        SpeciesShape {
            sci_name: "Turdus merula".into(),
            com_name: "Eurasian Blackbird".into(),
            total: 400,
            distinct_days: 120,
            min_confidence: 0.55,
            max_confidence: 0.98,
            confirmed: 12,
            rejected: 1,
        }
    }

    /// A textbook phantom: a repeated non-bird sound.
    fn phantom() -> SpeciesShape {
        SpeciesShape {
            sci_name: "Phantomus fictus".into(),
            com_name: "Not A Bird".into(),
            total: 80,
            distinct_days: 2,
            min_confidence: 0.71,
            max_confidence: 0.73,
            confirmed: 0,
            rejected: 6,
        }
    }

    // ── the signals ─────────────────────────────────────────────────────

    #[test]
    fn a_healthy_resident_fires_nothing() {
        // The gate that matters most: this must not flag ordinary birds, or
        // the operator stops reading it and the feature is worse than absent.
        assert!(signals_for(&resident()).is_empty());
        assert!(report(&[resident()]).is_empty());
    }

    #[test]
    fn a_textbook_phantom_fires_every_signal() {
        let signals = signals_for(&phantom());
        for expected in [
            PhantomSignal::NeverConfirmed,
            PhantomSignal::NeverConfident,
            PhantomSignal::NoConfidenceSpread,
            PhantomSignal::ConfinedToFewDays,
        ] {
            assert!(signals.contains(&expected), "{expected:?} did not fire");
        }
    }

    #[test]
    fn never_confirmed_needs_enough_reviews_to_mean_anything() {
        // One rejection is an opinion about one detection. The signal claims
        // something about the species.
        assert_eq!(MIN_REVIEWS, 3, "the counts below assume this minimum");
        let mut s = resident();
        s.confirmed = 0;
        s.rejected = 2;
        assert!(!signals_for(&s).contains(&PhantomSignal::NeverConfirmed));
        s.rejected = 3;
        assert!(signals_for(&s).contains(&PhantomSignal::NeverConfirmed));
    }

    #[test]
    fn a_single_confirmation_clears_never_confirmed() {
        // Counterpart: the signal is about *never*, and one confirmation from
        // the operator settles the question the heuristic is guessing at.
        let mut s = phantom();
        s.confirmed = 1;
        assert!(!signals_for(&s).contains(&PhantomSignal::NeverConfirmed));
    }

    #[test]
    fn an_unreviewed_species_does_not_fire_never_confirmed() {
        // No reviews is not evidence of rejection. Treating it as such would
        // fire on every species at a station whose operator never reviews.
        let mut s = phantom();
        s.confirmed = 0;
        s.rejected = 0;
        assert!(!signals_for(&s).contains(&PhantomSignal::NeverConfirmed));
    }

    #[test]
    fn confidence_spread_is_measured_against_the_species_own_floor() {
        // Not against a fixed 0.9: what counts as confident depends on where
        // the operator set their threshold, and an absolute cut would fire on
        // every species at a station running a low one.
        let mut low_but_varied = resident();
        low_but_varied.min_confidence = 0.30;
        low_but_varied.max_confidence = 0.62;
        assert!(
            !signals_for(&low_but_varied).contains(&PhantomSignal::NeverConfident),
            "a station with a low threshold had a normal species flagged"
        );

        let mut high_but_flat = resident();
        high_but_flat.min_confidence = 0.90;
        high_but_flat.max_confidence = 0.95;
        assert!(
            signals_for(&high_but_flat).contains(&PhantomSignal::NeverConfident),
            "a flat band high up is still a flat band"
        );
    }

    #[test]
    fn each_signal_fires_on_one_side_of_its_constant_and_not_the_other() {
        // A clear margin either side rather than the exact boundary. `0.70 +
        // FLAT_SPREAD` is not representable, and lands fractionally *below*
        // the sum — an earlier version of this test asserted the exact
        // boundary and failed for that reason alone, which says nothing about
        // the code. What is worth pinning is the direction: too flat fires,
        // varied does not.
        let margin = 0.01;

        let mut s = resident();
        s.min_confidence = 0.70;
        s.max_confidence = 0.70 + FLAT_SPREAD + margin;
        assert!(!signals_for(&s).contains(&PhantomSignal::NoConfidenceSpread));
        s.max_confidence = 0.70 + FLAT_SPREAD - margin;
        assert!(signals_for(&s).contains(&PhantomSignal::NoConfidenceSpread));

        let mut s = resident();
        s.min_confidence = 0.70;
        s.max_confidence = 0.70 + CONFIDENT_MARGIN + margin;
        assert!(!signals_for(&s).contains(&PhantomSignal::NeverConfident));
        s.max_confidence = 0.70 + CONFIDENT_MARGIN - margin;
        assert!(signals_for(&s).contains(&PhantomSignal::NeverConfident));

        // Days are integers, so this boundary *is* exact and worth pinning as
        // one: `<=` versus `<` changes which stations see the signal at all.
        let mut s = resident();
        s.distinct_days = FEW_DAYS;
        assert!(signals_for(&s).contains(&PhantomSignal::ConfinedToFewDays));
        s.distinct_days = FEW_DAYS + 1;
        assert!(!signals_for(&s).contains(&PhantomSignal::ConfinedToFewDays));
    }

    #[test]
    fn a_species_at_full_confidence_is_not_called_unconfident() {
        // A species pinned at 1.0 has zero spread, but "never detected
        // confidently" is plainly false of it. Without the guard the label
        // would contradict the number printed beside it.
        let mut s = resident();
        s.min_confidence = 1.0;
        s.max_confidence = 1.0;
        assert!(!signals_for(&s).contains(&PhantomSignal::NeverConfident));
        // It is still flat, which is a different and honest claim.
        assert!(signals_for(&s).contains(&PhantomSignal::NoConfidenceSpread));
    }

    // ── what gets reported ──────────────────────────────────────────────

    #[test]
    fn a_species_with_too_few_detections_is_never_flagged() {
        // A scarce migrant heard twice in one morning has no shape to read,
        // and flagging it is the one thing this must not do.
        assert_eq!(MIN_DETECTIONS, 10, "the counts below assume this minimum");
        let mut s = phantom();
        s.total = 9;
        assert!(signals_for(&s).is_empty());
        s.total = 10;
        assert!(!signals_for(&s).is_empty());
    }

    #[test]
    fn one_signal_alone_is_not_reported() {
        // A genuinely scarce visitor recorded on one day trips
        // `ConfinedToFewDays` and nothing else. Two independent signals is
        // where the shape stops looking like a bird.
        assert_eq!(MIN_SIGNALS, 2, "the counts below assume this minimum");
        let mut scarce = resident();
        scarce.total = 20;
        scarce.distinct_days = 1;
        assert_eq!(signals_for(&scarce), vec![PhantomSignal::ConfinedToFewDays]);
        assert!(
            report(&[scarce]).is_empty(),
            "a one-day visitor was reported as a phantom"
        );
    }

    #[test]
    fn two_signals_are_reported() {
        // Counterpart: a threshold that reported nothing would satisfy every
        // negative gate above.
        let mut two = resident();
        two.distinct_days = 1;
        two.min_confidence = 0.70;
        two.max_confidence = 0.72;
        let signals = signals_for(&two);
        assert!(signals.len() >= MIN_SIGNALS, "{signals:?}");
        assert_eq!(report(&[two]).len(), 1);
    }

    #[test]
    fn the_worst_offenders_are_reported_first() {
        // The list is acted on from the top, so the ordering is the feature.
        let mut two_signals = resident();
        two_signals.sci_name = "Two signals".into();
        two_signals.distinct_days = 1;
        two_signals.min_confidence = 0.70;
        two_signals.max_confidence = 0.72;

        let mut louder = phantom();
        louder.sci_name = "Four signals".into();

        let out = report(&[two_signals, louder]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].shape.sci_name, "Four signals", "ordered by signals");
    }

    #[test]
    fn equal_signal_counts_are_ordered_by_how_much_they_pollute() {
        // Two species flagged for the same reasons: the one with 500
        // detections has done more to the record than the one with 12.
        let mut small = phantom();
        small.sci_name = "Small".into();
        small.total = 12;
        let mut large = phantom();
        large.sci_name = "Large".into();
        large.total = 500;

        let out = report(&[small, large]);
        assert_eq!(out[0].shape.sci_name, "Large");
        assert_eq!(
            out[0].signals.len(),
            out[1].signals.len(),
            "test setup: the two must be tied on signals"
        );
    }

    #[test]
    fn every_reported_species_carries_its_reasons() {
        // The operator is being asked to exclude a species from their own
        // record. A verdict with no reasons attached is not something anyone
        // can act on.
        for r in report(&[phantom(), resident()]) {
            assert!(
                !r.signals.is_empty(),
                "{} was reported bare",
                r.shape.sci_name
            );
        }
    }
}
