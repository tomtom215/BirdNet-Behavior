//! SQL query builders for phenological timing analysis.
//!
//! Generates `DuckDB` SQL that extracts first and last detection dates per
//! species per year, computes day-of-year values, and derives multi-year
//! migration windows.
//!
//! Queries read the `detections_ts` view rather than the `detections` table, so
//! `detection_date` arrives already typed as a `DATE`. `Date` itself is
//! free-form `TEXT NOT NULL`, and a station's history can hold rows that name
//! no point in time; the view's `TRY_CAST` turns those into NULL, and every
//! query here excludes them explicitly rather than grouping them under a NULL
//! year.

use crate::phenology::types::PhenologyParams;

/// Calendar year of a detection, as an INTEGER.
///
/// Note the argument order: `DuckDB` is `strftime(value, format)`, the reverse
/// of `SQLite`'s `strftime(format, value)`. These builders carried the SQLite
/// order against the engine this crate actually talks to, so every query using
/// it failed to bind with "Could not choose a best candidate function for the
/// function call `strftime(STRING_LITERAL, VARCHAR)`" — invisible to tests that
/// only asserted on the generated text.
const YEAR_EXPR: &str = "CAST(strftime(detection_date, '%Y') AS INTEGER)";

/// Rows that name a real point in time. Everything here buckets by date, so a
/// row the view could not parse has no bucket to belong to.
const PLACEABLE: &str = "detection_date IS NOT NULL";

// ---------------------------------------------------------------------------
// Phenology timing SQL builders
// ---------------------------------------------------------------------------

/// Build SQL for per-species, per-year phenological timing records.
///
/// Returns one row per (species, year) combination containing:
/// - `first_detection`, `last_detection` (ISO 8601 dates)
/// - `first_doy`, `last_doy` (day-of-year 1–366)
/// - `presence_days` (approximate number of days between first and last)
/// - `detection_count`
pub fn phenology_timing_sql(params: &PhenologyParams) -> String {
    let where_sql = where_clause(&[
        Some(PLACEABLE.to_string()),
        params.species.as_deref().map(species_condition),
        year_conditions(params.year_start, params.year_end),
    ]);
    let having_clause = format!("HAVING COUNT(*) >= {}", params.min_detections);

    format!(
        "SELECT
            Com_Name                                        AS species,
            {YEAR_EXPR}                                     AS year,
            CAST(MIN(detection_date) AS VARCHAR)            AS first_detection,
            CAST(MAX(detection_date) AS VARCHAR)            AS last_detection,
            COUNT(*)                                        AS detection_count,
            CAST(strftime(MIN(detection_date), '%j') AS INTEGER) AS first_doy,
            CAST(strftime(MAX(detection_date), '%j') AS INTEGER) AS last_doy,
            date_diff('day', MIN(detection_date), MAX(detection_date)) + 1
                                                            AS presence_days,
            COUNT(DISTINCT detection_date)                  AS detected_days
        FROM detections_ts
        {where_sql}
        GROUP BY Com_Name, year
        {having_clause}
        ORDER BY year DESC, Com_Name
        LIMIT {limit}",
        limit = params.limit,
    )
}

