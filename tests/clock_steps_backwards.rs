//! A clock that steps backwards mid-run.
//!
//! # The hole this closes
//!
//! The repository handles an *unsynchronised* clock thoroughly: `secs_look_synced`
//! in `capture/schedule.rs`, the unit's `After=time-sync.target`,
//! `doctor/clock.rs` flagging a pre-2020 clock, `capture/runloop.rs` recording
//! continuously while unsynced, `maintenance.rs` re-anchoring a last-run
//! timestamp that is in the future, and `station_health.rs` refusing to
//! manufacture staleness from a backwards jump.
//!
//! Every one of those is about *starting up* wrong, or about one handler's own
//! stored timestamp. None of them is about the detections table. On a station
//! that has been up for weeks, NTP correcting a drifted RTC backwards by
//! minutes is the ordinary case, and what it means for the data had never been
//! established — only assumed.
//!
//! # What the step actually does to the data
//!
//! The clock goes back, so the recorder walks over wall-clock ground it has
//! already covered and produces detections whose `(Date, Time, Sci_Name,
//! File_Name, chunk_offset_secs)` already exist. That tuple is
//! `idx_detections_unique` (migration 11). So the question is not "is there a
//! collision" — there is — but what the collision does:
//!
//!  * it must be **reported**, because `insert_detection` uses a plain `INSERT`
//!    and a station silently discarding a re-recorded window looks identical to
//!    a station recording nothing;
//!  * it must **not overwrite** the detection already stored, which is the real
//!    observation of that moment;
//!  * and the rejected insert must **not** move `species_summary`. The rollup is
//!    maintained by an `AFTER INSERT` trigger, and a rollup that counted a row
//!    the table refused would drift by one per collision — invisibly, since
//!    both tables would stay well-formed.
//!
//! # What was checked and is not asserted here
//!
//! Negative session durations were the first suspicion, on the theory that a
//! backwards step would make `session_end < session_start`. Reading
//! `timeseries::executor::sessions` and the `SessionSpec` SQL it builds: the
//! grouping orders by `Time` within a date rather than by arrival, so
//! out-of-order arrival produces the same sessions as in-order arrival. The
//! suspicion was wrong, and there is nothing here to gate.

use birdnet_db::sqlite::{DetectionRecord, insert_detection};
use rusqlite::Connection;

/// A detection at a fixed wall-clock position, so a second one with the same
/// arguments is exactly what a re-recorded window produces.
const fn at(
    time: &'static str,
    file_name: &'static str,
    confidence: f64,
) -> DetectionRecord<'static> {
    DetectionRecord {
        date: "2026-03-16",
        time,
        sci_name: "Turdus merula",
        com_name: "Eurasian Blackbird",
        confidence,
        lat: None,
        lon: None,
        cutoff: None,
        week: Some(11),
        sensitivity: None,
        overlap: None,
        file_name,
        chunk_offset_secs: Some(0.0),
        correlation_id: None,
        source: None,
        duration_secs: None,
        detected_at_utc: None,
    }
}

/// A migrated, empty station database.
fn station() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    // `open_or_create` rather than `open_connection`: the latter requires the
    // file to exist already.
    let conn = birdnet_db::sqlite::open_or_create(&dir.path().join("birds.db")).expect("open");
    (dir, conn)
}

/// Buckets where `species_summary` disagrees with a recount of `detections`.
/// Bucket expression and filter copied from migration 24's triggers.
fn rollup_disagreements(conn: &Connection) -> i64 {
    conn.query_row(
        "WITH recomputed AS (
             SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2) AS hour,
                    COUNT(*) AS detections, SUM(Confidence) AS confidence_sum
             FROM detections
             WHERE review_verdict IS NOT 'rejected'
             GROUP BY Com_Name, Sci_Name, hour
         )
         SELECT COUNT(*) FROM (
             SELECT r.Com_Name FROM recomputed r
             FULL OUTER JOIN species_summary s
               ON s.Com_Name = r.Com_Name AND s.Sci_Name = r.Sci_Name AND s.hour = r.hour
             WHERE r.Com_Name IS NULL OR s.Com_Name IS NULL
                OR s.detections <> r.detections
                OR ABS(s.confidence_sum - r.confidence_sum) > 1e-9
         )",
        [],
        |r| r.get::<_, i64>(0),
    )
    .expect("rollup comparison")
}

/// The rollup total for the one species these tests use.
fn summary_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(SUM(detections), 0) FROM species_summary",
        [],
        |r| r.get(0),
    )
    .expect("summary count")
}

/// The station re-records a window it has already recorded.
#[test]
fn a_re_recorded_window_is_reported_and_changes_nothing() {
    let (_dir, conn) = station();

    // Before the step: the real observation.
    insert_detection(
        &conn,
        &at("08:30:00", "2026-03-16-birdnet-08:30:00.wav", 0.91),
    )
    .expect("first detection");
    let rows_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    let summary_before = summary_count(&conn);

    // NTP steps the clock back; the recorder covers 08:30 again and the model
    // returns a different confidence for the same bird, as it will.
    let err = insert_detection(
        &conn,
        &at("08:30:00", "2026-03-16-birdnet-08:30:00.wav", 0.42),
    )
    .expect_err("a duplicate of an existing detection must be reported");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("unique") || msg.to_lowercase().contains("constraint"),
        "the caller has to be able to tell a duplicate from a disk failure: {msg}"
    );

    // The original observation stands.
    let confidence: f64 = conn
        .query_row(
            "SELECT Confidence FROM detections WHERE Time = '08:30:00'",
            [],
            |r| r.get(0),
        )
        .expect("read back");
    assert!(
        (confidence - 0.91).abs() < 1e-9,
        "the re-recording overwrote the original detection: {confidence}"
    );

    // No row, and — the part nothing would otherwise notice — no rollup movement.
    let rows_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows_after, rows_before, "a rejected insert added a row");
    assert_eq!(
        summary_count(&conn),
        summary_before,
        "species_summary counted a detection the table refused — the AFTER \
         INSERT trigger fired for a statement that did not commit"
    );
    assert_eq!(rollup_disagreements(&conn), 0);
}

