//! SQL query builders for duckdb-behavioral functions.
//!
//! Generates the SQL queries that use the `behavioral` `DuckDB` extension.
//! These queries are designed to be executed against a `DuckDB` connection
//! that has the behavioral extension loaded and a `detections_ts` view
//! with a proper TIMESTAMP column.
//!
//! Every builder targets the published extension's real function signatures
//! (verified against the live extension by the tests in `connection::live`):
//!
//! | Function              | Shape used here                                    |
//! |-----------------------|----------------------------------------------------|
//! | `sessionize`          | window fn in a subquery, then `GROUP BY session_id`|
//! | `retention`           | `retention(BOOLEAN, …)` -> `BOOLEAN[]` aggregate   |
//! | `window_funnel`       | variadic `BOOLEAN` step conditions                 |
//! | `window_funnel_events`| like `window_funnel` -> `TIMESTAMP[]` of step times |
//! | `sequence_match`      | `sequence_match(pattern, ts, BOOLEAN, …)` -> bool  |
//! | `sequence_count`      | like `sequence_match` -> `BIGINT` occurrence count |
//! | `sequence_next_node`  | `(direction, mode, ts, value, BOOLEAN, …)` -> text |

use std::fmt::Write as _;

use crate::types::{FunnelParams, PatternParams, RetentionParams, SessionizeParams};

/// SQL to create the timestamp view for behavioral queries.
///
/// This view adds a proper TIMESTAMP column from the Date and Time text fields.
///
/// `TRY_CAST`, not `CAST`, and the distinction is load-bearing. `Date` and
/// `Time` are free-form `TEXT NOT NULL` in SQLite: the column type forbids only
/// NULL, and the BirdNET-Pi importer turns a NULL `Date` into `""` and copies
/// malformed values through verbatim, so a real station's history can hold rows
/// that name no point in time. Under `CAST`, a single such row did not degrade
/// the analytics — it *aborted* them. DuckDB raises
///
///   Conversion Error: invalid timestamp field format: " "
///
/// for the entire query, so one bad row anywhere in a multi-year history
/// emptied every behavioural and time-series dashboard at once, while the rest
/// of the app — served from SQLite, which never parses these columns — looked
/// perfectly healthy. `COUNT(*)` over this view kept working throughout
/// (DuckDB does not evaluate projected columns it does not need), which is why
/// the health endpoint stayed green and the failure presented as "the analytics
/// dashboards are broken" with nothing anywhere reporting an error.
///
/// `TRY_CAST` yields NULL for those rows instead. Every aggregate here is
/// time-bucketed, and SQL grouping and comparison both drop NULLs, so an
/// unplaceable row falls out of the results rather than taking the query with
/// it. Coercing to an epoch default was rejected: it would invent detections on
/// 1970-01-01. Rows excluded this way are counted by
/// [`COUNT_UNPLACEABLE_DETECTIONS`] so the loss is reported rather than silent.
///
/// # Two clocks, named apart
///
/// `detection_timestamp` is the station's **local wall clock** and stays that
/// way: it is what hour-of-day filters, calendar-date grouping and every
/// displayed session time are asked in, and all three are local questions.
/// `detection_instant` is the same detection as a **point in time**, from
/// `detected_at_utc` (migration 32).
///
/// The distinction is not decorative. Local wall clock is not monotonic — one
/// hour repeats every autumn and one never happens every spring — so anything
/// that measures *elapsed time* or *order* on it is wrong across a transition:
/// a 30-minute sessionisation gap sees a **negative 55-minute** gap on the
/// autumn night and splits a session that never broke, and reads **two hours**
/// for one real hour on the spring one. Both were live in every `sessionize`,
/// `window_funnel`, `sequence_match` and gap query here.
///
/// So the rule this view exists to make expressible, and which the queries
/// downstream follow: **elapsed time and ordering ask `detection_instant`;
/// clock position, calendar date and anything shown to a human ask
/// `detection_timestamp`.** Naming the two apart is the same move
/// `DailySchedule::clock()` made for the recording gate — a caller that has to
/// remember which clock a bare `ts` means will eventually not.
///
/// `to_timestamp` yields a `TIMESTAMP WITH TIME ZONE`; the cast to plain
/// `TIMESTAMP` keeps it the same type as `detection_timestamp` so the two are
/// interchangeable as arguments to the extension's functions. Rows with no
/// instant — a history predating migration 32, or one whose wall clock names no
/// point in time — yield NULL and drop out of ordered and bucketed results
/// exactly as they already do for `detection_timestamp`.
pub const CREATE_DETECTIONS_TS_VIEW: &str = "
CREATE OR REPLACE VIEW detections_ts AS
SELECT *,
    TRY_CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp,
    CAST(to_timestamp(detected_at_utc) AS TIMESTAMP) AS detection_instant,
    TRY_CAST(Date AS DATE) AS detection_date
