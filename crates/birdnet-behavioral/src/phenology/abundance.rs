//! SQL query builders for species abundance analytics.
//!
//! Generates SQL for weekly and monthly detection counts, peak weeks, weekly
//! species richness, and a weekly detection rate.
//!
//! None of these is an abundance of birds. They count *detections*: one bird
//! singing for an hour makes many, and how often a species is detected depends
//! on how loud, how close and how recognisable it is as much as on how many
//! there are. Compare a species with itself across weeks, not with another
//! species.
//!
//! ## Relative index (no effort correction)
//!
//! [`weekly_abundance_sql`] divides each week's detection count by the
//! species' busiest week of the year, giving values in \[0.0, 1.0\]. That is
//! a rescaling of the raw count, and it does **not** correct for recording
//! effort: a week the station was off for reads as a week the species was
//! scarce. (This module's documentation used to say it did.)
//!
//! ## Detection rate (detections per listened hour)
//!
//! [`effort_corrected_abundance_sql`] divides detections by the hours the
//! station recorded as listening (`recording_effort`), per species per ISO
//! week. It reports a zero for every listened week a reported species was not
//! detected in, and an exact 95 % Poisson interval for each rate; its own
//! documentation says what the denominator is and what the interval does and
//! does not cover.

use crate::phenology::types::AbundanceParams;

/// Calendar year of a detection, as an INTEGER.
///
/// Note the argument order: `DuckDB` is `strftime(value, format)`, the reverse
/// of `SQLite`'s `strftime(format, value)`. These builders carried the SQLite
/// order against the engine this crate actually talks to, so every query using
/// it failed to bind — invisible to tests that only asserted on generated text.
const YEAR_EXPR: &str = "CAST(strftime(detection_date, '%Y') AS INTEGER)";

/// ISO-8601 week of year (1–53), as an INTEGER.
///
/// # Why `%V` and not `%W`
///
/// The alias on every query below has said `iso_week` since this module was
/// written, and the expression behind it was `%W` — the Monday-based week
/// number, which is **not** the ISO week. Probed against the real bundled
/// engines rather than assumed (SQLite 3.53.2 and DuckDB agree exactly):
///
/// ```text
///                %W    %V (ISO)   %Y     %G (ISO year)
/// 2024-12-30     53       01      2024      2025
/// 2024-12-31     53       01      2024      2025
/// 2025-01-01     00       01      2025      2025
/// 2025-01-05     00       01      2025      2025
/// 2025-01-06     01       02      2025      2025
/// ```
///
/// Two consequences for a chart whose entire purpose is comparing week-of-year
/// *across years*:
///
/// * `%W` has a **week 00** that is one to six days long, and often a **week
///   53** that is another stub. Both were drawn at full height beside complete
///   seven-day weeks, so the first and last bars of every year meant something
///   different from all the others.
/// * The same real week was **split across two years' charts**: 30–31 December
///   filed under `%Y=2024, %W=53`, and 1–5 January under `%Y=2025, %W=00`.
///
/// Every ISO week is exactly seven days, and week *N* sits within three days of
/// the same seasonal position each year, which is the property a phenology
/// comparison needs.
const WEEK_EXPR: &str = "CAST(strftime(detection_date, '%V') AS INTEGER)";

/// ISO-8601 week-numbering year, as an INTEGER.
///
/// Must travel with [`WEEK_EXPR`]: an ISO week belongs to the ISO year, and
/// filtering ISO weeks by the *calendar* year is what produces the split
/// described above from the other direction — 2024-12-30 would arrive as
/// "2024, week 01" and collide with 2025-01-01's own week 01.
///
/// Deliberately **not** used by [`monthly_totals_sql`], which buckets by
/// calendar month and therefore wants the calendar year: a December row belongs
/// to December, whatever ISO says about its week.
const ISO_YEAR_EXPR: &str = "CAST(strftime(detection_date, '%G') AS INTEGER)";

/// Rows that name a real point in time. Every query here buckets by week or
/// month, so a row the view could not parse has no bucket to belong to.
const PLACEABLE: &str = "detection_date IS NOT NULL";

