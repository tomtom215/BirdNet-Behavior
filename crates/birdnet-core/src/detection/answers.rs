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
    /// Where the recorded audio in its window ended: the window's nominal end,
    /// or the recording's end when that comes first (see [`window_support`]).
    pub window_end_secs: f32,
    /// How much of its window was zero padding past the recording's end.
    pub padded_secs: f32,
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
            .field("padded_secs", &self.padded_secs)
            .field("answered", &self.scores.is_some())
            .finish()
    }
}

/// The recorded audio a classifier's window covers, as `(end, padded)`.
///
/// `start` is where the window begins, `window_secs` how long the classifier's
/// window is, and `audio_end` where the recording's audio ends within the chunk
/// (the chunk's own `end_secs`, which the pipeline clamps to the recording).
///
/// The pipeline zero-pads the last chunk of a recording to the chunk's length,
/// so a classifier handed it scores a window part of which is silence nobody
/// recorded. Its answer used to claim the whole nominal window as heard audio,
/// running past the end of the recording, while the detection row beside it
/// stopped at that end (review F09). The support is clamped to the audio that
/// exists and the rest is reported as padding. Audio that ends before the
/// window starts leaves no support at all.
#[must_use]
pub fn window_support(start: f32, window_secs: f32, audio_end: f32) -> (f32, f32) {
    let nominal_end = start + window_secs;
    let end = nominal_end.min(audio_end.max(start));
    (end, (nominal_end - end).max(0.0))
}

/// How many seconds of recorded audio two answers' windows share.
///
/// Classifiers' windows over one chunk all start at the chunk's start and
/// differ in length, so two classifiers that "agree" on a species heard
/// overlapping, not identical, audio, and on a padded tail less of it than
/// their nominal windows suggest. This is the overlap of their supported spans
/// ([`ModelAnswer::window_start_secs`] to [`ModelAnswer::window_end_secs`]),
/// padding excluded; `0.0` when they do not overlap.
#[must_use]
pub fn shared_support_secs(a: &ModelAnswer, b: &ModelAnswer) -> f32 {
    let start = a.window_start_secs.max(b.window_start_secs);
    let end = a.window_end_secs.min(b.window_end_secs);
    (end - start).max(0.0)
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
                padded_secs: w.padded_secs,
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
            padded_secs: 0.0,
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

    /// Review F09: the window's recorded support stops at the recording's end,
    /// and the rest is counted as padding. Counterparts: a window inside the
    /// audio is untouched, and one ending exactly at the audio's end has none.
    #[test]
    fn a_window_past_the_recording_s_end_is_clamped_and_its_padding_counted() {
        let close =
            |(a, b): (f32, f32), (c, d): (f32, f32)| (a - c).abs() < 1e-6 && (b - d).abs() < 1e-6;
        assert!(
            close(window_support(6.0, 3.0, 7.0), (7.0, 2.0)),
            "{:?}",
            window_support(6.0, 3.0, 7.0)
        );
        assert!(close(window_support(6.0, 3.0, 9.5), (9.0, 0.0)));
        assert!(close(window_support(6.0, 3.0, 9.0), (9.0, 0.0)));
        // Audio that ends before the window starts gives no support, all padding.
        assert!(close(window_support(6.0, 3.0, 5.0), (6.0, 3.0)));
    }

    /// Review F09: agreement is over overlapping windows. BirdNET's 3 s and
    /// v3's 2 s from one chunk's start share 2 s; on a tail that ends after
    /// one second they share one; answers from different chunks that do not
    /// meet share nothing.
    #[test]
    fn two_answers_share_the_recorded_audio_their_windows_overlap() {
        let answer = |start: f32, end: f32| ModelAnswer {
            model_id: "m".into(),
            state: AnswerState::Heard,
            score: Some(0.9),
            bar: Some(0.5),
            window_start_secs: start,
            window_end_secs: end,
            padded_secs: 0.0,
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(close(
            shared_support_secs(&answer(6.0, 9.0), &answer(6.0, 8.0)),
            2.0
        ));
        assert!(
            close(
                shared_support_secs(&answer(6.0, 8.0), &answer(6.0, 9.0)),
                2.0
            ),
            "symmetric"
        );
        assert!(close(
            shared_support_secs(&answer(6.0, 7.0), &answer(6.0, 7.0)),
            1.0
        ));
        assert!(close(
            shared_support_secs(&answer(0.0, 3.0), &answer(4.5, 9.5)),
            0.0
        ));
    }

    #[test]
    fn no_classifiers_means_no_answers() {
        assert!(answers_for(&[], "x").is_empty());
    }
}
