//! Trend analysis queries: moving averages, year-over-year, and anomalies.
//!
//! These queries smooth or contextualise the raw daily/hourly counts to
//! surface longer-range patterns — useful for understanding whether
//! bird activity is increasing, decreasing, or unusual.

use super::QueryPlan;

/// N-day trailing moving average of daily detections.
///
/// Each day's average is over that day and the `window_days - 1` days before
/// it, and is only reported when all of those days are ones the station was
/// running — otherwise it is `NULL`, never an average over whichever days
/// happened to be in reach.
///
/// Three things this used to get wrong, each of which moved the line:
///
/// * **Silent days.** It grouped the days that had detections and averaged
///   those, so a day the station heard nothing was not a zero — it was not
///   there, and the window stepped over it to a busier day further out. The
///   days are now a zero-filled spine from the station's first detection.
/// * **Today.** A day a few hours old was averaged in as if complete, pulling
///   the newest points down every morning. The series ends yesterday.
/// * **The edges.** A centred `RANGE` frame at the newest end had no days
///   after it and averaged the half-window it had; at the oldest end, the
///   same. The newest end is what a reader looks at, which is why the window
///   now trails: yesterday's average is over the week that ended yesterday.
#[derive(Debug, Clone)]
pub struct MovingAverage {
    /// Window width in days, the day itself included (default: 7).
    pub window_days: u32,
    /// First date reported, interpolated **raw** into the SQL so it may be a
    /// `DuckDB` expression (e.g. `CURRENT_DATE - INTERVAL 90 DAYS`). Because it
    /// is not quoted/escaped, callers MUST only pass trusted input or a value
    /// already validated and quoted at the trust boundary — never a raw HTTP
    /// parameter. (The web layer validates to `YYYY-MM-DD` and quotes it.)
    ///
    /// Days before it still feed the averages of the days after it.
    pub from_date: Option<String>,
    /// Last date reported. Same raw-interpolation contract as `from_date`.
    /// Never later than yesterday, whatever is passed.
    pub to_date: Option<String>,
    /// Optional species filter (single-quote-escaped before interpolation).
    /// The spine still starts at the *station's* first detection, so a
    /// species' days of absence while the station ran are zeros.
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
        let n = self.window_days.max(1);
        let lead = n - 1;
        let station_start = "(SELECT MIN(detection_date) FROM detections_ts)::TIMESTAMP";
        let spine_start = self.from_date.as_ref().map_or_else(
            || station_start.to_string(),
            // `GREATEST` skips a NULL, so on a station with no detections it
            // would start a spine of zeros at `from_date`; a NULL start makes
            // the spine — and the result — empty instead.
            |f| {
                format!(
                    "CASE WHEN {station_start} IS NOT NULL THEN \
                     GREATEST(({f})::DATE::TIMESTAMP - INTERVAL {lead} DAY, {station_start}) END"
                )
            },
        );
        let shown_from = self.from_date.as_ref().map_or_else(
            || "(SELECT MIN(detection_date) FROM detections_ts)".to_string(),
            |f| format!("({f})::DATE"),
        );
        // Exclusive end: the day after `to_date`, and never past the start of
        // today.
        let spine_end = self.to_date.as_ref().map_or_else(
            || "CURRENT_DATE::TIMESTAMP".to_string(),
            |t| format!("LEAST(({t})::DATE::TIMESTAMP + INTERVAL 1 DAY, CURRENT_DATE::TIMESTAMP)"),
        );
        let species = self.species.as_ref().map_or_else(String::new, |sp| {
            format!("AND Com_Name = '{}'", sp.replace('\'', "''"))
        });
        format!(
            "WITH bounds AS (
    SELECT {spine_start} AS spine_start, {spine_end} AS spine_end, {shown_from} AS shown_from
),
spine AS (
    SELECT CAST(range AS DATE) AS detection_date
    FROM range(
        (SELECT spine_start FROM bounds),
        (SELECT spine_end FROM bounds),
        INTERVAL 1 DAY
    )
),
counts AS (
    SELECT detection_date, COUNT(*) AS n
    FROM detections_ts
    WHERE detection_date >= (SELECT spine_start FROM bounds)
      AND detection_date < (SELECT spine_end FROM bounds)
      {species}
    GROUP BY detection_date
),
daily AS (
    SELECT s.detection_date, COALESCE(c.n, 0) AS daily_detections
    FROM spine s LEFT JOIN counts c USING (detection_date)
),
smoothed AS (
    SELECT
        detection_date,
        daily_detections,
        CASE WHEN COUNT(*) OVER w = {n} THEN AVG(daily_detections) OVER w END AS moving_avg,
        CASE WHEN COUNT(*) OVER w = {n} THEN MIN(daily_detections) OVER w END AS rolling_min,
        CASE WHEN COUNT(*) OVER w = {n} THEN MAX(daily_detections) OVER w END AS rolling_max
    FROM daily
    WINDOW w AS (ORDER BY detection_date ROWS BETWEEN {lead} PRECEDING AND CURRENT ROW)
)
SELECT
    strftime(detection_date, '%Y-%m-%d') AS det_date,
    daily_detections,
    moving_avg,
    rolling_min,
    rolling_max
FROM smoothed
WHERE detection_date >= (SELECT shown_from FROM bounds)
ORDER BY detection_date"
        )
    }
}

