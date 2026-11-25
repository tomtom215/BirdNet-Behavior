//! Trend analysis queries: moving averages, year-over-year, and anomalies.
//!
//! These queries smooth or contextualise the raw daily/hourly counts to
//! surface longer-range patterns — useful for understanding whether
//! bird activity is increasing, decreasing, or unusual.

use super::QueryPlan;

/// The share of the station's typical day a local date must have recorded to
/// count as a day the station listened to: half.
///
/// Below it, a day's detections say more about the station than the birds. A
/// day that lost most of its hours to an outage reads as a collapse, and if it
/// is judged at all it is judged as a `low` anomaly and then sits in the
/// baseline the next month is judged against. Half is a line, not a
/// measurement: a day above it may still have missed the dawn chorus, and is
/// labelled partial (see [`FULL_DAY_SHARE`]) rather than excluded.
///
/// The typical day is the median of the station's days with any audio in the
/// ledger, not the clock's 24 hours: the recording schedule (`all-day`,
/// `solar`, `fixed:`, night inhibit) is resolved by the capture supervisor and
/// not recorded where the analytics store can read it, and a fraction of 24
/// hours judged every day of a station recording ten hours a day as not
/// listened to. A station whose ledger holds only zeros has no typical day,
/// and none of its days is listened to.
///
/// `recording_effort` holds seconds per date *and source*; a day's effort is
/// the sum over sources.
pub const EFFORT_FLOOR_SHARE: f64 = 0.5;

/// A listened day with less of its typical day recorded than this share is
/// labelled partial: on an all-day station, about 23 hours.
///
/// The ledger is credited in five-minute ticks, and a restart loses up to one
/// of them, so a day a few minutes short of typical is a whole day. An hour
/// short is not noise.
pub const FULL_DAY_SHARE: f64 = 0.95;

/// The `effort` and `ledger` CTEs every effort-aware trend query joins.
///
/// `effort` is the recorded seconds per local date, summed over sources;
/// `ledger` is the first date the ledger holds anything for. With
/// `has_ledger` false (the connection has no `recording_effort` table; a
/// store opened through `AnalyticsDb::open` always has one) both are empty, so the queries
/// still bind and every day is judged as it was before the ledger existed.
pub(super) const fn effort_ctes(has_ledger: bool) -> &'static str {
    if has_ledger {
        "effort AS (
    SELECT TRY_CAST(date AS DATE) AS day, SUM(seconds) AS seconds
    FROM recording_effort
    WHERE TRY_CAST(date AS DATE) IS NOT NULL
    GROUP BY day
),
ledger AS (
    SELECT MIN(day) AS first_day,
           MEDIAN(seconds) FILTER (WHERE seconds > 0) AS typical
    FROM effort
),"
    } else {
        "effort AS (
    SELECT * FROM (VALUES (NULL::DATE, NULL::DOUBLE)) t(day, seconds) WHERE false
),
ledger AS (
    SELECT NULL::DATE AS first_day, NULL::DOUBLE AS typical
),"
    }
}

/// The recorded seconds for the date `day` (a column), given `effort e` and
/// `ledger l` joined to it: `NULL` where the ledger says nothing about that
/// date — no ledger at all, or a date before its first row — and the sum, or
/// zero for a date it has no row for, from its first row on.
///
/// Unknown is not zero. A station imported from BirdNET-Pi, or one that ran
/// before the ledger was added, has years of days nobody measured; reading
/// them as unlistened would blank its whole history.
pub(super) fn effort_seconds(day: &str) -> String {
    format!(
        "CASE WHEN l.first_day IS NOT NULL AND {day} >= l.first_day \
         THEN COALESCE(e.seconds, 0) END"
    )
}

/// `seconds` (a column holding [`effort_seconds`]) is a day the station
/// listened to: unknown, or at least [`EFFORT_FLOOR_SHARE`] of the typical day
/// (`typical`, the `ledger` CTE's column of that name).
pub(super) fn listened(seconds: &str, typical: &str) -> String {
    format!("({seconds} IS NULL OR COALESCE({seconds} >= {EFFORT_FLOOR_SHARE} * {typical}, false))")
}

/// `seconds` is a listened day short of [`FULL_DAY_SHARE`] of the typical day.
fn partial(seconds: &str, typical: &str) -> String {
    format!(
        "COALESCE({seconds} >= {EFFORT_FLOOR_SHARE} * {typical} \
         AND {seconds} < {FULL_DAY_SHARE} * {typical}, false)"
    )
}