FROM detections
WHERE review_verdict IS DISTINCT FROM 'rejected';
";

/// Every analytic reads `detections_ts`, so the `WHERE` above is where a
/// reviewer's verdict becomes real.
///
/// `IS DISTINCT FROM` rather than `<> 'rejected'`: SQL three-valued logic makes
/// `NULL <> 'rejected'` evaluate to NULL, which a `WHERE` treats as false — so
/// the plain comparison would have excluded every *unreviewed* detection, which
/// is almost all of them. That would have turned "hide the rejects" into "hide
/// everything nobody has looked at yet", and the dashboards would have gone
/// nearly empty on any station with a review backlog.
///
/// Rejected rows stay in `detections` and in `detection_reviews`. Nothing is
/// deleted: the evidence and the verdict both remain, and clearing the verdict
/// brings the detection straight back.
///
/// Number of synced rows whose `Date`/`Time` cannot be parsed as a timestamp.
///
/// These are exactly the rows [`CREATE_DETECTIONS_TS_VIEW`] gives a NULL
/// `detection_timestamp`, so they are absent from every time-bucketed
/// analytic. Counting them is what keeps that exclusion honest: a station can
/// be told how many of its detections no dashboard can place, instead of
/// quietly reporting totals that do not add up.
pub const COUNT_UNPLACEABLE_DETECTIONS: &str =
    "SELECT COUNT(*) FROM detections_ts WHERE detection_timestamp IS NULL";

/// SQL to load the behavioral extension (offline-safe).
///
/// Tries `LOAD` first — this succeeds if the extension was previously installed
/// and cached locally, avoiding a network round-trip on every startup.
/// Only falls back to `INSTALL FROM community` when the extension is not yet cached.
pub const LOAD_BEHAVIORAL_CACHED: &str = "LOAD behavioral;";

/// SQL to install and load the behavioral extension from the community registry.
///
/// Requires network on first run; subsequent runs use the locally cached extension.
pub const INSTALL_BEHAVIORAL: &str = "
INSTALL behavioral FROM community;
LOAD behavioral;
";

/// SQL to force-reinstall the behavioral extension from the community registry
/// and load it.
///
/// Unlike [`INSTALL_BEHAVIORAL`], `FORCE INSTALL` always re-downloads the latest
/// build for the bundled `DuckDB` version even when a cached copy exists. Backs
/// the manual `--refresh-extension` maintenance command.
pub const FORCE_INSTALL_BEHAVIORAL: &str = "
FORCE INSTALL behavioral FROM community;
LOAD behavioral;
";

/// SQL to read the loaded behavioral extension version (e.g. `v0.8.0`).
///
/// Filters on `loaded` so it reports the version active in the current
/// connection, not one merely present in `DuckDB`'s shared extension cache.
/// Returns no rows when the extension is not loaded.
pub const BEHAVIORAL_EXTENSION_VERSION: &str = "SELECT extension_version FROM duckdb_extensions() \
     WHERE extension_name = 'behavioral' AND loaded";

