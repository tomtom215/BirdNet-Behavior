//! DuckDB executor for time-series analytics queries.
//!
//! `TimeSeriesDb` borrows a `duckdb::Connection` (typically from
//! `birdnet_behavioral::connection::AnalyticsDb::conn()`) and exposes
//! high-level methods for each time-series analytics goal.
//!
//! This module is only compiled with the `analytics` feature because
//! it depends on `duckdb`. All other modules in this crate are always
//! available.

use duckdb::Connection;

use crate::error::TimeSeriesError;
use crate::queries::{
    activity::{DailyActivity, HourlyActivity, HourlyHeatmap, WeeklyActivity},
    diversity::{AccumulationCurve, DailyRichness, DailyShannon, TopSpeciesByCount},
    gap::{DailyMaxGap, IntraDay, QuietDays},
    peak::{PeakWindows, SpeciesPeak},
    trend::{AnomalyDetection, MovingAverage, YearOverYear},
    QueryPlan, ENSURE_TS_VIEW,
};
use crate::types::{
    params::{
        AnomalyParams, DailyParams, DiversityParams, HourlyParams, PeakParams, SessionParams,
        TrendParams, WeeklyParams,
    },
    results::{
        AccumulationRow, AnomalyRow, DiversityRow, GapRow, HourlyHeatmapRow, PeakWindowRow,
        SessionRow, TrendRow, WindowRow, YearOverYearRow,
    },
};
use crate::window::{SessionSpec, WindowSpec};

