//! Require a species to be heard more than once before it is recorded.
//!
//! # The gap this closes
//!
//! Every other quality control in this project is *subtractive*: the noise-class
//! filter discards a chunk a dog barked in, the night filter quarantines a day
//! bird heard at 03:00, the SNR gate drops audio too poor to judge, the
//! duplicate-prediction interval collapses one song into one row. Each answers
//! "is this chunk trustworthy?".
//!
//! None of them asks the question that separates a real bird from a model
//! artefact better than any of them: **did it happen more than once?**
//!
//! A blackbird singing produces the same species in window after window. A
//! classifier reaching for the nearest bird when a car door slams produces one
//! confident hit in one window and nothing on either side. Confidence cannot
//! tell those apart — the artefact is often the more confident of the two — but
//! repetition can.
//!
//! # How it works
//!
//! Off by default. When a level is set, a detection of species *X* in a chunk
//! survives only if *X* also appears in enough of the chunks around it: the
//! **neighbourhood**, every chunk starting within [`REFERENCE_SPAN`] / 2 either
//! side. The required count is a fraction of the neighbourhood's size, so it
//! scales with the overlap rather than being a number that means different
//! things at different settings.
//!
//! | Level | Fraction of the neighbourhood that must agree |
//! |---|---|
//! | `off` | — (nothing is filtered) |
//! | `lenient` | 20 % |
//! | `moderate` | 30 % |
//! | `balanced` | 50 % |
//! | `strict` | 70 % |
//!
//! # The two gentlest levels do nothing without overlap
//!
//! This paragraph originally said something simpler and wrong — that *no* level
//! worked without overlap — and the test written to pin it failed on the first
//! run. The arithmetic, which is what matters:
//!
//! With 3-second chunks and no overlap the step is a whole chunk, so a
//! 6-second neighbourhood holds three windows (one either side). Twenty per
//! cent of three rounds to one, and thirty per cent of three rounds to one —
//! so `lenient` and `moderate` ask for nothing. Fifty per cent of three is two
//! and seventy is three, so `balanced` and `strict` do bite, demanding a bird
//! that sings across six or nine seconds.
//!
//! | Overlap | Neighbourhood | lenient | moderate | balanced | strict |
//! |---|---|---|---|---|---|
//! | 0.0 s | 3 | 1 (no-op) | 1 (no-op) | 2 | 3 |
//! | 1.5 s | 5 | 1 (no-op) | 2 | 3 | 4 |
//! | 2.0 s | 7 | 2 | 3 | 4 | 5 |
//!
//! A level that asks for one confirmation is configured and inert, which is the
//! silent misconfiguration this project keeps finding. So it is not left to the
//! reader: [`ConfirmationLevel::minimum_overlap`] *derives* — by asking the same
//! arithmetic the filter will ask — the smallest overlap at which each level
//! demands a second opinion, and [`ConfirmationLevel::is_effective_at`] is what
//! the startup warning and the `--doctor` check both call.
//!
//! # What it costs
//!
//! Overlap is CPU. At 2.0 s overlap a 3-second window advances 1 s, so the
//! station runs three times the inferences it would at 0.0 — which a Pi 4 can
//! do in real time and a Pi 3 cannot. That is the actual trade being made, and
//! it is why this is off unless asked for.

use std::collections::HashMap;

use super::types::Detection;

/// The span, in seconds, over which corroboration is looked for.
///
/// Two BirdNET windows. Long enough that a phrase spanning a window boundary is
/// seen twice; short enough that two calls a minute apart do not corroborate
/// each other, which would defeat the point.
pub const REFERENCE_SPAN: f32 = 6.0;

/// How strongly a detection must be corroborated before it is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmationLevel {
    /// Record every detection, as every release before this one did.
    #[default]
    Off,
    /// A fifth of the neighbourhood must agree.
    Lenient,
    /// Three tenths.
    Moderate,
    /// Half.
    Balanced,
    /// Seven tenths — for a station where a false record costs more than a
    /// missed one.
    Strict,
}