/// Assemble the present conditions into a `WHERE` clause.
///
/// Joined from a list rather than each condition carrying its own `WHERE `/
/// `AND ` prefix — the prefix approach is what left
/// `effort_corrected_abundance_sql` emitting `AND d Com_Name IS NOT NULL`
/// straight after `FROM detections d`, a parser error in a shipped public API.
fn where_clause(conditions: &[Option<String>]) -> String {
    let present: Vec<&str> = conditions
        .iter()
        .filter_map(|c| c.as_deref())
        .filter(|c| !c.is_empty())
        .collect();
    if present.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", present.join(" AND "))
    }
}

/// Species equality condition on `column`, single-quote escaped.
fn species_condition(species: &str, column: &str) -> String {
    format!("{column} = '{}'", species.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// Weekly abundance
// ---------------------------------------------------------------------------

/// Build SQL for weekly relative abundance per species.
///
/// Returns one row per (species, year, `iso_week`) with:
/// - `detection_count` — raw count for the week
/// - `relative_abundance` — count divided by the peak week count
pub fn weekly_abundance_sql(params: &AbundanceParams) -> String {
    let where_sql = where_clause(&[
        Some(PLACEABLE.to_string()),
        params
            .species
            .as_deref()
            .map(|s| species_condition(s, "Com_Name")),
        Some(format!("{ISO_YEAR_EXPR} = {}", params.year)),
    ]);
    let min_clause = if params.min_weekly_count > 1 {
        format!("HAVING COUNT(*) >= {}", params.min_weekly_count)
    } else {
        String::new()
    };

    format!(
        "WITH weekly_counts AS (
            SELECT
                Com_Name                                        AS species,
                {ISO_YEAR_EXPR}                                     AS year,
                {WEEK_EXPR}                                     AS iso_week,
                COUNT(*)                                        AS detection_count
            FROM detections_ts
            {where_sql}
            GROUP BY species, year, iso_week
            {min_clause}
        ),
        weekly_peaks AS (
            SELECT
                species,
                year,
                MAX(detection_count) AS peak_count
            FROM weekly_counts
            GROUP BY species, year
        )
        SELECT
            wc.species,
            wc.year,
            wc.iso_week,
            wc.detection_count,
            ROUND(
                CAST(wc.detection_count AS REAL) / NULLIF(wp.peak_count, 0),
                4
            ) AS relative_abundance
        FROM weekly_counts wc
        JOIN weekly_peaks wp
            ON wc.species = wp.species AND wc.year = wp.year
        ORDER BY wc.species, wc.iso_week"
    )
}

/// Build SQL for a species' peak activity weeks within a year.
///
/// Returns the top `top_n` weeks by detection count, along with
/// the cumulative share of total detections.  Useful for identifying
/// the core breeding/migration window.
pub fn peak_weeks_sql(params: &AbundanceParams, top_n: u32) -> String {
    let where_sql = where_clause(&[
        Some(PLACEABLE.to_string()),
        params
            .species
            .as_deref()
            .map(|s| species_condition(s, "Com_Name")),
        Some(format!("{ISO_YEAR_EXPR} = {}", params.year)),
    ]);

    format!(
        "WITH weekly AS (
            SELECT
                Com_Name                                        AS species,
                {WEEK_EXPR}                                     AS iso_week,
                COUNT(*)                                        AS detection_count
            FROM detections_ts
            {where_sql}
            GROUP BY species, iso_week
        ),
        totals AS (
            SELECT species, SUM(detection_count) AS total_count
            FROM weekly
            GROUP BY species
        )
        SELECT
            w.species,
            w.iso_week,
            w.detection_count,
            ROUND(
                100.0 * CAST(w.detection_count AS REAL) / NULLIF(t.total_count, 0),
                1
            ) AS pct_of_annual_total
        FROM weekly w
        JOIN totals t ON w.species = t.species
        ORDER BY w.species, w.detection_count DESC
        LIMIT {top_n}"
    )
}

/// Build SQL for monthly detection totals across all years.
///
/// Returns one row per (species, year, month) ordered chronologically.
/// Useful for phenological bar charts and seasonal summaries.
pub fn monthly_totals_sql(params: &AbundanceParams) -> String {
    let where_sql = where_clause(&[
        Some(PLACEABLE.to_string()),
        params
            .species
            .as_deref()
            .map(|s| species_condition(s, "Com_Name")),
        Some(format!("{YEAR_EXPR} = {}", params.year)),
    ]);

    format!(
        "SELECT
            Com_Name                                        AS species,
            {YEAR_EXPR}                                     AS year,
            CAST(strftime(detection_date, '%m') AS INTEGER) AS month,
            COUNT(*)                                        AS detection_count,
            AVG(Confidence)                                 AS mean_confidence
        FROM detections_ts
        {where_sql}
        GROUP BY species, year, month
        ORDER BY species, month"
    )
}

