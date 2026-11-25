//! When the station was listening, for the charts that must tell a quiet day from
//! an unrecorded one.
//!
//! A day with no detections is two different facts. Either the station listened all
//! day and heard nothing (a fog, a gale, an empty yard), or it was switched off, or
//! its microphone was down, and heard nothing because it was not there. A chart that
//! draws both as a zero tells the owner their yard was silent on the day the power
//! failed. The `recording_effort` ledger (migration 27) is the station's own account
//! of how many seconds of audio it analysed (captured, before migration 63), per local
//! date and source, so it can say which.
//!
//! # A listened day (RES-26)
//!
//! One rule on every surface: a local date is a day the station listened to when the
//! ledger credits it with at least half the station's typical day of audio, summed
//! over sources (`birdnet_timeseries::queries::trend::EFFORT_FLOOR_SHARE` and
//! `typical_day`, the same rule and the same typical day as the trend queries and the
//! species page's reporting frequency). The typical day is the median, over every date
//! the ledger holds with any audio, of the seconds recorded that date. It is the
//! station's own day and not the clock's: a station on a two-hours-a-day schedule
//! listened on every one of its days. This used to be a fixed 60 seconds, so a
//! station that was up for a minute before a power cut "listened" to that day here
//! and not on the species page.
//!
//! # A day the ledger does not cover
//!
//! The charts use the ledger to *add* a statement ("not listening"). Two summaries also
//! divide by the days it counts: Year in review's daily average (UIC-01) and the weekly
//! report's comparison with the week before (UIR-08). Either way it is used only where
//! the ledger holds something for the period at all: a station imported from
//! BirdNET-Pi has no ledger, and a chart claiming it was "off" every day would be a lie
//! of the opposite kind. No rows in the period means unknown, and unknown draws
//! nothing. Within a period that has rows, a date without one is off. That is where
//! these surfaces still differ from the other two: the trend queries count a date
//! before the ledger's first row as listened (a line through imported years must not
//! go blank), and the reporting frequency leaves it out of both sides of its share (a
//! date nobody measured cannot be a trial). Here the question is asked of a period, so
//! a period that straddles the ledger's first row draws its earlier dates as off.

#![allow(clippy::cast_possible_wrap)]

use std::collections::BTreeMap;

use birdnet_timeseries::queries::trend::{is_listened_day, typical_day};

use super::viz::kit;

/// The station's whole ledger as the listened-day rule reads it: the seconds of each
/// local date, summed over sources, and its typical day.
pub struct StationLedger {
    daily: BTreeMap<String, f64>,
    typical: Option<f64>,
}

impl StationLedger {
    /// Read every date of the ledger. A few thousand rows on a station of many years.
    ///
    /// # Errors
    ///
    /// A database error, returned and not defaulted.
    pub fn read(conn: &rusqlite::Connection) -> Result<Self, rusqlite::Error> {
        // `date(date) IS NOT NULL` is the analytics copy's `TRY_CAST(date AS DATE)`: a row
        // that names no date has no day to belong to.
        let mut stmt = conn.prepare(
            "SELECT date, SUM(seconds) FROM recording_effort \
             WHERE date(date) IS NOT NULL GROUP BY date",
        )?;
        let daily = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let typical = typical_day(daily.values().copied());
        Ok(Self { daily, typical })
    }

    /// The typical day, in seconds; `None` when no date has any audio.
    pub const fn typical(&self) -> Option<f64> {
        self.typical
    }

    /// The first date the ledger holds.
    pub fn first(&self) -> Option<&str> {
        self.daily.keys().next().map(String::as_str)
    }

    /// The seconds recorded on `date`, summed over sources; zero for a date with no row.
    pub fn seconds(&self, date: &str) -> f64 {
        self.daily.get(date).copied().unwrap_or(0.0)
    }

