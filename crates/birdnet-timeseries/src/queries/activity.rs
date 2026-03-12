//! Detection activity queries: counting detections over time windows.
//!
//! These are the most frequently used time-series queries — they answer
//! "how many detections occurred in each time bucket?"

use super::QueryPlan;

/// Hourly activity bucketed by `time_bucket`.
///
/// Returns one row per hour that had at least one detection.
#[derive(Debug, Clone)]
pub struct HourlyActivity {
    /// Look back this many days from today (default: 7).
    pub lookback_days: u32,
    /// Optional species filter.
    pub species: Option<String>,
}

impl Default for HourlyActivity {
    fn default() -> Self {
        Self {
            lookback_days: 7,
            species: None,
        }
    }
}

impl QueryPlan for HourlyActivity {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        let species_filter = self.species.as_deref().map(|s| {
            let esc = s.replace('\'', "''");
            format!("AND Com_Name = '{esc}'")
        }).unwrap_or_default();
        format!(
            "SELECT
    time_bucket(INTERVAL 1 HOUR, detection_timestamp) AS window_start,
    window_start + INTERVAL 1 HOUR                    AS window_end,
    COUNT(*)                                           AS detection_count,
    COUNT(DISTINCT Com_Name)                           AS species_count,
    AVG(Confidence)                                    AS avg_confidence
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
  {species_filter}
GROUP BY ALL
ORDER BY window_start"
        )
    }
}

/// Daily activity totals.
#[derive(Debug, Clone)]
pub struct DailyActivity {
    /// Look back this many days (default: 30).
    pub lookback_days: u32,
    /// Optional species filter.
    pub species: Option<String>,
}

impl Default for DailyActivity {
    fn default() -> Self {
        Self {
            lookback_days: 30,
            species: None,
        }
    }
}

impl QueryPlan for DailyActivity {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        let species_filter = self.species.as_deref().map(|s| {
            let esc = s.replace('\'', "''");
            format!("AND Com_Name = '{esc}'")
        }).unwrap_or_default();
        format!(
            "SELECT
    detection_date                AS window_start,
    detection_date + INTERVAL 1 DAY AS window_end,
    COUNT(*)                      AS detection_count,
    COUNT(DISTINCT Com_Name)      AS species_count,
    AVG(Confidence)               AS avg_confidence,
    MAX(Confidence)               AS max_confidence
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
  {species_filter}
GROUP BY detection_date
ORDER BY detection_date"
        )
    }
}

/// Weekly activity totals (ISO weeks).
#[derive(Debug, Clone)]
pub struct WeeklyActivity {
    /// Look back this many weeks (default: 52).
    pub lookback_weeks: u32,
}

impl Default for WeeklyActivity {
    fn default() -> Self {
        Self { lookback_weeks: 52 }
    }
}

impl QueryPlan for WeeklyActivity {
    fn sql(&self) -> String {
        let weeks = self.lookback_weeks;
        format!(
            "SELECT
    date_trunc('week', detection_date)::DATE             AS window_start,
    (date_trunc('week', detection_date) + INTERVAL 7 DAYS)::DATE AS window_end,
    COUNT(*)                                              AS detection_count,
    COUNT(DISTINCT Com_Name)                              AS species_count,
    AVG(Confidence)                                       AS avg_confidence
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {weeks} WEEKS
GROUP BY date_trunc('week', detection_date)
ORDER BY window_start"
        )
    }
}

/// Hourly activity heatmap: average detections per hour-of-day across all days.
///
/// Useful for showing the typical daily rhythm (dawn chorus, midday lull, etc.)
#[derive(Debug, Clone)]
pub struct HourlyHeatmap {
    /// Number of days of history to include (default: 90).
    pub lookback_days: u32,
}

impl Default for HourlyHeatmap {
    fn default() -> Self {
        Self { lookback_days: 90 }
    }
}

impl QueryPlan for HourlyHeatmap {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        format!(
            "SELECT
    hour(detection_timestamp)    AS hour_of_day,
    COUNT(*)                     AS total_detections,
    COUNT(DISTINCT detection_date) AS active_days,
    COUNT(*) * 1.0 / COUNT(DISTINCT detection_date) AS avg_detections_per_day,
    COUNT(DISTINCT Com_Name)     AS unique_species
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
GROUP BY hour(detection_timestamp)
ORDER BY hour_of_day"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_activity_sql() {
        let q = HourlyActivity::default();
        let sql = q.sql();
        assert!(sql.contains("time_bucket"));
        assert!(sql.contains("INTERVAL 7 DAYS"));
    }

    #[test]
    fn daily_activity_sql() {
        let q = DailyActivity { lookback_days: 14, species: None };
        let sql = q.sql();
        assert!(sql.contains("INTERVAL 14 DAYS"));
        assert!(sql.contains("detection_date"));
    }

    #[test]
    fn weekly_activity_sql() {
        let q = WeeklyActivity::default();
        let sql = q.sql();
        assert!(sql.contains("date_trunc('week'"));
    }

    #[test]
    fn heatmap_sql_groups_by_hour() {
        let q = HourlyHeatmap::default();
        let sql = q.sql();
        assert!(sql.contains("hour(detection_timestamp)"));
        assert!(sql.contains("GROUP BY hour("));
    }
}
