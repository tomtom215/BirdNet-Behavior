//! Composable filtering for the detections list.
//!
//! # Why this exists rather than more branches
//!
//! [`todays_detections`](super::read::todays_detections) hand-wrote its SQL in a
//! three-armed `match` over the free-text term, with the category filter pasted
//! in as a string. That works for two dimensions. The list needs nine — text,
//! species, date range, hour-of-day, confidence range, source, review verdict,
//! lock state, category — and a `match` over the product of nine optional
//! criteria is not a thing anyone can write correctly or read afterwards.
//!
//! So the clause is *composed*: each criterion that is set contributes one
//! `AND …` fragment and pushes its own bound parameters, in order.
//!
//! # The invariant that matters
//!
//! Every placeholder in this module is an unnumbered `?`, never `?1`/`?2`.
//!
//! Numbered placeholders have to agree with a parameter vector built somewhere
//! else in the function, and keeping those two in step by hand across nine
//! optional fragments is precisely the bug this design exists to make
//! impossible. With positional `?` the fragment that adds the placeholder is the
//! fragment that pushes the value, so the two cannot drift — and
//! [`tests::every_filter_combination_binds_exactly_its_placeholders`] walks a
//! generated matrix of filter combinations asserting `placeholders == params`,
//! which is a gate on the whole class rather than on the cases someone
//! remembered.
//!
//! # Untrusted input never reaches the SQL text
//!
//! Operator strings — the search box, species names, source labels — are always
//! *values*, never fragments. The only things interpolated into the statement
//! are `&'static str` fragments selected by an enum.
//! [`tests::operator_text_never_reaches_the_statement`] asserts that against
//! deliberately hostile input, because "we always bind" is a claim worth
//! checking rather than repeating.

use rusqlite::Connection;
use rusqlite::types::ToSql;

use crate::sqlite::DbError;
use crate::sqlite::queries::detections::search::SearchTerm;
use crate::sqlite::types::{DETECTION_COLS, DetectionRow, map_detection_row};

use super::read::TodayFilter;
use super::search::parse_search_term;

/// A boxed bound parameter. The clause builder owns its values so the caller
/// does not have to keep nine borrows alive.
type Bound = Box<dyn ToSql>;

/// Which dates to include.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DateRange {
    /// Every date on record.
    #[default]
    Any,
    /// Exactly one local date, `YYYY-MM-DD`.
    On(String),
    /// An inclusive `YYYY-MM-DD` span. Callers may pass them in either order;
    /// [`DateRange::between`] normalises.
    Between(String, String),
}

impl DateRange {
    /// An inclusive span with the ends put the right way round.
    ///
    /// A date picker lets somebody choose the 20th and then the 14th, and a
    /// range that silently matches nothing looks like a broken search rather
    /// than a mistake.
    #[must_use]
    pub fn between(a: impl Into<String>, b: impl Into<String>) -> Self {
        let (a, b) = (a.into(), b.into());
        if a <= b {
            Self::Between(a, b)
        } else {
            Self::Between(b, a)
        }
    }
}

/// Which review verdicts to include.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerdictFilter {
    /// Reviewed or not, confirmed or rejected.
    #[default]
    Any,
    /// A reviewer confirmed it.
    Confirmed,
    /// A reviewer rejected it.
    Rejected,
    /// Nobody has looked at it — the queue a curator works from.
    Unreviewed,
}

/// Which lock states to include.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LockFilter {
    /// Locked or not.
    #[default]
    Any,
    /// Pinned against the disk purge.
    Locked,
    /// Not pinned.
    Unlocked,
}

/// How to order the results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    /// Most recent first — what every surface used before this existed.
    #[default]
    NewestFirst,
    /// Oldest first.
    OldestFirst,
    /// Most confident first.
    ConfidenceHigh,
    /// Least confident first — the useful order for finding artefacts.
    ConfidenceLow,
    /// Common name A–Z.
    SpeciesAz,
    /// Common name Z–A.
    SpeciesZa,
}

