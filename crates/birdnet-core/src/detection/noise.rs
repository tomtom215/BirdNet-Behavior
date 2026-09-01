//! Non-bird noise-class filter: suppress a chunk a dog was barking in.
//!
//! # The problem
//!
//! `BirdNET`'s label set is not birds only. Alongside the species it carries a
//! handful of non-bird classes — `Dog`, `Siren`, `Engine`, `Fireworks`,
//! `Power tools`, `Gun`, `Environmental`, `Noise` — because the training data
//! contains them and a classifier that could not name them would have to call
//! them something.
//!
//! A dog barking near the microphone is broadband and percussive, and the
//! classifier does not simply answer `Dog` and stop: it also produces
//! confident-looking scores for whatever species the bark most resembles. The
//! station then records a bird that was never there, and because the barking
//! is regular — the same dog, the same garden, every evening — the phantom
//! accumulates until it looks like a resident.
//!
//! # What this does
//!
//! When a watched noise class scores at or above [`NoiseFilter::threshold`] in
//! a chunk, every detection in that chunk is dropped. It is the same position
//! in the chain as [`crate::detection::privacy::PrivacyFilter`], and runs
//! immediately after it.
//!
//! # Two deliberate differences from the privacy filter
//!
//! **It gates on confidence, not rank.** The privacy filter asks whether a
//! human label appears within the top *N* predictions. That reads as a
//! sensitivity control and, at the shipped `top_n` of 10, is not one: the
//! cutoff is `max(10, …)` and the list handed to it is already truncated to
//! `top_n`, so it never excludes anything. (It starts to bite only if an
//! operator raises `top_n` above 10 — verified by probe.) A confidence
//! threshold means what it says at any `top_n`, and is the same unit the
//! operator already sets everywhere else in the detection settings.
//!
//! **It does not suppress neighbouring chunks.** The privacy filter masks the
//! chunks either side, which is right for speech: a conversation runs for
//! seconds and a person is identifiable across the whole of it. A bark is a
//! few hundred milliseconds. Spreading would discard several seconds of
//! usable dawn chorus for each one, and it is not needed to catch a bark that
//! straddles a boundary — the pipeline's chunks overlap, so such a bark is
//! present in both chunks and each is judged on its own evidence.

use crate::detection::types::Detection;

/// Noise classes watched unless the operator names their own.
///
/// Only the dog. The other non-bird classes are deliberately not here:
/// `Noise` and `Environmental` score highly on ordinary quiet recordings, so
/// watching them by default would suppress most of the night; `Siren` and
/// `Engine` matter only near a road, and an operator there can say so.
pub const DEFAULT_NOISE_CLASSES: &[&str] = &["Dog"];

/// Suppresses whole chunks in which a non-bird noise class was heard.
#[derive(Debug, Clone)]
pub struct NoiseFilter {
    /// Confidence at or above which a watched class suppresses its chunk.
    ///
    /// `0.0` disables the filter entirely.
    threshold: f32,
    /// Label names to watch, as the operator entered them.
    classes: Vec<String>,
}

impl NoiseFilter {
    /// A filter watching `classes` at `threshold`.
    ///
    /// A `threshold` of `0.0`, or an empty `classes`, disables it — a filter
    /// that watches nothing must not be reported as enabled, or the startup
    /// log tells the operator a protection is on that is not.
    #[must_use]
    pub fn new(threshold: f32, classes: Vec<String>) -> Self {
        Self {
            threshold,
            classes: classes
                .into_iter()
                .filter(|c| !c.trim().is_empty())
                .collect(),
        }
    }

    /// A filter watching [`DEFAULT_NOISE_CLASSES`] at `threshold`.
    #[must_use]
    pub fn with_default_classes(threshold: f32) -> Self {
        Self::new(
            threshold,
            DEFAULT_NOISE_CLASSES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        )
    }

    /// Whether the filter will suppress anything.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.threshold > 0.0 && !self.classes.is_empty()
    }

    /// The configured threshold.
    #[must_use]
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// The classes being watched.
    #[must_use]
    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    /// The watched class heard in this chunk, if any.
    ///
    /// Returns the matching detection so the caller can log *what* silenced
    /// the chunk and at what confidence. "Detections were dropped" and "a dog
    /// barked at 0.91" are very different lines to find in a journal when an
    /// operator is asking why their garden went quiet.
    #[must_use]
    pub fn offending<'a>(&self, chunk: &'a [Detection]) -> Option<&'a Detection> {
        if !self.is_enabled() {
            return None;
        }
        chunk.iter().find(|d| {
            d.confidence >= self.threshold
                && self.classes.iter().any(|class| names_detection(class, d))
        })
    }

    /// Drop every detection in each chunk a watched class was heard in.
    #[must_use]
    pub fn filter_predictions(&self, predictions: &[Vec<Detection>]) -> Vec<Vec<Detection>> {
        if !self.is_enabled() {
            return predictions.to_vec();
        }
        predictions
            .iter()
            .map(|chunk| {
                self.offending(chunk).map_or_else(
                    || chunk.clone(),
                    |noise| {
                        tracing::debug!(
                            class = %noise.common_name,
                            confidence = noise.confidence,
                            suppressed = chunk.len(),
                            "noise filter: discarding a chunk"
                        );
                        Vec::new()
                    },
                )
            })
            .collect()
    }
}

