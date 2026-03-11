//! SQL query builders for duckdb-behavioral functions.
//!
//! Generates the SQL queries that use the `behavioral` `DuckDB` extension.
//! These queries are designed to be executed against a `DuckDB` connection
//! that has the behavioral extension loaded and a `detections_ts` view
//! with a proper TIMESTAMP column.

use crate::types::{FunnelParams, RetentionParams, SessionizeParams};

/// SQL to create the timestamp view for behavioral queries.
///
/// This view adds a proper TIMESTAMP column from the Date and Time text fields.
pub const CREATE_DETECTIONS_TS_VIEW: &str = "
CREATE OR REPLACE VIEW detections_ts AS
SELECT *,
    CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp,
    CAST(Date AS DATE) AS detection_date
FROM detections;
";

/// SQL to load the behavioral extension.
pub const LOAD_BEHAVIORAL: &str = "
INSTALL behavioral FROM community;
LOAD behavioral;
";

/// Build SQL for activity sessionization.
///
/// Uses `sessionize()` from duckdb-behavioral to group continuous
/// bird activity into sessions.
pub fn sessionize_sql(params: &SessionizeParams) -> String {
    let species_filter = params.species.as_ref().map_or_else(
        String::new,
        |s| format!("WHERE Com_Name = '{}'", s.replace('\'', "''")),
    );

    format!(
        "SELECT
            Com_Name as species,
            sessionize(detection_timestamp, INTERVAL '{gap} MINUTE')
                OVER (PARTITION BY Sci_Name ORDER BY detection_timestamp)
                AS session_id,
            COUNT(*) as detection_count,
            MIN(detection_timestamp) as start_time,
            MAX(detection_timestamp) as end_time,
            DATEDIFF('second', MIN(detection_timestamp), MAX(detection_timestamp)) as duration_secs
        FROM detections_ts
        {species_filter}
        GROUP BY Com_Name, session_id
        ORDER BY start_time DESC
        LIMIT {limit}",
        gap = params.gap_minutes,
        limit = params.limit,
    )
}

/// Build SQL for species retention analysis.
///
/// Uses `retention()` from duckdb-behavioral to track species
/// return patterns at specified day intervals.
pub fn retention_sql(params: &RetentionParams) -> String {
    let intervals_str = params
        .intervals
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "SELECT
            Com_Name as species,
            retention(detection_date, [{intervals}]) AS retention_rates
        FROM (
            SELECT DISTINCT Com_Name, detection_date
            FROM detections_ts
        )
        GROUP BY Com_Name
        HAVING COUNT(DISTINCT detection_date) >= {min}
        ORDER BY retention_rates[1] DESC",
        intervals = intervals_str,
        min = params.min_detections,
    )
}

/// Build SQL for dawn chorus funnel analysis.
///
/// Uses `window_funnel()` from duckdb-behavioral to check how many
/// steps of an expected species sequence occur each morning.
pub fn funnel_sql(params: &FunnelParams) -> String {
    let conditions: Vec<String> = params
        .species_sequence
        .iter()
        .map(|s| format!("Com_Name = '{}'", s.replace('\'', "''")))
        .collect();

    let conditions_array = conditions.join(",\n        ");

    format!(
        "SELECT
            CAST(detection_timestamp AS DATE) as date,
            window_funnel(
                INTERVAL '{window} MINUTE',
                detection_timestamp,
                [
                    {conditions}
                ]
            ) AS steps_completed
        FROM detections_ts
        WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN {start} AND {end}
        GROUP BY CAST(detection_timestamp AS DATE)
        ORDER BY date DESC",
        window = params.window_minutes,
        conditions = conditions_array,
        start = params.hour_start,
        end = params.hour_end,
    )
}

/// Build SQL for next-species prediction.
///
/// Uses `sequence_next_node()` from duckdb-behavioral to predict
/// which species typically follows a given trigger species.
pub fn next_species_sql(trigger_species: &str, window_minutes: u32, limit: u32) -> String {
    let escaped = trigger_species.replace('\'', "''");
    format!(
        "SELECT
            sequence_next_node(
                detection_timestamp,
                INTERVAL '{window_minutes} MINUTE',
                Com_Name = '{escaped}',
                1,
                'strict'
            ) AS predicted_species,
            COUNT(*) as frequency
        FROM detections_ts
        GROUP BY predicted_species
        HAVING predicted_species IS NOT NULL
        ORDER BY frequency DESC
        LIMIT {limit}",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessionize_sql_all_species() {
        let sql = sessionize_sql(&SessionizeParams::default());
        assert!(sql.contains("sessionize"));
        assert!(sql.contains("INTERVAL '30 MINUTE'"));
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
    }

    #[test]
    fn retention_sql_default() {
        let sql = retention_sql(&RetentionParams::default());
        assert!(sql.contains("retention("));
        assert!(sql.contains("[1, 2, 3, 7, 14, 30]"));
        assert!(sql.contains(">= 5"));
    }

    #[test]
    fn funnel_sql_default() {
        let sql = funnel_sql(&FunnelParams::default());
        assert!(sql.contains("window_funnel"));
        assert!(sql.contains("European Robin"));
        assert!(sql.contains("BETWEEN 4 AND 8"));
    }

    #[test]
    fn next_species_sql_escapes_quotes() {
        let sql = next_species_sql("O'Brien's Warbler", 60, 10);
        assert!(sql.contains("O''Brien''s Warbler"));
        assert!(sql.contains("LIMIT 10"));
    }

    #[test]
    fn create_view_sql_is_valid() {
        assert!(CREATE_DETECTIONS_TS_VIEW.contains("detection_timestamp"));
        assert!(CREATE_DETECTIONS_TS_VIEW.contains("TIMESTAMP"));
    }
}
