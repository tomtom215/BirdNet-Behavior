//! What each classifier said about a species in one chunk (`G-10`, migration 57).
//!
//! [`merge_verdicts`](super::merge::merge_verdicts) keeps one detection per
//! species, carrying the winning classifier's confidence and a count of how many
//! agreed. Every other classifier's own score died there, so a species two
//! models scored 91% and 88% could not be told from one a second model scored
//! 12%, and nobody could later test whether agreement predicts a correct
//! identification, because the numbers that would be tested were never kept.
//!
//! This module builds the missing half: for a species, one
//! [`ModelAnswer`] per classifier that listened to the chunk.
//!
//! # The score is the classifier's own
//!
//! It is the same number the classifier's confidence mapping produces (so a
//! row's winning classifier's answer equals the row's `Confidence`), on that
//! classifier's own scale. Nothing here combines scores: two classifiers are not
//! calibrated alike, and a blended number would sit beside a real model output
//! while meaning something nobody has checked.
//!
//! # Four states, never a zero
//!
//! A classifier with no class for the species has said nothing, and printing 0%
//! would report its silence as an opinion. So an answer is one of
//! [`AnswerState::Heard`], [`AnswerState::BelowBar`], [`AnswerState::NoLabel`]
//! or [`AnswerState::Failed`], and only the first two carry a score.
//!
//! # What "heard it" means against `model_agreement`
//!
//! `heard` is the classifier's own score at or above its own bar. The
//! classifier's report is also capped at its `top_n` highest species per window,
//! which is a limit on what it *reports* and not an opinion, so in a window with
//! more species over the bar than that cap a classifier can be `heard` here
//! without having been counted in `model_agreement`. The rows are the more
//! exact of the two.
//!
//! # Species are matched by scientific name as the labels write it
//!
//! Classifiers' label sets are operator-supplied and may follow different
//! taxonomies. A species one model names differently is a different species to
//! the merge and to this module alike; synonyms are not reconciled here.

use super::types::{AnswerState, ModelAnswer};

/// What a classifier scored one class in its most recent window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClassScore {
    /// The classifier has no class by that name.
    NoClass,
    /// It has the class, and its output for it was not a usable number.
    Unusable,
    /// Its score, 0 to 1, on its own scale.
    Score(f32),
}

/// One classifier's window over one chunk, as far as an answer needs to know it.
pub struct WindowOutcome<'a> {
    /// The classifier's id.
    pub model_id: &'a str,
    /// The bar its scores are judged against: its own threshold, as loaded.
    pub bar: f32,
    /// Where its window began, seconds into the recording.
    pub window_start_secs: f32,
    /// Where its window ended.
    pub window_end_secs: f32,
    /// Looks a species up in the window's scores. `None` when the window failed,
    /// so there is nothing to look up and a stale answer from an earlier window
    /// can never be mistaken for this one's.
    pub scores: Option<&'a dyn Fn(&str) -> ClassScore>,
}

impl std::fmt::Debug for WindowOutcome<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowOutcome")
            .field("model_id", &self.model_id)
            .field("bar", &self.bar)
            .field("window_start_secs", &self.window_start_secs)
            .field("window_end_secs", &self.window_end_secs)
            .field("answered", &self.scores.is_some())
            .finish()
    }
}

