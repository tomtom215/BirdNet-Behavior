//! Inactivity gap queries: detecting silence and absence periods.
//!
//! "Gap" queries identify stretches of time with no (or few) detections.
//! They answer questions like:
//! - "Were there any unexplained silent periods today?"
//! - "What days had fewer than N detections?"
//! - "How long was the longest gap between detections this week?"
//!
//! Implementation follows the session-window pattern: LAG over ordered
//! timestamps, then filtering for rows where the gap exceeds a threshold.

use super::QueryPlan;

/// Detect inactivity gaps within a single day.
///
/// Returns all pairs of consecutive detections where the gap between
/// them exceeded `threshold_minutes`.
#[derive(Debug, Clone)]
pub struct IntraDay {
    /// Calendar date to analyse (ISO-8601).
    pub date: String,
    /// Minimum gap in minutes to report (default: 30).
    pub threshold_minutes: u32,
}

impl IntraDay {
    /// Create for the given date with the default 30-minute threshold.
    pub fn for_date(date: String) -> Self {
        Self {
            date,
            threshold_minutes: 30,
        }
    }
}

impl QueryPlan for IntraDay {
    fn sql(&self) -> String {
        let date = self.date.replace('\'', "''");
        let thresh = self.threshold_minutes;
        format!(
            "SELECT
    detection_timestamp             AS gap_end,
    LAG(detection_timestamp) OVER (
        ORDER BY detection_timestamp
    )                               AS gap_start,
    date_diff('minute',
        LAG(detection_timestamp) OVER (ORDER BY detection_timestamp),
        detection_timestamp
    )                               AS gap_minutes
FROM detections_ts
WHERE detection_date = '{date}'
QUALIFY gap_minutes >= {thresh}
ORDER BY gap_start"
        )
    }
}

/// Days with fewer than N detections (quiet days).
///
/// Useful for identifying equipment outages, bad weather or genuine
/// low-activity periods.
#[derive(Debug, Clone)]
pub struct QuietDays {
    /// Maximum detections threshold (days at or below this are returned; default: 5).
    pub max_detections: u32,
    /// Look back this many days (default: 90).
    pub lookback_days: u32,
}

impl Default for QuietDays {
    fn default() -> Self {
        Self {
            max_detections: 5,
            lookback_days: 90,
        }
    }
}

impl QueryPlan for QuietDays {
    fn sql(&self) -> String {
        let max_d = self.max_detections;
        let days = self.lookback_days;
        format!(
            "SELECT
    detection_date          AS date,
    COUNT(*)                AS detection_count,
    COUNT(DISTINCT Com_Name) AS species_count
FROM detections_ts
WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
GROUP BY detection_date
HAVING COUNT(*) <= {max_d}
ORDER BY detection_date"
        )
    }
}

/// Longest inter-detection gap per day over a date range.
///
/// Surfaces the date(s) with the worst daily silence, for diagnostics.
#[derive(Debug, Clone)]
pub struct DailyMaxGap {
    /// Look back this many days (default: 30).
    pub lookback_days: u32,
    /// Minimum gap in minutes to include a day in the results (default: 10).
    pub min_gap_minutes: u32,
}

impl Default for DailyMaxGap {
    fn default() -> Self {
        Self {
            lookback_days: 30,
            min_gap_minutes: 10,
        }
    }
}

impl QueryPlan for DailyMaxGap {
    fn sql(&self) -> String {
        let days = self.lookback_days;
        let min_gap = self.min_gap_minutes;
        format!(
            "WITH gaps AS (
    SELECT
        detection_date,
        detection_timestamp,
        date_diff('minute',
            LAG(detection_timestamp) OVER (
                PARTITION BY detection_date
                ORDER BY detection_timestamp
            ),
            detection_timestamp
        ) AS gap_minutes
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
)
SELECT
    detection_date             AS date,
    MAX(gap_minutes)           AS max_gap_minutes,
    COUNT(*)                   AS detection_count,
    COUNT(CASE WHEN gap_minutes >= {min_gap} THEN 1 END) AS gap_count
FROM gaps
GROUP BY detection_date
HAVING MAX(gap_minutes) >= {min_gap}
ORDER BY max_gap_minutes DESC"
        )
    }
}

/// Species absence streak: consecutive days a species was NOT seen.
#[derive(Debug, Clone)]
pub struct AbsenceStreak {
    /// Species common name.
    pub species: String,
    /// Look back this many days (default: 90).
    pub lookback_days: u32,
}

impl AbsenceStreak {
    /// Build for the given species.
    pub fn for_species(species: String) -> Self {
        Self {
            species,
            lookback_days: 90,
        }
    }
}

impl QueryPlan for AbsenceStreak {
    fn sql(&self) -> String {
        let sp = self.species.replace('\'', "''");
        let days = self.lookback_days;
        format!(
            "WITH date_series AS (
    SELECT unnest(generate_series(
        (CURRENT_DATE - INTERVAL {days} DAYS)::DATE,
        CURRENT_DATE::DATE,
        INTERVAL 1 DAY
    ))::DATE AS d
),
seen_days AS (
    SELECT DISTINCT detection_date
    FROM detections_ts
    WHERE Com_Name = '{sp}'
      AND detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
),
presence AS (
    SELECT
        ds.d               AS date,
        sd.detection_date IS NOT NULL AS seen
    FROM date_series ds
    LEFT JOIN seen_days sd ON ds.d = sd.detection_date
)
SELECT
    date,
    seen,
    SUM(CASE WHEN seen THEN 1 ELSE 0 END) OVER (
        ORDER BY date ROWS UNBOUNDED PRECEDING
    ) AS cumulative_seen_days,
    SUM(CASE WHEN NOT seen THEN 1 ELSE 0 END) OVER (
        ORDER BY date
        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
    ) -
    SUM(CASE WHEN NOT seen THEN 1 ELSE 0 END) OVER (
        ORDER BY date
        ROWS BETWEEN UNBOUNDED PRECEDING AND
            (LAST_VALUE(CASE WHEN seen THEN date END) OVER (
                ORDER BY date ROWS UNBOUNDED PRECEDING
            ) - INTERVAL 1 DAY)
    ) AS current_absence_streak
FROM presence
ORDER BY date"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intra_day_uses_qualify() {
        let q = IntraDay::for_date("2026-03-12".into());
        let sql = q.sql();
        assert!(sql.contains("QUALIFY gap_minutes"));
        assert!(sql.contains("2026-03-12"));
    }

    #[test]
    fn quiet_days_having_clause() {
        let q = QuietDays { max_detections: 3, lookback_days: 14 };
        let sql = q.sql();
        assert!(sql.contains("HAVING COUNT(*) <= 3"));
    }

    #[test]
    fn daily_max_gap_partitions_by_date() {
        let q = DailyMaxGap::default();
        let sql = q.sql();
        assert!(sql.contains("PARTITION BY detection_date"));
        assert!(sql.contains("max_gap_minutes"));
    }
}
