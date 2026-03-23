//! SQL query builders for phenological timing analysis.
//!
//! Generates standard SQL (compatible with both `SQLite` and `DuckDB`) that
//! extracts first and last detection dates per species per year, computes
//! day-of-year values, and derives multi-year migration windows.
//!
//! All queries operate directly on the `detections` table with columns
//! `Com_Name`, `Sci_Name`, `Date` (TEXT `YYYY-MM-DD`), and `Time`.

use crate::phenology::types::PhenologyParams;

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
///
/// Compatible with both `SQLite` 3.x and `DuckDB` 1.x.
pub fn phenology_timing_sql(params: &PhenologyParams) -> String {
    let species_filter = species_where_clause(params.species.as_deref(), "WHERE");
    let year_conditions = year_conditions(params.year_start, params.year_end);
    let having_clause = format!("HAVING COUNT(*) >= {}", params.min_detections);
    let and_year = if year_conditions.is_empty() {
        String::new()
    } else {
        format!("AND {year_conditions}")
    };

    format!(
        "SELECT
            Com_Name                                        AS species,
            CAST(strftime('%Y', Date) AS INTEGER)           AS year,
            MIN(Date)                                       AS first_detection,
            MAX(Date)                                       AS last_detection,
            COUNT(*)                                        AS detection_count,
            CAST(strftime('%j', MIN(Date)) AS INTEGER)      AS first_doy,
            CAST(strftime('%j', MAX(Date)) AS INTEGER)      AS last_doy,
            CAST(julianday(MAX(Date)) - julianday(MIN(Date)) AS INTEGER) + 1
                                                            AS presence_days
        FROM detections
        {species_filter}
        {and_year}
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
/// **Note:** This query requires `DuckDB`.  For SQLite-only deployments
/// use [`phenology_timing_sql`] and compute percentiles client-side.
pub fn migration_window_sql(min_years: u32, params: &PhenologyParams) -> String {
    let species_filter = species_where_clause(params.species.as_deref(), "WHERE");

    format!(
        "WITH yearly AS (
            SELECT
                Com_Name                                        AS species,
                CAST(strftime('%Y', Date) AS INTEGER)           AS year,
                CAST(strftime('%j', MIN(Date)) AS INTEGER)      AS first_doy,
                CAST(strftime('%j', MAX(Date)) AS INTEGER)      AS last_doy
            FROM detections
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
                                                            AS departure_late_doy
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
    let species_filter = species_where_clause(params.species.as_deref(), "WHERE");
    format!(
        "SELECT
            Com_Name    AS species,
            MIN(Date)   AS first_ever_date,
            COUNT(*)    AS total_detections
        FROM detections
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
/// **Note:** Uses `DuckDB` `LAG` window function; not compatible with `SQLite`.
pub fn interannual_trend_sql(params: &PhenologyParams) -> String {
    let species_filter = species_where_clause(params.species.as_deref(), "WHERE");
    format!(
        "WITH yearly AS (
            SELECT
                Com_Name                                AS species,
                CAST(strftime('%Y', Date) AS INTEGER)   AS year,
                COUNT(*)                                AS detection_count
            FROM detections
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

/// Construct a `WHERE` or bare condition for optional species filtering.
fn species_where_clause(species: Option<&str>, prefix: &str) -> String {
    species.map_or_else(String::new, |s| {
        format!("{prefix} Com_Name = '{}'", s.replace('\'', "''"))
    })
}

/// Construct the year range condition (without `WHERE`/`AND` prefix).
fn year_conditions(start: Option<u32>, end: Option<u32>) -> String {
    match (start, end) {
        (Some(s), Some(e)) => {
            format!("CAST(strftime('%Y', Date) AS INTEGER) BETWEEN {s} AND {e}")
        }
        (Some(s), None) => format!("CAST(strftime('%Y', Date) AS INTEGER) >= {s}"),
        (None, Some(e)) => format!("CAST(strftime('%Y', Date) AS INTEGER) <= {e}"),
        (None, None) => String::new(),
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
        assert!(sql.contains("WHERE Com_Name = 'Eurasian Blackbird'"));
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
        assert!(sql.contains("MIN(Date)"));
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
