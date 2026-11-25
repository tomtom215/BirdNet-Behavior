//! SQL query builders for seasonal activity: weekly and monthly detection
//! counts, and the effort-corrected detection rate.
//!
//! Nothing here measures abundance — how many birds there are. Every query
//! counts detections, and a detection count moves with a species' numbers,
//! its call rate, its loudness, the model's skill with it and the station's
//! listening, all at once.
//! The functions keep their historical `abundance` names for compatibility.
//!
//! ## Relative index (no effort correction)
//!
//! [`weekly_abundance_sql`] divides each week's detection count by the
//! species' peak weekly count that year, giving an index in \[0.0, 1.0\]. It
//! does **not** correct for recording effort: a week with half the listening
//! reads as half as active. It is the shape of a season at a station that
//! listened evenly, nothing more.
//!
//! ## Detection rate (effort-corrected)
//!
//! [`effort_corrected_abundance_sql`] divides detections by the hours the
//! station recorded listening (`recording_effort`), giving detections per
//! listened hour with a 95 % Poisson interval, and reports a listened week
//! with no detections as a rate of zero rather than leaving it out.

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

/// Build SQL for the weekly relative detection index per species.
///
/// Not effort-corrected (see the module docs): for a rate per listened hour,
/// use [`effort_corrected_abundance_sql`].
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

