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

// ---------------------------------------------------------------------------
// Weekly abundance
// ---------------------------------------------------------------------------

/// Build SQL for weekly relative abundance per species.
///
/// Returns one row per (species, year, `iso_week`) with:
/// - `detection_count` — raw count for the week
/// - `relative_abundance` — count divided by the peak week count
///
/// Compatible with both `SQLite` 3.x and `DuckDB` 1.x.
pub fn weekly_abundance_sql(params: &AbundanceParams) -> String {
    let species_filter = species_clause(params.species.as_deref(), "WHERE");
    let min_clause = if params.min_weekly_count > 1 {
        format!("HAVING COUNT(*) >= {}", params.min_weekly_count)
    } else {
        String::new()
    };

    format!(
        "WITH weekly_counts AS (
            SELECT
                Com_Name                                        AS species,
                CAST(strftime('%Y', Date) AS INTEGER)           AS year,
                CAST(strftime('%W', Date) AS INTEGER)           AS iso_week,
                COUNT(*)                                        AS detection_count
            FROM detections
            {species_filter}
            AND CAST(strftime('%Y', Date) AS INTEGER) = {year}
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
        ORDER BY wc.species, wc.iso_week",
        year = params.year,
    )
}

/// Build SQL for a species' peak activity weeks within a year.
///
/// Returns the top `top_n` weeks by detection count, along with
/// the cumulative share of total detections.  Useful for identifying
/// the core breeding/migration window.
pub fn peak_weeks_sql(params: &AbundanceParams, top_n: u32) -> String {
    let species_filter = species_clause(params.species.as_deref(), "WHERE");

    format!(
        "WITH weekly AS (
            SELECT
                Com_Name                                        AS species,
                CAST(strftime('%W', Date) AS INTEGER)           AS iso_week,
                COUNT(*)                                        AS detection_count
            FROM detections
            {species_filter}
            AND CAST(strftime('%Y', Date) AS INTEGER) = {year}
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
        LIMIT {top_n}",
        year = params.year,
        top_n = top_n,
    )
}

/// Build SQL for monthly detection totals across all years.
///
/// Returns one row per (species, year, month) ordered chronologically.
/// Useful for phenological bar charts and seasonal summaries.
pub fn monthly_totals_sql(params: &AbundanceParams) -> String {
    let species_filter = species_clause(params.species.as_deref(), "WHERE");

    format!(
        "SELECT
            Com_Name                                        AS species,
            CAST(strftime('%Y', Date) AS INTEGER)           AS year,
            CAST(strftime('%m', Date) AS INTEGER)           AS month,
            COUNT(*)                                        AS detection_count,
            AVG(Confidence)                                 AS mean_confidence
        FROM detections
        {species_filter}
        AND CAST(strftime('%Y', Date) AS INTEGER) = {year}
        GROUP BY species, year, month
        ORDER BY species, month",
        year = params.year,
    )
}

/// Build SQL for species richness (distinct species) per week.
///
/// Returns ISO week number and species count for the given year.
/// High-richness weeks typically correspond to migration peaks.
pub fn weekly_richness_sql(year: u32) -> String {
    format!(
        "SELECT
            CAST(strftime('%W', Date) AS INTEGER)   AS iso_week,
            COUNT(DISTINCT Com_Name)                AS species_count,
            COUNT(*)                                AS total_detections
        FROM detections
        WHERE CAST(strftime('%Y', Date) AS INTEGER) = {year}
        GROUP BY iso_week
        ORDER BY iso_week"
    )
}

/// Build SQL for effort-corrected abundance (`DuckDB` only).
///
/// When recording effort data is available (e.g., from a separate
/// `recordings` table with `date` and `duration_hours` columns), this
/// query normalises detection counts per recording hour to remove
/// effort bias.
///
/// **Requires:** A `recordings` table with columns `date` (TEXT
/// `YYYY-MM-DD`) and `duration_hours` (REAL).
pub fn effort_corrected_abundance_sql(params: &AbundanceParams) -> String {
    let species_filter = species_clause(params.species.as_deref(), "AND d");

    format!(
        "WITH effort AS (
            SELECT
                CAST(strftime('%W', date) AS INTEGER)   AS iso_week,
                SUM(duration_hours)                     AS hours
            FROM recordings
            WHERE CAST(strftime('%Y', date) AS INTEGER) = {year}
            GROUP BY iso_week
        ),
        weekly AS (
            SELECT
                d.Com_Name                                      AS species,
                CAST(strftime('%W', d.Date) AS INTEGER)         AS iso_week,
                COUNT(*)                                        AS raw_count
            FROM detections d
            {species_filter}.Com_Name IS NOT NULL
            AND CAST(strftime('%Y', d.Date) AS INTEGER) = {year}
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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn species_clause(species: Option<&str>, prefix: &str) -> String {
    species.map_or_else(
        || format!("{prefix} Com_Name IS NOT NULL"),
        |s| format!("{prefix} Com_Name = '{}'", s.replace('\'', "''")),
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
        assert!(sql.contains("duration_hours"));
        assert!(sql.contains("effort_hours"));
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
