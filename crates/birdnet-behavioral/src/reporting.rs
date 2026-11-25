//! Reporting frequency: on what share of the days the station listened was a
//! species heard, through the year (CHT-16 / G-01).
//!
//! # The unit
//!
//! A **listened day** is a local date the `recording_effort` ledger credits
//! with at least `floor_share` of the station's typical day of audio — the
//! same rule and the same typical day the trend queries use
//! (`birdnet_timeseries::queries::trend::EFFORT_FLOOR_SHARE`, passed in by the
//! caller so there is one number): the typical day is the median, over the
//! ledger's dates with any audio, of the seconds recorded that date summed over
//! sources. A **day heard** is a listened day with at least one detection of
//! the species that review has not rejected.
//!
//! One difference from the trend queries, on purpose: they count a date the
//! ledger says nothing about (before its first row, or no ledger at all) as
//! listened, because a line through years of imported history must not go
//! blank. A share cannot do that. Its denominator *is* the listening, and a
//! date nobody measured would enter it as a day listened on the strength of a
//! guess; on an imported station every such date with a detection would also
//! be a day heard, and the share would be drawn towards one. So only the
//! ledger's own listened days are counted, and the days a species was heard
//! that the ledger does not count are reported beside the result
//! ([`ReportingFrequency::heard_days_not_counted`]) rather than dropped in
//! silence.
//!
//! # The period
//!
//! The year is cut into 48 periods, four to a month: days 1–7, 8–14, 15–21 and
//! 22 to the month's end (7 to 10 days). Periods line up with the months a
//! reader names, every year has the same boundaries, so years pool exactly (a
//! period's listened days are summed over every year the ledger covers), and
//! there is no 53rd week of one or two days. The last period of a month is
//! longer than the others, which changes how many days it can hold and nothing
//! about the share.

/// Periods in a year: four to a month.
pub const PERIODS_PER_YEAR: usize = 48;

/// The month abbreviations the labels use.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The period (`0..48`) a day of the year falls in: `month` is 1..=12 and
/// `day` 1..=31.
#[must_use]
pub fn period_of(month: u32, day: u32) -> usize {
    let quarter = (day.saturating_sub(1) / 7).min(3);
    (month.clamp(1, 12) as usize - 1) * 4 + quarter as usize
}

/// The month (0-based) a period is in.
#[must_use]
pub const fn period_month(period: usize) -> usize {
    period / 4
}

/// The days a period covers, as a reader says them: `1–7 Jan`, `22–31 Jan`.
///
/// February's last period is `22–29 Feb`: it holds the 29th in a leap year,
/// and a range that names a day some years lack is still true of every year.
#[must_use]
pub fn period_label(period: usize) -> String {
    let m = period_month(period).min(11);
    let q = period % 4;
    let month = MONTHS[m];
    let last = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m];
    match q {
        0 => format!("1–7 {month}"),
        1 => format!("8–14 {month}"),
        2 => format!("15–21 {month}"),
        _ => format!("22–{last} {month}"),
    }
}

/// The CTEs both reporting queries share: `effort` (seconds per local date,
/// summed over sources), `ledger` (the typical day), `listened` (the dates
/// with at least `floor_share` of it) and `heard` (the species' dates with a
/// detection review has not rejected; the species is the statement's one
/// parameter).
fn ctes(floor_share: f64) -> String {
    format!(
        "WITH effort AS (
            SELECT TRY_CAST(date AS DATE) AS day, SUM(seconds) AS seconds
            FROM recording_effort
            WHERE TRY_CAST(date AS DATE) IS NOT NULL
            GROUP BY day
        ),
        ledger AS (
            SELECT MEDIAN(seconds) FILTER (WHERE seconds > 0) AS typical FROM effort
        ),
        listened AS (
            SELECT e.day FROM effort e CROSS JOIN ledger l
            WHERE l.typical IS NOT NULL AND e.seconds >= {floor_share} * l.typical
        ),
        heard AS (
            SELECT DISTINCT detection_date AS day FROM detections_ts
            WHERE Com_Name = ? AND detection_date IS NOT NULL
        )"
    )
}

