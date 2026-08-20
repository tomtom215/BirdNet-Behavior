//! Sessions must be cut on elapsed time, not on the local wall clock.
//!
//! # What these gates are for
//!
//! A session is "detections separated by less than `gap_minutes` of silence".
//! That is an *elapsed-time* statement, and local wall clock cannot express it:
//! one local hour repeats every autumn and one never happens every spring, so
//! the wall-clock gap between two detections is an hour out in one direction or
//! the other on those two nights — either side of the 30-minute default.
//!
//! Both directions are gated here, because a gate that only catches one of them
//! passes for a builder that swapped the columns in half the query:
//!
//! * **Autumn** — two detections a real hour apart carry the *same* wall clock,
//!   so the wall clock says zero minutes and merges two sessions that were
//!   genuinely separate.
//! * **Spring** — two detections fifteen real minutes apart carry wall clocks
//!   seventy-five minutes apart, so the wall clock splits one session that
//!   never broke, and reports a duration five times too long.
//!
//! The `*_sql_*` unit tests in this crate assert on substrings, which cannot
//! see either: the text is well-formed in both cases and only the numbers are
//! wrong. So these execute against a real `DuckDB` and assert on the numbers.
//!
//! Instants are literal and were checked against the tz database rather than
//! derived here — the fixture must state what it means without depending on the
//! zone the test runner happens to be in.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::SessionParams;

/// Europe/Berlin, 2026-10-25: the offset moves +2 -> +1 at 01:00 UTC, so local
/// 02:30 happens at 00:30Z and again at 01:30Z — one real hour apart, one wall
/// clock.
const AUTUMN_FIRST: i64 = 1_792_888_200;
const AUTUMN_SECOND: i64 = 1_792_891_800;

/// Europe/Berlin, 2026-03-29: the offset moves +1 -> +2 at 01:00 UTC, so local
/// 01:45 is 00:45Z and local 03:00 is 01:00Z — fifteen real minutes apart, and
/// seventy-five on the wall clock.
const SPRING_BEFORE: i64 = 1_774_745_100;
const SPRING_AFTER: i64 = 1_774_746_000;

/// A store holding one fixture day, seeded with explicit instants.
fn store(rows: &[(&str, &str, i64)]) -> (tempfile::TempDir, AnalyticsDb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    let values: Vec<String> = rows
        .iter()
        .map(|(date, time, instant)| {
            format!("('{date}','{time}','Turdus merula','Eurasian Blackbird',0.9,{instant})")
        })
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             VALUES {};",
            values.join(", ")
        ))
        .expect("seed");
    (dir, db)
}

/// Sessions for one fixture date, through the `date_filter` builder.
fn sessions_for_date(
    db: &AnalyticsDb,
    date: &str,
) -> Vec<birdnet_timeseries::types::results::SessionRow> {
    TimeSeriesDb::new(db.conn())
        .expect("the ts view builds")
        .activity_sessions(&SessionParams {
            gap_minutes: 30,
            date_filter: Some(date.to_owned()),
            ..SessionParams::default()
        })
        .expect("activity_sessions")
}

/// Two detections a real hour apart must be two sessions, even though their
/// wall clocks are identical.
///
/// On the wall clock the gap is zero, which is below every threshold, so the
/// query returned a single two-detection session — merging the last session of
/// the evening with the first of the night, once a year, silently.
#[test]
fn the_repeated_autumn_hour_is_two_sessions_not_one() {
    let (_dir, db) = store(&[
        ("2026-10-25", "02:30:00", AUTUMN_FIRST),
        ("2026-10-25", "02:30:00", AUTUMN_SECOND),
    ]);
    let sessions = sessions_for_date(&db, "2026-10-25");
    assert_eq!(
        sessions.len(),
        2,
        "an hour of silence separates them; the wall clock says zero and merges \
         them into one: {sessions:?}"
    );
    assert!(
        sessions.iter().all(|s| s.detection_count == 1),
        "each pass is its own session: {sessions:?}"
    );
}

/// …and the counterpart, which a gate on the autumn case alone would miss:
/// fifteen real minutes must stay one session and report fifteen minutes.
///
/// On the wall clock the same pair is seventy-five minutes apart — over the
/// threshold — so the query split a session that never broke and reported a
/// duration five times too long.
#[test]
fn the_skipped_spring_hour_is_one_session_of_the_length_it_really_was() {
    let (_dir, db) = store(&[
        ("2026-03-29", "01:45:00", SPRING_BEFORE),
        ("2026-03-29", "03:00:00", SPRING_AFTER),
    ]);
    let sessions = sessions_for_date(&db, "2026-03-29");
    assert_eq!(
        sessions.len(),
        1,
        "fifteen real minutes is under the 30-minute threshold; the wall clock \
         reads seventy-five and splits them: {sessions:?}"
    );
    assert_eq!(
        sessions[0].duration_minutes, 15,
        "and the duration is the elapsed time, not the wall-clock difference"
    );
    // The displayed extent stays local — a session is reported in the clock the
    // station's operator reads, which is the whole reason both columns exist.
    assert!(
        sessions[0].session_start.contains("01:45:00"),
        "the start shown to a human is their own clock: {:?}",
        sessions[0].session_start
    );
}

/// The date-range builder is a second, near-identical copy of the session SQL,
/// and it has to obey the same rule.
///
/// It filters on a look-back from `CURRENT_DATE`, so the look-back here is
/// deliberately enormous: the fixture's date is fixed, and a seven-day window
/// would make this pass by returning nothing the moment the calendar moved past
/// it.
#[test]
fn the_date_range_session_builder_cuts_on_elapsed_time_too() {
    let (_dir, db) = store(&[
        ("2026-10-25", "02:30:00", AUTUMN_FIRST),
        ("2026-10-25", "02:30:00", AUTUMN_SECOND),
    ]);
    let sessions = TimeSeriesDb::new(db.conn())
        .expect("the ts view builds")
        .activity_sessions(&SessionParams {
            gap_minutes: 30,
            date_filter: None,
            lookback_days: 40_000,
            limit: 100,
        })
        .expect("activity_sessions");
    assert_eq!(
        sessions.len(),
        2,
        "the date-range copy merged what the date-filtered one splits: {sessions:?}"
    );
}

/// An intra-day gap is elapsed silence, so it must be measured on the instant.
#[test]
fn an_intraday_gap_is_measured_in_real_minutes() {
    let (_dir, db) = store(&[
        ("2026-10-25", "02:30:00", AUTUMN_FIRST),
        ("2026-10-25", "02:30:00", AUTUMN_SECOND),
    ]);
    let gaps = TimeSeriesDb::new(db.conn())
        .expect("the ts view builds")
        .intraday_gaps("2026-10-25", 30)
        .expect("intraday_gaps");
    assert_eq!(
        gaps.len(),
        1,
        "one hour of silence, above the 30-minute threshold: {gaps:?}"
    );
    assert_eq!(
        gaps[0].gap_minutes, 60,
        "sixty real minutes; the wall clock reads zero and reports no gap at all"
    );
}