    /// Whether the station listened to `date`, for a date the ledger covers (from its
    /// first row on); `None` for one it does not (no ledger, or before its first row),
    /// which each figure answers for itself.
    pub fn listened(&self, date: &str) -> Option<bool> {
        let first = self.first()?;
        (date >= first).then(|| is_listened_day(self.seconds(date), self.typical))
    }

    /// The ledger's dates over `from..=to` with the rule applied: a listened date keeps its
    /// seconds, a date short of the floor reads zero. Empty when the ledger holds no row in
    /// the period (unknown).
    pub fn judged(&self, from: &str, to: &str) -> BTreeMap<String, f64> {
        self.daily
            .range(from.to_owned()..=to.to_owned())
            .map(|(d, s)| {
                let kept = if is_listened_day(*s, self.typical) {
                    *s
                } else {
                    0.0
                };
                (d.clone(), kept)
            })
            .collect()
    }
}

/// The ledger's dates over `from..=to` (ISO dates), judged by the listened-day rule: the
/// seconds recorded on each date the station listened to, summed over sources, and zero
/// for a date it holds that fell short of half the typical day. A date with no row is not
/// in the map. Every surface that hatches "not listening" or divides by listened days
/// reads this, so they cannot disagree about a day.
///
/// The values stay seconds so the chart kit's calendar (`viz::heat::Calendar::effort`)
/// can take them; a listened day is always at least half a typical day.
///
/// # Errors
///
/// A database error is returned, not defaulted: the caller decides whether a chart
/// without the ledger is still honest (it is, it just says less).
pub fn effort_by_date(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
) -> Result<BTreeMap<String, f64>, rusqlite::Error> {
    Ok(StationLedger::read(conn)?.judged(from, to))
}

/// Whether a date of a judged map ([`effort_by_date`]) was listened to.
fn on(effort: &BTreeMap<String, f64>, date: &str) -> bool {
    effort.get(date).is_some_and(|s| *s > 0.0)
}

/// How many days of `effort` ([`effort_by_date`]) the station listened on, or `None` when
/// the ledger has no row (unknown, so nothing is claimed).
pub fn listened_day_count(effort: &BTreeMap<String, f64>) -> Option<usize> {
    (!effort.is_empty()).then(|| effort.values().filter(|s| **s > 0.0).count())
}

/// For each of the `days` ending on `end` (oldest first): was the station listening?
///
/// `None` when the ledger has no row anywhere in the period (unknown, so nothing is
/// claimed). The last day is never reported as off: the recorder credits on a timer, so
/// the first minutes of a day have no row yet.
pub fn listened_days(effort: &BTreeMap<String, f64>, end: &str, days: usize) -> Option<Vec<bool>> {
    if effort.is_empty() || days == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(days);
    for d in 0..days {
        let back = d as i64 - (days as i64 - 1);
        let listened = kit::shift_date(end, back)
            .is_none_or(|(y, m, dd)| back == 0 || on(effort, &format!("{y:04}-{m:02}-{dd:02}")));
        out.push(listened);
    }
    Some(out)
}

/// For each of `dates` (ISO, any order of days): was the station listening? `None` when
/// the ledger has nothing for the period. `today` is never off.
pub fn listened_on(
    effort: &BTreeMap<String, f64>,
    dates: &[String],
    today: &str,
) -> Option<Vec<bool>> {
    if effort.is_empty() || dates.is_empty() {
        return None;
    }
    Some(dates.iter().map(|d| d == today || on(effort, d)).collect())
}