/// SQL for the listened and heard days of one species per period, pooled
/// over years.
///
/// Rows: `period` (0..48), `listened_days`, `heard_days`. A period
/// with no listened day has no row: it is not a period with a share of zero.
///
/// Takes one parameter, the species' stored common name.
#[must_use]
pub fn reporting_frequency_sql(floor_share: f64) -> String {
    format!(
        "{ctes}
        SELECT CAST((month(li.day) - 1) * 4 + LEAST(3, (day(li.day) - 1) // 7) AS INTEGER) AS period,
               COUNT(*) AS listened_days,
               COUNT(h.day) AS heard_days
        FROM listened li LEFT JOIN heard h ON h.day = li.day
        GROUP BY period
        ORDER BY period",
        ctes = ctes(floor_share),
    )
}

/// SQL for what the figure says beside its periods.
///
/// One row: the species' days heard in all, those the ledger does not count
/// as listened, the first and last listened dates, the years they span, and
/// the typical day in seconds.
///
/// Takes one parameter, the species' stored common name.
#[must_use]
pub fn reporting_summary_sql(floor_share: f64) -> String {
    format!(
        "{ctes}
        SELECT (SELECT COUNT(*) FROM heard) AS heard_days,
               (SELECT COUNT(*) FROM heard WHERE day NOT IN (SELECT day FROM listened))
                   AS heard_days_not_counted,
               (SELECT CAST(MIN(day) AS VARCHAR) FROM listened) AS first_listened,
               (SELECT CAST(MAX(day) AS VARCHAR) FROM listened) AS last_listened,
               (SELECT COUNT(DISTINCT year(day)) FROM listened) AS years,
               (SELECT typical FROM ledger) AS typical_seconds",
        ctes = ctes(floor_share),
    )
}

/// One period's counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ReportingPeriod {
    /// The period, `0..48` ([`period_label`] says it).
    pub period: usize,
    /// Listened days in the period, summed over years.
    pub listened_days: u32,
    /// Of those, the days the species was heard.
    pub heard_days: u32,
}

/// A species' reporting frequency through the year, and what it rests on.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ReportingFrequency {
    /// Whether the store has a listening ledger with at least one row. Without
    /// one there is no denominator, and only the days heard are filled.
    pub has_ledger: bool,
    /// The periods with at least one listened day, in order. A period that is
    /// absent was not listened to; it is not a share of zero.
    pub periods: Vec<ReportingPeriod>,
    /// Days the species was heard, in all.
    pub heard_days: u32,
    /// Days the species was heard that the ledger does not count as listened
    /// (before its first row, or short of the floor): left out of both sides of
    /// the share, and said.
    pub heard_days_not_counted: u32,
    /// The first listened date, `YYYY-MM-DD`.
    pub first_listened: Option<String>,
    /// The last listened date, `YYYY-MM-DD`.
    pub last_listened: Option<String>,
    /// How many calendar years the listened days touch.
    pub years: u32,
    /// The typical day, in seconds of audio summed over sources.
    pub typical_seconds: Option<f64>,
}

impl ReportingFrequency {
    /// The counts for `period`, if the station listened in it.
    #[must_use]
    pub fn period(&self, period: usize) -> Option<&ReportingPeriod> {
        self.periods.iter().find(|p| p.period == period)
    }
}