/// Year-over-year comparison: each recent week against the same days of the
/// week 52 weeks earlier.
#[derive(Debug, Clone)]
pub struct YearOverYear {
    /// Number of weeks to compare, the running week included (default: 52).
    pub weeks: u32,
}

impl Default for YearOverYear {
    fn default() -> Self {
        Self { weeks: 52 }
    }
}

impl QueryPlan for YearOverYear {
    fn sql(&self) -> String {
        let back = self.weeks.saturating_sub(1);
        // One row per week, for the `weeks` weeks ending with the one
        // yesterday belongs to (so a Monday does not open on an empty week).
        //
        // Each week is compared over the *same days* in both years: the
        // complete days of it so far this year — the running week stops at
        // yesterday — against the same number of days from the Monday 52
        // weeks earlier. It compared this week's partial count with last
        // year's whole seven days, so every current week read as a decline.
        //
        // A week the station was running and heard nothing in is a zero, not
        // a missing row: the weeks are a spine, not the weeks that happened to
        // have detections. A span that ends before the station's first
        // detection has no count at all, so a year the station did not exist
        // still has no delta (rather than a delta against zero).
        format!(
            "WITH station AS (
    SELECT MIN(detection_date)::TIMESTAMP AS first_day FROM detections_ts
),
weeks AS (
    SELECT
        range AS week_start,
        LEAST(range + INTERVAL 7 DAY, CURRENT_DATE::TIMESTAMP) AS week_end
    FROM range(
        date_trunc('week', CURRENT_DATE - INTERVAL 1 DAY)::TIMESTAMP - INTERVAL {back} WEEK,
        CURRENT_DATE::TIMESTAMP,
        INTERVAL 7 DAY
    )
),
spans AS (
    SELECT
        week_start,
        week_end,
        week_start - INTERVAL 52 WEEK AS prior_start,
        week_end - INTERVAL 52 WEEK   AS prior_end
    FROM weeks
),
daily AS (
    SELECT detection_date, Com_Name, COUNT(*) AS n
    FROM detections_ts
    WHERE detection_date >= (SELECT MIN(prior_start) FROM spans)
    GROUP BY detection_date, Com_Name
),
cur AS (
    SELECT s.week_start, SUM(d.n) AS n, COUNT(DISTINCT d.Com_Name) AS sp
    FROM spans s
    JOIN daily d ON d.detection_date >= s.week_start AND d.detection_date < s.week_end
    GROUP BY s.week_start
),
pri AS (
    SELECT s.week_start, SUM(d.n) AS n, COUNT(DISTINCT d.Com_Name) AS sp
    FROM spans s
    JOIN daily d ON d.detection_date >= s.prior_start AND d.detection_date < s.prior_end
    GROUP BY s.week_start
),
counted AS (
    SELECT
        s.week_start,
        s.week_end,
        s.prior_end,
        COALESCE(cur.n, 0)::BIGINT  AS cur_n,
        COALESCE(cur.sp, 0)::BIGINT AS cur_sp,
        COALESCE(pri.n, 0)::BIGINT  AS pri_n,
        COALESCE(pri.sp, 0)::BIGINT AS pri_sp
    FROM spans s
    LEFT JOIN cur USING (week_start)
    LEFT JOIN pri USING (week_start)
),
judged AS (
    SELECT
        c.*,
        c.week_end > st.first_day  AS cur_ran,
        c.prior_end > st.first_day AS pri_ran
    FROM counted c, station st
)
SELECT
    strftime(week_start, '%Y-%m-%d') AS week_start,
    cur_n AS current_year_count,
    CASE WHEN pri_ran THEN pri_n END AS prior_year_count,
    CASE WHEN pri_ran THEN cur_n - pri_n END AS yoy_delta,
    cur_sp AS current_year_species,
    CASE WHEN pri_ran THEN pri_sp END AS prior_year_species
FROM judged
WHERE cur_ran
ORDER BY week_start"
        )
    }
}