/// The counterpart: the collision must be duplicate-suppression, not a station
/// that has stopped accepting anything after the step.
///
/// Without this, the test above is equally satisfied by a database that rejects
/// every insert once one has been rejected — which is what "the station stopped
/// recording after the clock jumped" would look like, and is the failure the
/// operator would actually care about.
#[test]
fn genuinely_new_detections_still_land_after_the_collision() {
    let (_dir, conn) = station();
    insert_detection(
        &conn,
        &at("08:30:00", "2026-03-16-birdnet-08:30:00.wav", 0.91),
    )
    .expect("first");
    let _ = insert_detection(
        &conn,
        &at("08:30:00", "2026-03-16-birdnet-08:30:00.wav", 0.42),
    );

    // A different second in the re-recorded window: a real second observation.
    insert_detection(
        &conn,
        &at("08:30:01", "2026-03-16-birdnet-08:30:00.wav", 0.77),
    )
    .expect("a different second is not a duplicate");

    // Same second, different chunk within the file: also a real one — this is
    // exactly what migration 11 widened the unique key to allow.
    let mut chunked = at("08:30:00", "2026-03-16-birdnet-08:30:00.wav", 0.66);
    chunked.chunk_offset_secs = Some(3.0);
    insert_detection(&conn, &chunked).expect("a different chunk is not a duplicate");

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 3, "the station kept recording through the collision");
    assert_eq!(summary_count(&conn), 3);
    assert_eq!(rollup_disagreements(&conn), 0);
}

/// Many collisions in a row must not accumulate drift.
///
/// A 40-minute backwards step is 40 minutes of collisions, not one. An error
/// of one per collision would be invisible on the first and obvious only after
/// the season.
#[test]
fn a_whole_re_recorded_window_leaves_the_rollup_exact() {
    let (_dir, conn) = station();

    // Twenty minutes of detections, one a minute.
    for m in 0..20 {
        let time: &'static str = Box::leak(format!("08:{m:02}:00").into_boxed_str());
        let file: &'static str =
            Box::leak(format!("2026-03-16-birdnet-08:{m:02}:00.wav").into_boxed_str());
        insert_detection(&conn, &at(time, file, 0.9)).expect("original");
    }
    assert_eq!(summary_count(&conn), 20);

    // The clock steps back twenty minutes and every one is re-recorded.
    let mut rejected = 0;
    for m in 0..20 {
        let time: &'static str = Box::leak(format!("08:{m:02}:00").into_boxed_str());
        let file: &'static str =
            Box::leak(format!("2026-03-16-birdnet-08:{m:02}:00.wav").into_boxed_str());
        if insert_detection(&conn, &at(time, file, 0.5)).is_err() {
            rejected += 1;
        }
    }
    assert_eq!(rejected, 20, "every re-recorded minute should collide");

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 20);
    assert_eq!(
        summary_count(&conn),
        20,
        "the rollup drifted across a re-recorded window"
    );
    assert_eq!(rollup_disagreements(&conn), 0);

    // And the original confidences are intact — the window was not rewritten
    // with the second pass's numbers.
    let sum: f64 = conn
        .query_row("SELECT SUM(Confidence) FROM detections", [], |r| r.get(0))
        .expect("sum");
    assert!(
        (sum - 18.0).abs() < 1e-9, // 20 detections at 0.9
        "the re-recording changed the stored confidences: {sum}"
    );
}

/// A row left behind in the future by the correction must stay visible.
///
/// When the clock is corrected backwards, everything written before the
/// correction is now dated in the future. Those rows are real detections and
/// have to keep counting: a filter that quietly drops "impossible" timestamps
/// would delete the last stretch of data before every clock correction.
#[test]
fn detections_stranded_in_the_future_are_still_counted() {
    let (_dir, conn) = station();

    // "Now" is 2026-03-16; this row was written before the clock stepped back.
    let future = DetectionRecord {
        date: "2027-01-01",
        ..at("08:30:00", "2027-01-01-birdnet-08:30:00.wav", 0.88)
    };
    insert_detection(&conn, &future).expect("insert a future-dated detection");
    insert_detection(
        &conn,
        &at("08:30:00", "2026-03-16-birdnet-08:30:00.wav", 0.91),
    )
    .expect("insert a present-dated detection");

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 2);
    assert_eq!(
        summary_count(&conn),
        2,
        "a future-dated detection was dropped from the rollup"
    );
    assert_eq!(rollup_disagreements(&conn), 0);

    // Migration 32's trigger derives the instant column; a future date must
    // still get one rather than being left NULL and falling out of any
    // instant-based query.
    let missing_instant: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM detections WHERE detected_at_utc IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(
        missing_instant, 0,
        "a future-dated row did not get an instant, so it is invisible to every \
         query that uses detected_at_utc"
    );

    // And the instants are ordered as the wall clock says, not as they arrived.
    let (first, second): (i64, i64) = conn
        .query_row(
            "SELECT MIN(detected_at_utc), MAX(detected_at_utc) FROM detections",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("instants");
    assert!(
        first < second,
        "the instant column is not ordered by wall clock: {first} !< {second}"
    );
}
