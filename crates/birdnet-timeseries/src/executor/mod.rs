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
    /// Checks that the `detections_ts` view is present before any query runs.
    ///
    /// # Why this checks rather than creates
    ///
    /// This used to run `CREATE OR REPLACE VIEW` with [`crate::queries::ENSURE_TS_VIEW`]. That
    /// worked only while the view had a single definition. It no longer does:
    /// migration 34 gave `detections_ts` a second rule — on a station that has
    /// imported another site's history and asked for it to be excluded,
    /// `birdnet_behavioral::queries::detections_ts_view_sql` appends
    /// `AND import_batch_id IS NULL` — and that rule depends on a setting which
    /// lives in `SQLite`, a store this crate cannot see.
    ///
    /// `birdnet-behavioral` and this crate run against the *same* `DuckDB`
    /// connection, so replacing the view here dropped the second rule for the
    /// rest of that connection's life. Sessionize, retention, funnel,
    /// next-species, co-occurrence and phenology then counted another station's
    /// records as this one's, until a later sync happened to reinstall the
    /// right definition — damage `birdnet_migrate`'s own provenance warning
    /// calls "not detectable after the fact", caused by opening a chart.
    ///
    /// So the view has exactly one owner, `birdnet-behavioral`, which is the
    /// crate that knows the flag; it creates the view on open, on every sync,
    /// and when the operator flips the setting. Constructing an executor is a
    /// read, and a read must not rewrite the catalog underneath the other
    /// crate. [`crate::queries::ENSURE_TS_VIEW`] remains as the definition this crate's queries
    /// are written against, and `tests/analytics_view_ownership.rs` still holds
    /// the two crates' texts equal.
    ///
    /// # Errors
    ///
    /// Returns [`TimeSeriesError::MissingView`] if `detections_ts` does not
    /// exist on this connection — which, for a connection that came from
    /// `AnalyticsDb::open`, it always does.
    pub fn new(conn: &'conn Connection) -> Result<Self, TimeSeriesError> {
        conn.prepare("SELECT 1 FROM detections_ts LIMIT 0")
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
