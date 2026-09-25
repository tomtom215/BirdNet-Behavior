//! Activity session and inactivity gap query methods.

use crate::error::TimeSeriesError;
use crate::queries::QueryPlan;
use crate::queries::gap::{DailyMaxGap, IntraDay, QuietDays};
use crate::types::{
    params::SessionParams,
    results::{GapRow, SessionRow, WindowRow},
};
use crate::window::{SessionSpec, WindowSpec};

impl super::TimeSeriesDb<'_> {
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
            Self::build_daterange_session_sql(params)
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

    /// Inactivity gaps within a day longer than `threshold_minutes`.
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

    /// Each day's longest inactivity gap, with the local times of the two
    /// detections that bound it, for days whose longest gap is longer than
    /// `min_gap_minutes`, over the last `lookback_days` dates.
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
                gap_start: row.get(4)?,
                gap_end: row.get(5)?,
                gap_minutes: row.get(1)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Build a date-range session query without SQL injection risk
    /// (all interpolated values are validated u32 integers): sessions over the
    /// last `lookback_days` dates, today included.
    ///
    /// The same SQL as [`SessionSpec`] — see
    /// [`crate::window::session::session_sql`] for its rules — over a look-back
    /// rather than a single date. A session that started before the look-back
    /// and ran into it is cut at the boundary, as a single-date query cuts at
    /// midnight.
    pub(super) fn build_daterange_session_sql(params: &SessionParams) -> String {
        let where_sql = format!("WHERE {}", crate::queries::last_days(params.lookback_days));
        crate::window::session::session_sql(&where_sql, params.gap_minutes, params.limit)
    }
}