/// Build SQL for species richness (distinct species) per week.
///
/// Returns ISO week number and species count for the given year.
/// High-richness weeks typically correspond to migration peaks.
pub fn weekly_richness_sql(year: u32) -> String {
    format!(
        "SELECT
            {WEEK_EXPR}                             AS iso_week,
            COUNT(DISTINCT Com_Name)                AS species_count,
            COUNT(*)                                AS total_detections
        FROM detections_ts
        WHERE {PLACEABLE} AND {ISO_YEAR_EXPR} = {year}
        GROUP BY iso_week
        ORDER BY iso_week"
    )
}

/// Build SQL for the weekly detection rate: detections per listened hour
/// (`DuckDB` only).
///
/// The name says "abundance" for compatibility; the number is a **detection
/// rate**, not an abundance of birds (see the module documentation).
///
/// **Requires:** A `recording_effort` table with columns `date` (TEXT
/// `YYYY-MM-DD`, local civil date), `source` and `seconds` (REAL).
///
/// Columns, in this order: `species`, `iso_week`, `raw_count` (every detection
/// that week), `effort_count` (those on days with recorded effort — the
/// numerator), `effort_hours`, `detections_per_hour`,
/// `detections_per_hour_lower95`, `detections_per_hour_upper95`.
///
/// # What is divided by what
///
/// The rate divides the detections made *on days with recorded effort* by the
/// station time recorded on those days. It divided every detection of the ISO
/// week by whatever effort the week had, so a week whose sampler ran on two
/// days of seven had seven days of detections over two days of listening —
/// and every week before the sampler existed, or with any day it was not
/// running, was inflated the same way.
///
/// A day's station time is its **longest** single source, not the sum across
/// sources. Detections are not per source, so two microphones listening to
/// the same garden for an hour are an hour of listening for the station's one
/// list of detections; summed, they were two, and a two-microphone station's
/// rate read half a one-microphone station's for the same birds. The longest
/// source is a lower bound on the day's coverage — sources that ran at
/// *different* times of one day are under-counted — and the per-day record
/// cannot say more than that. The interval below divides by the same hours,
/// so it inherits that denominator as it is.
///
/// # Which rows
///
/// Every species-week with at least `min_weekly_count` detections, as before,
/// **and**, for every species that has at least one such week, a row for each
/// ISO week of the year with recorded effort in which that species was not
/// detected at all: `raw_count` and `effort_count` 0, the week's
/// `effort_hours`, and a rate of 0. Without those rows a listened week with no
/// detection was simply absent, which a chart or a model reads as "no data"
/// rather than as "listened, and heard nothing".
///
/// A week that *had* detections but fewer than `min_weekly_count` is left out
/// as before and is not turned into a zero: it was not a zero. A week with no
/// recorded effort gets no zero row — nothing was listened to — and a week
/// with detections but no effort keeps its row with a null rate. A species
/// with no week at the minimum gets no rows at all.
///
/// # The interval
///
/// `detections_per_hour_lower95` and `detections_per_hour_upper95` are the
/// exact two-sided 95 % Poisson interval for the count `effort_count` (Garwood
/// 1936: the bounds are `χ²(0.025; 2k) / 2` and `χ²(0.975; 2k + 2) / 2`, with a
/// lower bound of 0 at `k = 0`), divided by `effort_hours`, to four decimals.
/// For `k = 0` the upper bound is `ln 40 ≈ 3.689` detections, so a listened
/// week with no detection reads "0, and at most about 3.7 per however many
/// hours were listened": an absence over two hours says much less than an
/// absence over sixty, and the interval is what shows that.
///
/// The bounds are exact (computed in Rust by [`garwood_interval`] and inlined
/// as a lookup table) for `k ≤ 100`; above that, Byar's approximation is used
/// in SQL, which agrees with the exact bound to better than 3 × 10⁻⁵
/// relative there — below the four-decimal rounding of every rate this
/// station would report.
///
/// What the interval assumes, and so what it does **not** mean:
///
/// * It treats the week's detections as a Poisson process: independent events
///   at a constant rate across the listened hours. They are neither. A bird
///   that sings once usually sings again, analysis windows can overlap, and
///   activity follows the dawn chorus, so detections are clustered
///   (overdispersed) and the real uncertainty is **wider** than this interval.
///   It is a floor on the uncertainty, not a confidence statement about the
///   species.
/// * It covers counting noise only. Misidentifications, the confidence
///   threshold, how detectable the species is, and error in the recorded
///   effort are all outside it.
/// * It is an interval on detections per listened hour at this station, not
///   on the number of birds.
pub fn effort_corrected_abundance_sql(params: &AbundanceParams) -> String {
    // `recording_effort.date` is TEXT (local civil date, matching the
    // detections it will be divided into), so it is cast here rather than read
    // from the view.
    //
    // This used to read a table called `recordings` that existed only in this
    // module's own tests, which is a large part of why the whole module had no
    // production consumer. It now reads `recording_effort`, populated by the
    // station's own sampler (migration 27, `integrations::effort`).
    //
    // ISO week and ISO year, matching `WEEK_EXPR` / `ISO_YEAR_EXPR`: both sides
    // are keyed by the *date*, and the week is derived from it once, so the two
    // cannot disagree on what a week is.
    let where_sql = where_clause(&[
        Some("d.detection_date IS NOT NULL".to_string()),
        params
            .species
            .as_deref()
            .map(|s| species_condition(s, "d.Com_Name")),
        // ISO year, to match the ISO week this query buckets by. Spelled out
        // rather than using `ISO_YEAR_EXPR` because this query aliases the
        // detections table as `d` — which is also how the `%W` this replaced
        // escaped the earlier switch to `%V`, being a literal rather than the
        // shared constant.
        Some(format!(
            "CAST(strftime(d.detection_date, '%G') AS INTEGER) = {}",
            params.year
        )),
    ]);
    // Applied to the counted weeks rather than as a `HAVING` on the count, so
    // that the zero rows below can tell a week under the minimum (detected, so
    // not a zero) from a week with no detection at all.
    let min_where = if params.min_weekly_count > 1 {
        format!("WHERE raw_count >= {}", params.min_weekly_count)
    } else {
        String::new()
    };
    let table = garwood_table_sql();

    format!(
        "WITH effort_days AS (
            SELECT
                TRY_CAST(date AS DATE)                  AS day,
                MAX(seconds)                            AS seconds
            FROM recording_effort
            WHERE TRY_CAST(date AS DATE) IS NOT NULL
            GROUP BY day
            HAVING MAX(seconds) > 0
        ),
        effort AS (
            SELECT
                CAST(strftime(day, '%V') AS INTEGER)    AS iso_week,
                SUM(seconds) / 3600.0                   AS hours
            FROM effort_days
            WHERE CAST(strftime(day, '%G') AS INTEGER) = {year}
            GROUP BY iso_week
        ),
        counted AS (
            SELECT
                d.Com_Name                                      AS species,
                CAST(strftime(d.detection_date, '%V') AS INTEGER) AS iso_week,
                COUNT(*)                                        AS raw_count,
                COUNT(e.day)                                    AS effort_count
            FROM detections_ts d
            LEFT JOIN effort_days e ON e.day = d.detection_date
            {where_sql}
            GROUP BY species, iso_week
        ),
        weekly AS (
            SELECT species, iso_week, raw_count, effort_count
            FROM counted
            {min_where}
        ),
        absences AS (
            SELECT r.species, e.iso_week, 0 AS raw_count, 0 AS effort_count
            FROM (SELECT DISTINCT species FROM weekly) r
            CROSS JOIN effort e
            WHERE NOT EXISTS (
                SELECT 1 FROM counted c
                WHERE c.species = r.species AND c.iso_week = e.iso_week
            )
        ),
        reported AS (
            SELECT * FROM weekly
            UNION ALL
            SELECT * FROM absences
        ),
        garwood(k, lo, hi) AS (
            VALUES {table}
        ),
        bounded AS (
            SELECT
                w.*,
                COALESCE(
                    g.lo,
                    w.effort_count * POW(
                        1 - 1 / (9.0 * w.effort_count)
                          - {z} / (3 * SQRT(w.effort_count)),
                        3)
                )                                               AS count_lo,
                COALESCE(
                    g.hi,
                    (w.effort_count + 1) * POW(
                        1 - 1 / (9.0 * (w.effort_count + 1))
                          + {z} / (3 * SQRT(w.effort_count + 1)),
                        3)
                )                                               AS count_hi
            FROM reported w
            LEFT JOIN garwood g ON g.k = w.effort_count
        )
        SELECT
            w.species,
            w.iso_week,
            w.raw_count,
            w.effort_count,
            e.hours                                             AS effort_hours,
            ROUND(
                CAST(w.effort_count AS REAL) / NULLIF(e.hours, 0),
                4
            )                                                   AS detections_per_hour,
            ROUND(w.count_lo / NULLIF(e.hours, 0), 4)           AS detections_per_hour_lower95,
            ROUND(w.count_hi / NULLIF(e.hours, 0), 4)           AS detections_per_hour_upper95
        FROM bounded w
        LEFT JOIN effort e ON w.iso_week = e.iso_week
        ORDER BY w.species, w.iso_week",
        year = params.year,
        z = Z_975,
    )
}

