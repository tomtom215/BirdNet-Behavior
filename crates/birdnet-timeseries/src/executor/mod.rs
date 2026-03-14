//! DuckDB executor for time-series analytics queries.
//!
//! `TimeSeriesDb` borrows a `duckdb::Connection` and exposes high-level
//! methods for each analytics goal, split across focused sub-modules:
//!
//! | Sub-module  | Methods                                                  |
//! |-------------|----------------------------------------------------------|
//! | `activity`  | `hourly_activity`, `daily_activity`, `weekly_activity`, `hourly_heatmap` |
//! | `trend`     | `moving_average`, `year_over_year`, `anomalies`          |
//! | `diversity` | `daily_richness`, `accumulation_curve`                   |
//! | `peak`      | `peak_windows`, `species_peak_hours`, `top_species`      |
//! | `sessions`  | `activity_sessions`, `intraday_gaps`, `quiet_days`, `daily_max_gaps` |

mod activity;
mod diversity;
mod peak;
mod sessions;
mod trend;

use duckdb::Connection;

use crate::error::TimeSeriesError;
use crate::queries::ENSURE_TS_VIEW;
use crate::types::results::WindowRow;

/// Executes time-series analytics queries against a DuckDB connection.
///
/// Borrows the connection for its lifetime; typically created per-request
/// inside an `AppState::with_timeseries` closure.
#[derive(Debug)]
pub struct TimeSeriesDb<'conn> {
    pub(super) conn: &'conn Connection,
}

impl<'conn> TimeSeriesDb<'conn> {
    /// Create a new executor borrowing `conn`.
    ///
    /// Ensures the `detections_ts` view is present before any query runs.
    ///
    /// # Errors
    ///
    /// Returns an error if the view cannot be created (e.g. `detections` table
    /// is missing).
    pub fn new(conn: &'conn Connection) -> Result<Self, TimeSeriesError> {
        conn.execute_batch(ENSURE_TS_VIEW)
            .map_err(|e| TimeSeriesError::MissingView(format!("detections_ts: {e}")))?;
        Ok(Self { conn })
    }

    /// Run a generic window query and collect `WindowRow` results.
    pub(super) fn run_window_query(&self, sql: &str) -> Result<Vec<WindowRow>, TimeSeriesError> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(WindowRow {
                window_start: row.get(0)?,
                window_end: row.get(1)?,
                detection_count: row.get(2)?,
                species_count: row.get(3)?,
                avg_confidence: row.get(4)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }
}
