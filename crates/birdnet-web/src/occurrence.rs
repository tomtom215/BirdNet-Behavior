//! What the station's species-occurrence ("range") filter says about a place
//! and a week, for the pages and the API that explain it.
//!
//! # Why this is a trait and not the filter
//!
//! The filter lives in the detection daemon (`birdnet-core`), on the daemon's
//! own thread, behind an ONNX session that needs `&mut` to run. The web crate
//! must not own an inference runtime, reach into that thread, or block the
//! async runtime waiting on it. So the binary hands the web crate one thing: an
//! [`OccurrenceSource`], installed on [`AppState`](crate::state::AppState) at
//! start-up the way the capture status is, which answers "what does the filter
//! say for this week?" with an owned, immutable [`OccurrenceReport`].
//!
//! [`OccurrenceSource::report`] is **blocking** (it may run the model), and is
//! to be called from [`AppState::run_blocking`](crate::state::AppState::run_blocking),
//! as every other database or inference read in this crate is.
//!
//! # What "no answer" means
//!
//! There are three reasons a station cannot say, and each is different from
//! "nothing occurs here". [`Unavailable`] names them, so an API client and a
//! reader are never handed an empty list that reads as an empty region.

use std::sync::Arc;

use birdnet_core::inference::species_filter::OccurrenceScore;

/// The model's year has 48 weeks, four to a month; `week` is one of them.
///
/// Anything else is read by the model as "no seasonal information" and answered
/// with a year-round list roughly twice as long as any real week's, which is
/// not what a person asking about *this* week wants. weeks 1 to [`MODEL_WEEKS`] only.
pub const MODEL_WEEKS: u32 = 48;

/// One reading of the occurrence filter for one place and one week.
#[derive(Debug, Clone, PartialEq)]
pub struct OccurrenceReport {
    /// The station's latitude, decimal degrees.
    pub latitude: f64,
    /// The station's longitude, decimal degrees.
    pub longitude: f64,
    /// The model's week, from 1 to [`MODEL_WEEKS`].
    pub week: u32,
    /// The threshold in force: a species is admitted when the model scores it
    /// at or above this (or the operator whitelisted it), and the operator's
    /// include and exclude lists allow it.
    pub threshold: f32,
    /// Every species the model scores, most likely first, each with the
    /// station's actual decision for it.
    pub species: Vec<OccurrenceScore>,
}

/// Why the station cannot say what its occurrence filter does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The station has no coordinates, so the model has no place to be asked
    /// about. (It also means the filter is not filtering by place.)
    NoLocation,
    /// No range model is loaded: the filter is off, and every species the
    /// classifier knows is a candidate wherever the station is.
    NoModel,
    /// The model is configured and could not be loaded or run. The text is for
    /// the operator, not for a client to parse.
    Failed(String),
}

/// The station's occurrence filter, as the web crate may read it.
pub trait OccurrenceSource: Send + Sync + std::fmt::Debug {
    /// The filter's decisions for `week` (1 to [`MODEL_WEEKS`]).
    ///
    /// Blocking: it may run the range model. Call it from
    /// [`AppState::run_blocking`](crate::state::AppState::run_blocking).
    ///
    /// # Errors
    ///
    /// [`Unavailable`] when the station has no location, no range model, or a
    /// model that failed.
    fn report(&self, week: u32) -> Result<Arc<OccurrenceReport>, Unavailable>;
}

/// The model week for a civil date `YYYY-MM-DD`, or `None` for a string that
/// names no day.
///
/// The one conversion the daemon uses for a recording
/// ([`birdnet_core::civil::birdnet_week_from_date`]), so "this week" on the page
/// is the week a recording made now would be scored against.
#[must_use]
pub fn week_of(date: &str) -> Option<u32> {
    birdnet_core::civil::birdnet_week_from_date(date)
}

