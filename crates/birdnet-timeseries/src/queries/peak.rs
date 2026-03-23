//! Peak activity queries: finding the busiest time windows.
//!
//! Uses hopping windows (generated with `range()`) to locate the
//! N-minute intervals with the highest detection counts.

use super::QueryPlan;

/// Find the top-N busiest N-minute windows over a given date range.
///
/// Generates overlapping `window_minutes`-wide buckets, hopping every
/// `hop_minutes`, then ranks them by detection count.
#[derive(Debug, Clone)]
pub struct PeakWindows {
    /// Width of each candidate window in minutes (default: 15).
    pub window_minutes: u32,
    /// Hop size in minutes (default: 5).
    pub hop_minutes: u32,
    /// Range start as a `DuckDB` timestamp expression.
    pub range_start: String,
    /// Range end as a `DuckDB` timestamp expression.
    pub range_end: String,
    /// Maximum windows to return (default: 10).
    pub limit: u32,
}

impl Default for PeakWindows {
    fn default() -> Self {
        Self {
            window_minutes: 15,
            hop_minutes: 5,
            range_start: "CURRENT_TIMESTAMP - INTERVAL 1 DAY".into(),
            range_end: "CURRENT_TIMESTAMP".into(),
            limit: 10,
        }
    }
}

impl PeakWindows {
    /// Peak windows over the last `days` days.
    pub fn last_n_days(days: u32) -> Self {
        Self {
            range_start: format!("CURRENT_TIMESTAMP - INTERVAL {days} DAYS"),
            range_end: "CURRENT_TIMESTAMP".into(),
            ..Default::default()
        }
    }
}

impl QueryPlan for PeakWindows {
    fn sql(&self) -> String {
        let wm = self.window_minutes;
        let hm = self.hop_minutes;
        let rs = &self.range_start;
        let re = &self.range_end;
        let limit = self.limit;
        format!(
            "WITH windows AS (
    SELECT
        range                                  AS window_start,
        range + INTERVAL {wm} MINUTE           AS window_end
    FROM range(
        ({rs})::TIMESTAMP,
        ({re})::TIMESTAMP,
        INTERVAL {hm} MINUTE
    )
)
SELECT
    w.window_start,
    w.window_end,
    COUNT(d.Com_Name)          AS detection_count,
    COUNT(DISTINCT d.Com_Name) AS species_count,
    MAX(d.Confidence)          AS peak_confidence
FROM windows w
LEFT JOIN detections_ts d
    ON d.detection_timestamp >= w.window_start
   AND d.detection_timestamp <  w.window_end
GROUP BY w.window_start, w.window_end
ORDER BY detection_count DESC
LIMIT {limit}"
        )
    }
}

/// Dawn chorus peak: the single busiest window between sunrise hours.
#[derive(Debug, Clone)]
pub struct DawnChorusPeak {
    /// Date to analyse (ISO-8601).
    pub date: String,
    /// Window width in minutes (default: 15).
    pub window_minutes: u32,
    /// Start of the dawn window in hours (default: 4).
    pub hour_start: u32,
    /// End of the dawn window in hours (default: 9).
    pub hour_end: u32,
}

impl DawnChorusPeak {
    /// Create for a specific date with defaults.
    pub const fn for_date(date: String) -> Self {
        Self {
            date,
            window_minutes: 15,
            hour_start: 4,
            hour_end: 9,
        }
    }
}

impl QueryPlan for DawnChorusPeak {
    fn sql(&self) -> String {
        let date = self.date.replace('\'', "''");
        let wm = self.window_minutes;
        let hs = self.hour_start;
        let he = self.hour_end;
        format!(
            "WITH windows AS (
    SELECT
        range                         AS window_start,
        range + INTERVAL {wm} MINUTE  AS window_end
    FROM range(
        ('{date} 0{hs}:00:00')::TIMESTAMP,
        ('{date} {he}:00:00')::TIMESTAMP,
        INTERVAL 5 MINUTE
    )
)
SELECT
    w.window_start,
    w.window_end,
    COUNT(d.Com_Name)          AS detection_count,
    COUNT(DISTINCT d.Com_Name) AS species_count,
    list(DISTINCT d.Com_Name ORDER BY d.Com_Name) AS species_list
FROM windows w
LEFT JOIN detections_ts d
    ON d.detection_timestamp >= w.window_start
   AND d.detection_timestamp <  w.window_end
GROUP BY w.window_start, w.window_end
ORDER BY detection_count DESC
LIMIT 5"
        )
    }
}

/// Species-specific peak: when is a given species most active?
#[derive(Debug, Clone)]
pub struct SpeciesPeak {
    /// Species common name.
    pub species: String,
    /// Granularity for peak analysis: `"hour"` or `"day"`.
    pub granularity: String,
    /// Look back this many days (default: 90).
    pub lookback_days: u32,
    /// Maximum rows returned.
    pub limit: u32,
}

impl SpeciesPeak {
    /// Create a hourly peak query for the given species.
    pub fn hourly(species: String) -> Self {
        Self {
            species,
            granularity: "hour".into(),
            lookback_days: 90,
            limit: 24,
        }
    }
}

impl QueryPlan for SpeciesPeak {
    fn sql(&self) -> String {
        let sp = self.species.replace('\'', "''");
        let days = self.lookback_days;
        let limit = self.limit;
        format!(
            "SELECT
    hour(detection_timestamp)         AS hour_of_day,
    COUNT(*)                          AS detection_count,
    COUNT(DISTINCT detection_date)    AS active_days,
    AVG(Confidence)                   AS avg_confidence,
    COUNT(*) * 1.0 / COUNT(DISTINCT detection_date) AS avg_per_active_day
FROM detections_ts
WHERE Com_Name = '{sp}'
  AND detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
GROUP BY hour(detection_timestamp)
ORDER BY detection_count DESC
LIMIT {limit}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_windows_sql_has_range_join() {
        let q = PeakWindows::default();
        let sql = q.sql();
        assert!(sql.contains("FROM range("));
        assert!(sql.contains("LEFT JOIN detections_ts"));
    }

    #[test]
    fn species_peak_filters_species() {
        let q = SpeciesPeak::hourly("European Robin".into());
        let sql = q.sql();
        assert!(sql.contains("European Robin"));
        assert!(sql.contains("hour(detection_timestamp)"));
    }
}
