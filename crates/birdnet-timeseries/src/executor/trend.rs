//! Trend analysis query methods: moving average, year-over-year, anomalies.

use crate::error::TimeSeriesError;
use crate::queries::QueryPlan;
use crate::queries::trend::{AnomalyDetection, MovingAverage, YearOverYear};
use crate::types::{
    params::{AnomalyParams, TrendParams, WeeklyParams},
    results::{AnomalyRow, TrendRow, YearOverYearRow},
};

impl super::TimeSeriesDb<'_> {
    /// N-day trailing moving average of daily detections, over a zero-filled
    /// series of complete days — see [`MovingAverage`] for what that rules
    /// out. `moving_avg_detections` is `None` where the window is incomplete
    /// or holds a day the station did not listen to.
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn moving_average(&self, params: &TrendParams) -> Result<Vec<TrendRow>, TimeSeriesError> {
        let q = MovingAverage {
            window_days: params.window_days,
            from_date: params.from_date.clone(),
            to_date: params.to_date.clone(),
            species: params.species.clone(),
            has_effort_ledger: self.has_effort_ledger()?,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(TrendRow {
                date: row.get(0)?,
                daily_detections: row.get(1)?,
                species_richness: 0,
                moving_avg_detections: row.get(2)?,
                moving_avg_species: None,
                effort_hours: row.get(5)?,
                listened: row.get(6)?,
                partial_effort: row.get(7)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Year-over-year weekly comparison, each week against the same days 52
    /// weeks earlier — see [`YearOverYear`].
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn year_over_year(
        &self,
        params: &WeeklyParams,
    ) -> Result<Vec<YearOverYearRow>, TimeSeriesError> {
        let q = YearOverYear {
            weeks: params.lookback_weeks,
            has_effort_ledger: self.has_effort_ledger()?,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(YearOverYearRow {
                week_start: row.get(0)?,
                current_year_count: row.get(1)?,
                prior_year_count: row.get(2)?,
                yoy_delta: row.get(3)?,
                current_year_species: row.get(4)?,
                prior_year_species: row.get(5)?,
                current_listened_days: row.get(6)?,
                prior_listened_days: row.get(7)?,
                current_per_listened_day: row.get(8)?,
                prior_per_listened_day: row.get(9)?,
                per_listened_day_delta: row.get(10)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Anomaly detection: complete days with unusually high or low activity
    /// against the days before them — see [`AnomalyDetection`].
    ///
    /// # Errors
    ///
    /// Returns an error if the DuckDB query fails.
    pub fn anomalies(&self, params: &AnomalyParams) -> Result<Vec<AnomalyRow>, TimeSeriesError> {
        let q = AnomalyDetection {
            z_threshold: params.z_threshold,
            window_days: params.window_days,
            lookback_days: params.lookback_days,
            has_effort_ledger: self.has_effort_ledger()?,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(AnomalyRow {
                date: row.get(0)?,
                detections: row.get(1)?,
                rolling_mean: row.get(2)?,
                rolling_stddev: row.get(3)?,
                z_score: row.get(4)?,
                anomaly_flag: row.get(5)?,
                effort_hours: row.get(6)?,
                listened: row.get(7)?,
                partial_effort: row.get(8)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }
}