/// Build SQL for activity sessionization.
///
/// `sessionize()` is a window function that assigns a session id per detection,
/// starting a new session whenever the gap since the same species' previous
/// detection exceeds `gap_minutes`. A window expression cannot appear in
/// `GROUP BY`, so the id is materialised in an inner query and the outer query
/// aggregates each session.
pub fn sessionize_sql(params: &SessionizeParams) -> String {
    let species_filter = params.species.as_ref().map_or_else(String::new, |s| {
        format!("WHERE Com_Name = '{}'", s.replace('\'', "''"))
    });

    format!(
        "SELECT
            species,
            session_id,
            COUNT(*) AS detection_count,
            CAST(MIN(detection_timestamp) AS VARCHAR) AS start_time,
            CAST(MAX(detection_timestamp) AS VARCHAR) AS end_time,
            DATEDIFF('second', MIN(detection_instant), MAX(detection_instant)) AS duration_secs
        FROM (
            SELECT
                Com_Name AS species,
                detection_timestamp,
                detection_instant,
                sessionize(detection_instant, INTERVAL '{gap} MINUTE')
                    OVER (PARTITION BY Sci_Name ORDER BY detection_instant)
                    AS session_id
            FROM detections_ts
            {species_filter}
        )
        GROUP BY species, session_id
        ORDER BY start_time DESC
        LIMIT {limit}",
        gap = params.gap_minutes,
        limit = params.limit,
    )
}

/// Build SQL for per-species day-N retention.
///
/// Every distinct detection day of a species is a cohort anchor. The
/// `retention()` aggregate reports, per anchor, whether the species recurred
/// within each day interval (its first argument anchors the cohort and is
/// always satisfied; argument `i+1` is "seen again within interval `i`").
/// Averaging the boolean array across a species' anchors gives its retention
/// rate at each interval; the final (long-term) rate drives the residency
/// classification.
///
/// Callers must pass at least one interval and at most 31 (the aggregate
/// accepts 2..=32 conditions including the anchor); [`crate::connection`]
/// enforces this before building the SQL.
pub fn retention_sql(params: &RetentionParams) -> String {
    // Argument 1 anchors the cohort; one further condition per interval.
    let conditions: Vec<String> = std::iter::once("b.d = a.d".to_string())
        .chain(
            params
                .intervals
                .iter()
                .map(|days| format!("b.d > a.d AND b.d <= a.d + INTERVAL '{days} day'")),
        )
        .collect();
    // retention()[1] is the anchor; interval `i` (0-based) is retention()[i+2].
    let rate_exprs: Vec<String> = (0..params.intervals.len())
        .map(|i| format!("AVG(CASE WHEN r[{}] THEN 1.0 ELSE 0.0 END)", i + 2))
        .collect();
    let long_term_idx = params.intervals.len(); // 1-based index of the last rate

    format!(
        "WITH sd AS (
            SELECT DISTINCT Com_Name, detection_date AS d FROM detections_ts
        ),
        cohort AS (
            SELECT a.Com_Name AS species, a.d AS anchor,
                   retention({conditions}) AS r
            FROM sd a JOIN sd b ON a.Com_Name = b.Com_Name
            GROUP BY a.Com_Name, a.d
        )
        SELECT species, [{rates}] AS retention_rates
        FROM cohort
        GROUP BY species
        HAVING COUNT(*) >= {min}
        ORDER BY retention_rates[{long_term_idx}] DESC",
        conditions = conditions.join(", "),
        rates = rate_exprs.join(", "),
        min = params.min_detections,
    )
}

/// Build SQL for dawn chorus funnel analysis.
///
/// `window_funnel()` reports, per day, how many leading steps of the expected
/// species sequence occurred within the time window. The step conditions are
/// passed as variadic boolean arguments — the real signature — not as an array.
///
/// Callers must pass 2..=32 species; [`crate::connection`] enforces this.
pub fn funnel_sql(params: &FunnelParams) -> String {
    let conditions = species_conditions(&params.species_sequence);

    format!(
        "SELECT
            CAST(CAST(detection_timestamp AS DATE) AS VARCHAR) AS date,
            window_funnel(
                INTERVAL '{window} MINUTE',
                detection_instant,
                {conditions}
            ) AS steps_completed
        FROM detections_ts
        WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN {start} AND {end}
        GROUP BY CAST(detection_timestamp AS DATE)
        ORDER BY date DESC",
        window = params.window_minutes,
        start = params.hour_start,
        end = params.hour_end,
    )
}