/// N-day trailing moving average of daily detections.
///
/// Each day's average is over that day and the `window_days - 1` days before
/// it, and is only reported when all of those days are days the station
/// listened to — otherwise it is `NULL`, never an average over whichever days
/// happened to be in reach.
///
/// "Listened to" is the `recording_effort` ledger's answer: at least
/// [`EFFORT_FLOOR_SHARE`] of its typical day of audio recorded that local date. A day below it
/// keeps its raw count, and its own average and that of every day whose window
/// holds it are `NULL`. Where the ledger has nothing to say — no
/// `recording_effort` table, an empty one, or a date before its first row —
/// a day counts as listened, which is what "running" meant before the ledger
/// existed: on or after the station's first detection. Such a station still
/// reads an outage as a run of zeros.
///
/// Four things this used to get wrong, each of which moved the line:
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
/// * **Outages.** The doc said the average was only reported over days "the
///   station was running", and "running" meant after its first detection: a
///   week the station was switched off was a week of zeros, and the line fell
///   through it. Those days are now excluded by the ledger.
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
    /// Whether the store has a `recording_effort` table to read. `false`
    /// judges every day as listened; the executor sets it from the catalog.
    pub has_effort_ledger: bool,
}

impl Default for MovingAverage {
    fn default() -> Self {
        Self {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 90 DAYS".into()),
            to_date: None,
            species: None,
            has_effort_ledger: false,
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
        let effort = effort_ctes(self.has_effort_ledger);
        let seconds = effort_seconds("s.detection_date");
        let listened = listened("effort_seconds", "typical");
        let partial = partial("effort_seconds", "typical");
        format!(
            "WITH {effort}
bounds AS (
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
    SELECT
        s.detection_date,
        COALESCE(c.n, 0) AS daily_detections,
        {seconds} AS effort_seconds,
        l.typical AS typical
    FROM spine s
    LEFT JOIN counts c USING (detection_date)
    LEFT JOIN effort e ON e.day = s.detection_date
    CROSS JOIN ledger l
),
judged AS (
    SELECT *, {listened} AS listened, {partial} AS partial_effort
    FROM daily
),
smoothed AS (
    SELECT
        detection_date,
        daily_detections,
        effort_seconds,
        listened,
        partial_effort,
        SUM(CASE WHEN listened THEN 1 ELSE 0 END) OVER w = {n} AS full_window,
        AVG(daily_detections) OVER w AS avg_n,
        MIN(daily_detections) OVER w AS min_n,
        MAX(daily_detections) OVER w AS max_n
    FROM judged
    WINDOW w AS (ORDER BY detection_date ROWS BETWEEN {lead} PRECEDING AND CURRENT ROW)
)
SELECT
    strftime(detection_date, '%Y-%m-%d') AS det_date,
    daily_detections,
    CASE WHEN full_window THEN avg_n END AS moving_avg,
    CASE WHEN full_window THEN min_n END AS rolling_min,
    CASE WHEN full_window THEN max_n END AS rolling_max,
    effort_seconds / 3600.0 AS effort_hours,
    listened,
    partial_effort
FROM smoothed
WHERE detection_date >= (SELECT shown_from FROM bounds)
ORDER BY detection_date"
        )
    }
}

/// Year-over-year comparison: each recent week against the same days of the
/// week 52 weeks earlier.
///
/// Two comparisons per week. The raw one (`yoy_delta`) is detections against
/// detections over the same span of days, whatever the station was doing on
/// them. The effort one (`per_listened_day_delta`) is detections per listened
/// day — the detections on days with at least [`EFFORT_FLOOR_SHARE`] of its typical day of
/// recorded audio, over the number of such days — so a week of downtime in
/// either year is left out of that year's figure rather than read as a
/// decline. Days the ledger has nothing to say about count as listened, as in
/// [`MovingAverage`]; on a station without the ledger the two comparisons
/// differ only by the span's length.
#[derive(Debug, Clone)]
pub struct YearOverYear {
    /// Number of weeks to compare, the running week included (default: 52).
    pub weeks: u32,
    /// Whether the store has a `recording_effort` table to read; see
    /// [`MovingAverage::has_effort_ledger`].
    pub has_effort_ledger: bool,
}

impl Default for YearOverYear {
    fn default() -> Self {
        Self {
            weeks: 52,
            has_effort_ledger: false,
        }
    }
}

impl QueryPlan for YearOverYear {
    fn sql(&self) -> String {
        let back = self.weeks.saturating_sub(1);
        let effort = effort_ctes(self.has_effort_ledger);
        let listened = listened(&effort_seconds("d.day"), "l.typical");
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
        //
        // The per-listened-day figures count only days from the station's
        // first detection: a day before it was not a day of listening, ledger
        // or none.
        format!(
            "WITH {effort}
station AS (
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
day_totals AS (
    SELECT detection_date, SUM(n) AS n FROM daily GROUP BY detection_date
),
listened_days AS (
    SELECT d.day, COALESCE(t.n, 0) AS n
    FROM (
        SELECT CAST(range AS DATE) AS day
        FROM range(
            GREATEST((SELECT MIN(prior_start) FROM spans), (SELECT first_day FROM station)),
            CURRENT_DATE::TIMESTAMP,
            INTERVAL 1 DAY
        )
    ) d
    LEFT JOIN day_totals t ON t.detection_date = d.day
    LEFT JOIN effort e ON e.day = d.day
    CROSS JOIN ledger l
    WHERE {listened}
),
cur_l AS (
    SELECT s.week_start, COUNT(*) AS days, SUM(ld.n) AS n
    FROM spans s
    JOIN listened_days ld ON ld.day >= s.week_start AND ld.day < s.week_end
    GROUP BY s.week_start
),
pri_l AS (
    SELECT s.week_start, COUNT(*) AS days, SUM(ld.n) AS n
    FROM spans s
    JOIN listened_days ld ON ld.day >= s.prior_start AND ld.day < s.prior_end
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
        COALESCE(pri.sp, 0)::BIGINT AS pri_sp,
        COALESCE(cur_l.days, 0)::BIGINT     AS cur_days,
        cur_l.n / NULLIF(cur_l.days, 0)     AS cur_rate,
        COALESCE(pri_l.days, 0)::BIGINT     AS pri_days,
        pri_l.n / NULLIF(pri_l.days, 0)     AS pri_rate
    FROM spans s
    LEFT JOIN cur USING (week_start)
    LEFT JOIN pri USING (week_start)
    LEFT JOIN cur_l USING (week_start)
    LEFT JOIN pri_l USING (week_start)
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
    CASE WHEN pri_ran THEN pri_sp END AS prior_year_species,
    cur_days AS current_listened_days,
    CASE WHEN pri_ran THEN pri_days END AS prior_listened_days,
    cur_rate::DOUBLE AS current_per_listened_day,
    (CASE WHEN pri_ran THEN pri_rate END)::DOUBLE AS prior_per_listened_day,
    (CASE WHEN pri_ran THEN cur_rate - pri_rate END)::DOUBLE AS per_listened_day_delta
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
///
/// A day with less than [`EFFORT_FLOOR_SHARE`] of its typical day of recorded audio is
/// `unlistened`: it is not judged (its `z_score` is `NULL`) and it is left out
/// of every later day's baseline, so an outage is neither a `low` anomaly
/// itself nor a run of zeros that widens the next month's deviation. It was
/// both: the spine zero-filled every day from the first detection, ledger or
/// not. Days the ledger has nothing to say about are judged as they always
/// were — see [`MovingAverage`] — so a station without the ledger still
/// flags its outages `low`.
#[derive(Debug, Clone)]
pub struct AnomalyDetection {
    /// Z-score threshold for flagging a day as anomalous (default: 2.0).
    pub z_threshold: f64,
    /// Rolling window in days for computing mean and stddev (default: 30).
    pub window_days: u32,
    /// Look back this many days total (default: 180).
    pub lookback_days: u32,
    /// Whether the store has a `recording_effort` table to read; see
    /// [`MovingAverage::has_effort_ledger`].
    pub has_effort_ledger: bool,
}

impl Default for AnomalyDetection {
    fn default() -> Self {
        Self {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 180,
            has_effort_ledger: false,
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
        let effort = effort_ctes(self.has_effort_ledger);
        let seconds = effort_seconds("s.detection_date");
        let listened = listened("effort_seconds", "typical");
        let partial = partial("effort_seconds", "typical");
        format!(
            "WITH {effort}
counts AS (
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
    SELECT
        s.detection_date,
        COALESCE(c.n, 0) AS detections,
        {seconds} AS effort_seconds,
        l.typical AS typical
    FROM spine s
    LEFT JOIN counts c USING (detection_date)
    LEFT JOIN effort e ON e.day = s.detection_date
    CROSS JOIN ledger l
),
judged AS (
    SELECT *, {listened} AS listened, {partial} AS partial_effort
    FROM daily
),
with_stats AS (
    SELECT
        detection_date,
        detections,
        effort_seconds,
        listened,
        partial_effort,
        AVG(CASE WHEN listened THEN detections END) OVER prior AS rolling_mean,
        STDDEV_SAMP(CASE WHEN listened THEN detections END) OVER prior AS rolling_stddev,
        COUNT(CASE WHEN listened THEN 1 END) OVER prior AS prior_days
    FROM judged
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
    CASE WHEN listened THEN (detections - rolling_mean) / NULLIF(rolling_stddev, 0) END AS z_score,
    CASE
        WHEN NOT listened THEN 'unlistened'
        WHEN prior_days < {min_history} THEN 'normal'
        WHEN rolling_stddev IS NULL OR rolling_stddev = 0 THEN 'normal'
        WHEN detections > rolling_mean + {z} * rolling_stddev THEN 'high'
        WHEN detections < rolling_mean - {z} * rolling_stddev THEN 'low'
        ELSE 'normal'
    END AS anomaly_flag,
    effort_seconds / 3600.0 AS effort_hours,
    listened,
    partial_effort
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