#[cfg(feature = "analytics")]
impl crate::connection::AnalyticsDb {
    /// The reporting frequency of `species` (its stored common name), with
    /// `floor_share` of the typical day as the listened-day floor.
    ///
    /// Plain SQL over `detections_ts` and the mirrored `recording_effort`
    /// ledger: no extension function is involved, so this does not require
    /// the behavioral extension.
    ///
    /// # Errors
    ///
    /// `InvalidData` for a floor outside `(0, 1]`; any database error.
    pub fn reporting_frequency(
        &self,
        species: &str,
        floor_share: f64,
    ) -> Result<ReportingFrequency, crate::connection::AnalyticsError> {
        use crate::connection::AnalyticsError;
        if !(floor_share.is_finite() && floor_share > 0.0 && floor_share <= 1.0) {
            return Err(AnalyticsError::InvalidData(format!(
                "a listened-day floor is a share of the typical day in (0, 1], got {floor_share}"
            )));
        }
        let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
        // A store with no ledger table (one older than it) or an empty one
        // (a fresh store, or a station whose ledger was never written) has
        // nothing to divide by. The table check comes first because a query
        // naming a table that is not there would not bind.
        let tables: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM duckdb_tables() \
             WHERE table_name = 'recording_effort' \
               AND database_name = current_database() \
               AND schema_name = current_schema()",
            [],
            |r| r.get(0),
        )?;
        let rows: i64 = if tables == 0 {
            0
        } else {
            self.conn()
                .query_row("SELECT COUNT(*) FROM recording_effort", [], |r| r.get(0))?
        };
        if rows == 0 {
            let heard: i64 = self.conn().query_row(
                "SELECT COUNT(DISTINCT detection_date) FROM detections_ts \
                 WHERE Com_Name = ? AND detection_date IS NOT NULL",
                [species],
                |r| r.get(0),
            )?;
            return Ok(ReportingFrequency {
                has_ledger: false,
                periods: Vec::new(),
                heard_days: count(heard),
                heard_days_not_counted: count(heard),
                first_listened: None,
                last_listened: None,
                years: 0,
                typical_seconds: None,
            });
        }
        let mut stmt = self.conn().prepare(&reporting_frequency_sql(floor_share))?;
        let periods = stmt
            .query_map([species], |r| {
                let period: i32 = r.get(0)?;
                let listened: i64 = r.get(1)?;
                let heard: i64 = r.get(2)?;
                Ok((period, listened, heard))
            })?
            .map(|row| {
                let (period, listened, heard) = row?;
                let period = usize::try_from(period)
                    .ok()
                    .filter(|p| *p < PERIODS_PER_YEAR)
                    .ok_or_else(|| {
                        AnalyticsError::InvalidData(format!("period {period} is not in 0..48"))
                    })?;
                Ok(ReportingPeriod {
                    period,
                    listened_days: count(listened),
                    heard_days: count(heard),
                })
            })
            .collect::<Result<Vec<_>, AnalyticsError>>()?;
        let summary =
            self.conn()
                .query_row(&reporting_summary_sql(floor_share), [species], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<f64>>(5)?,
                    ))
                })?;
        let (heard, not_counted, first, last, years, typical) = summary;
        Ok(ReportingFrequency {
            has_ledger: true,
            periods,
            heard_days: count(heard),
            heard_days_not_counted: count(not_counted),
            first_listened: first,
            last_listened: last,
            years: count(years),
            typical_seconds: typical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_is_four_periods_and_the_last_takes_the_rest() {
        assert_eq!(period_of(1, 1), 0);
        assert_eq!(period_of(1, 7), 0);
        assert_eq!(period_of(1, 8), 1);
        assert_eq!(period_of(1, 21), 2);
        assert_eq!(period_of(1, 22), 3);
        assert_eq!(period_of(1, 31), 3);
        assert_eq!(period_of(2, 29), 7);
        assert_eq!(period_of(12, 31), 47);
    }

    #[test]
    fn every_period_says_its_own_days() {
        assert_eq!(period_label(0), "1–7 Jan");
        assert_eq!(period_label(3), "22–31 Jan");
        assert_eq!(period_label(7), "22–29 Feb");
        assert_eq!(period_label(14), "15–21 Apr");
        assert_eq!(period_label(15), "22–30 Apr");
        assert_eq!(period_label(47), "22–31 Dec");
    }
}