/// Build SQL for dawn-chorus funnel *step timings* (`window_funnel_events`,
/// v0.8.0).
///
/// Same shape as [`funnel_sql`], but `window_funnel_events()` returns the
/// `TIMESTAMP[]` of when each completed step fired rather than a step count.
/// Each element is cast to `VARCHAR` via `list_transform` so the result is a
/// plain string list the typed layer can read without timestamp-array decoding.
///
/// Callers must pass 2..=32 species; [`crate::connection`] enforces this.
pub fn funnel_events_sql(params: &FunnelParams) -> String {
    let conditions = species_conditions(&params.species_sequence);

    format!(
        "SELECT
            CAST(CAST(detection_timestamp AS DATE) AS VARCHAR) AS date,
            list_transform(
                window_funnel_events(
                    INTERVAL '{window} MINUTE',
                    detection_instant,
                    {conditions}
                ),
                x -> CAST(x AS VARCHAR)
            ) AS step_times
        FROM detections_ts
        WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN {start} AND {end}
        GROUP BY CAST(detection_timestamp AS DATE)
        ORDER BY date DESC",
        window = params.window_minutes,
        start = params.hour_start,
        end = params.hour_end,
    )
}

/// Build SQL for ordered sequence pattern matching.
///
/// `sequence_match()` tests, per day, whether the configured species were
/// detected in order (with any other events allowed between steps). When
/// `max_gap_minutes` is set a `(?t<=secs)` time constraint is inserted before
/// each subsequent step.
///
/// Callers must pass 2..=32 species; [`crate::connection`] enforces this.
pub fn sequence_match_sql(params: &PatternParams) -> String {
    let pattern = ordered_pattern(params.species_sequence.len(), params.max_gap_minutes);
    let conditions = species_conditions(&params.species_sequence);

    format!(
        "SELECT
            CAST(CAST(detection_timestamp AS DATE) AS VARCHAR) AS date,
            sequence_match('{pattern}', detection_instant,
                {conditions}
            ) AS matched
        FROM detections_ts
        WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN {start} AND {end}
        GROUP BY CAST(detection_timestamp AS DATE)
        ORDER BY date DESC",
        start = params.hour_start,
        end = params.hour_end,
    )
}

/// Build SQL for ordered sequence *occurrence counts* (`sequence_count`,
/// v0.8.0).
///
/// Same pattern + conditions as [`sequence_match_sql`], but `sequence_count()`
/// returns the `BIGINT` number of non-overlapping times the ordered sequence
/// occurred that day rather than a single boolean — so "did A→B→C happen?"
/// becomes "how many times did A→B→C happen?".
///
/// Callers must pass 2..=32 species; [`crate::connection`] enforces this.
pub fn sequence_count_sql(params: &PatternParams) -> String {
    let pattern = ordered_pattern(params.species_sequence.len(), params.max_gap_minutes);
    let conditions = species_conditions(&params.species_sequence);

    format!(
        "SELECT
            CAST(CAST(detection_timestamp AS DATE) AS VARCHAR) AS date,
            sequence_count('{pattern}', detection_instant,
                {conditions}
            ) AS match_count
        FROM detections_ts
        WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN {start} AND {end}
        GROUP BY CAST(detection_timestamp AS DATE)
        ORDER BY date DESC",
        start = params.hour_start,
        end = params.hour_end,
    )
}

/// Build SQL for ordered sequence *match-event timings* (`sequence_match_events`,
/// v0.8.0).
///
/// Same pattern + conditions as [`sequence_match_sql`], but
/// `sequence_match_events()` returns the `TIMESTAMP[]` of the events that
/// satisfied the pattern — the longest in-order prefix reached that day (the
/// full set when the sequence completes, a partial otherwise), like
/// `window_funnel_events`. Each element is cast to `VARCHAR` via
/// `list_transform`, mirroring [`funnel_events_sql`], so the result reads back
/// as a plain string list.
///
/// Callers must pass 2..=32 species; [`crate::connection`] enforces this.
pub fn sequence_match_events_sql(params: &PatternParams) -> String {
    let pattern = ordered_pattern(params.species_sequence.len(), params.max_gap_minutes);
    let conditions = species_conditions(&params.species_sequence);

    format!(
        "SELECT
            CAST(CAST(detection_timestamp AS DATE) AS VARCHAR) AS date,
            list_transform(
                sequence_match_events('{pattern}', detection_instant,
                    {conditions}
                ),
                x -> CAST(x AS VARCHAR)
            ) AS step_times
        FROM detections_ts
        WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN {start} AND {end}
        GROUP BY CAST(detection_timestamp AS DATE)
        ORDER BY date DESC",
        start = params.hour_start,
        end = params.hour_end,
    )
}

