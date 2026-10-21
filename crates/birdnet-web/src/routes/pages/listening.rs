//! When the station was listening, for the charts that must tell a quiet day from
//! an unrecorded one.
//!
//! A day with no detections is two different facts. Either the station listened all
//! day and heard nothing (a fog, a gale, an empty yard), or it was switched off, or
//! its microphone was down, and heard nothing because it was not there. A chart that
//! draws both as a zero tells the owner their yard was silent on the day the power
//! failed. The `recording_effort` ledger (migration 27) is the station's own account
//! of how many seconds of audio it captured, per local date and source, so it can say
//! which.
//!
//! It is only ever used to *add* a statement ("not listening"), never to scale or
//! correct a count, and only where the ledger holds something for the period at all:
//! a station imported from BirdNET-Pi has no ledger, and a chart claiming it was "off"
//! every day would be a lie of the opposite kind. No rows in the period means unknown,
//! and unknown draws nothing.

use std::collections::BTreeMap;

use super::viz::kit;

/// Seconds below which a day counts as not listened to: a station that was up for a
/// minute before a power cut did not listen to that day.
const MIN_LISTENED_SECS: f64 = 60.0;

/// Seconds recorded per local date over `from..=to` (ISO dates), summed over sources.
///
/// # Errors
///
/// A database error is returned, not defaulted: the caller decides whether a chart
/// without the ledger is still honest (it is, it just says less).
pub(crate) fn effort_by_date(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
) -> Result<BTreeMap<String, f64>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT date, SUM(seconds) FROM recording_effort WHERE date >= ?1 AND date <= ?2 GROUP BY date",
    )?;
    let rows = stmt.query_map([from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
    rows.collect()
}

/// For each of the `days` ending on `end` (oldest first): was the station listening?
///
/// `None` when the ledger has no row anywhere in the period (unknown, so nothing is
/// claimed). The last day is never reported as off: the recorder credits on a timer, so
/// the first minutes of a day have no row yet.
pub(crate) fn listened_days(effort: &BTreeMap<String, f64>, end: &str, days: usize) -> Option<Vec<bool>> {
    if effort.is_empty() || days == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(days);
    for d in 0..days {
        let back = d as i64 - (days as i64 - 1);
        let on = kit::shift_date(end, back).is_none_or(|(y, m, dd)| {
            back == 0 || effort.get(&format!("{y:04}-{m:02}-{dd:02}")).is_some_and(|s| *s >= MIN_LISTENED_SECS)
        });
        out.push(on);
    }
    Some(out)
}

/// The first date of the `days` ending on `end`, as `YYYY-MM-DD`.
pub(crate) fn first_date(end: &str, days: usize) -> Option<String> {
    let (y, m, d) = kit::shift_date(end, -(days as i64 - 1))?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(rows: &[(&str, f64)]) -> BTreeMap<String, f64> {
        rows.iter().map(|(d, s)| ((*d).to_string(), *s)).collect()
    }

    /// The whole point: a day with no row is *off*, a day with a row is listened to
    /// whether or not it heard anything, and a minute of audio is not a day.
    #[test]
    fn a_day_without_a_row_was_not_listened_to() {
        let l = ledger(&[("2026-10-18", 86_400.0), ("2026-10-20", 30.0), ("2026-10-21", 5.0)]);
        let got = listened_days(&l, "2026-10-21", 4).expect("the ledger has rows");
        // 18th listened, 19th no row, 20th only 30 s, 21st (today) never "off".
        assert_eq!(got, vec![true, false, false, true]);
    }

    /// No ledger at all (an imported station) is unknown, and unknown claims nothing.
    #[test]
    fn an_empty_ledger_claims_nothing() {
        assert!(listened_days(&BTreeMap::new(), "2026-10-21", 7).is_none());
        assert!(listened_days(&ledger(&[("2026-10-21", 1.0)]), "2026-10-21", 0).is_none());
    }

    #[test]
    fn the_period_starts_where_the_chart_does() {
        assert_eq!(first_date("2026-10-21", 7).as_deref(), Some("2026-10-15"));
        assert_eq!(first_date("2026-03-02", 5).as_deref(), Some("2026-02-26"));
        assert_eq!(first_date("nonsense", 5), None);
    }
}
