//! Detections per listening hour divide a week's detections by the listening
//! that could have produced them.
//!
//! `effort_corrected_abundance_sql` summed a week's `recording_effort` and
//! divided it into *every* detection of that week — including detections on
//! days no effort was recorded for at all, which is every day before the
//! sampler existed and every day it was not running. It also ignored
//! `min_weekly_count`, which the `/analytics/abundance` endpoint fills from its
//! `min_detections` parameter.
//!
//! Effort is summed across sources, and that is right: detections are per
//! source (each stream that hears a bird records it), so the rate is
//! detections per source-hour. A fix for a fixture that seeded two
//! microphones' effort but only one detection briefly took the day's longest
//! source instead, which doubled a two-microphone station's rate for the same
//! birds (MSC-03 / RES-03); the fixtures below hold the realistic shape, one
//! row per microphone that heard the bird.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{AbundanceParams, abundance};
use tempfile::TempDir;

/// ISO 2025-W10 runs Monday 3 March to Sunday 9 March.
///
/// Great Tit: one detection each of the seven days, and on the 3rd a second
/// row, because both microphones listening that day heard it. Robin: one, on
/// the 3rd. Effort: the 3rd has an hour from each of two microphones, the 4th
/// an hour from one. Nothing else was recorded as listened to.
fn store() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    let mut values: Vec<String> = (3..=9)
        .map(|d| {
            format!(
                "('2025-03-{d:02}','06:00:00','Parus major','Great Tit',0.9,\
                  epoch(TIMESTAMP '2025-03-{d:02} 06:00:00'))"
            )
        })
        .collect();
    // The same Great Tit, logged by the second microphone too.
    values.push(
        "('2025-03-03','06:00:00','Parus major','Great Tit',0.9,\
          epoch(TIMESTAMP '2025-03-03 06:00:00'))"
            .into(),
    );
    values.push(
        "('2025-03-03','07:00:00','Erithacus rubecula','Robin',0.9,\
          epoch(TIMESTAMP '2025-03-03 07:00:00'))"
            .into(),
    );
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             VALUES {};
             CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES
                 ('2025-03-03','mic-a',3600), ('2025-03-03','mic-b',3600),
                 ('2025-03-04','mic-a',3600);",
            values.join(",")
        ))
        .expect("seed");
    (db, dir)
}

/// `(species, raw_count, effort_hours, detections_per_hour)` per row.
fn rows(
    db: &AnalyticsDb,
    params: &AbundanceParams,
) -> Vec<(String, i64, Option<f64>, Option<f64>)> {
    let sql = abundance::effort_corrected_abundance_sql(params);
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>("species")?,
            r.get::<_, i64>("raw_count")?,
            r.get::<_, Option<f64>>("effort_hours")?,
            r.get::<_, Option<f64>>("detections_per_hour")?,
        ))
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

/// Three detections on the two days with listening recorded (two microphones'
/// rows on the 3rd, one on the 4th), over three source-hours: 1.0 an hour. It
/// reported every detection of the week over the effort, 8 / 3 = 2.67, and
/// with the longest source alone it read 3 / 2 = 1.5.
#[test]
fn the_rate_counts_only_days_that_were_listened_to_once() {
    let (db, _dir) = store();
    let out = rows(&db, &AbundanceParams::for_year(2025));
    let tit = out
        .iter()
        .find(|r| r.0 == "Great Tit")
        .unwrap_or_else(|| panic!("{out:?}"));
    // Counterpart: the raw count is still every detection of the week, so a
    // week with no effort recorded stays visible as such.
    assert_eq!(tit.1, 8, "{out:?}");
    assert_eq!(
        tit.2,
        Some(3.0),
        "two source-hours on the 3rd and one on the 4th: {out:?}"
    );
    assert_eq!(tit.3, Some(1.0), "{out:?}");
}

/// `min_weekly_count` drops a species-week below it.
#[test]
fn min_weekly_count_is_honoured() {
    let (db, _dir) = store();
    let params = AbundanceParams {
        species: None,
        year: 2025,
        min_weekly_count: 2,
    };
    let out = rows(&db, &params);
    assert!(
        out.iter().all(|r| r.0 != "Robin"),
        "one Robin detection is under a minimum of two: {out:?}"
    );
    // Counterpart: the species above the minimum is still there.
    assert!(out.iter().any(|r| r.0 == "Great Tit"), "{out:?}");
}

/// MSC-03 / RES-03: a bird heard on two microphones is the same rate as one
/// heard on one.
///
/// A multi-source station records the same bird once per stream that heard it
/// (`birdnet-db`'s migration notes: "the same bird heard by two streams is
/// recorded as two detections"), so the day with two microphones carries two
/// rows for the one Robin. Dividing those two rows by the *longest* source's
/// hour reported 2.0 an hour against the one-microphone week's 1.0, for the
/// same bird.
///
/// ISO 2025-W10 has one microphone listening for an hour on Monday 3 March and
/// one Robin. 2025-W11 has two microphones listening for an hour each on
/// Monday 10 March, and the Robin logged by both.
#[test]
fn a_bird_heard_on_two_microphones_is_one_an_hour_as_on_one() {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.conn()
        .execute_batch(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
             VALUES
               ('2025-03-03','07:00:00','Erithacus rubecula','Robin',0.9,'a-1.wav',
                epoch(TIMESTAMP '2025-03-03 07:00:00')),
               ('2025-03-10','07:00:00','Erithacus rubecula','Robin',0.9,'a-2.wav',
                epoch(TIMESTAMP '2025-03-10 07:00:00')),
               ('2025-03-10','07:00:00','Erithacus rubecula','Robin',0.9,'b-2.wav',
                epoch(TIMESTAMP '2025-03-10 07:00:00'));
             CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES
                 ('2025-03-03','mic-a',3600),
                 ('2025-03-10','mic-a',3600), ('2025-03-10','mic-b',3600);",
        )
        .expect("seed");
    let sql = abundance::effort_corrected_abundance_sql(&AbundanceParams::for_year(2025));
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    let out: Vec<(i64, i64, Option<f64>, Option<f64>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>("iso_week")?,
                r.get::<_, i64>("effort_count")?,
                r.get::<_, Option<f64>>("effort_hours")?,
                r.get::<_, Option<f64>>("detections_per_hour")?,
            ))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    let week = |n: i64| {
        *out.iter()
            .find(|r| r.0 == n)
            .unwrap_or_else(|| panic!("week {n}: {out:?}"))
    };
    // The fixture did what it says: both microphones' rows are in week 11.
    assert_eq!(
        week(11).1,
        2,
        "the Robin logged by both microphones: {out:?}"
    );
    // Counterpart: one source, one bird, one hour stays at 1/h.
    assert_eq!(week(10).3, Some(1.0), "one microphone: {out:?}");
    assert_eq!(
        week(11).3,
        week(10).3,
        "two microphones hearing the same bird for an hour each must give the \
         one-microphone rate, detections per source-hour: {out:?}"
    );
    assert_eq!(
        week(11).2,
        Some(2.0),
        "two sources listening an hour each are two source-hours: {out:?}"
    );
}