/// Whether an operator-entered `class` names this detection.
///
/// Exact match on the trimmed common *or* scientific name, case-insensitively
/// — the same rule as
/// [`crate::inference::species_filter::matches_species`], which is what the
/// operator's other name lists use.
///
/// Exact rather than substring, unlike the privacy filter's
/// `name.contains("human")`. A substring rule cannot be given to an operator:
/// `Gun` would match a Guineafowl, `Dog` a Dogwood-named taxon, and the
/// resulting silence is invisible — the detections simply never appear.
#[must_use]
fn names_detection(class: &str, detection: &Detection) -> bool {
    let class = class.trim();
    !class.is_empty()
        && (class.eq_ignore_ascii_case(detection.common_name.trim())
            || class.eq_ignore_ascii_case(detection.scientific_name.trim()))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_NOISE_CLASSES, NoiseFilter, names_detection};
    use crate::detection::types::Detection;

    /// A detection with the given names and confidence.
    fn d(sci: &str, com: &str, confidence: f32) -> Detection {
        Detection {
            date: "2026-03-14".into(),
            time: "19:40:00".into(),
            scientific_name: sci.into(),
            common_name: com.into(),
            confidence,
            start: 0.0,
            stop: 3.0,
            week: 11,
            file_name_extr: None,
        }
    }

    /// A bark at `confidence`, alongside the bird it was misheard as.
    fn barked_chunk(confidence: f32) -> Vec<Detection> {
        vec![
            d("Dog", "Dog", confidence),
            d("Turdus merula", "Eurasian Blackbird", 0.82),
        ]
    }

    // ── the suppression decision ────────────────────────────────────────

    #[test]
    fn a_confident_bark_takes_its_chunk_with_it() {
        // The whole point: the blackbird in this chunk is what the bark was
        // misheard as, and it is what would otherwise be recorded.
        let f = NoiseFilter::with_default_classes(0.5);
        let out = f.filter_predictions(&[barked_chunk(0.91)]);
        assert!(out[0].is_empty(), "the misheard bird survived the bark");
    }

    #[test]
    fn a_bark_below_the_threshold_suppresses_nothing() {
        // Counterpart, and the reason the threshold is a threshold: `Dog`
        // scores a little on all sorts of things. A filter that fired on any
        // non-zero dog score would silence most of a suburban evening.
        let f = NoiseFilter::with_default_classes(0.5);
        let out = f.filter_predictions(&[barked_chunk(0.49)]);
        assert_eq!(out[0].len(), 2, "a quiet dog score suppressed the chunk");
    }

    #[test]
    fn the_threshold_boundary_is_inclusive() {
        // Pinned because the rejecting side alone would be satisfied by `>`
        // and by `>=`, and the two differ for an operator who sets the
        // threshold to exactly the score they saw in the log.
        let f = NoiseFilter::with_default_classes(0.5);
        assert!(f.filter_predictions(&[barked_chunk(0.5)])[0].is_empty());
    }

    #[test]
    fn only_the_chunk_that_barked_is_discarded() {
        // A bark is a few hundred milliseconds. Spreading to the neighbours,
        // as the privacy filter does for speech, would throw away seconds of
        // dawn chorus for each one.
        let f = NoiseFilter::with_default_classes(0.5);
        let quiet = vec![d("Parus major", "Great Tit", 0.9)];
        let out = f.filter_predictions(&[quiet.clone(), barked_chunk(0.91), quiet]);
        assert_eq!(out[0].len(), 1, "the chunk before the bark was discarded");
        assert!(out[1].is_empty());
        assert_eq!(out[2].len(), 1, "the chunk after the bark was discarded");
    }

    #[test]
    fn a_chunk_with_no_watched_class_is_passed_through_untouched() {
        let f = NoiseFilter::with_default_classes(0.5);
        let chunk = vec![
            d("Turdus merula", "Eurasian Blackbird", 0.95),
            d("Siren", "Siren", 0.99),
        ];
        let out = f.filter_predictions(std::slice::from_ref(&chunk));
        assert_eq!(
            out[0].len(),
            2,
            "a class nobody asked to watch suppressed a chunk"
        );
    }

    #[test]
    fn an_operator_can_watch_the_classes_their_site_actually_has() {
        // Counterpart to the gate above: `Siren` is not watched by default,
        // but a station beside a fire station must be able to say so.
        let f = NoiseFilter::new(0.5, vec!["Siren".into(), "Engine".into()]);
        let chunk = vec![
            d("Siren", "Siren", 0.88),
            d("Turdus merula", "Eurasian Blackbird", 0.82),
        ];
        assert!(f.filter_predictions(&[chunk])[0].is_empty());
    }

    // ── what "enabled" means ────────────────────────────────────────────

    #[test]
    fn a_zero_threshold_disables_the_filter() {
        let f = NoiseFilter::with_default_classes(0.0);
        assert!(!f.is_enabled());
        assert_eq!(f.filter_predictions(&[barked_chunk(0.99)])[0].len(), 2);
    }

    #[test]
    fn a_filter_watching_nothing_does_not_report_itself_as_enabled() {
        // The startup log prints this. A filter that says it is on while
        // watching an empty list tells the operator a protection exists that
        // does not, which is worse than saying it is off.
        for classes in [vec![], vec!["   ".to_owned()], vec![String::new()]] {
            let f = NoiseFilter::new(0.5, classes.clone());
            assert!(!f.is_enabled(), "enabled while watching {classes:?}");
            assert_eq!(f.filter_predictions(&[barked_chunk(0.99)])[0].len(), 2);
        }
    }

    #[test]
    fn blank_entries_are_dropped_but_real_ones_alongside_them_survive() {
        // Counterpart: discarding the whole list because it contains a stray
        // blank would silently disable the filter for an operator whose
        // config has a trailing comma.
        let f = NoiseFilter::new(0.5, vec!["  ".into(), "Dog".into()]);
        assert!(f.is_enabled());
        assert_eq!(f.classes(), ["Dog"]);
        assert!(f.filter_predictions(&[barked_chunk(0.91)])[0].is_empty());
    }

    // ── name matching ───────────────────────────────────────────────────

    #[test]
    fn a_class_is_matched_on_either_name_and_ignoring_case() {
        let dog = d("Dog", "Dog", 0.9);
        for entry in ["Dog", "dog", "DOG", "  Dog  "] {
            assert!(names_detection(entry, &dog), "{entry:?} did not match");
        }
        // Either name, as the operator's other lists allow.
        let coyote = d("Canis latrans", "Coyote", 0.9);
        assert!(names_detection("Canis latrans", &coyote));
        assert!(names_detection("Coyote", &coyote));
    }

    #[test]
    fn matching_is_exact_rather_than_by_substring() {
        // The privacy filter uses `name.contains("human")`. That rule cannot
        // be handed to an operator: watching `Gun` would silence every
        // Guineafowl, and the loss is invisible — the bird simply never
        // appears — and the operator has no way to connect the two.
        let guineafowl = d("Numida meleagris", "Helmeted Guineafowl", 0.93);
        assert!(!names_detection("Gun", &guineafowl));
        assert!(!names_detection(
            "Dog",
            &d("Cornus florida", "Dogwood Warbler", 0.9)
        ));

        let f = NoiseFilter::new(0.5, vec!["Gun".into()]);
        assert_eq!(
            f.filter_predictions(&[vec![guineafowl]])[0].len(),
            1,
            "a Guineafowl was silenced by a filter watching for gunshots"
        );
    }

    #[test]
    fn the_offending_detection_is_reported_not_just_the_fact_of_it() {
        // The log line names what silenced the chunk. "Detections were
        // dropped" and "a dog barked at 0.91" are very different things to
        // find when an operator asks why their garden went quiet.
        let f = NoiseFilter::with_default_classes(0.5);
        let chunk = barked_chunk(0.91);
        let offender = f.offending(&chunk).expect("the bark is found");
        assert_eq!(offender.common_name, "Dog");
        assert!((offender.confidence - 0.91).abs() < f32::EPSILON);

        assert!(
            f.offending(&[d("Parus major", "Great Tit", 0.99)])
                .is_none(),
            "a chunk with no watched class reported an offender"
        );
    }

    #[test]
    fn the_default_watch_list_is_the_dog_alone() {
        // `Noise` and `Environmental` score highly on ordinary quiet
        // recordings; defaulting to them would suppress most of the night.
        assert_eq!(DEFAULT_NOISE_CLASSES, ["Dog"]);
    }
}