/// Fewest earlier days a day's anomaly baseline must hold before the day is
/// judged at all (fewer when the window itself is shorter).
pub const MIN_BASELINE_DAYS: u32 = 7;

/// Anomaly detection: days whose detection count deviates > N standard deviations
/// from the rolling mean of the days before it.
///
/// A day with fewer than [`MIN_BASELINE_DAYS`] earlier days in its window, or
/// whose earlier days all had the same count, is `normal`: there is no
/// baseline to be anomalous against.
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
        // A day is only judged against a baseline it can be judged against:
        // at least `MIN_BASELINE_DAYS` earlier days (or the whole window, if
        // that is shorter), with some spread among them. `STDDEV_SAMP` is
        // defined from two points, so a new station's third day scored z ≈ 34
        // off two days of history; and a baseline of identical days has a
        // deviation of zero, an undefined z, and flagged any change at all as
        // `high` beside a blank z-score.
        let min_history = window.clamp(1, MIN_BASELINE_DAYS);
        format!(
            "WITH counts AS (
    SELECT detection_date, COUNT(*) AS n
    FROM detections_ts
    WHERE detection_date >= CURRENT_DATE - INTERVAL {lookback} DAYS
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
),
daily AS (
    SELECT s.detection_date, COALESCE(c.n, 0) AS detections
    FROM spine s LEFT JOIN counts c USING (detection_date)
),
with_stats AS (
    SELECT
        detection_date,
        detections,
        AVG(detections) OVER prior AS rolling_mean,
        STDDEV_SAMP(detections) OVER prior AS rolling_stddev,
        COUNT(*) OVER prior AS prior_days
    FROM daily
    WINDOW prior AS (
        ORDER BY detection_date
        RANGE BETWEEN INTERVAL {window} DAYS PRECEDING AND INTERVAL 1 DAY PRECEDING
    )
)
SELECT
    strftime(detection_date, '%Y-%m-%d') AS detection_date,
    detections,
    rolling_mean,
    rolling_stddev,
    (detections - rolling_mean) / NULLIF(rolling_stddev, 0) AS z_score,
    CASE
        WHEN prior_days < {min_history} THEN 'normal'
        WHEN rolling_stddev IS NULL OR rolling_stddev = 0 THEN 'normal'
        WHEN detections > rolling_mean + {z} * rolling_stddev THEN 'high'
        WHEN detections < rolling_mean - {z} * rolling_stddev THEN 'low'
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
    fn moving_avg_sql_has_trailing_row_frame() {
        let q = MovingAverage::default();
        let sql = q.sql();
        assert!(sql.contains("ROWS BETWEEN 6 PRECEDING AND CURRENT ROW"));
        assert!(sql.contains("moving_avg"));
    }

    #[test]
    fn yoy_sql_compares_against_52_weeks_earlier() {
        let q = YearOverYear::default();
        let sql = q.sql();
        assert!(sql.contains("INTERVAL 52 WEEK"));
        assert!(sql.contains("yoy_delta"));
    }

    #[test]
    fn anomaly_sql_has_stddev() {
        let q = AnomalyDetection::default();
        let sql = q.sql();
        // Sample deviation of the days *before* the one judged (ANA13).
        assert!(sql.contains("STDDEV_SAMP"));
        assert!(sql.contains("AND INTERVAL 1 DAY PRECEDING"));
        assert!(sql.contains("anomaly_flag"));
    }
}