/// Executes time-series analytics queries against a DuckDB connection.
///
/// Borrows the connection for its lifetime; typically created per-request
/// inside an `AppState::with_analytics` closure.
#[derive(Debug)]
pub struct TimeSeriesDb<'conn> {
    conn: &'conn Connection,
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

    // -----------------------------------------------------------------
    // Activity
    // -----------------------------------------------------------------

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
    pub fn weekly_activity(
        &self,
        params: &WeeklyParams,
    ) -> Result<Vec<WindowRow>, TimeSeriesError> {
        let q = WeeklyActivity {
            lookback_weeks: params.lookback_weeks,
        };
        self.run_window_query(&q.sql())
    }

    /// Hourly activity heatmap (average per hour-of-day across all days).
    ///
    /// # Errors
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

    // -----------------------------------------------------------------
    // Trend
    // -----------------------------------------------------------------

    /// N-day centred moving average of daily detections.
    ///
    /// # Errors
    pub fn moving_average(
        &self,
        params: &TrendParams,
    ) -> Result<Vec<TrendRow>, TimeSeriesError> {
        let q = MovingAverage {
            window_days: params.window_days,
            from_date: params.from_date.clone(),
            to_date: params.to_date.clone(),
            species: params.species.clone(),
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
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Year-over-year weekly comparison.
    ///
    /// # Errors
    pub fn year_over_year(
        &self,
        params: &WeeklyParams,
    ) -> Result<Vec<YearOverYearRow>, TimeSeriesError> {
        let q = YearOverYear {
            weeks: params.lookback_weeks,
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
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Anomaly detection: days with unusually high or low activity.
    ///
    /// # Errors
    pub fn anomalies(
        &self,
        params: &AnomalyParams,
    ) -> Result<Vec<AnomalyRow>, TimeSeriesError> {
        let q = AnomalyDetection {
            z_threshold: params.z_threshold,
            window_days: params.window_days,
            lookback_days: params.lookback_days,
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
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    // -----------------------------------------------------------------
    // Diversity
    // -----------------------------------------------------------------

    /// Daily species richness (distinct species per day).
    ///
    /// # Errors
    pub fn daily_richness(
        &self,
        params: &DiversityParams,
    ) -> Result<Vec<DiversityRow>, TimeSeriesError> {
        if params.include_shannon {
            let q = DailyShannon {
                lookback_days: params.lookback_days,
            };
            let sql = q.sql();
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok(DiversityRow {
                    date: row.get(0)?,
                    species_richness: row.get(1)?,
                    total_detections: row.get(2)?,
                    shannon_h: row.get(3)?,
                    pielou_evenness: row.get(4)?,
                })
            })?;
            rows.map(|r| r.map_err(Into::into)).collect()
        } else {
            let q = DailyRichness {
                lookback_days: params.lookback_days,
            };
            let sql = q.sql();
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok(DiversityRow {
                    date: row.get(0)?,
                    species_richness: row.get(1)?,
                    total_detections: row.get(2)?,
                    shannon_h: None,
                    pielou_evenness: None,
                })
            })?;
            rows.map(|r| r.map_err(Into::into)).collect()
        }
    }

    /// Species accumulation curve: new species seen each day.
    ///
    /// # Errors
    pub fn accumulation_curve(
        &self,
        from_date: Option<String>,
        to_date: Option<String>,
    ) -> Result<Vec<AccumulationRow>, TimeSeriesError> {
        let q = AccumulationCurve { from_date, to_date };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(AccumulationRow {
                date: row.get(0)?,
                new_species_today: row.get(1)?,
                cumulative_species: row.get(2)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    // -----------------------------------------------------------------
    // Peak
    // -----------------------------------------------------------------

    /// Busiest N-minute windows.
    ///
    /// # Errors
    pub fn peak_windows(
        &self,
        params: &PeakParams,
    ) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let days = params.lookback_days;
        let q = PeakWindows {
            window_minutes: params.window_minutes,
            hop_minutes: params.hop_minutes,
            range_start: format!("CURRENT_TIMESTAMP - INTERVAL {days} DAYS"),
            range_end: "CURRENT_TIMESTAMP".into(),
            limit: params.limit,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(PeakWindowRow {
                window_start: row.get(0)?,
                window_end: row.get(1)?,
                detection_count: row.get(2)?,
                species_count: row.get(3)?,
                peak_confidence: row.get(4)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Peak hours for a specific species.
    ///
    /// # Errors
    pub fn species_peak_hours(
        &self,
        species: &str,
        lookback_days: u32,
    ) -> Result<Vec<PeakWindowRow>, TimeSeriesError> {
        let q = SpeciesPeak::hourly(species.to_string());
        let _ = lookback_days; // SpeciesPeak uses its own default; extend if needed
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let hour: i64 = row.get(0)?;
            Ok(PeakWindowRow {
                window_start: format!("{hour:02}:00"),
                window_end: format!("{}:00", (hour + 1) % 24),
                detection_count: row.get(1)?,
                species_count: 1,
                peak_confidence: row.get(3)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    // -----------------------------------------------------------------
    // Sessions / Gaps
    // -----------------------------------------------------------------

    /// Activity sessions grouped by inactivity gap.
    ///
    /// # Errors
    pub fn activity_sessions(
        &self,
        params: &SessionParams,
    ) -> Result<Vec<SessionRow>, TimeSeriesError> {
        let spec = if let Some(date) = &params.date_filter {
            SessionSpec::for_date(date.clone(), params.gap_minutes)
        } else {
            let days = params.lookback_days;
            SessionSpec {
                gap_threshold_minutes: params.gap_minutes,
                date_filter: Some(format!(
                    "' AND detection_date >= CURRENT_DATE - INTERVAL {days} DAYS --"
                )),
                limit: params.limit,
                ..SessionSpec::default()
            }
        };

        // Build safe SQL using SessionSpec.build_sql()
        let sql = if params.date_filter.is_none() {
            // Override: build a date-range session query without injection risk
            self.build_daterange_session_sql(params)
        } else {
            spec.build_sql()
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

    /// Days with unusually few detections (quiet days).
    ///
    /// # Errors
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

    // -----------------------------------------------------------------
    // Top species
    // -----------------------------------------------------------------

    /// Top species by detection count over a date window.
    ///
    /// # Errors
    pub fn top_species(
        &self,
        lookback_days: u32,
        limit: u32,
    ) -> Result<Vec<crate::types::results::PeakWindowRow>, TimeSeriesError> {
        let q = TopSpeciesByCount {
            lookback_days,
            limit,
        };
        let sql = q.sql();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(PeakWindowRow {
                window_start: row.get::<_, String>(3)?,
                window_end: row.get::<_, String>(4)?,
                detection_count: row.get(1)?,
                species_count: 1,
                peak_confidence: row.get(2)?,
            })
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    // -----------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------

    fn run_window_query(&self, sql: &str) -> Result<Vec<WindowRow>, TimeSeriesError> {
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

    fn build_daterange_session_sql(&self, params: &SessionParams) -> String {
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