/// Build SQL to compute multi-year migration windows per species.
///
/// Uses the 10th, 50th (median), and 90th percentiles of `first_doy`
/// and `last_doy` across multiple years to produce a robust seasonal
/// window estimate insensitive to outlier years.
///
/// Requires at least `min_years` years of observations per species.
/// Uses `DuckDB` `percentile_cont` window functions.
///
/// # Species this cannot describe, and why it now says so
///
/// The window is computed per **calendar year**, so it is only meaningful for a
/// species that is absent across the New Year. For a resident or an
/// overwintering visitor — a Fieldfare or a Brambling present October to March —
/// `first_doy` lands in early January and `last_doy` in late December, and the
/// query reports "arrived 1 January, departed 31 December". That is arithmetically
/// correct and ornithologically meaningless, and nothing said so.
///
/// Rather than silently emitting it, the result now carries `year_crossing`:
/// true when the species was detected in both the first and last fortnight of
/// the year, which is exactly the condition under which the calendar-year
/// window stops describing a migration. A caller can then present the window,
/// suppress it, or label it — but cannot mistake it for an arrival date.
///
/// `presence_days` is likewise a **span**, not an occupancy: `last - first + 1`
/// gives 365 to a species detected once in January and once in December.
/// `detected_days` is reported alongside it so the two cannot be confused;
/// `min_detections` narrows but does not close that gap.
pub fn migration_window_sql(min_years: u32, params: &PhenologyParams) -> String {
    let species_filter = where_clause(&[
        Some(PLACEABLE.to_string()),
        params.species.as_deref().map(species_condition),
    ]);

    format!(
        "WITH yearly AS (
            SELECT
                Com_Name                                        AS species,
                {YEAR_EXPR}                                     AS year,
                CAST(strftime(MIN(detection_date), '%j') AS INTEGER) AS first_doy,
                CAST(strftime(MAX(detection_date), '%j') AS INTEGER) AS last_doy,
                -- Detected in both the first and last fortnight of the year:
                -- the signature of a resident or overwintering species, for
                -- which a calendar-year window is not a migration window.
                MAX(CASE WHEN CAST(strftime(detection_date, '%j') AS INTEGER) <= 14
                         THEN 1 ELSE 0 END)
                  * MAX(CASE WHEN CAST(strftime(detection_date, '%j') AS INTEGER) >= 351
                             THEN 1 ELSE 0 END)                 AS spans_new_year
            FROM detections_ts
            {species_filter}
            GROUP BY Com_Name, year
            HAVING COUNT(*) >= {min_det}
        )
        SELECT
            species,
            COUNT(year)                                     AS years_observed,
            percentile_cont(0.10) WITHIN GROUP (ORDER BY first_doy)
                                                            AS arrival_early_doy,
            percentile_cont(0.50) WITHIN GROUP (ORDER BY first_doy)
                                                            AS arrival_median_doy,
            percentile_cont(0.90) WITHIN GROUP (ORDER BY first_doy)
                                                            AS arrival_late_doy,
            percentile_cont(0.10) WITHIN GROUP (ORDER BY last_doy)
                                                            AS departure_early_doy,
            percentile_cont(0.50) WITHIN GROUP (ORDER BY last_doy)
                                                            AS departure_median_doy,
            percentile_cont(0.90) WITHIN GROUP (ORDER BY last_doy)
                                                            AS departure_late_doy,
            MAX(spans_new_year) = 1                         AS year_crossing
        FROM yearly
        GROUP BY species
        HAVING COUNT(year) >= {min_years}
        ORDER BY arrival_median_doy
        LIMIT {limit}",
        min_det = params.min_detections,
        min_years = min_years,
        limit = params.limit,
    )
}

/// Build SQL to find the first ever detection date per species.
///
/// Useful for "life list" or "year first" summaries.
pub fn first_detection_sql(params: &PhenologyParams) -> String {
    let species_filter = where_clause(&[
        Some(PLACEABLE.to_string()),
        params.species.as_deref().map(species_condition),
    ]);
    format!(
        "SELECT
            Com_Name                                AS species,
            CAST(MIN(detection_date) AS VARCHAR)    AS first_ever_date,
            COUNT(*)                                AS total_detections
        FROM detections_ts
        {species_filter}
        GROUP BY Com_Name
        ORDER BY first_ever_date
        LIMIT {limit}",
        limit = params.limit,
    )
}

