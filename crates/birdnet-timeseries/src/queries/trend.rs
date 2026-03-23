//! Trend analysis queries: moving averages, year-over-year, and anomalies.
//!
//! These queries smooth or contextualise the raw daily/hourly counts to
//! surface longer-range patterns — useful for understanding whether
//! bird activity is increasing, decreasing, or unusual.

use super::QueryPlan;

/// N-day centred moving average of daily detections.
///
/// Produces a smooth trend line by averaging each day with its N/2
/// preceding and N/2 following days.
#[derive(Debug, Clone)]
pub struct MovingAverage {
    /// Total window width in days (must be odd for a symmetric centre; default: 7).
    pub window_days: u32,
    /// Date range start (`DuckDB` expression or ISO-8601 literal).
    pub from_date: Option<String>,
    /// Date range end.
    pub to_date: Option<String>,
    /// Optional species filter.
    pub species: Option<String>,
}

impl Default for MovingAverage {
    fn default() -> Self {
        Self {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 90 DAYS".into()),
            to_date: None,
            species: None,
        }
    }
}

impl QueryPlan for MovingAverage {
    fn sql(&self) -> String {
        let half = self.window_days / 2;
        let mut where_clauses = Vec::new();
        if let Some(f) = &self.from_date {
            where_clauses.push(format!("detection_date >= {f}"));
        }
        if let Some(t) = &self.to_date {
            where_clauses.push(format!("detection_date <= {t}"));
        }
        if let Some(sp) = &self.species {
            let esc = sp.replace('\'', "''");
            where_clauses.push(format!("Com_Name = '{esc}'"));
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };
        format!(
            "WITH daily AS (
    SELECT
        detection_date,
        COUNT(*) AS daily_detections
    FROM detections_ts
    {where_sql}
    GROUP BY detection_date
)
SELECT
    detection_date,
    daily_detections,
    AVG(daily_detections) OVER (
        ORDER BY detection_date
        RANGE BETWEEN INTERVAL {half} DAYS PRECEDING
                  AND INTERVAL {half} DAYS FOLLOWING
    ) AS moving_avg,
    MIN(daily_detections) OVER (
        ORDER BY detection_date
        RANGE BETWEEN INTERVAL {half} DAYS PRECEDING
                  AND INTERVAL {half} DAYS FOLLOWING
    ) AS rolling_min,
    MAX(daily_detections) OVER (
        ORDER BY detection_date
        RANGE BETWEEN INTERVAL {half} DAYS PRECEDING
                  AND INTERVAL {half} DAYS FOLLOWING
    ) AS rolling_max
FROM daily
ORDER BY detection_date"
        )
    }
}

/// Year-over-year comparison: same calendar week, current vs. prior year.
#[derive(Debug, Clone)]
pub struct YearOverYear {
    /// Number of calendar weeks to compare (counting back from today; default: 52).
    pub weeks: u32,
}

impl Default for YearOverYear {
    fn default() -> Self {
        Self { weeks: 52 }
    }
}

impl QueryPlan for YearOverYear {
    fn sql(&self) -> String {
        let weeks = self.weeks;
        format!(
            "WITH weekly AS (
    SELECT
        date_trunc('week', detection_date)::DATE AS week_start,
        year(detection_date)                      AS yr,
        COUNT(*)                                  AS detection_count,
        COUNT(DISTINCT Com_Name)                  AS species_count
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {weeks} WEEKS
    GROUP BY week_start, yr
)
SELECT
    w1.week_start,
    w1.detection_count  AS current_year_count,
    w2.detection_count  AS prior_year_count,
    w1.detection_count - COALESCE(w2.detection_count, 0) AS yoy_delta,
    w1.species_count    AS current_year_species,
    w2.species_count    AS prior_year_species
FROM weekly w1
LEFT JOIN weekly w2
    ON w2.week_start = w1.week_start - INTERVAL 52 WEEKS
   AND w2.yr         = w1.yr - 1
WHERE w1.yr = year(CURRENT_DATE)
ORDER BY w1.week_start"
        )
    }
}

/// Anomaly detection: days whose detection count deviates > N standard deviations
/// from the rolling mean.
#[derive(Debug, Clone)]
pub struct AnomalyDetection {
    /// Z-score threshold for flagging a day as anomalous (default: 2.0).
    pub z_threshold: f64,
    /// Rolling window in days for computing mean and stddev (default: 30).
    pub window_days: u32,
    /// Look back this many days total (default: 180).
    pub lookback_days: u32,
}

impl Default for AnomalyDetection {
    fn default() -> Self {
        Self {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 180,
        }
    }
}

impl QueryPlan for AnomalyDetection {
    fn sql(&self) -> String {
        let window = self.window_days;
        let lookback = self.lookback_days;
        let z = self.z_threshold;
        format!(
            "WITH daily AS (
    SELECT detection_date, COUNT(*) AS detections
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {lookback} DAYS
    GROUP BY detection_date
),
with_stats AS (
    SELECT
        detection_date,
        detections,
        AVG(detections) OVER (
            ORDER BY detection_date
            RANGE BETWEEN INTERVAL {window} DAYS PRECEDING AND CURRENT ROW
        ) AS rolling_mean,
        STDDEV_POP(detections) OVER (
            ORDER BY detection_date
            RANGE BETWEEN INTERVAL {window} DAYS PRECEDING AND CURRENT ROW
        ) AS rolling_stddev
    FROM daily
)
SELECT
    detection_date,
    detections,
    rolling_mean,
    rolling_stddev,
    (detections - rolling_mean) / NULLIF(rolling_stddev, 0) AS z_score,
    CASE
        WHEN detections > rolling_mean + {z} * COALESCE(rolling_stddev, 0)
             THEN 'high'
        WHEN detections < rolling_mean - {z} * COALESCE(rolling_stddev, 0)
             THEN 'low'
        ELSE 'normal'
    END AS anomaly_flag
FROM with_stats
ORDER BY detection_date"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_avg_sql_has_range_frame() {
        let q = MovingAverage::default();
        let sql = q.sql();
        assert!(sql.contains("RANGE BETWEEN"));
        assert!(sql.contains("moving_avg"));
    }

    #[test]
    fn yoy_sql_has_left_join() {
        let q = YearOverYear::default();
        let sql = q.sql();
        assert!(sql.contains("LEFT JOIN weekly w2"));
        assert!(sql.contains("yoy_delta"));
    }

    #[test]
    fn anomaly_sql_has_stddev() {
        let q = AnomalyDetection::default();
        let sql = q.sql();
        assert!(sql.contains("STDDEV_POP"));
        assert!(sql.contains("anomaly_flag"));
    }
}