/// The 0.975 quantile of the standard normal, for Byar's approximation.
const Z_975: f64 = 1.959_963_984_540_054;

/// Counts up to this are given their exact Garwood bounds in a lookup table;
/// larger counts use Byar's approximation in SQL.
const GARWOOD_EXACT_MAX: u32 = 100;

/// `garwood(k, lo, hi)` rows for `k` in `0..=GARWOOD_EXACT_MAX`.
fn garwood_table_sql() -> String {
    (0..=GARWOOD_EXACT_MAX)
        .map(|k| {
            let (lo, hi) = garwood_interval(k);
            format!("({k}, {lo:.10}::DOUBLE, {hi:.10}::DOUBLE)")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The exact (Garwood) two-sided 95 % confidence interval for the mean of a
/// Poisson distribution, given one observed count `k`.
///
/// Returns `(lower, upper)` in events — divide by the exposure for a rate.
/// The lower bound is the mean at which `P(X ≥ k) = 0.025` (0 when `k = 0`);
/// the upper bound is the mean at which `P(X ≤ k) = 0.025`. These are the
/// `χ²(0.025; 2k) / 2` and `χ²(0.975; 2k + 2) / 2` of the usual statement,
/// found here by bisection on the Poisson distribution function so that no
/// special-function library is needed.
///
/// The interval's assumptions (independent events at a constant rate) are
/// spelled out on [`effort_corrected_abundance_sql`]; detections do not meet
/// them, and the interval is narrower than the real uncertainty.
#[must_use]
pub fn garwood_interval(k: u32) -> (f64, f64) {
    const TAIL: f64 = 0.025;
    let span = f64::from(k) + 20.0 * f64::from(k + 1).sqrt() + 20.0;
    // P(X <= k) falls as the mean rises: find where it crosses `TAIL`.
    let upper = bisect(|mean| poisson_cdf(k, mean) > TAIL, 0.0, span);
    // P(X >= k) = 1 - P(X <= k - 1) rises with the mean.
    let lower = if k == 0 {
        0.0
    } else {
        bisect(|mean| poisson_cdf(k - 1, mean) > 1.0 - TAIL, 0.0, span)
    };
    (lower, upper)
}

/// `P(X ≤ k)` for `X ~ Poisson(mean)`, summed in log space.
fn poisson_cdf(k: u32, mean: f64) -> f64 {
    if mean <= 0.0 {
        return 1.0;
    }
    let ln_mean = mean.ln();
    let mut ln_term = -mean; // ln P(X = 0)
    let mut sum = ln_term.exp();
    for i in 1..=k {
        ln_term += ln_mean - f64::from(i).ln();
        sum += ln_term.exp();
    }
    sum.min(1.0)
}

/// The point in `[lo, hi]` where `below` turns from true to false.
fn bisect(below: impl Fn(f64) -> bool, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..200 {
        let mid = f64::midpoint(lo, hi);
        if below(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f64::midpoint(lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekly_abundance_sql_structure() {
        let params = AbundanceParams::for_year(2026);
        let sql = weekly_abundance_sql(&params);
        assert!(sql.contains("relative_abundance"));
        assert!(sql.contains("weekly_counts"));
        assert!(sql.contains("weekly_peaks"));
        assert!(sql.contains("2026"));
    }

    #[test]
    fn weekly_abundance_species_filter() {
        let params = AbundanceParams {
            species: Some("Common Swift".into()),
            year: 2026,
            min_weekly_count: 1,
        };
        let sql = weekly_abundance_sql(&params);
        assert!(sql.contains("Common Swift"));
    }

    #[test]
    fn weekly_abundance_min_count_having() {
        let params = AbundanceParams {
            species: None,
            year: 2026,
            min_weekly_count: 5,
        };
        let sql = weekly_abundance_sql(&params);
        assert!(sql.contains("HAVING COUNT(*) >= 5"));
    }

    #[test]
    fn peak_weeks_sql_top_n() {
        let params = AbundanceParams::for_year(2026);
        let sql = peak_weeks_sql(&params, 10);
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("pct_of_annual_total"));
    }

    #[test]
    fn monthly_totals_sql_structure() {
        let params = AbundanceParams::for_year(2026);
        let sql = monthly_totals_sql(&params);
        assert!(sql.contains("month"));
        assert!(sql.contains("mean_confidence"));
        assert!(sql.contains("2026"));
    }

    #[test]
    fn weekly_richness_sql_structure() {
        let sql = weekly_richness_sql(2026);
        assert!(sql.contains("species_count"));
        assert!(sql.contains("COUNT(DISTINCT Com_Name)"));
    }

    #[test]
    fn effort_corrected_abundance_sql_structure() {
        let params = AbundanceParams::for_year(2026);
        let sql = effort_corrected_abundance_sql(&params);
        assert!(sql.contains("detections_per_hour"));
        assert!(sql.contains("effort_hours"));
        assert!(
            sql.contains("FROM recording_effort"),
            "the effort join must read the station's own effort table. It read \
             `recordings`, which existed only in this module's tests — a large \
             part of why the module had no production consumer at all."
        );
        assert!(
            !sql.contains("FROM recordings"),
            "the fictional table must not come back"
        );
    }

    /// Published exact values (Garwood 1936; e.g. the tables in Ulm 1990).
    #[test]
    fn garwood_interval_matches_the_exact_bounds() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-5;
        let (lo, hi) = garwood_interval(0);
        assert!(lo == 0.0 && close(hi, 40f64.ln()), "k=0: {lo} {hi}");
        let (lo, hi) = garwood_interval(1);
        assert!(
            close(lo, 0.025_318) && close(hi, 5.571_643),
            "k=1: {lo} {hi}"
        );
        let (lo, hi) = garwood_interval(3);
        assert!(
            close(lo, 0.618_672) && close(hi, 8.767_273),
            "k=3: {lo} {hi}"
        );
        let (lo, hi) = garwood_interval(10);
        assert!(
            close(lo, 4.795_389) && close(hi, 18.390_356),
            "k=10: {lo} {hi}"
        );
    }

    /// Byar's approximation, used in SQL above the exact table, agrees with the
    /// exact bound to the precision the documentation claims wherever it is
    /// used — and does not at k = 1, which is why there is a table.
    #[test]
    fn byar_takes_over_only_where_it_is_accurate() {
        let byar = |k: f64| {
            let lo = k * (1.0 - 1.0 / (9.0 * k) - Z_975 / (3.0 * k.sqrt())).powi(3);
            let k1 = k + 1.0;
            let hi = k1 * (1.0 - 1.0 / (9.0 * k1) + Z_975 / (3.0 * k1.sqrt())).powi(3);
            (lo, hi)
        };
        for k in (GARWOOD_EXACT_MAX + 1)..=2000 {
            let (elo, ehi) = garwood_interval(k);
            let (blo, bhi) = byar(f64::from(k));
            assert!(
                ((blo - elo) / elo).abs() < 3e-5 && ((bhi - ehi) / ehi).abs() < 3e-5,
                "k={k}: exact ({elo}, {ehi}), Byar ({blo}, {bhi})"
            );
        }
        let (elo, _) = garwood_interval(1);
        let (blo, _) = byar(1.0);
        assert!(((blo - elo) / elo).abs() > 0.1, "{elo} {blo}");
    }

    #[test]
    fn species_clause_escapes_apostrophe() {
        let params = AbundanceParams {
            species: Some("O'Grady's Sparrow".into()),
            year: 2026,
            min_weekly_count: 1,
        };
        let sql = weekly_abundance_sql(&params);
        assert!(sql.contains("O''Grady''s Sparrow"));
    }
}