/// Build SQL for inter-annual presence comparison.
///
/// Returns detection counts per species per year, plus year-over-year
/// change percentage (`yoy_change_pct`).  Useful for trend analysis.
///
/// Uses the `LAG` window function.
pub fn interannual_trend_sql(params: &PhenologyParams) -> String {
    let species_filter = where_clause(&[
        Some(PLACEABLE.to_string()),
        params.species.as_deref().map(species_condition),
    ]);
    format!(
        "WITH yearly AS (
            SELECT
                Com_Name                                AS species,
                {YEAR_EXPR}                             AS year,
                COUNT(*)                                AS detection_count
            FROM detections_ts
            {species_filter}
            GROUP BY Com_Name, year
            HAVING COUNT(*) >= {min_det}
        )
        SELECT
            species,
            year,
            detection_count,
            LAG(detection_count) OVER (PARTITION BY species ORDER BY year)
                AS prev_year_count,
            CASE
                WHEN LAG(detection_count) OVER (PARTITION BY species ORDER BY year) IS NULL
                    THEN NULL
                ELSE ROUND(
                    100.0 * (detection_count - LAG(detection_count)
                        OVER (PARTITION BY species ORDER BY year))
                    / NULLIF(LAG(detection_count)
                        OVER (PARTITION BY species ORDER BY year), 0),
                    1)
            END AS yoy_change_pct
        FROM yearly
        ORDER BY species, year
        LIMIT {limit}",
        min_det = params.min_detections,
        limit = params.limit,
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Assemble the present conditions into a `WHERE` clause.
///
/// Conditions are collected as `Option`s and joined here rather than each being
/// rendered with its own `"WHERE "` or `"AND "` prefix. The prefix approach is
/// what produced
///
/// ```text
/// Parser Error: syntax error at or near "AND"
/// ```
///
/// from `phenology_timing_sql`: with no species filter the species clause was
/// the empty string, so a year range that rendered itself as `AND …` followed
/// `FROM detections` directly. Deciding the keyword from how many conditions
/// there actually are makes that unrepresentable.
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

/// Species equality condition, single-quote escaped.
fn species_condition(species: &str) -> String {
    format!("Com_Name = '{}'", species.replace('\'', "''"))
}

/// Construct the year range condition (without `WHERE`/`AND` prefix).
fn year_conditions(start: Option<u32>, end: Option<u32>) -> Option<String> {
    match (start, end) {
        (Some(s), Some(e)) => Some(format!("{YEAR_EXPR} BETWEEN {s} AND {e}")),
        (Some(s), None) => Some(format!("{YEAR_EXPR} >= {s}")),
        (None, Some(e)) => Some(format!("{YEAR_EXPR} <= {e}")),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phenology_timing_sql_no_filter() {
        let params = PhenologyParams::default();
        let sql = phenology_timing_sql(&params);
        assert!(sql.contains("first_detection"));
        assert!(sql.contains("last_detection"));
        assert!(sql.contains("presence_days"));
        assert!(sql.contains("GROUP BY Com_Name, year"));
        assert!(sql.contains("HAVING COUNT(*) >= 3"));
    }

    #[test]
    fn phenology_timing_sql_species_filter() {
        let params = PhenologyParams {
            species: Some("Eurasian Blackbird".into()),
            ..PhenologyParams::default()
        };
        let sql = phenology_timing_sql(&params);
        assert!(sql.contains("Com_Name = 'Eurasian Blackbird'"));
        // The species condition is joined with AND onto the placeable-rows
        // condition rather than each carrying its own keyword, so exactly one
        // WHERE is emitted however many filters are set.
        assert_eq!(sql.matches("WHERE").count(), 1, "{sql}");
    }

    #[test]
    fn phenology_timing_sql_year_range() {
        let params = PhenologyParams {
            year_start: Some(2024),
            year_end: Some(2026),
            ..PhenologyParams::default()
        };
        let sql = phenology_timing_sql(&params);
        assert!(sql.contains("BETWEEN 2024 AND 2026"));
    }

    #[test]
    fn phenology_timing_sql_year_start_only() {
        let params = PhenologyParams {
            year_start: Some(2025),
            ..PhenologyParams::default()
        };
        let sql = phenology_timing_sql(&params);
        assert!(sql.contains(">= 2025"));
    }

    #[test]
    fn migration_window_sql_contains_percentile() {
        let params = PhenologyParams::default();
        let sql = migration_window_sql(3, &params);
        assert!(sql.contains("percentile_cont"));
        assert!(sql.contains("arrival_median_doy"));
        assert!(sql.contains("departure_median_doy"));
        assert!(sql.contains("HAVING COUNT(year) >= 3"));
    }

    #[test]
    fn first_detection_sql_structure() {
        let params = PhenologyParams::default();
        let sql = first_detection_sql(&params);
        assert!(sql.contains("first_ever_date"));
        assert!(sql.contains("MIN(detection_date)"));
    }

    #[test]
    fn interannual_trend_sql_contains_lag() {
        let params = PhenologyParams::default();
        let sql = interannual_trend_sql(&params);
        assert!(sql.contains("LAG(detection_count)"));
        assert!(sql.contains("yoy_change_pct"));
    }

    #[test]
    fn species_filter_escapes_single_quotes() {
        let params = PhenologyParams {
            species: Some("O'Brien's Warbler".into()),
            ..PhenologyParams::default()
        };
        let sql = phenology_timing_sql(&params);
        assert!(
            sql.contains("O''Brien''s Warbler"),
            "should escape single quotes"
        );
    }
}