impl ConfirmationLevel {
    /// The token used in configuration.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Lenient => "lenient",
            Self::Moderate => "moderate",
            Self::Balanced => "balanced",
            Self::Strict => "strict",
        }
    }

    /// Parse the token. Unknown values are an error rather than a default: a
    /// typo must not silently turn a quality control off.
    ///
    /// # Errors
    ///
    /// Returns the offending token when it is not one of the five.
    pub fn parse(token: &str) -> Result<Self, String> {
        match token.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" | "" => Ok(Self::Off),
            "lenient" => Ok(Self::Lenient),
            "moderate" => Ok(Self::Moderate),
            "balanced" => Ok(Self::Balanced),
            "strict" => Ok(Self::Strict),
            other => Err(other.to_string()),
        }
    }

    /// The fraction of a neighbourhood that must carry the species.
    #[must_use]
    pub const fn required_fraction(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Lenient => 0.20,
            Self::Moderate => 0.30,
            Self::Balanced => 0.50,
            Self::Strict => 0.70,
        }
    }

    /// Whether this level filters anything at all.
    #[must_use]
    pub const fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// The smallest overlap, in seconds, at which this level demands a second
    /// opinion.
    ///
    /// **Derived, not asserted.** Searched by asking
    /// [`required_confirmations`] the same question the filter will ask at run
    /// time, so the number cannot drift away from the arithmetic it describes —
    /// which is what would happen to a table of constants the first time a
    /// fraction changed.
    ///
    /// `None` for [`Self::Off`], which needs no overlap because it demands
    /// nothing.
    #[must_use]
    pub fn minimum_overlap(self, chunk_secs: f32) -> Option<f32> {
        if !self.enabled() {
            return None;
        }
        // Tenths of a second, up to one step short of the whole chunk: an
        // overlap equal to the chunk length is an infinite loop, not a setting.
        #[allow(clippy::cast_possible_truncation)]
        // `chunk_secs` is a window length in seconds — 3.0 for every model this
        // runs on, and bounded by the audio segment length in any case, so the
        // tenths never approach `i32`'s range.
        let max_tenths = (chunk_secs * 10.0) as i32 - 1;
        (0..=max_tenths).find_map(|tenths| {
            #[allow(clippy::cast_precision_loss)]
            let overlap = tenths as f32 / 10.0;
            (self.required_confirmations_at(overlap, chunk_secs) >= 2).then_some(overlap)
        })
    }

    /// Whether this level does anything at the given overlap.
    #[must_use]
    pub fn is_effective_at(self, overlap_secs: f32, chunk_secs: f32) -> bool {
        !self.enabled() || self.required_confirmations_at(overlap_secs, chunk_secs) >= 2
    }

    /// How many chunks in a neighbourhood must carry a species, for a station
    /// whose chunks are `chunk_secs` long and advance by `chunk_secs -
    /// overlap_secs`.
    #[must_use]
    pub fn required_confirmations_at(self, overlap_secs: f32, chunk_secs: f32) -> usize {
        let step = (chunk_secs - overlap_secs).max(0.1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // The neighbourhood spans REFERENCE_SPAN centred on a chunk, so it holds
        // one chunk plus however many fit in half a span either side.
        let neighbours = (REFERENCE_SPAN / 2.0 / step) as usize;
        let possible = neighbours * 2 + 1;
        required_confirmations(self, possible)
    }
}

