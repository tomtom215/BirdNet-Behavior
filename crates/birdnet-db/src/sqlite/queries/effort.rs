//! Recording-effort queries.
//!
//! `recording_effort` (migration 27) accumulates seconds of audio actually
//! captured, keyed `(local date, source)`. `src/integrations/effort.rs` credits
//! it on a timer while capture is running, so it is the station's own record of
//! how long it has been listening — independent of whether it heard anything.

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
/// switched off.
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
