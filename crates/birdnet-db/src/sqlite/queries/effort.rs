//! Recording-effort queries.
//!
//! `recording_effort` (migration 27) accumulates seconds of audio, keyed
//! `(local date, source)`. Until migration 63 `src/integrations/effort.rs`
//! credited it on a timer while a source's capture was up; from migration 63 it
//! is credited with the audio the detection pipeline analysed, to the date the
//! audio was recorded on (RES-01). `recording_effort_hourly` (migration 63) holds
//! the analysed seconds by local hour, beside the seconds each source was up.

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
    //
    // Time *up*, not audio analysed: a station whose classifier analyses nothing
    // is exactly what the deadman is for, and it accumulates no analysed audio.
    // `recording_effort` held uptime until migration 63 and analysed audio after
    // it, and every analysed second after it is also in the hourly table's
    // `seconds_analysed`; so the old ledger, less the analysed audio the hourly
    // table repeats, plus the hourly uptime, is uptime throughout.
    let secs: Option<i64> = conn.query_row(
        "SELECT CASE
             WHEN NOT EXISTS (SELECT 1 FROM recording_effort)
              AND NOT EXISTS (SELECT 1 FROM recording_effort_hourly) THEN NULL
             ELSE CAST(
                 COALESCE((SELECT SUM(seconds) FROM recording_effort), 0)
                 - COALESCE((SELECT SUM(seconds_analysed) FROM recording_effort_hourly), 0)
                 + COALESCE((SELECT SUM(seconds_up) FROM recording_effort_hourly), 0)
               AS INTEGER)
           END",
        [],
        |row| row.get(0),
    )?;
    // A negative total cannot arise — the column is credited with elapsed
    // durations — but clamp rather than wrap if one ever did.
    Ok(secs.map(|s| u64::try_from(s).unwrap_or(0)))
}

/// One row of the hourly effort ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct EffortHour {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour, 0–23.
    pub hour: u8,
    /// The source, as `detections.Source` names it.
    pub source: String,
    /// Seconds of audio the detection pipeline analysed.
    pub seconds_analysed: f64,
    /// Seconds the source's capture was up.
    pub seconds_up: f64,
}