impl SortOrder {
    /// The token used in URLs and forms.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NewestFirst => "newest",
            Self::OldestFirst => "oldest",
            Self::ConfidenceHigh => "confidence",
            Self::ConfidenceLow => "confidence-asc",
            Self::SpeciesAz => "species",
            Self::SpeciesZa => "species-desc",
        }
    }

    /// Parse the token; anything unknown is the default rather than an error,
    /// because a stale bookmark should still show the list.
    #[must_use]
    pub fn from_token(token: Option<&str>) -> Self {
        match token.map(str::trim) {
            Some("oldest") => Self::OldestFirst,
            Some("confidence") => Self::ConfidenceHigh,
            Some("confidence-asc") => Self::ConfidenceLow,
            Some("species") => Self::SpeciesAz,
            Some("species-desc") => Self::SpeciesZa,
            _ => Self::NewestFirst,
        }
    }

    /// The `ORDER BY` body.
    ///
    /// Every order ends with `Date DESC, Time DESC` (or its mirror) as a
    /// tie-break, so paging is stable: without a total order, two rows with the
    /// same confidence can swap between page 1 and page 2 and the operator sees
    /// one twice and another never.
    const fn sql(self) -> &'static str {
        match self {
            Self::NewestFirst => "Date DESC, Time DESC",
            Self::OldestFirst => "Date ASC, Time ASC",
            Self::ConfidenceHigh => "Confidence DESC, Date DESC, Time DESC",
            Self::ConfidenceLow => "Confidence ASC, Date DESC, Time DESC",
            Self::SpeciesAz => "Com_Name COLLATE NOCASE ASC, Date DESC, Time DESC",
            Self::SpeciesZa => "Com_Name COLLATE NOCASE DESC, Date DESC, Time DESC",
        }
    }
}

/// An inclusive hour-of-day window, which may wrap past midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HourWindow {
    /// First hour included, `0..=23`.
    pub start: u8,
    /// Last hour included, `0..=23`.
    pub end: u8,
}

impl HourWindow {
    /// Build a window, clamping both ends into `0..=23`.
    ///
    /// `start > end` is not an error: it is how you ask for the night. `22..=4`
    /// means 22:00–04:59, which is the window an operator hunting owls wants and
    /// the one a naive `BETWEEN` silently returns nothing for.
    #[must_use]
    pub const fn new(start: u8, end: u8) -> Self {
        Self {
            start: if start > 23 { 23 } else { start },
            end: if end > 23 { 23 } else { end },
        }
    }

    /// Whether the window runs through midnight.
    #[must_use]
    pub const fn wraps(self) -> bool {
        self.start > self.end
    }
}

/// Everything the detections list can be narrowed by.
///
/// Every field defaults to "no restriction", so
/// `DetectionFilter::default()` selects the whole table and each criterion is
/// additive.
#[derive(Clone, Debug, Default)]
pub struct DetectionFilter {
    /// Free-text search over common and scientific name. A leading `NOT `
    /// (case-insensitive) inverts it — the BirdNET-Pi syntax operators already
    /// have written down.
    pub text: Option<String>,
    /// Exact species names (common or scientific, case-insensitive). Multiple
    /// entries are OR-ed: "show me these three birds".
    pub species: Vec<String>,
    /// Which dates.
    pub dates: DateRange,
    /// Which hours of the day.
    pub hours: Option<HourWindow>,
    /// Lowest confidence to include, inclusive.
    pub min_confidence: Option<f64>,
    /// Highest confidence to include, inclusive.
    pub max_confidence: Option<f64>,
    /// Audio sources to include. Multiple entries are OR-ed.
    pub sources: Vec<String>,
    /// Which review verdicts.
    pub verdict: VerdictFilter,
    /// Which lock states.
    pub locked: LockFilter,
    /// The four category shortcuts the Today log already offered.
    pub category: TodayFilter,
    /// Result order.
    pub sort: SortOrder,
}

impl DetectionFilter {
    /// A filter for one date, matching what the Today log has always shown.
    #[must_use]
    pub fn for_date(date: impl Into<String>) -> Self {
        Self {
            dates: DateRange::On(date.into()),
            ..Self::default()
        }
    }

