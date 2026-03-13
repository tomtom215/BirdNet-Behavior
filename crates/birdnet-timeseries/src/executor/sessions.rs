//! Activity session and inactivity gap query methods.

use crate::error::TimeSeriesError;
use crate::queries::gap::{DailyMaxGap, IntraDay, QuietDays};
use crate::types::{
    params::SessionParams,
    results::{GapRow, SessionRow, WindowRow},
};
use crate::window::SessionSpec;

impl<'conn> super::TimeSeriesDb<'conn> {
    /// Group detections into activity sessions separated by inactivity gaps.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn activity_sessions(
        &self,
        params: &SessionParams,
    ) -> Result<Vec<SessionRow>, TimeSeriesError> {
        let sql = if params.date_filter.is_some() {
            let spec = SessionSpec::for_date(
                params.date_filter.clone().unwrap_or_default(),
                params.gap_minutes,
            );
            spec.build_sql()
        } else {
            self.build_daterange_session_sql(params)
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRow {
                session_id: row.get(0)?,
                date: row.get(1)?,
                session_start: row.get(2)?,
                session_end: row.get(3)?,
                detection_count: row.get(4)?,
                species_count: row.get(5)?,
                duration_minutes: row.get(6)?,
                max_internal_gap_minutes: row.get(7)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Inactivity gaps within a day exceeding the threshold.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn intraday_gaps(
        &self,
        date: &str,
        threshold_minutes: u32,
    ) -> Result<Vec<GapRow>, TimeSeriesError> {
        let q = IntraDay {
            date: date.to_string(),
            threshold_minutes,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(GapRow {
                gap_end: row.get(0)?,
                gap_start: row.get(1)?,
                gap_minutes: row.get(2)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Days with fewer than `max_detections` detections (quiet days).
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn quiet_days(
        &self,
        max_detections: u32,
        lookback_days: u32,
    ) -> Result<Vec<WindowRow>, TimeSeriesError> {
        let q = QuietDays {
            max_detections,
            lookback_days,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(WindowRow {
                window_start: row.get::<_, String>(0)?,
                window_end: String::new(),
                detection_count: row.get(1)?,
                species_count: row.get(2)?,
                avg_confidence: None,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Daily maximum inactivity gaps.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn daily_max_gaps(
        &self,
        lookback_days: u32,
        min_gap_minutes: u32,
    ) -> Result<Vec<GapRow>, TimeSeriesError> {
        let q = DailyMaxGap {
            lookback_days,
            min_gap_minutes,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(GapRow {
                gap_end: row.get::<_, String>(0)?,
                gap_start: None,
                gap_minutes: row.get(1)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Build a date-range session query without SQL injection risk
    /// (all interpolated values are validated u32 integers).
    pub(super) fn build_daterange_session_sql(&self, params: &SessionParams) -> String {
        let threshold = params.gap_minutes;
        let days = params.lookback_days;
        let limit = params.limit;
        format!(
            "WITH ordered AS (
    SELECT
        detection_timestamp,
        detection_date,
        Com_Name,
        Confidence,
        LAG(detection_timestamp) OVER (ORDER BY detection_timestamp) AS prev_ts,
        date_diff('minute', prev_ts, detection_timestamp) AS gap_minutes
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
),
with_session_id AS (
    SELECT
        detection_timestamp, detection_date, Com_Name, Confidence, gap_minutes,
        SUM(CASE WHEN gap_minutes >= {threshold} OR gap_minutes IS NULL THEN 1 ELSE 0 END)
            OVER (ORDER BY detection_timestamp ROWS UNBOUNDED PRECEDING) AS session_id
    FROM ordered
)
SELECT
    session_id, detection_date,
    MIN(detection_timestamp), MAX(detection_timestamp),
    COUNT(*), COUNT(DISTINCT Com_Name),
    date_diff('minute', MIN(detection_timestamp), MAX(detection_timestamp)),
    MAX(gap_minutes)
FROM with_session_id
GROUP BY session_id, detection_date
ORDER BY MIN(detection_timestamp)
LIMIT {limit}"
        )
    }
}