/// Build SQL for the effort-corrected **detection rate** (`DuckDB` only):
/// detections per listened hour, per species per ISO week, with a 95 %
/// Poisson interval.
///
/// This is a vocal-activity rate, not an abundance: one bird singing all
/// morning and ten birds calling once each can give the same rate, and a
/// species' detectability (loudness, call rate, the model's skill with it)
/// sets its level as much as its numbers do. Compare a species with itself
/// across weeks and years, not with another species. The function keeps its
/// name for compatibility.
///
/// **Requires:** A `recording_effort` table with columns `date` (TEXT
/// `YYYY-MM-DD`, local civil date), `source` and `seconds` (REAL).
///
/// Columns, in this order: `species`, `iso_week`, `raw_count` (every
/// detection that week), `effort_count` (those on days with recorded effort —
/// the numerator), `effort_hours`, `detections_per_hour`, `rate_ci_lower`,
/// `rate_ci_upper`.
///
/// # Which rows there are
///
/// A row for every species-week with a detection, and a **zero** row
/// (`raw_count` 0, `detections_per_hour` 0) for every week with recorded
/// listening in which the species was not detected. "Listened and not heard"
/// is an observation; it used to be a missing row, which a chart drew as a
/// line straight across the silence and a reader could not tell from "not
/// listened". The species filled in are the one named by `species`, whether
/// or not it was detected that year, or else every species detected in that
/// ISO year. A week with neither listening nor a detection has no row, and a
/// week with detections but no listening keeps its row with NULL hours, rate
/// and interval.
///
/// `min_weekly_count` above 1 drops every row whose `raw_count` is under it,
/// zero rows included; at 0 or 1 nothing is dropped.
///
/// # The interval
///
/// `rate_ci_lower` and `rate_ci_upper` are the exact two-sided 95 % Poisson
/// (Garwood) interval for `effort_count`, divided by `effort_hours`: for `k`
/// detections the bounds on the expected count are `χ²(0.025; 2k) / 2` and
/// `χ²(0.975; 2k + 2) / 2`. They are tabulated exactly for `k` up to
/// [`EXACT_POISSON_LIMIT`] and taken from Byar's approximation (the
/// Wilson–Hilferty cube-root form of those quantiles) above it, where its
/// error is far below the four decimals the rate is rounded to. A zero week
/// has a lower bound of 0 and an upper bound of `−ln(0.025) ≈ 3.69`
/// detections over its hours, which is what makes a short silent week
/// distinguishable from a long one.
///
/// The Poisson interval is the narrowest honest one: detections of one bird
/// are clustered (a singing bird is detected in run after run), so the real
/// variance is larger and the true interval wider. Read it as a floor on the
/// uncertainty, not as the whole of it.
///
/// # What is divided by what
///
/// The rate divides the detections made *on days with recorded effort* by the
/// listening recorded on those days, summed over sources. It divided every
/// detection of the ISO week by whatever effort the week had, so a week whose
/// sampler ran on two days of seven had seven days of detections over two
/// days of listening — and every week before the sampler existed, or with any
/// day it was not running, was inflated the same way.
///
/// A day's listening time is the **sum** of every source's seconds, so the
/// rate is detections per *source-hour*. Detections are per source: a station
/// with several streams records the same bird once for each stream that heard
/// it (`birdnet-db` migration 18: a detection's unique key includes its clip's
/// file name, which carries the stream id, and no cross-stream collapse
/// exists), so the numerator counts every source's rows and the denominator
/// must count every source's listening. Two co-located microphones that each
/// log one Robin in an hour give two detections over two source-hours, 1.0 an
/// hour, the same as one microphone logging it once. This used to take the
/// day's **longest** single source, on the belief that detections were not
/// per source, and so read 2.0 for that station: twice a one-microphone
/// station's rate for the same birds.
///
/// The rate is pooled across sources, not reported per source, and that is a
/// choice rather than a limit: the analytics copy of the detections carries
/// each row's `Source` (added in `AnalyticsDb::open`) and `recording_effort`
/// keeps its seconds per source, so a per-source rate could be formed. Whether
/// it should waits on which sources count as one site (decision 2 of
/// `docs/audit-2026-11/decisions-2026-11-25.md`). Until then, sources pointed
/// at different places give the station's average rate per microphone-hour,
/// not either site's.
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
    // The species a zero row can be written for: the one asked about, even if
    // it was never detected that year, or every species detected in it.
    let species_list = params.species.as_deref().map_or_else(
        || "SELECT DISTINCT species FROM weekly".to_string(),
        |s| format!("SELECT '{}' AS species", s.replace('\'', "''")),
    );
    let min_clause = if params.min_weekly_count > 1 {
        format!("WHERE f.raw_count >= {}", params.min_weekly_count)
    } else {
        String::new()
    };
    let z = POISSON_Z_975;

    format!(
        "WITH effort_days AS (
            SELECT
                TRY_CAST(date AS DATE)                  AS day,
                SUM(seconds)                            AS seconds
            FROM recording_effort
            WHERE TRY_CAST(date AS DATE) IS NOT NULL
            GROUP BY day
            HAVING SUM(seconds) > 0
        ),
        effort AS (
            SELECT
                CAST(strftime(day, '%V') AS INTEGER)    AS iso_week,
                SUM(seconds) / 3600.0                   AS hours
            FROM effort_days
            WHERE CAST(strftime(day, '%G') AS INTEGER) = {year}
            GROUP BY iso_week
        ),
        weekly AS (
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
        species_list AS (
            {species_list}
        ),
        -- Listened weeks with no detection of the species: rate 0, not absent.
        filled AS (
            SELECT species, iso_week, raw_count, effort_count FROM weekly
            UNION ALL
            SELECT s.species, e.iso_week, 0, 0
            FROM species_list s
            CROSS JOIN effort e
            WHERE NOT EXISTS (
                SELECT 1 FROM weekly w
                WHERE w.species = s.species AND w.iso_week = e.iso_week
            )
        ),
        -- Exact 95 % Garwood bounds on the expected count, k = 0..={limit}.
        poisson_exact (k, lo, hi) AS (
            VALUES {exact}
        ),
        bounded AS (
            SELECT
                f.species,
                f.iso_week,
                f.raw_count,
                f.effort_count,
                e.hours,
                COALESCE(
                    p.lo,
                    f.effort_count * POWER(
                        1 - 1 / (9.0 * f.effort_count)
                          - {z} / (3 * SQRT(f.effort_count)), 3)
                ) AS count_lo,
                COALESCE(
                    p.hi,
                    (f.effort_count + 1) * POWER(
                        1 - 1 / (9.0 * (f.effort_count + 1))
                          + {z} / (3 * SQRT(f.effort_count + 1)), 3)
                ) AS count_hi
            FROM filled f
            LEFT JOIN effort e ON f.iso_week = e.iso_week
            LEFT JOIN poisson_exact p ON p.k = f.effort_count
            {min_clause}
        )
        SELECT
            species,
            iso_week,
            raw_count,
            effort_count,
            hours                                               AS effort_hours,
            ROUND(
                CAST(effort_count AS REAL) / NULLIF(hours, 0),
                4
            )                                                   AS detections_per_hour,
            ROUND(count_lo / NULLIF(hours, 0), 4)               AS rate_ci_lower,
            ROUND(count_hi / NULLIF(hours, 0), 4)               AS rate_ci_upper
        FROM bounded
        ORDER BY species, iso_week",
        year = params.year,
        limit = EXACT_POISSON_LIMIT,
        exact = exact_poisson_values(),
    )
}