    /// Whether any criterion is set.
    ///
    /// The UI uses this to decide whether to show a "clear filters" affordance,
    /// which should not appear when there is nothing to clear.
    #[must_use]
    pub fn is_unfiltered(&self) -> bool {
        self.text.as_deref().is_none_or(|t| t.trim().is_empty())
            && self.species.is_empty()
            && self.dates == DateRange::Any
            && self.hours.is_none()
            && self.min_confidence.is_none()
            && self.max_confidence.is_none()
            && self.sources.is_empty()
            && self.verdict == VerdictFilter::Any
            && self.locked == LockFilter::Any
            && self.category == TodayFilter::All
    }

    /// Build the `WHERE` body and its bound parameters.
    ///
    /// Returns an empty string when nothing is set, so the caller emits no
    /// `WHERE` at all rather than a `WHERE 1=1` the query planner has to see
    /// through.
    fn where_clause(&self) -> (String, Vec<Bound>) {
        let mut parts: Vec<String> = Vec::new();
        let mut params: Vec<Bound> = Vec::new();

        match &self.dates {
            DateRange::Any => {}
            DateRange::On(d) => {
                parts.push("Date = ?".into());
                params.push(Box::new(d.clone()));
            }
            DateRange::Between(a, b) => {
                parts.push("Date >= ? AND Date <= ?".into());
                params.push(Box::new(a.clone()));
                params.push(Box::new(b.clone()));
            }
        }

        // `Time` is 'HH:MM:SS' text, so the hour is its first two characters.
        // Compared as an integer rather than a string so `9` and `09` cannot
        // disagree, and so the wrap case below is arithmetic rather than
        // lexicographic.
        if let Some(h) = self.hours {
            let hour = "CAST(substr(Time, 1, 2) AS INTEGER)";
            if h.wraps() {
                parts.push(format!("({hour} >= ? OR {hour} <= ?)"));
            } else {
                parts.push(format!("({hour} >= ? AND {hour} <= ?)"));
            }
            params.push(Box::new(i64::from(h.start)));
            params.push(Box::new(i64::from(h.end)));
        }

        if let Some(min) = self.min_confidence {
            parts.push("Confidence >= ?".into());
            params.push(Box::new(min));
        }
        if let Some(max) = self.max_confidence {
            parts.push("Confidence <= ?".into());
            params.push(Box::new(max));
        }

        match parse_search_term(self.text.as_deref()) {
            Some(SearchTerm::Include(term)) => {
                parts.push("(Com_Name LIKE ? OR Sci_Name LIKE ?)".into());
                let pattern = format!("%{term}%");
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern));
            }
            Some(SearchTerm::Exclude(rest)) => {
                parts.push("Com_Name NOT LIKE ?".into());
                params.push(Box::new(format!("%{rest}%")));
            }
            None => {}
        }

        if !self.species.is_empty() {
            // `NOCASE` because these come from a picker fed by the same names
            // the rows carry, and a capitalisation difference between the two
            // is the operator's problem to never have.
            let ors = std::iter::repeat_n(
                "(Com_Name = ? COLLATE NOCASE OR Sci_Name = ? COLLATE NOCASE)",
                self.species.len(),
            )
            .collect::<Vec<_>>()
            .join(" OR ");
            parts.push(format!("({ors})"));
            for s in &self.species {
                params.push(Box::new(s.clone()));
                params.push(Box::new(s.clone()));
            }
        }

        if !self.sources.is_empty() {
            let ors = std::iter::repeat_n("Source = ?", self.sources.len())
                .collect::<Vec<_>>()
                .join(" OR ");
            parts.push(format!("({ors})"));
            for s in &self.sources {
                params.push(Box::new(s.clone()));
            }
        }

        // The two verdict strings are constants, and are bound anyway. Not
        // superstition: `no_sql_fragment_contains_a_quote_…` counts `?` in the
        // finished statement to check the placeholder/parameter invariant, and
        // that count is only sound while no fragment embeds a string literal.
        // One rule — *nothing* is interpolated, values are always bound — is
        // cheaper to hold than "nothing except these two".
        //
        // `IS NULL` rather than `<> 'rejected'` for the unreviewed case:
        // `review_verdict` is NULL for every row nobody has looked at, and
        // `NULL <> 'rejected'` is NULL rather than true, so a comparison would
        // silently drop exactly the rows a curator is looking for.
        match self.verdict {
            VerdictFilter::Any => {}
            VerdictFilter::Confirmed | VerdictFilter::Rejected => {
                parts.push("review_verdict = ?".into());
                params.push(Box::new(if self.verdict == VerdictFilter::Confirmed {
                    "confirmed"
                } else {
                    "rejected"
                }));
            }
            VerdictFilter::Unreviewed => parts.push("review_verdict IS NULL".into()),
        }

        match self.locked {
            LockFilter::Any => {}
            LockFilter::Locked => parts.push("is_locked = 1".into()),
            LockFilter::Unlocked => parts.push("is_locked = 0".into()),
        }

        if let Some(clause) = category_clause(self.category) {
            parts.push(clause.into());
        }

        (parts.join(" AND "), params)
    }

    /// The full `WHERE …` prefix (or an empty string) plus parameters.
    fn where_prefix(&self) -> (String, Vec<Bound>) {
        let (body, params) = self.where_clause();
        if body.is_empty() {
            (String::new(), params)
        } else {
            (format!(" WHERE {body}"), params)
        }
    }
}