/// The hourly ledger over `from..=to` (local dates), oldest first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn effort_hours(conn: &Connection, from: &str, to: &str) -> Result<Vec<EffortHour>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT date, hour, source, seconds_analysed, seconds_up FROM recording_effort_hourly
          WHERE date >= ?1 AND date <= ?2 ORDER BY date, hour, source",
    )?;
    let rows = stmt
        .query_map([from, to], |r| {
            Ok(EffortHour {
                date: r.get(0)?,
                hour: r.get(1)?,
                source: r.get(2)?,
                seconds_analysed: r.get(3)?,
                seconds_up: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One survey occasion of a species' detection history.
#[derive(Debug, Clone, PartialEq)]
pub struct Occasion {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local hour for an hourly history, `None` for a daily one.
    pub hour: Option<u8>,
    /// Audio analysed in the occasion by its best-covered source, in seconds.
    pub seconds_analysed: f64,
    /// Detections of the species in the occasion, from `detections_analytic`
    /// (a detection a reviewer rejected is not counted).
    pub detections: i64,
    /// `Some(true)` detected, `Some(false)` listened to and not detected, `None`
    /// not surveyed: less than `min_seconds` of audio analysed.
    pub detected: Option<bool>,
}

/// A species' detection history over `from..=to` (local dates), one occasion a
/// day or an hour (RES-02).
///
/// An occasion is surveyed when one source analysed at least `min_seconds` of it,
/// by the effort ledger: the best-covered source, not the sum, so two
/// microphones that each heard half an hour do not make a surveyed hour. Effort
/// by the hour exists from migration 63, so an hourly history of earlier dates
/// is all not-surveyed; a daily one reads `recording_effort`, which held capture
/// uptime before migration 63. Ranges are capped at 366 days by the caller.
///
/// Each source is its own site (decision 2 of 2026-11-25), so a surveyed
/// occasion is detected only when a source that itself surveyed it detected
/// the species (review F14): one source listening all day and hearing nothing,
/// and another listening ten seconds and hearing it, is a day surveyed and not
/// detected, not a detection on the first source's effort. `detections` still
/// counts every detection. A row with no recorded source (imported history,
/// rows from before sources were recorded) cannot be placed on a source and is
/// read as before: a detection on a surveyed occasion.
///
/// `source` scopes the history to one source's effort and detections; `None`
/// is the station.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_history(
    conn: &Connection,
    sci_name: &str,
    from: &str,
    to: &str,
    hourly: bool,
    min_seconds: f64,
    source: Option<&str>,
) -> Result<Vec<Occasion>, DbError> {
    let sql = if hourly {
        "WITH RECURSIVE days(d) AS (
             SELECT date(?2) UNION ALL SELECT date(d, '+1 day') FROM days WHERE d < date(?3)
         ),
         hours(h) AS (SELECT 0 UNION ALL SELECT h + 1 FROM hours WHERE h < 23)
         SELECT d, h,
                COALESCE((SELECT MAX(seconds_analysed) FROM recording_effort_hourly e
                           WHERE e.date = d AND e.hour = h
                             AND (?5 IS NULL OR e.source = ?5)), 0),
                (SELECT COUNT(*) FROM detections_analytic x
                  WHERE x.Date = d AND CAST(substr(x.Time, 1, 2) AS INTEGER) = h
                    AND x.Sci_Name = ?1 AND (?5 IS NULL OR x.Source = ?5)),
                (SELECT COUNT(*) FROM detections_analytic x
                  WHERE x.Date = d AND CAST(substr(x.Time, 1, 2) AS INTEGER) = h
                    AND x.Sci_Name = ?1 AND (?5 IS NULL OR x.Source = ?5)
                    AND (x.Source IS NULL OR EXISTS (
                         SELECT 1 FROM recording_effort_hourly e
                          WHERE e.date = d AND e.hour = h AND e.source = x.Source
                            AND e.seconds_analysed >= ?4)))
           FROM days, hours ORDER BY d, h"
    } else {
        "WITH RECURSIVE days(d) AS (
             SELECT date(?2) UNION ALL SELECT date(d, '+1 day') FROM days WHERE d < date(?3)
         )
         SELECT d, NULL,
                COALESCE((SELECT MAX(seconds) FROM recording_effort e
                           WHERE e.date = d AND (?5 IS NULL OR e.source = ?5)), 0),
                (SELECT COUNT(*) FROM detections_analytic x
                  WHERE x.Date = d AND x.Sci_Name = ?1 AND (?5 IS NULL OR x.Source = ?5)),
                (SELECT COUNT(*) FROM detections_analytic x
                  WHERE x.Date = d AND x.Sci_Name = ?1 AND (?5 IS NULL OR x.Source = ?5)
                    AND (x.Source IS NULL OR EXISTS (
                         SELECT 1 FROM recording_effort e
                          WHERE e.date = d AND e.source = x.Source AND e.seconds >= ?4)))
           FROM days ORDER BY d"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![sci_name, from, to, min_seconds, source],
            |r| {
                let seconds_analysed: f64 = r.get(2)?;
                let detections: i64 = r.get(3)?;
                let on_surveyed: i64 = r.get(4)?;
                Ok(Occasion {
                    date: r.get(0)?,
                    hour: r.get(1)?,
                    seconds_analysed,
                    detections,
                    detected: (seconds_analysed >= min_seconds).then_some(on_surveyed > 0),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
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

    /// The deadman's reference is time up, not audio analysed: a station whose
    /// classifier analyses nothing has listened, by this measure, as long as
    /// its capture was up, which is what lets the deadman fire on it.
    #[test]
    fn the_deadman_reference_counts_uptime_when_nothing_is_analysed() {
        let conn = conn();
        conn.execute(
            "INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up)
             VALUES ('2026-09-01', 6, 'local', 0, 3600)",
            [],
        )
        .expect("seed");
        assert_eq!(total_recording_seconds(&conn).expect("query"), Some(3600));
        // After an upgrade: an hour of uptime credited the old way, then an hour
        // up of which half was analysed (in both ledgers).
        conn.execute(
            "INSERT INTO recording_effort (date, source, seconds) VALUES
                 ('2026-08-31', 'local', 3600), ('2026-09-01', 'local', 1800)",
            [],
        )
        .expect("seed");
        conn.execute(
            "UPDATE recording_effort_hourly SET seconds_analysed = 1800",
            [],
        )
        .expect("seed");
        assert_eq!(total_recording_seconds(&conn).expect("query"), Some(7200));
    }

    /// RES-02. An hourly detection history: a detection is 1, a listened hour
    /// with none is 0, and an hour the source was down is NA, not 0.
    #[test]
    fn an_hour_without_effort_is_not_a_zero() {
        let conn = conn();
        for (hour, analysed) in [(4, 3600.0), (5, 0.0), (6, 0.0), (7, 3600.0), (8, 3600.0)] {
            conn.execute(
                "INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up)
                 VALUES ('2026-05-01', ?1, 'local', ?2, ?2)",
                rusqlite::params![hour, analysed],
            )
            .expect("seed");
        }
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Source)
             VALUES ('2026-05-01', '04:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav', 'local')",
            [],
        )
        .expect("seed");
        let h = detection_history(
            &conn,
            "Turdus merula",
            "2026-05-01",
            "2026-05-01",
            true,
            1800.0,
            None,
        )
        .expect("query");
        assert_eq!(h.len(), 24);
        let at = |hour: u8| {
            h.iter()
                .find(|o| o.hour == Some(hour))
                .expect("hour")
                .detected
        };
        assert_eq!(at(4), Some(true));
        assert_eq!(at(5), None, "the source was down");
        assert_eq!(at(6), None, "the source was down");
        assert_eq!(at(7), Some(false), "listened, nothing heard");
        assert_eq!(at(3), None, "no ledger row at all");
    }

    /// Review F14. Each source is its own site (decision 2): source A listened
    /// all day and heard nothing, source B listened ten seconds and heard the
    /// species. The day is surveyed (by A) and the species was not detected by
    /// any source that surveyed it, so it is 0, not a 1 made of A's effort and
    /// B's detection. The detection still counts in `detections`.
    #[test]
    fn a_detection_on_a_source_that_did_not_survey_the_day_is_not_a_surveyed_one() {
        let conn = conn();
        for (source, seconds) in [("mic-a", 86_400.0), ("mic-b", 10.0)] {
            conn.execute(
                "INSERT INTO recording_effort (date, source, seconds) VALUES ('2026-05-01', ?1, ?2)",
                rusqlite::params![source, seconds],
            )
            .expect("seed");
        }
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Source)
             VALUES ('2026-05-01', '04:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'b.wav', 'mic-b')",
            [],
        )
        .expect("seed");
        let h = detection_history(
            &conn,
            "Turdus merula",
            "2026-05-01",
            "2026-05-01",
            false,
            43_200.0,
            None,
        )
        .expect("query");
        assert_eq!(h[0].detections, 1, "the detection is still counted");
        assert_eq!(
            h[0].detected,
            Some(false),
            "a 10 s source's detection made a surveyed positive of another source's day"
        );
        let of = |source| {
            detection_history(
                &conn,
                "Turdus merula",
                "2026-05-01",
                "2026-05-01",
                false,
                43_200.0,
                Some(source),
            )
            .expect("query")
            .remove(0)
        };
        let a = of("mic-a");
        assert_eq!((a.detections, a.detected), (0, Some(false)), "{a:?}");
        let b = of("mic-b");
        assert_eq!(
            (b.detections, b.detected),
            (1, None),
            "B did not survey it: {b:?}"
        );
        // Counterpart: a row with no recorded source reads as it always did.
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', '05:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'old.wav')",
            [],
        )
        .expect("seed");
        let h = detection_history(
            &conn,
            "Turdus merula",
            "2026-05-01",
            "2026-05-01",
            false,
            43_200.0,
            None,
        )
        .expect("query");
        assert_eq!((h[0].detections, h[0].detected), (2, Some(true)), "{h:?}");
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