/// The largest count whose 95 % Poisson interval is tabulated exactly in
/// [`effort_corrected_abundance_sql`]; Byar's approximation covers the rest.
pub const EXACT_POISSON_LIMIT: u32 = 100;

/// The standard normal 97.5th percentile, for Byar's approximation.
const POISSON_Z_975: f64 = 1.959_963_984_540_054;

/// One minus the interval's coverage, split equally between the two tails.
const POISSON_TAIL: f64 = 0.025;

/// `P(X <= k)` for `X ~ Poisson(mu)`, summed term by term.
///
/// Fine in `f64` for the range used here: at `k = 100` the upper bound is
/// about 122, and `exp(-122)` is far above the smallest normal double.
fn poisson_cdf(k: u32, mu: f64) -> f64 {
    let mut term = (-mu).exp();
    let mut sum = term;
    for i in 1..=k {
        term *= mu / f64::from(i);
        sum += term;
    }
    sum
}

/// Bisect for the `mu` at which the decreasing function `f` falls to `target`.
fn bisect_decreasing(f: impl Fn(f64) -> f64, target: f64, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..200 {
        let mid = f64::midpoint(lo, hi);
        if f(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f64::midpoint(lo, hi)
}

/// The exact (Garwood) two-sided 95 % interval for a Poisson mean.
///
/// Given `k` observed events: the `mu` with `P(X >= k) = 0.025` and the `mu`
/// with `P(X <= k) = 0.025`, which are `χ²(0.025; 2k) / 2` and
/// `χ²(0.975; 2k + 2) / 2`.
#[must_use]
pub fn poisson_interval_exact(k: u32) -> (f64, f64) {
    let ceiling = 4.0f64.mul_add(f64::from(k + 1).sqrt(), f64::from(k)) + 10.0;
    let upper = bisect_decreasing(|mu| poisson_cdf(k, mu), POISSON_TAIL, 0.0, ceiling);
    let lower = if k == 0 {
        0.0
    } else {
        // P(X >= k) = 1 - P(X <= k - 1) rises with mu, so P(X <= k - 1) falls.
        bisect_decreasing(
            |mu| poisson_cdf(k - 1, mu),
            1.0 - POISSON_TAIL,
            0.0,
            f64::from(k),
        )
    };
    (lower, upper)
}

/// The `VALUES` rows of the exact interval table, computed once.
fn exact_poisson_values() -> &'static str {
    static VALUES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VALUES.get_or_init(|| {
        (0..=EXACT_POISSON_LIMIT)
            .map(|k| {
                let (lo, hi) = poisson_interval_exact(k);
                format!("({k}, {lo:.6}, {hi:.6})")
            })
            .collect::<Vec<_>>()
            .join(", ")
    })
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

    /// The exact interval against Garwood bounds (two-sided 95 %), as
    /// tabulated and as recomputed independently in log space (`lgamma`,
    /// compensated summation) rather than by this module's running product.
    #[test]
    fn poisson_interval_matches_published_values() {
        for (k, lo, hi) in [
            (0, 0.0, 3.688_879),
            (1, 0.025_318, 5.571_643),
            (5, 1.623_486, 11.668_332),
            (10, 4.795_389, 18.390_356),
            // χ²(0.025; 200) = 162.728, χ²(0.975; 202) = 243.254.
            (100, 81.363_991, 121.626_794),
        ] {
            let (l, h) = poisson_interval_exact(k);
            assert!((l - lo).abs() < 1e-4, "k={k}: lower {l} != {lo}");
            assert!((h - hi).abs() < 1e-4, "k={k}: upper {h} != {hi}");
        }
    }

    /// Above the table Byar's approximation takes over; at the seam it agrees
    /// with the exact bound to well under the rate's rounding.
    #[test]
    fn byar_continues_the_exact_table() {
        let k = f64::from(EXACT_POISSON_LIMIT + 1);
        let lo = k * (1.0 - 1.0 / (9.0 * k) - POISSON_Z_975 / (3.0 * k.sqrt())).powi(3);
        let hi = (k + 1.0)
            * (1.0 - 1.0 / (9.0 * (k + 1.0)) + POISSON_Z_975 / (3.0 * (k + 1.0).sqrt())).powi(3);
        let (l, h) = poisson_interval_exact(EXACT_POISSON_LIMIT + 1);
        assert!((lo - l).abs() / l < 1e-3, "lower: Byar {lo} vs exact {l}");
        assert!((hi - h).abs() / h < 1e-3, "upper: Byar {hi} vs exact {h}");
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