/// Build SQL for next-species prediction.
///
/// The timeline is split into activity sessions separated by gaps larger than
/// `window_minutes`; within each session `sequence_next_node()` finds the
/// species detected immediately after the first occurrence of the trigger.
/// Counting those across sessions yields a frequency distribution of what
/// typically follows the trigger species.
///
/// `sequence_next_node(direction, mode, ts, value, base_cond, event_cond)`
/// requires at least two boolean conditions; `forward`/`first_match` anchors on
/// the first event matching `base_cond` (the trigger) and the `TRUE` event
/// condition accepts whatever node comes next, so it returns that node's
/// species (verified against the live extension in `connection::live`).
pub fn next_species_sql(trigger_species: &str, window_minutes: u32, limit: u32) -> String {
    let escaped = trigger_species.replace('\'', "''");
    format!(
        "WITH sessioned AS (
            SELECT detection_instant, Com_Name,
                   sessionize(detection_instant, INTERVAL '{window_minutes} MINUTE')
                       OVER (ORDER BY detection_instant) AS sid
            FROM detections_ts
        ),
        per_session AS (
            SELECT sequence_next_node('forward', 'first_match',
                       detection_instant, Com_Name,
                       Com_Name = '{escaped}', TRUE) AS predicted
            FROM sessioned
            GROUP BY sid
        )
        SELECT predicted AS predicted_species, COUNT(*) AS frequency
        FROM per_session
        WHERE predicted IS NOT NULL
        GROUP BY predicted
        ORDER BY frequency DESC, predicted_species
        LIMIT {limit}",
    )
}