/// A probability as a percentage a person can read: `0.4 %`, `3 %`, `87 %`.
///
/// Enough digits to tell a 0.4 % from a 0.04 %, which the threshold sits
/// between, and no trailing zeros. A probability the model scores so low that
/// it rounds to nothing is `<0.01 %`, not `0 %`: the model did not say zero.
#[must_use]
pub fn percent_text(probability: f32) -> String {
    let pct = f64::from(probability) * 100.0;
    if pct <= 0.0 {
        return "0 %".to_owned();
    }
    if pct < 0.01 {
        return "<0.01 %".to_owned();
    }
    let decimals: usize = if pct >= 10.0 {
        0
    } else if pct >= 1.0 {
        1
    } else {
        2
    };
    let text = format!("{pct:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    format!("{text} %")
}

/// How the station's decision for one species came about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The model scores it at or above the threshold, and the lists allow it.
    Admitted,
    /// Below the threshold, and admitted anyway: the operator's "always
    /// allowed" list exempts it from the threshold.
    AdmittedByWhitelist,
    /// Below the threshold. Whatever the lists say, the range model holds it
    /// back for this place and week.
    HeldBackByRange,
    /// At or above the threshold, and still not admitted: the operator's
    /// include or exclude list is what keeps it out, not the range model.
    HeldBackByLists,
}

impl Verdict {
    /// Classify one score against the threshold in force.
    ///
    /// Total over `(probability >= threshold, admitted)`, and exactly the
    /// station's decision restated: `admitted` comes from the filter itself, so
    /// this only has to say *why*, never *whether*.
    #[must_use]
    pub fn of(score: &OccurrenceScore, threshold: f32) -> Self {
        match (score.probability >= threshold, score.admitted) {
            (true, true) => Self::Admitted,
            (false, true) => Self::AdmittedByWhitelist,
            (false, false) => Self::HeldBackByRange,
            (true, false) => Self::HeldBackByLists,
        }
    }

    /// Whether the station lets a detection of the species through.
    #[must_use]
    pub const fn is_admitted(self) -> bool {
        matches!(self, Self::Admitted | Self::AdmittedByWhitelist)
    }

    /// A short, stable word for the verdict, for an API client.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::AdmittedByWhitelist => "admitted-by-whitelist",
            Self::HeldBackByRange => "held-back-by-range",
            Self::HeldBackByLists => "held-back-by-lists",
        }
    }
}

/// The sentence a person is shown for one species, with the real numbers: the
/// model's probability for the species, the threshold in force and the model
/// week, all the report's own.
#[must_use]
pub fn sentence(score: &OccurrenceScore, report: &OccurrenceReport) -> String {
    let chance = percent_text(score.probability);
    let threshold = percent_text(report.threshold);
    let week = report.week;
    match Verdict::of(score, report.threshold) {
        Verdict::Admitted => format!(
            "Allowed: the range model gives it {chance} here in week {week}; this station admits species at {threshold} and above."
        ),
        Verdict::AdmittedByWhitelist => format!(
            "Allowed, by your Always allowed list: the range model gives it only {chance} here in week {week}, under the {threshold} this station admits species at."
        ),
        Verdict::HeldBackByRange => format!(
            "Held back: the range model gives it {chance} here in week {week}; this station admits species at {threshold} and above."
        ),
        Verdict::HeldBackByLists => format!(
            "Held back by your species lists, not by the range model: it scores {chance} here in week {week}, above the {threshold} this station admits species at, but your include or exclude list keeps it out."
        ),
    }
}

/// A probability or threshold as the JSON number a person would write.
///
/// `0.03_f32` widened to `f64` is `0.029999999329447746`, which is what a client
/// would be handed and have to round; going through the shortest decimal that
/// names the `f32` gives `0.03`.
#[must_use]
pub fn json_number(x: f32) -> f64 {
    x.to_string().parse().unwrap_or_else(|_| f64::from(x))
}

/// How many species one response or page lists unless asked for more or fewer.
pub const DEFAULT_LIMIT: usize = 100;

/// The most species one response may list. The model scores ten thousand
/// species or more; a client that wants them all pages through them.
pub const MAX_LIMIT: usize = 1000;

/// What a caller asked to see of a report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Case-insensitive text to find in the classifier's common name or the
    /// scientific name (and, through `also`, the name in the station's
    /// language).
    pub q: Option<String>,
    /// Only the species the station admits (`Some(true)`), or only those it
    /// holds back (`Some(false)`).
    pub admitted: Option<bool>,
}