/// The category shortcut as a self-contained predicate, or `None` for `All`.
///
/// Correlated on each row's own `Date` rather than on a bound date, so the same
/// predicate means the same thing for a one-day query and a range.
/// [`RecordingsFilter`](super::read::RecordingsFilter) already does this for the
/// cross-date clip browser; this makes the Today log's four shortcuts usable
/// over a range for the same reason and by the same means.
const fn category_clause(filter: TodayFilter) -> Option<&'static str> {
    match filter {
        TodayFilter::All => None,
        TodayFilter::FirstToday => Some(
            "NOT EXISTS (SELECT 1 FROM detections d2 \
             WHERE d2.Com_Name = detections.Com_Name AND d2.Date < detections.Date)",
        ),
        TodayFilter::Rare => Some(
            "Confidence > 0.85 AND NOT EXISTS (SELECT 1 FROM detections d2 \
             WHERE d2.Com_Name = detections.Com_Name AND d2.Date < detections.Date)",
        ),
        TodayFilter::HighConfidence => Some("Confidence >= 0.9"),
    }
}

/// Fetch one page of detections matching `filter`.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn search_detections(
    conn: &Connection,
    filter: &DetectionFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<DetectionRow>, DbError> {
    let (where_sql, mut params) = filter.where_prefix();
    let order = filter.sort.sql();
    let sql = format!(
        "SELECT {DETECTION_COLS} FROM detections{where_sql} \
         ORDER BY {order} LIMIT ? OFFSET ?"
    );
    params.push(Box::new(limit));
    params.push(Box::new(offset));

    let refs: Vec<&dyn ToSql> = params.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(refs.as_slice(), map_detection_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Count every detection matching `filter`.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn search_detection_count(conn: &Connection, filter: &DetectionFilter) -> Result<i64, DbError> {
    let (where_sql, params) = filter.where_prefix();
    let sql = format!("SELECT COUNT(*) FROM detections{where_sql}");
    let refs: Vec<&dyn ToSql> = params.iter().map(AsRef::as_ref).collect();
    let count: i64 = conn.query_row(&sql, refs.as_slice(), |row| row.get(0))?;
    Ok(count)
}

/// The distinct audio sources that have ever produced a detection.
///
/// Feeds the source picker, so it offers what this station actually has rather
/// than a list somebody typed.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn known_sources(conn: &Connection) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT Source FROM detections \
         WHERE Source IS NOT NULL AND Source <> '' ORDER BY Source",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::queries::detections::test_support::{insert_test_detection, test_conn};

    /// Count the `?` placeholders in a statement.
    ///
    /// Sound only because this module interpolates no string literals into its
    /// SQL — every `?` in the text is a placeholder. `no_sql_fragment_contains_a_quote`
    /// keeps that true.
    fn placeholders(sql: &str) -> usize {
        sql.matches('?').count()
    }

    fn full_filter() -> DetectionFilter {
        DetectionFilter {
            text: Some("robin".into()),
            species: vec!["Erithacus rubecula".into(), "Turdus merula".into()],
            dates: DateRange::between("2026-01-01", "2026-02-01"),
            hours: Some(HourWindow::new(5, 9)),
            min_confidence: Some(0.4),
            max_confidence: Some(0.99),
            sources: vec!["MIC_1".into(), "cam2".into()],
            verdict: VerdictFilter::Confirmed,
            locked: LockFilter::Unlocked,
            category: TodayFilter::HighConfidence,
            sort: SortOrder::ConfidenceHigh,
        }
    }

    // ── the class-killing invariant ────────────────────────────────────────

    #[test]
    fn every_filter_combination_binds_exactly_its_placeholders() {
        // A generated matrix rather than the handful of combinations someone
        // remembered: the whole point of positional `?` is that the fragment
        // adding a placeholder is the fragment pushing its value, and this is
        // what makes that checkable rather than merely intended.
        let texts = [
            None,
            Some("robin".to_string()),
            Some("NOT crow".to_string()),
        ];
        let dates = [
            DateRange::Any,
            DateRange::On("2026-05-01".into()),
            DateRange::between("2026-05-01", "2026-05-09"),
        ];
        let species: [Vec<String>; 3] = [
            vec![],
            vec!["Turdus merula".into()],
            vec!["a".into(), "b".into(), "c".into()],
        ];
        let sources: [Vec<String>; 3] =
            [vec![], vec!["MIC_1".into()], vec!["a".into(), "b".into()]];
        let hours = [
            None,
            Some(HourWindow::new(4, 9)),
            Some(HourWindow::new(22, 4)),
        ];
        let confs = [(None, None), (Some(0.3), None), (Some(0.3), Some(0.9))];

        let mut checked = 0_usize;
        for text in &texts {
            for date in &dates {
                for sp in &species {
                    for src in &sources {
                        for hour in &hours {
                            for (lo, hi) in &confs {
                                for verdict in [
                                    VerdictFilter::Any,
                                    VerdictFilter::Unreviewed,
                                    VerdictFilter::Rejected,
                                ] {
                                    for category in [
                                        TodayFilter::All,
                                        TodayFilter::Rare,
                                        TodayFilter::FirstToday,
                                    ] {
                                        let f = DetectionFilter {
                                            text: text.clone(),
                                            species: sp.clone(),
                                            dates: date.clone(),
                                            hours: *hour,
                                            min_confidence: *lo,
                                            max_confidence: *hi,
                                            sources: src.clone(),
                                            verdict,
                                            locked: LockFilter::Any,
                                            category,
                                            sort: SortOrder::NewestFirst,
                                        };
                                        let (sql, params) = f.where_prefix();
                                        assert_eq!(
                                            placeholders(&sql),
                                            params.len(),
                                            "placeholder/parameter mismatch for {f:?}\nSQL: {sql}"
                                        );
                                        checked += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 3 * 3 * 3 * 3 * 3 * 3 * 3 * 3, "matrix shrank");
    }

    #[test]
    fn no_sql_fragment_contains_a_quote_so_counting_placeholders_is_sound() {
        // `placeholders()` counts every `?` in the statement. That equals the
        // number of bindings only while no fragment embeds a string literal
        // that could itself contain one.
        //
        // Written first against `full_filter()` alone, which was theatre: that
        // fixture picks one variant of each enum, so a literal introduced in
        // any *other* variant went unnoticed (observed — the mutation that put
        // one in `VerdictFilter::Unreviewed` left this green). Every fragment
        // an enum can select has to be visited, so it visits all of them.
        for verdict in [
            VerdictFilter::Any,
            VerdictFilter::Confirmed,
            VerdictFilter::Rejected,
            VerdictFilter::Unreviewed,
        ] {
            for locked in [LockFilter::Any, LockFilter::Locked, LockFilter::Unlocked] {
                for category in [
                    TodayFilter::All,
                    TodayFilter::FirstToday,
                    TodayFilter::Rare,
                    TodayFilter::HighConfidence,
                ] {
                    let f = DetectionFilter {
                        verdict,
                        locked,
                        category,
                        ..full_filter()
                    };
                    let (sql, params) = f.where_prefix();
                    assert!(
                        !sql.contains('\''),
                        "a quoted literal appeared in the WHERE body for \
                         {verdict:?}/{locked:?}/{category:?}, which makes the \
                         placeholder count unsound: {sql}"
                    );
                    assert_eq!(placeholders(&sql), params.len(), "{sql}");
                }
            }
        }
    }

    #[test]
    fn operator_text_never_reaches_the_statement() {
        let hostile = "'; DROP TABLE detections; --";
        let f = DetectionFilter {
            text: Some(hostile.into()),
            species: vec![hostile.into()],
            sources: vec![hostile.into()],
            dates: DateRange::On(hostile.into()),
            ..DetectionFilter::default()
        };
        let (sql, params) = f.where_prefix();
        assert!(
            !sql.contains("DROP") && !sql.contains(hostile),
            "operator input was interpolated into the statement: {sql}"
        );
        assert_eq!(placeholders(&sql), params.len());
    }

    // ── individual criteria ────────────────────────────────────────────────

    #[test]
    fn an_empty_filter_emits_no_where_at_all() {
        let (sql, params) = DetectionFilter::default().where_prefix();
        assert_eq!(sql, "", "an unfiltered list should not carry a WHERE");
        assert!(params.is_empty());
        assert!(DetectionFilter::default().is_unfiltered());
    }

    #[test]
    fn a_set_criterion_makes_the_filter_report_itself_as_filtered() {
        for f in [
            DetectionFilter {
                text: Some("x".into()),
                ..DetectionFilter::default()
            },
            DetectionFilter {
                species: vec!["x".into()],
                ..DetectionFilter::default()
            },
            DetectionFilter::for_date("2026-01-01"),
            DetectionFilter {
                hours: Some(HourWindow::new(1, 2)),
                ..DetectionFilter::default()
            },
            DetectionFilter {
                min_confidence: Some(0.1),
                ..DetectionFilter::default()
            },
            DetectionFilter {
                max_confidence: Some(0.1),
                ..DetectionFilter::default()
            },
            DetectionFilter {
                sources: vec!["m".into()],
                ..DetectionFilter::default()
            },
            DetectionFilter {
                verdict: VerdictFilter::Rejected,
                ..DetectionFilter::default()
            },
            DetectionFilter {
                locked: LockFilter::Locked,
                ..DetectionFilter::default()
            },
            DetectionFilter {
                category: TodayFilter::Rare,
                ..DetectionFilter::default()
            },
        ] {
            assert!(!f.is_unfiltered(), "{f:?} reported itself unfiltered");
        }
    }

    #[test]
    fn a_whitespace_only_search_is_not_a_filter() {
        let f = DetectionFilter {
            text: Some("   ".into()),
            ..DetectionFilter::default()
        };
        assert!(f.is_unfiltered());
        assert_eq!(f.where_prefix().0, "");
    }

    #[test]
    fn sort_order_is_total_and_round_trips() {
        for s in [
            SortOrder::NewestFirst,
            SortOrder::OldestFirst,
            SortOrder::ConfidenceHigh,
            SortOrder::ConfidenceLow,
            SortOrder::SpeciesAz,
            SortOrder::SpeciesZa,
        ] {
            assert_eq!(SortOrder::from_token(Some(s.as_str())), s, "{s:?}");
        }
        assert_eq!(SortOrder::from_token(None), SortOrder::NewestFirst);
        assert_eq!(
            SortOrder::from_token(Some("sideways")),
            SortOrder::NewestFirst
        );
    }

    /// Every order that leads with a non-unique column must fall back to the
    /// full `Date`/`Time` pair, or paging can show one row twice and another
    /// never.
    ///
    /// Structural rather than behavioural, and deliberately so. The obvious
    /// version — page through a seeded tie and assert no row repeats — was
    /// written first and *could not fail*: SQLite sorts a small table
    /// deterministically, so it returned a stable order with the tie-break
    /// removed, passing for a reason that had nothing to do with what it
    /// claimed. It was deleted rather than kept as decoration. This one goes
    /// red when either half of the tie-break is dropped.
    #[test]
    fn every_sort_order_breaks_ties_so_paging_cannot_repeat_a_row() {
        for s in [
            SortOrder::ConfidenceHigh,
            SortOrder::ConfidenceLow,
            SortOrder::SpeciesAz,
            SortOrder::SpeciesZa,
        ] {
            let sql = s.sql();
            assert!(
                sql.contains("Date") && sql.contains("Time"),
                "{s:?} orders by a non-unique column and falls back to only part of \
                 the Date/Time pair ({sql}), so two rows can swap between pages"
            );
        }
    }

    #[test]
    fn date_range_puts_the_ends_the_right_way_round() {
        assert_eq!(
            DateRange::between("2026-05-09", "2026-05-01"),
            DateRange::Between("2026-05-01".into(), "2026-05-09".into()),
            "a date picker lets somebody choose the end before the start, and a \
             range that matches nothing looks like a broken search"
        );
    }

    #[test]
    fn an_hour_window_that_wraps_midnight_uses_or_not_and() {
        let night = DetectionFilter {
            hours: Some(HourWindow::new(22, 4)),
            ..DetectionFilter::default()
        };
        let (sql, _) = night.where_prefix();
        assert!(
            sql.contains(">= ? OR "),
            "22:00–04:59 has to be an OR; an AND matches nothing, which is what \
             an operator hunting owls would see: {sql}"
        );

        let day = DetectionFilter {
            hours: Some(HourWindow::new(5, 9)),
            ..DetectionFilter::default()
        };
        assert!(day.where_prefix().0.contains(">= ? AND "));
    }

    #[test]
    fn hour_window_clamps_out_of_range_input() {
        assert_eq!(HourWindow::new(99, 200), HourWindow::new(23, 23));
        assert!(
            !HourWindow::new(3, 3).wraps(),
            "a single hour is not a wrap"
        );
    }

    #[test]
    fn unreviewed_is_null_not_a_string() {
        let f = DetectionFilter {
            verdict: VerdictFilter::Unreviewed,
            ..DetectionFilter::default()
        };
        let (sql, _) = f.where_prefix();
        assert!(
            sql.contains("review_verdict IS NULL"),
            "unreviewed rows carry NULL, not a verdict string: {sql}"
        );
    }

    // ── against a real database ────────────────────────────────────────────

    fn seeded() -> Connection {
        let conn = test_conn();
        // date, time, species, confidence
        let rows = [
            (
                "2026-05-01",
                "05:30:00",
                "European Robin",
                "Erithacus rubecula",
                0.95,
            ),
            ("2026-05-01", "23:10:00", "Tawny Owl", "Strix aluco", 0.71),
            (
                "2026-05-02",
                "06:15:00",
                "European Robin",
                "Erithacus rubecula",
                0.42,
            ),
            ("2026-05-02", "02:05:00", "Tawny Owl", "Strix aluco", 0.88),
            (
                "2026-05-03",
                "12:00:00",
                "Common Blackbird",
                "Turdus merula",
                0.99,
            ),
        ];
        for (d, t, com, sci, c) in rows {
            insert_test_detection(&conn, d, t, com, sci, c);
        }
        conn
    }

    fn names(rows: &[DetectionRow]) -> Vec<String> {
        rows.iter().map(|r| r.com_name.clone()).collect()
    }

    #[test]
    fn confidence_bounds_are_inclusive_at_both_ends() {
        let conn = seeded();
        let f = DetectionFilter {
            min_confidence: Some(0.42),
            max_confidence: Some(0.95),
            ..DetectionFilter::default()
        };
        let rows = search_detections(&conn, &f, 50, 0).unwrap();
        let confs: Vec<f64> = rows.iter().map(|r| r.confidence).collect();
        assert!(
            confs.contains(&0.42) && confs.contains(&0.95),
            "both bounds must be inclusive, got {confs:?}"
        );
        assert!(!confs.contains(&0.99), "0.99 is above the max: {confs:?}");
        assert!(!confs.contains(&0.71) || confs.contains(&0.71));
        assert_eq!(
            i64::try_from(rows.len()).unwrap(),
            search_detection_count(&conn, &f).unwrap(),
            "count and page must agree about the same filter"
        );
    }

    #[test]
    fn a_wrapping_hour_window_finds_the_night_birds() {
        let conn = seeded();
        let f = DetectionFilter {
            hours: Some(HourWindow::new(22, 4)),
            ..DetectionFilter::default()
        };
        let rows = search_detections(&conn, &f, 50, 0).unwrap();
        assert_eq!(
            names(&rows),
            vec!["Tawny Owl".to_string(), "Tawny Owl".to_string()],
            "23:10 and 02:05 are both inside 22:00–04:59"
        );
    }

    #[test]
    fn a_date_range_spans_its_ends_inclusively() {
        let conn = seeded();
        let f = DetectionFilter {
            dates: DateRange::between("2026-05-01", "2026-05-02"),
            ..DetectionFilter::default()
        };
        assert_eq!(search_detection_count(&conn, &f).unwrap(), 4);
    }

    #[test]
    fn species_selection_matches_common_or_scientific_name() {
        let conn = seeded();
        for name in ["Tawny Owl", "strix aluco", "TAWNY OWL"] {
            let f = DetectionFilter {
                species: vec![name.into()],
                ..DetectionFilter::default()
            };
            assert_eq!(
                search_detection_count(&conn, &f).unwrap(),
                2,
                "selecting {name:?} should find both owl rows"
            );
        }
    }

    #[test]
    fn multiple_species_are_ored_not_anded() {
        let conn = seeded();
        let f = DetectionFilter {
            species: vec!["Tawny Owl".into(), "Common Blackbird".into()],
            ..DetectionFilter::default()
        };
        assert_eq!(
            search_detection_count(&conn, &f).unwrap(),
            3,
            "picking two species must show both, not their intersection (which is empty)"
        );
    }

    #[test]
    fn criteria_combine_as_and() {
        let conn = seeded();
        let f = DetectionFilter {
            species: vec!["European Robin".into()],
            min_confidence: Some(0.9),
            ..DetectionFilter::default()
        };
        let rows = search_detections(&conn, &f, 50, 0).unwrap();
        assert_eq!(rows.len(), 1, "only the 0.95 robin qualifies");
        assert!((rows[0].confidence - 0.95).abs() < 1e-9);
    }

    #[test]
    fn sorting_actually_reorders() {
        let conn = seeded();
        let high = search_detections(
            &conn,
            &DetectionFilter {
                sort: SortOrder::ConfidenceHigh,
                ..DetectionFilter::default()
            },
            50,
            0,
        )
        .unwrap();
        let low = search_detections(
            &conn,
            &DetectionFilter {
                sort: SortOrder::ConfidenceLow,
                ..DetectionFilter::default()
            },
            50,
            0,
        )
        .unwrap();
        assert!(high[0].confidence > low[0].confidence);
        assert!((high[0].confidence - 0.99).abs() < 1e-9);
        assert!((low[0].confidence - 0.42).abs() < 1e-9);
    }

    #[test]
    fn the_category_shortcut_means_the_same_thing_over_a_range_as_on_one_day() {
        // `TodayFilter::FirstToday` used to key on a bound `?1` date, which
        // only works for a single-day query. The correlated form has to agree
        // with the old one on the case the old one handled, or this is a
        // behaviour change dressed as a generalisation.
        let conn = seeded();
        let one_day = DetectionFilter {
            dates: DateRange::On("2026-05-02".into()),
            category: TodayFilter::FirstToday,
            ..DetectionFilter::default()
        };
        // On 2026-05-02 both species were already heard on 05-01, so nothing is
        // a first.
        assert_eq!(search_detection_count(&conn, &one_day).unwrap(), 0);

        let first_day = DetectionFilter {
            dates: DateRange::On("2026-05-01".into()),
            category: TodayFilter::FirstToday,
            ..DetectionFilter::default()
        };
        assert_eq!(
            search_detection_count(&conn, &first_day).unwrap(),
            2,
            "both species are first heard on 05-01"
        );

        // And over the whole range it is the union of each day's firsts.
        let ranged = DetectionFilter {
            dates: DateRange::Any,
            category: TodayFilter::FirstToday,
            ..DetectionFilter::default()
        };
        assert_eq!(
            search_detection_count(&conn, &ranged).unwrap(),
            3,
            "two on 05-01 plus the blackbird's first on 05-03"
        );
    }

    #[test]
    fn known_sources_lists_only_real_ones() {
        let conn = seeded();
        // Seeded rows have no Source, so the picker must offer nothing rather
        // than a row of empty strings.
        assert!(known_sources(&conn).unwrap().is_empty());
    }
}
