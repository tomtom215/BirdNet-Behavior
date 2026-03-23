//! Session window specification using LAG + cumulative SUM.
//!
//! A session window groups consecutive events separated by inactivity gaps
//! shorter than a configurable threshold. When the gap between two detections
//! exceeds the threshold a new session begins.
//!
//! Implementation follows the `DuckDB` pattern:
//! 1. Compute `LAG(detection_timestamp)` for each row
//! 2. Mark rows where the gap ≥ threshold as session boundaries
//! 3. Assign a monotonically increasing `session_id` via cumulative SUM
//! 4. Aggregate each session to its `[start, end]` extent and count
//!
//! Primary use cases:
//! - Identifying periods of continuous bird activity vs. silence
//! - Finding dawn-chorus sessions each morning
//! - Detecting service interruptions (unexpectedly long gaps)

use super::WindowSpec;

/// Specification for a session window query on `detections_ts`.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// Minimum gap in minutes that creates a new session boundary.
    pub gap_threshold_minutes: u32,
    /// Restrict to a single calendar date (ISO-8601 string), or `None` for all dates.
    pub date_filter: Option<String>,
    /// Only include sessions within these hours of day (0–23), or `None` for all hours.
    pub hour_start: Option<u32>,
    /// End hour (inclusive) for the daily window, or `None` for all hours.
    pub hour_end: Option<u32>,
    /// Optional species filter.
    pub species: Option<String>,
    /// Maximum sessions returned.
    pub limit: u32,
}

impl Default for SessionSpec {
    fn default() -> Self {
        Self {
            gap_threshold_minutes: 30,
            date_filter: None,
            hour_start: None,
            hour_end: None,
            species: None,
            limit: 200,
        }
    }
}

impl SessionSpec {
    /// Dawn-chorus sessions (04:00 – 09:00, gap threshold 10 min).
    pub const fn dawn_chorus(date: Option<String>) -> Self {
        Self {
            gap_threshold_minutes: 10,
            date_filter: date,
            hour_start: Some(4),
            hour_end: Some(9),
            species: None,
            limit: 50,
        }
    }

    /// Full-day activity sessions for a given date.
    pub fn for_date(date: String, gap_minutes: u32) -> Self {
        Self {
            gap_threshold_minutes: gap_minutes,
            date_filter: Some(date),
            ..Default::default()
        }
    }
}

impl WindowSpec for SessionSpec {
    fn build_sql(&self) -> String {
        let threshold = self.gap_threshold_minutes;

        let mut where_clauses = Vec::new();
        if let Some(date) = &self.date_filter {
            let escaped = date.replace('\'', "''");
            where_clauses.push(format!("detection_date = '{escaped}'"));
        }
        if let (Some(hs), Some(he)) = (self.hour_start, self.hour_end) {
            where_clauses.push(format!("hour(detection_timestamp) BETWEEN {hs} AND {he}"));
        }
        if let Some(sp) = &self.species {
            let escaped = sp.replace('\'', "''");
            where_clauses.push(format!("Com_Name = '{escaped}'"));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let limit = self.limit;
        format!(
            "WITH ordered AS (
    SELECT
        detection_timestamp,
        detection_date,
        Com_Name,
        Confidence,
        LAG(detection_timestamp) OVER (
            ORDER BY detection_timestamp
        ) AS prev_ts,
        date_diff('minute', prev_ts, detection_timestamp) AS gap_minutes
    FROM detections_ts
    {where_sql}
),
with_session_id AS (
    SELECT
        detection_timestamp,
        detection_date,
        Com_Name,
        Confidence,
        gap_minutes,
        SUM(
            CASE WHEN gap_minutes >= {threshold} OR gap_minutes IS NULL
                 THEN 1 ELSE 0 END
        ) OVER (
            ORDER BY detection_timestamp
            ROWS UNBOUNDED PRECEDING
        ) AS session_id
    FROM ordered
)
SELECT
    session_id,
    detection_date,
    MIN(detection_timestamp) AS session_start,
    MAX(detection_timestamp) AS session_end,
    COUNT(*)                 AS detection_count,
    COUNT(DISTINCT Com_Name) AS species_count,
    date_diff('minute',
        MIN(detection_timestamp),
        MAX(detection_timestamp)
    )                        AS duration_minutes,
    MAX(gap_minutes)         AS max_internal_gap_minutes
FROM with_session_id
GROUP BY session_id, detection_date
ORDER BY session_start
LIMIT {limit}"
        )
    }

    fn description(&self) -> &'static str {
        "Session window: events grouped by inactivity gap threshold"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sql_has_lag_and_sum() {
        let spec = SessionSpec::default();
        let sql = spec.build_sql();
        assert!(sql.contains("LAG(detection_timestamp)"));
        assert!(sql.contains("SUM("));
        assert!(sql.contains("session_id"));
    }

    #[test]
    fn date_filter_applied() {
        let spec = SessionSpec::for_date("2026-03-12".into(), 30);
        let sql = spec.build_sql();
        assert!(sql.contains("2026-03-12"));
    }

    #[test]
    fn dawn_chorus_has_hour_filter() {
        let spec = SessionSpec::dawn_chorus(None);
        let sql = spec.build_sql();
        assert!(sql.contains("hour(detection_timestamp) BETWEEN 4 AND 9"));
    }
}
