//! Peak activity queries: finding the busiest time windows.
//!
//! Uses hopping windows (generated with `range()`) to locate the
//! N-minute intervals with the highest detection counts.

use super::QueryPlan;

/// Find the top-N busiest N-minute windows over a given date range.
///
/// Generates overlapping `window_minutes`-wide buckets, hopping every
/// `hop_minutes`, then ranks them by detection count.
///
/// The ranked candidates overlap by construction, so one burst of activity
/// fills several consecutive ranks. [`crate::executor::TimeSeriesDb::peak_windows`]
/// keeps only the best of each overlapping group; callers of this raw SQL get
/// every candidate.
#[derive(Debug, Clone)]
pub struct PeakWindows {
    /// Width of each candidate window in minutes (default: 15).
    pub window_minutes: u32,
    /// Hop size in minutes (default: 5).
    pub hop_minutes: u32,
    /// Range start as a `DuckDB` timestamp expression, interpolated **raw**
    /// into the SQL so it can be a `CURRENT_TIMESTAMP - INTERVAL …` form.
    /// Because it is not quoted/escaped, callers MUST only pass trusted input
    /// or a value already validated and quoted at the trust boundary — never
    /// a raw HTTP parameter. Use [`Self::last_n_days`] when the input comes
    /// from untrusted sources.
    pub range_start: String,
    /// Range end as a `DuckDB` timestamp expression. Same raw-interpolation
    /// contract as [`Self::range_start`].
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
    strftime(w.window_start, '%Y-%m-%d %H:%M:%S') AS window_start,
    strftime(w.window_end, '%Y-%m-%d %H:%M:%S') AS window_end,
    COUNT(d.Com_Name)          AS detection_count,
    COUNT(DISTINCT d.Com_Name) AS species_count,
    MAX(d.Confidence)          AS peak_confidence
FROM windows w
LEFT JOIN detections_ts d
    ON d.detection_timestamp >= w.window_start
   AND d.detection_timestamp <  w.window_end
GROUP BY w.window_start, w.window_end
ORDER BY detection_count DESC, window_start
LIMIT {limit}"
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
        let window = super::last_days(self.lookback_days);
        let limit = self.limit;
        format!(
            "SELECT
    hour(detection_timestamp)         AS hour_of_day,
    COUNT(*)                          AS detection_count,
    COUNT(DISTINCT detection_date)    AS active_days,
    AVG(Confidence)                   AS avg_confidence,
    COUNT(*) * 1.0 / COUNT(DISTINCT detection_date) AS avg_per_active_day,
    MAX(Confidence)                   AS max_confidence
FROM detections_ts
WHERE Com_Name = '{sp}'
  AND {window}
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
