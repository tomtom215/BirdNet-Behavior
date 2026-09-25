//! Detections per listening hour divide a week's detections by the listening
//! that could have produced them.
//!
//! `effort_corrected_abundance_sql` summed a week's `recording_effort` and
//! divided it into *every* detection of that week — including detections on
//! days no effort was recorded for at all, which is every day before the
//! sampler existed and every day it was not running. And it summed effort
//! across sources, so two microphones listening to the same garden for an hour
//! counted as two hours, halving the rate of a two-microphone station against
//! a one-microphone one. It also ignored `min_weekly_count`, which the
//! `/analytics/abundance` endpoint fills from its `min_detections` parameter.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{AbundanceParams, abundance};
use tempfile::TempDir;

/// ISO 2025-W10 runs Monday 3 March to Sunday 9 March.
///
/// Great Tit: one detection each of the seven days. Robin: one, on the 3rd.
/// Effort: the 3rd has an hour from each of two microphones, the 4th an hour
/// from one. Nothing else was recorded as listened to.
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

/// Two detections on the two days with listening recorded, over two hours of
/// station time: 1.0 an hour. It reported 7 / 3 = 2.33.
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
    assert_eq!(tit.1, 7, "{out:?}");
    assert_eq!(
        tit.2,
        Some(2.0),
        "two days, one hour of station time each: {out:?}"
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