/// How many of `possible` chunks must carry a species at this level.
///
/// Always at least one — a detection cannot corroborate less than itself — and
/// rounded up, so "half of five" is three rather than two.
#[must_use]
pub fn required_confirmations(level: ConfirmationLevel, possible: usize) -> usize {
    if !level.enabled() || possible == 0 {
        return 1;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let required = (level.required_fraction() * possible as f32).ceil() as usize;
    required.max(1)
}

/// Drop every detection whose species is not corroborated near it.
///
/// `starts[i]` is the start time, in seconds within the recording, of the chunk
/// whose predictions are `predictions[i]`. The two must be the same length; a
/// mismatch returns the predictions untouched rather than guessing, because
/// silently filtering against the wrong timeline is worse than not filtering.
///
/// Returns a parallel structure with the same shape, so a caller zipping it
/// against its chunks keeps working.
#[must_use]
pub fn corroborate(
    level: ConfirmationLevel,
    starts: &[f32],
    predictions: &[Vec<Detection>],
) -> Vec<Vec<Detection>> {
    if !level.enabled() || starts.len() != predictions.len() {
        return predictions.to_vec();
    }

    // Which chunks carry which species, once, rather than re-scanning per
    // detection: a 15-second file at 2.5 s overlap is 25 chunks, and the naive
    // form is quadratic in that for every species in every one of them.
    let mut carriers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, chunk) in predictions.iter().enumerate() {
        for det in chunk {
            carriers
                .entry(det.scientific_name.as_str())
                .or_default()
                .push(idx);
        }
    }

    let half = REFERENCE_SPAN / 2.0;
    predictions
        .iter()
        .enumerate()
        .map(|(idx, chunk)| {
            let here = starts[idx];
            // Every chunk starting within half a span either side, this one
            // included. Computed per chunk rather than per detection because it
            // does not depend on the species.
            let neighbourhood: Vec<usize> = starts
                .iter()
                .enumerate()
                .filter(|(_, s)| (**s - here).abs() <= half)
                .map(|(j, _)| j)
                .collect();
            let required = required_confirmations(level, neighbourhood.len());

            chunk
                .iter()
                .filter(|det| {
                    let seen = carriers
                        .get(det.scientific_name.as_str())
                        .map_or(0, |idxs| {
                            idxs.iter().filter(|j| neighbourhood.contains(j)).count()
                        });
                    seen >= required
                })
                .cloned()
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(sci: &str, start: f32) -> Detection {
        Detection {
            date: "2026-05-01".into(),
            time: "06:30:00".into(),
            scientific_name: sci.into(),
            common_name: sci.into(),
            confidence: 0.9,
            start,
            stop: start + 3.0,
            week: 18,
            file_name_extr: None,
        }
    }

    /// `n` chunks stepping by `step`, with the species listed for each.
    fn scenario(step: f32, per_chunk: &[&[&str]]) -> (Vec<f32>, Vec<Vec<Detection>>) {
        #[allow(clippy::cast_precision_loss)]
        let starts: Vec<f32> = (0..per_chunk.len()).map(|i| i as f32 * step).collect();
        let preds = per_chunk
            .iter()
            .zip(&starts)
            .map(|(species, s)| species.iter().map(|sci| det(sci, *s)).collect())
            .collect();
        (starts, preds)
    }

    fn names(out: &[Vec<Detection>]) -> Vec<Vec<String>> {
        out.iter()
            .map(|c| c.iter().map(|d| d.scientific_name.clone()).collect())
            .collect()
    }

    // ── the levels themselves ─────────────────────────────────────────────

    #[test]
    fn every_level_round_trips_through_its_token() {
        for level in [
            ConfirmationLevel::Off,
            ConfirmationLevel::Lenient,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Strict,
        ] {
            assert_eq!(ConfirmationLevel::parse(level.as_str()), Ok(level));
        }
        assert_eq!(
            ConfirmationLevel::parse("  STRICT "),
            Ok(ConfirmationLevel::Strict)
        );
        assert_eq!(ConfirmationLevel::parse(""), Ok(ConfirmationLevel::Off));
    }

    #[test]
    fn an_unknown_level_is_an_error_not_a_default() {
        // A typo that silently turns a quality control off is the failure mode
        // this project keeps finding; it must be loud.
        assert_eq!(
            ConfirmationLevel::parse("balnced"),
            Err("balnced".to_string())
        );
    }

    #[test]
    fn the_fractions_increase_with_the_level() {
        let f = |l: ConfirmationLevel| l.required_fraction();
        assert!(f(ConfirmationLevel::Off) < f(ConfirmationLevel::Lenient));
        assert!(f(ConfirmationLevel::Lenient) < f(ConfirmationLevel::Moderate));
        assert!(f(ConfirmationLevel::Moderate) < f(ConfirmationLevel::Balanced));
        assert!(f(ConfirmationLevel::Balanced) < f(ConfirmationLevel::Strict));
    }

    #[test]
    fn required_confirmations_rounds_up_and_never_reaches_zero() {
        // "Half of five" is three: rounding down would let a species clear a
        // 50 % bar on 40 % of the evidence.
        assert_eq!(required_confirmations(ConfirmationLevel::Balanced, 5), 3);
        assert_eq!(required_confirmations(ConfirmationLevel::Balanced, 4), 2);
        // A fifth of one is 0.2, which must still be 1 — a detection cannot
        // corroborate less than itself.
        assert_eq!(required_confirmations(ConfirmationLevel::Lenient, 1), 1);
        assert_eq!(required_confirmations(ConfirmationLevel::Strict, 0), 1);
    }

    #[test]
    fn off_requires_nothing_whatever_the_neighbourhood() {
        for possible in [0, 1, 5, 50] {
            assert_eq!(required_confirmations(ConfirmationLevel::Off, possible), 1);
        }
    }

    // ── the overlap trap ──────────────────────────────────────────────────

    #[test]
    fn only_the_two_gentlest_levels_are_inert_without_overlap() {
        // Written first as "no level works without overlap", which is what the
        // module comment claimed, and which failed on the first run: at zero
        // overlap the neighbourhood is three windows, so 50 % is two and 70 %
        // is three. Both bite. This pins the arithmetic rather than the
        // intuition, and the comment was corrected to match.
        assert_eq!(
            ConfirmationLevel::Lenient.required_confirmations_at(0.0, 3.0),
            1
        );
        assert_eq!(
            ConfirmationLevel::Moderate.required_confirmations_at(0.0, 3.0),
            1
        );
        assert_eq!(
            ConfirmationLevel::Balanced.required_confirmations_at(0.0, 3.0),
            2
        );
        assert_eq!(
            ConfirmationLevel::Strict.required_confirmations_at(0.0, 3.0),
            3
        );

        assert!(
            !ConfirmationLevel::Lenient.is_effective_at(0.0, 3.0),
            "lenient asks for one confirmation at zero overlap — configured and inert"
        );
        assert!(
            !ConfirmationLevel::Moderate.is_effective_at(0.0, 3.0),
            "moderate is inert at zero overlap too"
        );
        assert!(
            ConfirmationLevel::Balanced.is_effective_at(0.0, 3.0),
            "balanced demands two windows even with no overlap, so it must not \
             be reported as needing any"
        );
    }

    #[test]
    fn the_neighbourhood_grows_with_the_overlap() {
        // The table in the module comment, checked. It is the whole basis for
        // the minimum-overlap advice, and prose is not a gate.
        for (overlap, lenient, moderate, balanced, strict) in
            [(0.0, 1, 1, 2, 3), (1.5, 1, 2, 3, 4), (2.0, 2, 3, 4, 5)]
        {
            assert_eq!(
                ConfirmationLevel::Lenient.required_confirmations_at(overlap, 3.0),
                lenient,
                "lenient at {overlap}s"
            );
            assert_eq!(
                ConfirmationLevel::Moderate.required_confirmations_at(overlap, 3.0),
                moderate,
                "moderate at {overlap}s"
            );
            assert_eq!(
                ConfirmationLevel::Balanced.required_confirmations_at(overlap, 3.0),
                balanced,
                "balanced at {overlap}s"
            );
            assert_eq!(
                ConfirmationLevel::Strict.required_confirmations_at(overlap, 3.0),
                strict,
                "strict at {overlap}s"
            );
        }
    }

    #[test]
    fn minimum_overlap_is_the_point_where_the_level_starts_working() {
        for level in [
            ConfirmationLevel::Lenient,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Strict,
        ] {
            let min = level
                .minimum_overlap(3.0)
                .expect("an enabled level has one");
            assert!(
                level.is_effective_at(min, 3.0),
                "{level:?} reports {min}s as its minimum but is not effective there"
            );
            // And one tenth below it, it is not — otherwise "minimum" is just a
            // number somebody liked.
            let below = (min - 0.1).max(0.0);
            if below < min {
                assert!(
                    !level.is_effective_at(below, 3.0),
                    "{level:?} is already effective at {below}s, so {min}s is not its minimum"
                );
            }
        }
    }

    #[test]
    fn a_stricter_level_needs_at_least_as_much_overlap() {
        let mins: Vec<f32> = [
            ConfirmationLevel::Strict,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Lenient,
        ]
        .iter()
        .map(|l| l.minimum_overlap(3.0).expect("enabled"))
        .collect();
        // Strict first: a higher fraction reaches 2 confirmations sooner, so
        // its minimum is the *lowest*. Stated as the ordering that actually
        // holds rather than the one that sounds right.
        for pair in mins.windows(2) {
            assert!(
                pair[0] <= pair[1],
                "minimum overlaps are not monotonic across levels: {mins:?}"
            );
        }
    }

    #[test]
    fn off_has_no_minimum_overlap() {
        assert_eq!(ConfirmationLevel::Off.minimum_overlap(3.0), None);
        assert!(ConfirmationLevel::Off.is_effective_at(0.0, 3.0));
    }

    // ── the filter ────────────────────────────────────────────────────────

    #[test]
    fn off_is_the_identity() {
        let (starts, preds) = scenario(1.0, &[&["a"], &["b"], &["a"]]);
        assert_eq!(
            names(&corroborate(ConfirmationLevel::Off, &starts, &preds)),
            names(&preds)
        );
    }

    #[test]
    fn a_species_heard_once_in_a_run_is_dropped() {
        // Seven chunks at 1 s: the neighbourhood is the whole run (±3 s), so
        // seven possible and Balanced needs four. The blackbird has seven, the
        // one-off has one.
        let (starts, preds) = scenario(
            1.0,
            &[
                &["merula"],
                &["merula"],
                &["merula", "artefact"],
                &["merula"],
                &["merula"],
                &["merula"],
                &["merula"],
            ],
        );
        let out = corroborate(ConfirmationLevel::Balanced, &starts, &preds);
        assert!(
            out.iter().flatten().all(|d| d.scientific_name == "merula"),
            "the single artefact survived: {:?}",
            names(&out)
        );
        assert_eq!(
            out.iter().flatten().count(),
            7,
            "every blackbird detection must survive: {:?}",
            names(&out)
        );
    }

    #[test]
    fn a_species_heard_throughout_survives_at_every_level() {
        let (starts, preds) = scenario(1.0, &[&["merula"][..]; 7]);
        for level in [
            ConfirmationLevel::Lenient,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Strict,
        ] {
            let out = corroborate(level, &starts, &preds);
            assert_eq!(
                out.iter().flatten().count(),
                7,
                "{level:?} dropped a species present in every window"
            );
        }
    }

    #[test]
    fn a_stricter_level_never_keeps_more() {
        // Half the windows carry the species, which is exactly the kind of case
        // where the levels should separate.
        let (starts, preds) = scenario(1.0, &[&["x"], &[], &["x"], &[], &["x"], &[], &["x"]]);
        let mut last = usize::MAX;
        for level in [
            ConfirmationLevel::Off,
            ConfirmationLevel::Lenient,
            ConfirmationLevel::Moderate,
            ConfirmationLevel::Balanced,
            ConfirmationLevel::Strict,
        ] {
            let kept = corroborate(level, &starts, &preds).iter().flatten().count();
            assert!(
                kept <= last,
                "{level:?} kept {kept}, more than the level below it kept ({last})"
            );
            last = kept;
        }
    }

    #[test]
    fn corroboration_is_local_not_file_wide() {
        // Two calls twenty seconds apart do not corroborate each other. If they
        // did, a file-wide count would let any species that appeared twice
        // anywhere through, which is not what "heard more than once" means.
        let starts = vec![0.0, 1.0, 2.0, 20.0, 21.0, 22.0];
        let preds: Vec<Vec<Detection>> = vec![
            vec![det("x", 0.0)],
            vec![],
            vec![],
            vec![det("x", 20.0)],
            vec![],
            vec![],
        ];
        let out = corroborate(ConfirmationLevel::Balanced, &starts, &preds);
        assert_eq!(
            out.iter().flatten().count(),
            0,
            "two isolated calls twenty seconds apart corroborated each other"
        );
    }

    #[test]
    fn the_shape_of_the_output_matches_the_input() {
        let (starts, preds) = scenario(1.0, &[&["a"], &[], &["a"], &["a"]]);
        let out = corroborate(ConfirmationLevel::Strict, &starts, &preds);
        assert_eq!(
            out.len(),
            preds.len(),
            "a caller zipping this against its chunks would silently misalign"
        );
    }

    #[test]
    fn a_mismatched_timeline_filters_nothing_rather_than_guessing() {
        let (_, preds) = scenario(1.0, &[&["a"], &["b"]]);
        let out = corroborate(ConfirmationLevel::Strict, &[0.0], &preds);
        assert_eq!(
            names(&out),
            names(&preds),
            "filtering against the wrong timeline is worse than not filtering"
        );
    }

    #[test]
    fn two_species_are_judged_independently() {
        // The common one survives, the rare artefact does not, in the same
        // chunks — a per-chunk decision would keep or drop both.
        let (starts, preds) = scenario(
            1.0,
            &[
                &["common", "artefact"],
                &["common"],
                &["common"],
                &["common"],
                &["common"],
            ],
        );
        let out = corroborate(ConfirmationLevel::Balanced, &starts, &preds);
        assert_eq!(names(&out)[0], vec!["common".to_string()]);
        assert_eq!(out.iter().flatten().count(), 5);
    }
}
