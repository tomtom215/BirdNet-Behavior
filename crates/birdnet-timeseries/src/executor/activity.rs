//! Hourly, daily, and weekly activity query methods.

use crate::error::TimeSeriesError;
use crate::queries::activity::{DailyActivity, HourlyActivity, HourlyHeatmap, WeeklyActivity};
use crate::types::{
    params::{DailyParams, HourlyParams, WeeklyParams},
    results::{HourlyHeatmapRow, WindowRow},
};

impl<'conn> super::TimeSeriesDb<'conn> {
    /// Hourly detection counts over the last `params.lookback_days`.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn hourly_activity(
        &self,
        params: &HourlyParams,
    ) -> Result<Vec<WindowRow>, TimeSeriesError> {
        let q = HourlyActivity {
            lookback_days: params.lookback_days,
            species: params.species.clone(),
        };
        self.run_window_query(&q.sql())
    }

    /// Daily detection counts over the last `params.lookback_days`.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn daily_activity(
        &self,
        params: &DailyParams,
    ) -> Result<Vec<WindowRow>, TimeSeriesError> {
        let q = DailyActivity {
            lookback_days: params.lookback_days,
            species: params.species.clone(),
        };
        self.run_window_query(&q.sql())
    }

    /// Weekly detection counts.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn weekly_activity(
        &self,
        params: &WeeklyParams,
    ) -> Result<Vec<WindowRow>, TimeSeriesError> {
        let q = WeeklyActivity {
            lookback_weeks: params.lookback_weeks,
        };
        self.run_window_query(&q.sql())
    }

    /// Hourly activity heatmap (average detections per hour-of-day).
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn hourly_heatmap(
        &self,
        params: &HourlyParams,
    ) -> Result<Vec<HourlyHeatmapRow>, TimeSeriesError> {
        let q = HourlyHeatmap {
            lookback_days: params.lookback_days,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(HourlyHeatmapRow {
                hour_of_day: row.get(0)?,
                total_detections: row.get(1)?,
                active_days: row.get(2)?,
                avg_detections_per_day: row.get(3)?,
                unique_species: row.get(4)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }
}
