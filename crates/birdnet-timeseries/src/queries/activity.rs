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
        let window = super::last_days(self.lookback_days);
        let species_filter = self
            .species
            .as_deref()
            .map(|s| {
                let esc = s.replace('\'', "''");
                format!("AND Com_Name = '{esc}'")
            })
            .unwrap_or_default();
        format!(
            "SELECT
    strftime(time_bucket(INTERVAL 1 HOUR, detection_timestamp), '%Y-%m-%d %H:%M:%S') AS window_start,
    strftime(time_bucket(INTERVAL 1 HOUR, detection_timestamp) + INTERVAL 1 HOUR, '%Y-%m-%d %H:%M:%S') AS window_end,
    COUNT(*)                                           AS detection_count,
    COUNT(DISTINCT Com_Name)                           AS species_count,
    AVG(Confidence)                                    AS avg_confidence
FROM detections_ts
WHERE {window}
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
        let window = super::last_days(self.lookback_days);
        let species_filter = self
            .species
            .as_deref()
            .map(|s| {
                let esc = s.replace('\'', "''");
                format!("AND Com_Name = '{esc}'")
            })
            .unwrap_or_default();
        format!(
            "SELECT
    strftime(detection_date, '%Y-%m-%d') AS window_start,
    strftime(detection_date + INTERVAL 1 DAY, '%Y-%m-%d') AS window_end,
    COUNT(*)                      AS detection_count,
    COUNT(DISTINCT Com_Name)      AS species_count,
    AVG(Confidence)               AS avg_confidence,
    MAX(Confidence)               AS max_confidence
FROM detections_ts
WHERE {window}
  {species_filter}
GROUP BY detection_date
ORDER BY detection_date"
        )
    }
}

/// Weekly activity totals (ISO weeks).
#[derive(Debug, Clone)]
pub struct WeeklyActivity {
    /// Number of weekly buckets, the running week included (default: 52).
    /// Each starts on a Monday, so every bucket but the running one is a
    /// whole week.
    pub lookback_weeks: u32,
}

impl Default for WeeklyActivity {
    fn default() -> Self {
        Self { lookback_weeks: 52 }
    }
}

impl QueryPlan for WeeklyActivity {
    fn sql(&self) -> String {
        // `weeks` buckets: the running week and the `weeks - 1` whole weeks
        // before it. Counting back from *today* started the first bucket part
        // way through a week, so the oldest bar was a stub of one to six days
        // drawn at the same scale as the whole weeks beside it.
        let back = self.lookback_weeks.saturating_sub(1);
        format!(
            "SELECT
    strftime(date_trunc('week', detection_date), '%Y-%m-%d') AS window_start,
    strftime(date_trunc('week', detection_date) + INTERVAL 7 DAYS, '%Y-%m-%d') AS window_end,
    COUNT(*)                                              AS detection_count,
    COUNT(DISTINCT Com_Name)                              AS species_count,
    AVG(Confidence)                                       AS avg_confidence
FROM detections_ts
WHERE detection_date >= date_trunc('week', CURRENT_DATE) - INTERVAL {back} WEEKS
GROUP BY date_trunc('week', detection_date)
ORDER BY window_start"
        )
    }
}

/// Hourly activity heatmap: average detections per hour-of-day across all days.
///
/// Useful for showing the typical daily rhythm (dawn chorus, midday lull, etc.)
///
/// The average divides by every day in the window on which the station heard
/// anything, not by the days that hour was busy (ANA11): an owl heard at 03:00
/// on two nights of ten averages 0.2 a day, not 1.0. Days with no detection at
/// all are left out, so a station installed last week is not averaged over
/// ninety days it was not running.
#[derive(Debug, Clone)]
pub struct HourlyHeatmap {
    /// Number of complete days of history to include (default: 90). Today is
    /// never one of them: a day a few hours old would count in the
    /// denominator in full and in the numerator only up to now.
    pub lookback_days: u32,
    /// Optional species filter. The denominator stays the days the *station*
    /// heard anything, so a species' figure is its rate per listening day.
    pub species: Option<String>,
}

impl Default for HourlyHeatmap {
    fn default() -> Self {
        Self {
            lookback_days: 90,
            species: None,
        }
    }
}

impl QueryPlan for HourlyHeatmap {
    fn sql(&self) -> String {
        let window = super::last_complete_days(self.lookback_days);
        let species_filter = self
            .species
            .as_deref()
            .map(|s| format!("WHERE Com_Name = '{}'", s.replace('\'', "''")))
            .unwrap_or_default();
        format!(
            "WITH windowed AS (
    SELECT * FROM detections_ts
    WHERE {window}
),
station_days AS (SELECT COUNT(DISTINCT detection_date) AS n FROM windowed),
selected AS (SELECT * FROM windowed {species_filter})
SELECT
    hour(detection_timestamp)    AS hour_of_day,
    COUNT(*)                     AS total_detections,
    COUNT(DISTINCT detection_date) AS active_days,
    COUNT(*) * 1.0 / ANY_VALUE(station_days.n) AS avg_detections_per_day,
    COUNT(DISTINCT Com_Name)     AS unique_species
FROM selected, station_days
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
        assert!(sql.contains("detection_date > CURRENT_DATE - INTERVAL 7 DAYS"));
    }

    #[test]
    fn daily_activity_sql() {
        let q = DailyActivity {
            lookback_days: 14,
            species: None,
        };
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