/// The species of a report that a [`Selection`] picks, with the counts a client
/// needs to page through them and to read "how many" off the page.
#[derive(Debug, Clone)]
pub struct Selected<'a> {
    /// Every species the model scores.
    pub scored: usize,
    /// Of those, how many the station admits.
    pub admitted: usize,
    /// How many the selection picked (before any paging).
    pub matched: usize,
    /// The picked species, most likely first.
    pub species: Vec<&'a OccurrenceScore>,
}

/// Pick species from `report`.
///
/// `also` is the set of scientific names whose name in the station's language
/// contains the typed text (`name_search::also_species`), so a station set to
/// German finds the magpie by *Elster*. Paging is the caller's: this returns
/// everything picked, in probability order, and the caller slices.
#[must_use]
pub fn select<'a, S: std::hash::BuildHasher>(
    report: &'a OccurrenceReport,
    selection: &Selection,
    also: &std::collections::HashSet<String, S>,
) -> Selected<'a> {
    let needle = selection
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);
    let species: Vec<&OccurrenceScore> = report
        .species
        .iter()
        .filter(|s| selection.admitted.is_none_or(|want| s.admitted == want))
        .filter(|s| {
            needle.as_deref().is_none_or(|n| {
                s.common_name.to_lowercase().contains(n)
                    || s.scientific_name.to_lowercase().contains(n)
                    || also.contains(&s.scientific_name)
            })
        })
        .collect();
    Selected {
        scored: report.species.len(),
        admitted: report.species.iter().filter(|s| s.admitted).count(),
        matched: species.len(),
        species,
    }
}

/// The week a request names, or why it is refused.
///
/// `None` is "this week", for the caller to fill in. A week outside
/// 1 to [`MODEL_WEEKS`] is refused, not clamped: the model reads anything else
/// as "no seasonal information" and answers with a year-round list, which is
/// the wrong answer to a question about one week.
///
/// # Errors
///
/// The text to show a person who typed it.
pub fn parse_week(raw: Option<&str>) -> Result<Option<u32>, String> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    match raw.parse::<u32>() {
        Ok(w) if (1..=MODEL_WEEKS).contains(&w) => Ok(Some(w)),
        _ => Err(format!(
            "The week must be a whole number from 1 to {MODEL_WEEKS} (the range model's year has {MODEL_WEEKS} weeks, four to a month). You entered \u{201c}{raw}\u{201d}."
        )),
    }
}

/// An `admitted=` value: `true` or `false`, or nothing.
///
/// # Errors
///
/// The text to show a person who typed anything else.
pub fn parse_admitted(raw: Option<&str>) -> Result<Option<bool>, String> {
    match raw.map(str::trim).filter(|r| !r.is_empty()) {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(format!(
            "`admitted` must be `true` or `false`. You entered \u{201c}{other}\u{201d}."
        )),
    }
}

/// A `limit=` or `offset=` value: a whole number, or nothing; `limit` is held
/// to between 1 and [`MAX_LIMIT`].
///
/// # Errors
///
/// The text to show a person who typed anything else.
pub fn parse_count(
    name: &str,
    raw: Option<&str>,
    min: usize,
    max: usize,
) -> Result<Option<usize>, String> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    match raw.parse::<usize>() {
        Ok(n) if (min..=max).contains(&n) => Ok(Some(n)),
        _ => Err(format!(
            "`{name}` must be a whole number from {min} to {max}. You entered \u{201c}{raw}\u{201d}."
        )),
    }
}

/// An [`OccurrenceSource`] that answers from a table: for tests and for the
/// screenshot fixture, which have no range model to run.
///
/// Remembers which weeks it was asked about, so a test can see that the week a
/// request named is the week that reached the source.
#[derive(Debug)]
pub struct FixedOccurrence {
    answer: Result<OccurrenceReport, Unavailable>,
    asked: std::sync::Mutex<Vec<u32>>,
}

