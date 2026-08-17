//! SQL query builders for species abundance analytics.
//!
//! Generates SQL for weekly, monthly, and seasonal abundance indices.
//! All queries normalise detection counts to produce relative abundance
//! values in \[0.0, 1.0\] where 1.0 represents the peak detection week.
//!
//! ## Relative abundance
//!
//! Raw detection counts are influenced by recording effort (number of
//! recording hours per week).  The relative abundance index corrects for
//! this by dividing each week's count by the maximum weekly count for that
//! species in that year.
//!
//! For effort-corrected abundance (detections per recording hour), use
//! [`effort_corrected_abundance_sql`] which requires an `effort_hours`
//! column in the data.

use crate::phenology::types::AbundanceParams;

/// Calendar year of a detection, as an INTEGER.
///
/// Note the argument order: `DuckDB` is `strftime(value, format)`, the reverse
/// of `SQLite`'s `strftime(format, value)`. These builders carried the SQLite
/// order against the engine this crate actually talks to, so every query using
/// it failed to bind — invisible to tests that only asserted on generated text.
const YEAR_EXPR: &str = "CAST(strftime(detection_date, '%Y') AS INTEGER)";

/// Week of year (Monday-based), as an INTEGER — `%W`, as before.
const WEEK_EXPR: &str = "CAST(strftime(detection_date, '%W') AS INTEGER)";

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
        Some(format!("{YEAR_EXPR} = {}", params.year)),
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
                {YEAR_EXPR}                                     AS year,
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
        Some(format!("{YEAR_EXPR} = {}", params.year)),
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
        WHERE {PLACEABLE} AND {YEAR_EXPR} = {year}
        GROUP BY iso_week
        ORDER BY iso_week"
    )
}

/// Build SQL for effort-corrected abundance (`DuckDB` only).
///
/// When recording effort data is available (e.g., from a separate
/// `recording_effort` table with `date` and `seconds` columns), this
/// query normalises detection counts per recording hour to remove
/// effort bias.
///
/// **Requires:** A `recording_effort` table with columns `date` (TEXT
/// `YYYY-MM-DD`, local civil date) and `seconds` (REAL).
pub fn effort_corrected_abundance_sql(params: &AbundanceParams) -> String {
    // `recording_effort.date` is TEXT (local civil date, matching the
    // detections it will be divided into), so it is cast here rather than read
    // from the view.
    //
    // This used to read a table called `recordings` that existed only in this
    // module's own tests, which is a large part of why the whole module had no
    // production consumer. It now reads `recording_effort`, populated by the
    // station's own sampler (migration 27, `integrations::effort`).
    let effort_week = "CAST(strftime(TRY_CAST(date AS DATE), '%W') AS INTEGER)";
    let effort_year = "CAST(strftime(TRY_CAST(date AS DATE), '%Y') AS INTEGER)";
    let where_sql = where_clause(&[
        Some("d.detection_date IS NOT NULL".to_string()),
        params
            .species
            .as_deref()
            .map(|s| species_condition(s, "d.Com_Name")),
        Some(format!(
            "CAST(strftime(d.detection_date, '%Y') AS INTEGER) = {}",
            params.year
        )),
    ]);

    format!(
        "WITH effort AS (
            SELECT
                {effort_week}                           AS iso_week,
                SUM(seconds) / 3600.0                   AS hours
            FROM recording_effort
            WHERE {effort_year} = {year}
            GROUP BY iso_week
        ),
        weekly AS (
            SELECT
                d.Com_Name                                      AS species,
                CAST(strftime(d.detection_date, '%W') AS INTEGER) AS iso_week,
                COUNT(*)                                        AS raw_count
            FROM detections_ts d
            {where_sql}
            GROUP BY species, iso_week
        )
        SELECT
            w.species,
            w.iso_week,
            w.raw_count,
            e.hours                                             AS effort_hours,
            ROUND(
                CAST(w.raw_count AS REAL) / NULLIF(e.hours, 0),
                4
            )                                                   AS detections_per_hour
        FROM weekly w
        LEFT JOIN effort e ON w.iso_week = e.iso_week
        ORDER BY w.species, w.iso_week",
        year = params.year,
    )
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