/// Every listening classifier's answer about one species, in the order given.
#[must_use]
pub fn answers_for(outcomes: &[WindowOutcome<'_>], scientific_name: &str) -> Vec<ModelAnswer> {
    outcomes
        .iter()
        .map(|w| {
            let (state, score, judged_against) =
                match w.scores.map(|look_up| look_up(scientific_name)) {
                    None | Some(ClassScore::Unusable) => (AnswerState::Failed, None, None),
                    Some(ClassScore::NoClass) => (AnswerState::NoLabel, None, None),
                    Some(ClassScore::Score(s)) if s >= w.bar => {
                        (AnswerState::Heard, Some(s), Some(w.bar))
                    }
                    Some(ClassScore::Score(s)) => (AnswerState::BelowBar, Some(s), Some(w.bar)),
                };
            ModelAnswer {
                model_id: w.model_id.to_owned(),
                state,
                score,
                bar: judged_against,
                window_start_secs: w.window_start_secs,
                window_end_secs: w.window_end_secs,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win<'a>(
        id: &'a str,
        bar: f32,
        scores: Option<&'a dyn Fn(&str) -> ClassScore>,
    ) -> WindowOutcome<'a> {
        WindowOutcome {
            model_id: id,
            bar,
            window_start_secs: 3.0,
            window_end_secs: 7.5,
            scores,
        }
    }

    #[test]
    fn a_score_at_its_own_bar_is_heard_and_one_under_it_is_below_the_bar() {
        let at = |_: &str| ClassScore::Score(0.70);
        let under = |_: &str| ClassScore::Score(0.69);
        let got = answers_for(
            &[win("a", 0.70, Some(&at)), win("b", 0.70, Some(&under))],
            "Turdus merula",
        );
        assert_eq!(got[0].state, AnswerState::Heard, "the bar itself clears");
        assert_eq!(got[0].score, Some(0.70));
        assert_eq!(got[0].bar, Some(0.70));
        assert_eq!(got[1].state, AnswerState::BelowBar);
        assert_eq!(
            got[1].score,
            Some(0.69),
            "a low score is kept, not rounded away"
        );
    }

    #[test]
    fn each_classifier_is_judged_against_its_own_bar() {
        // The same score clears a loose model's bar and misses a strict one's.
        let same = |_: &str| ClassScore::Score(0.60);
        let got = answers_for(
            &[
                win("loose", 0.50, Some(&same)),
                win("strict", 0.80, Some(&same)),
            ],
            "x",
        );
        assert_eq!(got[0].state, AnswerState::Heard);
        assert_eq!(got[1].state, AnswerState::BelowBar);
        assert_eq!(got[1].bar, Some(0.80));
    }

    #[test]
    fn a_classifier_with_no_class_has_no_score_and_no_bar_not_a_zero() {
        let none = |_: &str| ClassScore::NoClass;
        let got = answers_for(&[win("regional", 0.7, Some(&none))], "Turdus merula");
        assert_eq!(got[0].state, AnswerState::NoLabel);
        assert_eq!(got[0].score, None);
        assert_eq!(got[0].bar, None);
    }

    #[test]
    fn a_failed_window_never_looks_anything_up() {
        // `scores: None` is a window that failed. If the code consulted a
        // classifier's buffer anyway it would read the previous window's.
        let got = answers_for(&[win("m", 0.7, None)], "Turdus merula");
        assert_eq!(got[0].state, AnswerState::Failed);
        assert_eq!((got[0].score, got[0].bar), (None, None));
    }

    #[test]
    fn an_unusable_number_is_a_failure_and_not_a_score_of_zero() {
        let nan = |_: &str| ClassScore::Unusable;
        let got = answers_for(&[win("m", 0.7, Some(&nan))], "x");
        assert_eq!(got[0].state, AnswerState::Failed);
        assert_eq!(got[0].score, None);
    }

    #[test]
    fn the_species_asked_about_reaches_the_lookup() {
        let only_merula = |name: &str| {
            if name == "Turdus merula" {
                ClassScore::Score(0.9)
            } else {
                ClassScore::NoClass
            }
        };
        let w = [win("m", 0.5, Some(&only_merula))];
        assert_eq!(
            answers_for(&w, "Turdus merula")[0].state,
            AnswerState::Heard
        );
        assert_eq!(answers_for(&w, "Pica pica")[0].state, AnswerState::NoLabel);
    }

    #[test]
    fn the_window_each_classifier_listened_to_is_carried_and_the_order_is_kept() {
        let s = |_: &str| ClassScore::Score(0.9);
        let mut a = win("first", 0.5, Some(&s));
        a.window_end_secs = 7.5;
        let mut b = win("second", 0.5, Some(&s));
        b.window_end_secs = 8.0;
        let got = answers_for(&[a, b], "x");
        assert_eq!(
            got.iter().map(|g| g.model_id.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!((got[0].window_end_secs - 7.5).abs() < f32::EPSILON);
        assert!((got[1].window_end_secs - 8.0).abs() < f32::EPSILON);
        assert!((got[0].window_start_secs - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn no_classifiers_means_no_answers() {
        assert!(answers_for(&[], "x").is_empty());
    }
}
