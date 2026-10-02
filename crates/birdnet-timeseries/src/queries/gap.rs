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
/// Returns all pairs of consecutive detections where the silence between
/// them was *longer* than `threshold_minutes` of elapsed time — exactly the
/// silences that start a new activity session at the same threshold (see
/// [`crate::window::SessionSpec`]), so the two views of a day agree.
#[derive(Debug, Clone)]
pub struct IntraDay {
    /// Calendar date to analyse (ISO-8601).
    pub date: String,
    /// Report silences longer than this many minutes (default: 30).
    pub threshold_minutes: u32,
}

impl IntraDay {
    /// Create for the given date with the default 30-minute threshold.
    pub const fn for_date(date: String) -> Self {
        Self {
            date,
            threshold_minutes: 30,
        }
    }
}

impl QueryPlan for IntraDay {
    fn sql(&self) -> String {
        let date = self.date.replace('\'', "''");
        let thresh_us = u64::from(self.threshold_minutes) * 60_000_000;
        // Elapsed microseconds between instants, not `date_diff('minute', …)`:
        // that counts minute *boundaries* crossed, so 05:00:59 → 05:30:00 —
        // 29 minutes and a second — read as 30 and was reported as a 30-minute
        // gap. Reported minutes are whole minutes elapsed, rounded down.
        format!(
            "WITH gaps AS (
    SELECT
        detection_timestamp,
        LAG(detection_timestamp) OVER (ORDER BY detection_instant) AS prev_local,
        epoch_us(detection_instant)
            - epoch_us(LAG(detection_instant) OVER (ORDER BY detection_instant)) AS gap_us
    FROM detections_ts
    WHERE detection_date = '{date}'
)
SELECT
    strftime(detection_timestamp, '%Y-%m-%d %H:%M:%S') AS gap_end,
    strftime(prev_local, '%Y-%m-%d %H:%M:%S')          AS gap_start,
    gap_us // 60000000                                  AS gap_minutes
FROM gaps
WHERE gap_us > {thresh_us}
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
            "WITH counts AS (
    SELECT detection_date, COUNT(*) AS n, COUNT(DISTINCT Com_Name) AS species
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {days} DAYS
      AND detection_date < CURRENT_DATE
    GROUP BY detection_date
),
spine AS (
    SELECT CAST(range AS DATE) AS detection_date
    FROM range(
        (SELECT MIN(detection_date) FROM counts)::TIMESTAMP,
        CURRENT_DATE::TIMESTAMP,
        INTERVAL 1 DAY
    )
)
SELECT
    strftime(s.detection_date, '%Y-%m-%d') AS date,
    COALESCE(c.n, 0)       AS detection_count,
    COALESCE(c.species, 0) AS species_count
FROM spine s LEFT JOIN counts c USING (detection_date)
WHERE COALESCE(c.n, 0) <= {max_d}
ORDER BY s.detection_date"
        )
    }
}

/// Longest inter-detection gap per day over a date range.
///
/// Surfaces the date(s) with the worst daily silence, for diagnostics.
#[derive(Debug, Clone)]
pub struct DailyMaxGap {
    /// Look back this many dates, today included (default: 30).
    pub lookback_days: u32,
    /// Include a day only when its longest silence is longer than this many
    /// minutes (default: 10).
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
        let window = super::last_days(self.lookback_days);
        let min_us = u64::from(self.min_gap_minutes) * 60_000_000;
        // The longest gap is reported with the two detections that bound it
        // (`gap_start`, `gap_end`), as local wall-clock times. Elapsed time is
        // measured between instants, in microseconds — see `IntraDay`.
        format!(
            "WITH gaps AS (
    SELECT
        detection_date,
        detection_timestamp,
        LAG(detection_timestamp) OVER day_order AS prev_local,
        epoch_us(detection_instant)
            - epoch_us(LAG(detection_instant) OVER day_order) AS gap_us
    FROM detections_ts
    WHERE {window}
    WINDOW day_order AS (PARTITION BY detection_date ORDER BY detection_instant)
)
SELECT
    strftime(detection_date, '%Y-%m-%d') AS date,
    MAX(gap_us) // 60000000    AS max_gap_minutes,
    COUNT(*)                   AS detection_count,
    COUNT(*) FILTER (WHERE gap_us > {min_us}) AS gap_count,
    strftime(arg_max(prev_local, gap_us), '%Y-%m-%d %H:%M:%S') AS gap_start,
    strftime(arg_max(detection_timestamp, gap_us), '%Y-%m-%d %H:%M:%S') AS gap_end
FROM gaps
GROUP BY detection_date
HAVING MAX(gap_us) > {min_us}
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
    pub const fn for_species(species: String) -> Self {
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
    strftime(date, '%Y-%m-%d') AS dt,
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
    fn intra_day_measures_elapsed_time() {
        let q = IntraDay::for_date("2026-03-12".into());
        let sql = q.sql();
        assert!(sql.contains("epoch_us(detection_instant)"));
        assert!(sql.contains("2026-03-12"));
    }

    #[test]
    fn quiet_days_threshold_clause() {
        let q = QuietDays {
            max_detections: 3,
            lookback_days: 14,
        };
        let sql = q.sql();
        // Against the zero-filled count, so a silent day qualifies (ANA13).
        assert!(sql.contains("WHERE COALESCE(c.n, 0) <= 3"));
    }

    #[test]
    fn daily_max_gap_partitions_by_date() {
        let q = DailyMaxGap::default();
        let sql = q.sql();
        assert!(sql.contains("PARTITION BY detection_date"));
        assert!(sql.contains("max_gap_minutes"));
    }
}
