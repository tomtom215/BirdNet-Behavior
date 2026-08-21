//! Session window specification using LAG + cumulative SUM.
//!
//! A session window groups consecutive events separated by inactivity gaps
//! shorter than a configurable threshold. When the gap between two detections
//! exceeds the threshold a new session begins.
//!
//! Implementation follows the `DuckDB` pattern:
//! 1. Compute `LAG(detection_instant)` for each row
//! 2. Mark rows where the gap ≥ threshold as session boundaries
//! 3. Assign a monotonically increasing `session_id` via cumulative SUM
//! 4. Aggregate each session to its `[start, end]` extent and count
//!
//! # Which clock each step asks
//!
//! The gap, the ordering and the session duration are **elapsed time**, so they
//! read `detection_instant`. The hour-of-day filter and the displayed session
//! start/end are **local** questions, so they read `detection_timestamp`.
//!
//! That split is not stylistic. Local wall clock is not monotonic: the hour
//! daylight saving repeats each autumn makes `date_diff` between two detections
//! an hour apart return **zero**, and the hour it skips each spring makes five
//! real minutes read as sixty-five. Against a 30-minute threshold the first
//! merges two sessions that were separate and the second splits one that never
//! broke — on the two nights of the year when a dawn-chorus study is most
//! likely to be looking.
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
        detection_instant,
        detection_date,
        Com_Name,
        Confidence,
        LAG(detection_instant) OVER (
            ORDER BY detection_instant
        ) AS prev_ts,
        date_diff('minute', prev_ts, detection_instant) AS gap_minutes
    FROM detections_ts
    {where_sql}
),
with_session_id AS (
    SELECT
        detection_timestamp,
        detection_instant,
        detection_date,
        Com_Name,
        Confidence,
        gap_minutes,
        SUM(
            CASE WHEN gap_minutes >= {threshold} OR gap_minutes IS NULL
                 THEN 1 ELSE 0 END
        ) OVER (
            ORDER BY detection_instant
            ROWS UNBOUNDED PRECEDING
        ) AS session_id
    FROM ordered
)
SELECT
    session_id,
    strftime(detection_date, '%Y-%m-%d') AS detection_date,
    strftime(MIN(detection_timestamp), '%Y-%m-%d %H:%M:%S') AS session_start,
    strftime(MAX(detection_timestamp), '%Y-%m-%d %H:%M:%S') AS session_end,
    COUNT(*)                 AS detection_count,
    COUNT(DISTINCT Com_Name) AS species_count,
    date_diff('minute',
        MIN(detection_instant),
        MAX(detection_instant)
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
        assert!(sql.contains("LAG(detection_instant)"));
        assert!(sql.contains("SUM("));
        assert!(sql.contains("session_id"));
    }

    /// The split, asserted rather than described: the gap and the ordering ask
    /// the instant, the displayed extent asks the wall clock.
    ///
    /// A session query that measured its gap on the wall clock returned zero
    /// minutes between two detections an hour apart on the autumn transition,
    /// and sixty-five between two five minutes apart on the spring one — either
    /// side of a 30-minute threshold, so it merged sessions that were separate
    /// and split one that never broke.
    #[test]
    fn the_gap_is_elapsed_time_and_the_reported_extent_is_local() {
        let sql = SessionSpec::default().build_sql();
        assert!(
            sql.contains("date_diff('minute', prev_ts, detection_instant)"),
            "the gap between detections is elapsed time"
        );
        assert!(
            sql.contains("ORDER BY detection_instant"),
            "order is chronological, which local wall clock is not"
        );
        assert!(
            sql.contains("MIN(detection_instant)") && sql.contains("MAX(detection_instant)"),
            "a session's duration is elapsed time"
        );
        assert!(
            sql.contains("strftime(MIN(detection_timestamp)"),
            "but the start time shown to a human is their own clock"
        );
        assert!(
            !sql.contains("date_diff('minute', prev_ts, detection_timestamp)"),
            "no duration may be left on the wall clock"
        );
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