/// The first date of the `days` ending on `end`, as `YYYY-MM-DD`.
pub fn first_date(end: &str, days: usize) -> Option<String> {
    let (y, m, d) = kit::shift_date(end, -(days as i64 - 1))?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A judged map, as [`effort_by_date`] returns it: a date the ledger holds that fell
    /// short of the floor reads zero.
    fn ledger(rows: &[(&str, f64)]) -> BTreeMap<String, f64> {
        rows.iter().map(|(d, s)| ((*d).to_string(), *s)).collect()
    }

    /// The whole point: a day with no row is *off*, a day with a row is listened to
    /// whether or not it heard anything, and a minute of audio is not a day.
    #[test]
    fn a_day_without_a_row_was_not_listened_to() {
        let l = ledger(&[
            ("2026-10-18", 86_400.0),
            ("2026-10-20", 0.0),
            ("2026-10-21", 0.0),
        ]);
        let got = listened_days(&l, "2026-10-21", 4).expect("the ledger has rows");
        // 18th listened, 19th no row, 20th short of the floor, 21st (today) never "off".
        assert_eq!(got, vec![true, false, false, true]);
    }

    /// No ledger at all (an imported station) is unknown, and unknown claims nothing.
    #[test]
    fn an_empty_ledger_claims_nothing() {
        assert!(listened_days(&BTreeMap::new(), "2026-10-21", 7).is_none());
        assert!(listened_days(&ledger(&[("2026-10-21", 1.0)]), "2026-10-21", 0).is_none());
    }

    #[test]
    fn listened_on_asks_about_exactly_the_dates_given() {
        let l = ledger(&[("2026-10-01", 3600.0), ("2026-10-03", 0.0)]);
        let dates: Vec<String> = ["2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(
            listened_on(&l, &dates, "2026-10-04"),
            Some(vec![true, false, false, true])
        );
        assert!(listened_on(&BTreeMap::new(), &dates, "2026-10-04").is_none());
    }

    /// A station's `SQLite` store holding `rows` of `(date, source, seconds)`.
    fn station(rows: &[(String, &str, f64)]) -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = birdnet_db::sqlite::open_or_create(&dir.path().join("birds.db")).expect("db");
        for (date, source, seconds) in rows {
            conn.execute(
                "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, ?2, ?3)",
                rusqlite::params![date, source, seconds],
            )
            .expect("effort");
        }
        (dir, conn)
    }

    /// Every date of the given `(year, month, first day, last day)` runs, in order.
    fn span(runs: &[(i32, u32, u32, u32)]) -> Vec<String> {
        runs.iter()
            .flat_map(|&(y, m, a, b)| (a..=b).map(move |d| format!("{y:04}-{m:02}-{d:02}")))
            .collect()
    }

    /// RES-26. A station of whole days (86 400 s) from 25 Dec 2023 to 10 Mar 2024, but for
    /// 61 s on 31 Dec (the year's last day) and on 29 Feb (a leap day), and two sources of
    /// 30 000 s on 15 Jan. Its typical day is the median of 77 daily sums, 74 of them 86 400:
    /// 86 400, and the floor is 43 200. The two 61-second days are not listened to; 15 Jan
    /// (60 000 s, summed over sources) is. 77 dates, 75 listened.
    ///
    /// Under the 60-second rule both short days were whole days of listening.
    #[test]
    fn a_minute_of_audio_is_not_a_day_of_listening() {
        let dates = span(&[
            (2023, 12, 25, 31),
            (2024, 1, 1, 31),
            (2024, 2, 1, 29),
            (2024, 3, 1, 10),
        ]);
        assert_eq!(dates.len(), 77);
        let mut rows = Vec::new();
        for d in &dates {
            match d.as_str() {
                "2023-12-31" | "2024-02-29" => rows.push((d.clone(), "local", 61.0)),
                "2024-01-15" => {
                    rows.push((d.clone(), "local", 30_000.0));
                    rows.push((d.clone(), "rtsp_1", 30_000.0));
                }
                _ => rows.push((d.clone(), "local", 86_400.0)),
            }
        }
        let (_dir, conn) = station(&rows);
        let effort = effort_by_date(&conn, "2023-12-25", "2024-03-10").expect("read");
        assert_eq!(listened_day_count(&effort), Some(75));
        // `today` is the day after, so no date of the period is exempt.
        let on = listened_on(&effort, &dates, "2024-03-11").expect("the ledger has rows");
        let off: Vec<&str> = dates
            .iter()
            .zip(&on)
            .filter(|(_, l)| !**l)
            .map(|(d, _)| d.as_str())
            .collect();
        assert_eq!(off, ["2023-12-31", "2024-02-29"]);
        // The same verdicts by position, for the charts that ask by a span of days ending
        // on a today that is exempt (11 Mar has no row and is never "off").
        let to_11th = effort_by_date(&conn, "2023-12-25", "2024-03-11").expect("read");
        let by_span = listened_days(&to_11th, "2024-03-11", 78).expect("rows");
        assert_eq!(&by_span[..77], on.as_slice());
        // Across the year boundary: 28 Dec – 2 Jan is six dates, the 31st off.
        let effort = effort_by_date(&conn, "2023-12-28", "2024-01-02").expect("read");
        assert_eq!(listened_day_count(&effort), Some(5));
    }

    /// Counterpart: the floor is relative to the station's own typical day. A station on a
    /// two-hours-a-day schedule (7 200 s each day, 1 Jan – 10 Mar 2024, 70 days) has a typical
    /// day of 7 200 and listened on every one of them.
    #[test]
    fn a_short_schedule_is_still_a_day_of_listening_every_day() {
        let dates = span(&[(2024, 1, 1, 31), (2024, 2, 1, 29), (2024, 3, 1, 10)]);
        assert_eq!(dates.len(), 70);
        let rows: Vec<_> = dates
            .iter()
            .map(|d| (d.clone(), "local", 7_200.0))
            .collect();
        let (_dir, conn) = station(&rows);
        let effort = effort_by_date(&conn, "2024-01-01", "2024-03-10").expect("read");
        assert_eq!(listened_day_count(&effort), Some(70));
        assert_eq!(
            listened_days(&effort, "2024-03-10", 70).map(|v| v.iter().all(|l| *l)),
            Some(true)
        );
    }

    /// Both sides of the floor. Five dates: [43 199, 43 200, 86 400 × 3], so the typical day
    /// is 86 400 and the floor 43 200: the 29th is listened, 1 Mar a second short is not.
    #[test]
    fn half_the_typical_day_is_the_line() {
        let rows: Vec<_> = [
            ("2024-02-26", 86_400.0),
            ("2024-02-27", 86_400.0),
            ("2024-02-28", 86_400.0),
            ("2024-02-29", 43_200.0),
            ("2024-03-01", 43_199.0),
        ]
        .iter()
        .map(|(d, s)| ((*d).to_string(), "local", *s))
        .collect();
        let (_dir, conn) = station(&rows);
        let effort = effort_by_date(&conn, "2024-02-26", "2024-03-01").expect("read");
        let dates = vec!["2024-02-29".to_string(), "2024-03-01".to_string()];
        assert_eq!(
            listened_on(&effort, &dates, "2024-03-02"),
            Some(vec![true, false])
        );
    }

    /// A period the ledger holds no row in is unknown even when the station has a typical
    /// day from other dates: the charts claim nothing for it.
    #[test]
    fn a_period_the_ledger_does_not_cover_claims_nothing() {
        let (_dir, conn) = station(&[("2024-03-01".to_string(), "local", 86_400.0)]);
        let effort = effort_by_date(&conn, "2024-01-01", "2024-01-31").expect("read");
        assert_eq!(listened_day_count(&effort), None);
        assert!(listened_days(&effort, "2024-01-31", 31).is_none());
    }

    #[test]
    fn the_period_starts_where_the_chart_does() {
        assert_eq!(first_date("2026-10-21", 7).as_deref(), Some("2026-10-15"));
        assert_eq!(first_date("2026-03-02", 5).as_deref(), Some("2026-02-26"));
        assert_eq!(first_date("nonsense", 5), None);
    }
}