impl FixedOccurrence {
    /// A source that reports `template` for whatever week is asked (the
    /// report's `week` is set to the one asked).
    #[must_use]
    pub const fn answering(template: OccurrenceReport) -> Self {
        Self {
            answer: Ok(template),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// A source that cannot answer, for `reason`.
    #[must_use]
    pub const fn unavailable(reason: Unavailable) -> Self {
        Self {
            answer: Err(reason),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The weeks asked about so far, in order.
    #[must_use]
    pub fn weeks_asked(&self) -> Vec<u32> {
        self.asked.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl OccurrenceSource for FixedOccurrence {
    fn report(&self, week: u32) -> Result<Arc<OccurrenceReport>, Unavailable> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(week);
        }
        match &self.answer {
            Ok(template) => Ok(Arc::new(OccurrenceReport {
                week,
                ..template.clone()
            })),
            Err(reason) => Err(reason.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(probability: f32, admitted: bool) -> OccurrenceScore {
        OccurrenceScore {
            scientific_name: "Pica pica".to_owned(),
            common_name: "Eurasian Magpie".to_owned(),
            probability,
            admitted,
        }
    }

    fn report(threshold: f32) -> OccurrenceReport {
        OccurrenceReport {
            latitude: 52.52,
            longitude: 13.405,
            week: 22,
            threshold,
            species: Vec::new(),
        }
    }

    #[test]
    fn a_percentage_keeps_the_digit_the_threshold_sits_between() {
        assert_eq!(percent_text(0.03), "3 %");
        assert_eq!(percent_text(0.004), "0.4 %");
        assert_eq!(percent_text(0.0004), "0.04 %");
        assert_eq!(percent_text(0.874), "87 %");
        assert_eq!(percent_text(0.126), "13 %");
        assert_eq!(percent_text(0.05), "5 %");
        assert_eq!(percent_text(1.0), "100 %");
        // The model did not say zero.
        assert_eq!(percent_text(0.000_01), "<0.01 %");
        assert_eq!(percent_text(0.0), "0 %");
    }

    #[test]
    fn a_probability_is_written_as_the_number_it_was() {
        assert_eq!(json_number(0.03), 0.03);
        assert_eq!(json_number(0.004), 0.004);
        assert_eq!(json_number(0.62), 0.62);
        assert_eq!(json_number(1.0), 1.0);
        assert_eq!(json_number(0.0), 0.0);
    }

    #[test]
    fn the_four_verdicts_are_the_four_cells_of_score_and_decision() {
        let t = 0.03;
        assert_eq!(Verdict::of(&score(0.5, true), t), Verdict::Admitted);
        assert_eq!(
            Verdict::of(&score(0.01, true), t),
            Verdict::AdmittedByWhitelist
        );
        assert_eq!(
            Verdict::of(&score(0.01, false), t),
            Verdict::HeldBackByRange
        );
        assert_eq!(Verdict::of(&score(0.5, false), t), Verdict::HeldBackByLists);
        // The boundary is the filter's: at the threshold is admitted.
        assert_eq!(Verdict::of(&score(0.03, true), t), Verdict::Admitted);
        for v in [Verdict::Admitted, Verdict::AdmittedByWhitelist] {
            assert!(v.is_admitted());
        }
        for v in [Verdict::HeldBackByRange, Verdict::HeldBackByLists] {
            assert!(!v.is_admitted());
        }
    }

    #[test]
    fn the_sentence_carries_the_real_numbers() {
        let r = report(0.03);
        let held = sentence(&score(0.004, false), &r);
        assert_eq!(
            held,
            "Held back: the range model gives it 0.4 % here in week 22; this station admits species at 3 % and above."
        );
        let ok = sentence(&score(0.4, true), &r);
        assert!(
            ok.starts_with("Allowed:") && ok.contains("40 %") && ok.contains("3 %"),
            "{ok}"
        );
        let by_list = sentence(&score(0.4, false), &r);
        assert!(
            by_list.contains("species lists") && by_list.contains("40 %"),
            "{by_list}"
        );
        let white = sentence(&score(0.004, true), &r);
        assert!(
            white.contains("Always allowed") && white.contains("0.4 %"),
            "{white}"
        );
    }
}
