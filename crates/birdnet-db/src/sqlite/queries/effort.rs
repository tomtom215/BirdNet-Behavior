//! Recording-effort queries.
//!
//! `recording_effort` (migration 27) accumulates seconds of audio the detection
//! pipeline analysed, keyed `(local date, source)`; `analysis_effort`
//! (migration 61) holds the same by local hour, with the segments the shed
//! policy skipped and those lost before analysis. `src/integrations/effort.rs`
//! folds both from the pipeline's throughput every five minutes, so they are
//! the station's own record of how long it listened — independent of whether
//! it heard anything. Until migration 61 the daily table was credited from the
//! capture gauge, so it counted audio the pipeline shed or never saw (RES-01,
//! MSC-16).

use rusqlite::Connection;

use crate::sqlite::connection::DbError;

/// Total seconds of audio this station has ever recorded, across all sources.
///
/// `None` when the table is empty — a station that has not recorded yet.
///
/// # Why this exists
///
/// The detection deadman measures silence as "seconds since the last
/// detection", which is unmeasurable on a station that has **never** detected
/// anything: there is no last detection to measure from. Treating that as
/// "nothing to say" is right for the first hour after an install and wrong for
/// ever after, because a station whose microphone, gain, threshold or
/// occurrence filter is misconfigured from the start detects nothing on day one
/// and nothing on day three hundred — and that is the failure a deadman exists
/// to catch.
///
/// This is the reference the deadman needs to tell those two apart: how long
/// the station has been listening. A new station has recorded little and stays
/// quiet; one that has listened for a day and heard nothing is broken.
/// Recording effort is the right measure rather than process uptime, which a
/// restart resets, or wall-clock age, which counts the hours the station spent
/// switched off. It is audio the pipeline *analysed*, so a station whose
/// pipeline analyses nothing at all (no model) does not advance it; that case is
/// the detector-not-running condition's to report, not this one's.
///
/// Summed across sources, so a two-microphone station reaches the threshold in
/// half the wall-clock time. That is deliberate: two silent microphones are
/// twice the evidence, not half the patience.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn total_recording_seconds(conn: &Connection) -> Result<Option<u64>, DbError> {
    // `seconds` is REAL, and the truncation is done by SQLite rather than by a
    // Rust `as` cast: `f64 as u64` is lossy above 2^53 and this workspace's
    // clippy rejects it for that reason. `CAST(… AS INTEGER)` truncates toward
    // zero into an i64, which is the conversion actually wanted. `SUM` over no
    // rows is SQL NULL — the "never recorded" case — and survives as `None`.
    let secs: Option<i64> = conn.query_row(
        "SELECT CAST(SUM(seconds) AS INTEGER) FROM recording_effort",
        [],
        |row| row.get(0),
    )?;
    // A negative total cannot arise — the column is credited with elapsed
    // durations — but clamp rather than wrap if one ever did.
    Ok(secs.map(|s| u64::try_from(s).unwrap_or(0)))
}

/// One local hour's analysis effort for one source (`analysis_effort`,
/// migration 61).
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyEffort {
    /// Local recording date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour the audio was recorded in, `0..=23`.
    pub hour: u8,
    /// The source label, as `derive_source_label` gives it.
    pub source: String,
    /// Seconds of audio the pipeline finished analysing.
    pub seconds_analysed: f64,
    /// Segments analysed.
    pub segments_analysed: u64,
    /// Segments the shed policy skipped (backlog or heat).
    pub segments_shed: u64,
    /// Segments that vanished before the pipeline read them.
    pub segments_dropped: u64,
}

/// Add one hour's tally to `analysis_effort`, and its analysed seconds to the
/// day's `recording_effort` row for the same source.
///
/// Both additive, so folding the same hour twice in a day (a tally split
/// across two flushes) totals correctly.
///
/// # Errors
///
/// Returns `DbError` on write failure.
pub fn add_hourly_effort(conn: &Connection, e: &HourlyEffort) -> Result<(), DbError> {
    let analysed = i64::try_from(e.segments_analysed).unwrap_or(i64::MAX);
    let shed = i64::try_from(e.segments_shed).unwrap_or(i64::MAX);
    let dropped = i64::try_from(e.segments_dropped).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO analysis_effort
             (date, hour, source, seconds_analysed, segments_analysed, segments_shed, segments_dropped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(date, hour, source) DO UPDATE SET
             seconds_analysed = seconds_analysed + excluded.seconds_analysed,
             segments_analysed = segments_analysed + excluded.segments_analysed,
             segments_shed = segments_shed + excluded.segments_shed,
             segments_dropped = segments_dropped + excluded.segments_dropped",
        rusqlite::params![
            e.date,
            e.hour,
            e.source,
            e.seconds_analysed,
            analysed,
            shed,
            dropped
        ],
    )?;
    if e.seconds_analysed > 0.0 {
        conn.execute(
            "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, ?2, ?3)
             ON CONFLICT(date, source) DO UPDATE SET seconds = seconds + excluded.seconds",
            rusqlite::params![e.date, e.source, e.seconds_analysed],
        )?;
    }
    Ok(())
}

/// Every hourly row with `from <= date <= to`, by date, hour, then source.
///
/// # Errors
///
/// Returns `DbError` on read failure.
pub fn hourly_effort(
    conn: &Connection,
    from: &str,
    to: &str,
) -> Result<Vec<HourlyEffort>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, hour, source, seconds_analysed, segments_analysed, segments_shed, segments_dropped
           FROM analysis_effort WHERE date BETWEEN ?1 AND ?2
          ORDER BY date, hour, source",
    )?;
    let rows = stmt.query_map(rusqlite::params![from, to], |r| {
        let count = |i: usize| -> rusqlite::Result<u64> {
            Ok(u64::try_from(r.get::<_, i64>(i)?).unwrap_or(0))
        };
        Ok(HourlyEffort {
            date: r.get(0)?,
            hour: r.get(1)?,
            source: r.get(2)?,
            seconds_analysed: r.get(3)?,
            segments_analysed: count(4)?,
            segments_shed: count(5)?,
            segments_dropped: count(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    #[test]
    fn a_station_that_has_not_recorded_reports_nothing() {
        assert_eq!(total_recording_seconds(&conn()).expect("query"), None);
    }

    #[test]
    fn effort_sums_across_days_and_sources() {
        let conn = conn();
        for (date, source, seconds) in [
            ("2026-09-01", "mic-a", 3600.0),
            ("2026-09-01", "mic-b", 1800.0),
            ("2026-09-02", "mic-a", 7200.0),
        ] {
            conn.execute(
                "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, ?2, ?3)",
                rusqlite::params![date, source, seconds],
            )
            .expect("seed");
        }
        assert_eq!(
            total_recording_seconds(&conn).expect("query"),
            Some(12_600),
            "one hour plus half an hour plus two hours"
        );
    }

    #[test]
    fn a_fractional_second_does_not_become_a_huge_number() {
        let conn = conn();
        conn.execute(
            "INSERT INTO recording_effort (date, source, seconds) VALUES ('2026-09-01','m',0.5)",
            [],
        )
        .expect("seed");
        assert_eq!(total_recording_seconds(&conn).expect("query"), Some(0));
    }
}