/// Render an ordered list of `Com_Name = '…'` boolean conditions, escaping
/// embedded quotes, joined for use as variadic arguments.
fn species_conditions(species: &[String]) -> String {
    species
        .iter()
        .map(|s| format!("Com_Name = '{}'", s.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",\n                ")
}

/// Build the NFA pattern string for an ordered sequence of `steps` conditions.
///
/// `(?1).*(?2).*…(?N)` matches the conditions in order with any events between.
/// When `max_gap_minutes` is set, a `(?t<=secs)` constraint precedes each step
/// after the first so consecutive steps must occur within that gap.
fn ordered_pattern(steps: usize, max_gap_minutes: Option<u32>) -> String {
    let mut pattern = String::from("(?1)");
    for i in 2..=steps {
        pattern.push_str(".*");
        if let Some(gap) = max_gap_minutes {
            let _ = write!(pattern, "(?t<={})", u64::from(gap) * 60);
        }
        let _ = write!(pattern, "(?{i})");
    }
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessionize_sql_all_species() {
        let sql = sessionize_sql(&SessionizeParams::default());
        assert!(sql.contains("sessionize(detection_instant, INTERVAL '30 MINUTE')"));
        assert!(sql.contains("OVER (PARTITION BY Sci_Name ORDER BY detection_instant)"));
        // The window id is grouped from an inner query, never in a top-level
        // GROUP BY of a window expression.
        assert!(sql.contains("GROUP BY species, session_id"));
        assert!(sql.contains("LIMIT 100"));
        assert!(!sql.contains("WHERE"));
    }

    #[test]
    fn sessionize_sql_single_species() {
        let params = SessionizeParams {
            species: Some("European Robin".into()),
            gap_minutes: 15,
            limit: 50,
        };
        let sql = sessionize_sql(&params);
        assert!(sql.contains("WHERE Com_Name = 'European Robin'"));
        assert!(sql.contains("INTERVAL '15 MINUTE'"));
        assert!(sql.contains("LIMIT 50"));
    }

    #[test]
    fn retention_sql_default() {
        let sql = retention_sql(&RetentionParams::default());
        // Real signature: boolean conditions, not (date, array).
        assert!(sql.contains("retention(b.d = a.d,"));
        assert!(sql.contains("b.d <= a.d + INTERVAL '1 day'"));
        assert!(sql.contains("b.d <= a.d + INTERVAL '30 day'"));
        assert!(sql.contains("AVG(CASE WHEN r[2] THEN 1.0 ELSE 0.0 END)"));
        // 6 default intervals -> long-term rate is element 6.
        assert!(sql.contains("ORDER BY retention_rates[6] DESC"));
        assert!(sql.contains(">= 5"));
        // The old, non-existent `retention(date, [int,…])` form is gone.
        assert!(!sql.contains("retention(detection_date"));
        assert!(!sql.contains("[1, 2, 3"));
    }

    #[test]
    fn funnel_sql_default() {
        let sql = funnel_sql(&FunnelParams::default());
        assert!(sql.contains("window_funnel("));
        assert!(sql.contains("Com_Name = 'European Robin'"));
        assert!(sql.contains("BETWEEN 4 AND 8"));
        // Conditions are variadic, not wrapped in an array literal.
        assert!(!sql.contains("[Com_Name"));
    }

    #[test]
    fn sequence_match_sql_default() {
        let sql = sequence_match_sql(&PatternParams::default());
        assert!(sql.contains("sequence_match('(?1).*(?2).*(?3)', detection_instant"));
        assert!(sql.contains("Com_Name = 'European Robin'"));
        assert!(sql.contains("AS matched"));
    }

    #[test]
    fn sequence_match_sql_with_gap_inserts_time_token() {
        let params = PatternParams {
            species_sequence: vec!["A".into(), "B".into()],
            max_gap_minutes: Some(30),
            ..PatternParams::default()
        };
        let sql = sequence_match_sql(&params);
        assert!(sql.contains("sequence_match('(?1).*(?t<=1800)(?2)'"));
    }

    #[test]
    fn sequence_count_sql_default() {
        let sql = sequence_count_sql(&PatternParams::default());
        assert!(sql.contains("sequence_count('(?1).*(?2).*(?3)', detection_instant"));
        assert!(sql.contains("Com_Name = 'European Robin'"));
        assert!(sql.contains("AS match_count"));
    }

    #[test]
    fn sequence_match_events_sql_default() {
        let sql = sequence_match_events_sql(&PatternParams::default());
        assert!(sql.contains("sequence_match_events('(?1).*(?2).*(?3)', detection_instant"));
        // The TIMESTAMP[] is cast element-wise to VARCHAR for a plain list.
        assert!(sql.contains("list_transform("));
        assert!(sql.contains("x -> CAST(x AS VARCHAR)"));
        assert!(sql.contains("AS step_times"));
        assert!(sql.contains("Com_Name = 'European Robin'"));
        // Conditions are variadic, not wrapped in an array literal.
        assert!(!sql.contains("[Com_Name"));
    }

    #[test]
    fn sequence_match_events_sql_with_gap_inserts_time_token() {
        let params = PatternParams {
            species_sequence: vec!["A".into(), "B".into()],
            max_gap_minutes: Some(30),
            ..PatternParams::default()
        };
        let sql = sequence_match_events_sql(&params);
        assert!(sql.contains("sequence_match_events('(?1).*(?t<=1800)(?2)'"));
    }

    #[test]
    fn funnel_events_sql_default() {
        let sql = funnel_events_sql(&FunnelParams::default());
        assert!(sql.contains("window_funnel_events("));
        // The TIMESTAMP[] is cast element-wise to VARCHAR for a plain list.
        assert!(sql.contains("list_transform("));
        assert!(sql.contains("x -> CAST(x AS VARCHAR)"));
        assert!(sql.contains("AS step_times"));
        assert!(sql.contains("Com_Name = 'European Robin'"));
        // Conditions are variadic, not wrapped in an array literal.
        assert!(!sql.contains("[Com_Name"));
    }

    #[test]
    fn next_species_sql_uses_real_signature_and_escapes() {
        let sql = next_species_sql("O'Brien's Warbler", 60, 10);
        assert!(sql.contains("sequence_next_node('forward', 'first_match'"));
        assert!(sql.contains("Com_Name = 'O''Brien''s Warbler', TRUE)"));
        assert!(sql.contains("INTERVAL '60 MINUTE'"));
        assert!(sql.contains("LIMIT 10"));
    }

    #[test]
    fn ordered_pattern_shapes() {
        assert_eq!(ordered_pattern(3, None), "(?1).*(?2).*(?3)");
        assert_eq!(ordered_pattern(2, Some(60)), "(?1).*(?t<=3600)(?2)");
        assert_eq!(ordered_pattern(1, None), "(?1)");
    }

    #[test]
    fn create_view_sql_is_valid() {
        assert!(CREATE_DETECTIONS_TS_VIEW.contains("detection_timestamp"));
        assert!(CREATE_DETECTIONS_TS_VIEW.contains("TIMESTAMP"));
    }

    /// The rule the whole two-clock split exists to enforce, asserted over
    /// every builder at once rather than one string at a time.
    ///
    /// **Elapsed time and ordering ask `detection_instant`; clock position,
    /// calendar date and anything displayed ask `detection_timestamp`.**
    ///
    /// Local wall clock is not monotonic — one hour repeats every autumn and
    /// one never happens every spring — so a `sessionize` interval, a
    /// `window_funnel` window or a `sequence_match` pattern measured on it is
    /// wrong across a transition. `tests/two_clocks.rs` pins the value that
    /// makes this concrete: one real hour reads as 60 minutes on the instant
    /// and 0 on the wall clock.
    ///
    /// A single-string assertion cannot catch a *new* builder that reaches for
    /// the wrong column, and a new builder is exactly how this would come back.
    /// Every construct whose argument is an elapsed duration or an order.
    const ELAPSED: &[&str] = &[
        "sessionize(",
        "window_funnel(",
        "window_funnel_events(",
        "sequence_match(",
        "sequence_count(",
        "sequence_match_events(",
        "sequence_next_node(",
        "ORDER BY detection_",
        "DATEDIFF(",
    ];

    #[test]
    fn no_builder_measures_elapsed_time_on_the_wall_clock() {
        let sessionize = sessionize_sql(&SessionizeParams::default());
        let funnel_params = FunnelParams {
            species_sequence: vec!["A".into(), "B".into()],
            ..FunnelParams::default()
        };
        let pattern_params = PatternParams {
            species_sequence: vec!["A".into(), "B".into(), "C".into()],
            ..PatternParams::default()
        };
        let all = [
            ("sessionize", sessionize),
            ("funnel", funnel_sql(&funnel_params)),
            ("funnel_events", funnel_events_sql(&funnel_params)),
            ("sequence_match", sequence_match_sql(&pattern_params)),
            ("sequence_count", sequence_count_sql(&pattern_params)),
            (
                "sequence_match_events",
                sequence_match_events_sql(&pattern_params),
            ),
            ("next_species", next_species_sql("Robin", 30, 5)),
        ];
        for (name, sql) in &all {
            for marker in ELAPSED {
                let mut from = 0;
                while let Some(rel) = sql[from..].find(marker) {
                    let at = from + rel;
                    // The clock named within the construct's own argument list
                    // — bounded so a later, unrelated `detection_timestamp` in
                    // the same query cannot satisfy the check.
                    let window = &sql[at..(at + 200).min(sql.len())];
                    let cut = window.find(')').map_or(window.len(), |i| i + 1);
                    let args = &window[..cut.max(marker.len() + 30).min(window.len())];
                    assert!(
                        !args.contains("detection_timestamp"),
                        "{name}: `{marker}` measures elapsed time or order and must \
                         ask detection_instant, not the wall clock:\n  {args}"
                    );
                    from = at + 1;
                }
            }
            // …and the counterpart: the local questions must stay local, or
            // every hour-of-day chart and every daily bucket moves by the
            // station's offset.
            //
            // Stated as a prohibition, not as "contains the right form".
            // `funnel_sql` names the same expression twice — once in the
            // projection and once in its `GROUP BY` — so a containment check
            // was satisfied by the *other* occurrence and stayed green while
            // the projection was moved onto the instant. Watched escape before
            // this was rewritten.
            for bad in [
                "CAST(detection_instant AS DATE)",
                "EXTRACT(HOUR FROM detection_instant)",
            ] {
                assert!(
                    !sql.contains(bad),
                    "{name}: `{bad}` — the hour of day a bird sang, and the day \
                     it belongs to, are questions about the station's own clock"
                );
            }
            // And they must still be asked somewhere, or a builder that simply
            // dropped its local grouping would satisfy the prohibition above.
            if sql.contains("AS DATE)") {
                assert!(
                    sql.contains("CAST(detection_timestamp AS DATE)"),
                    "{name}: the calendar date a detection belongs to is local"
                );
            }
            if sql.contains("EXTRACT(HOUR FROM") {
                assert!(
                    sql.contains("EXTRACT(HOUR FROM detection_timestamp)"),
                    "{name}: hour-of-day is a local question"
                );
            }
        }
    }
}
