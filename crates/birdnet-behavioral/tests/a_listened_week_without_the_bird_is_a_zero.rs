//! RES-15: a week the station listened to and did not hear a species is a
//! detection rate of zero, with an interval — not a missing row.
//!
//! `effort_corrected_abundance_sql` built its weekly rows from detections
//! alone, so a species-week with listening and no detections was absent: a
//! chart of the rate drew a line straight across the silence, and a reader
//! could not tell "listened, nothing" from "not listened". Nor did any week
//! carry an interval, so one detection in a short week read as precisely as a
//! hundred in a long one.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{AbundanceParams, abundance};
use tempfile::TempDir;

/// One row of the effort-corrected query.
#[derive(Debug, Clone, Copy)]
struct RateRow {
    iso_week: i64,
    raw_count: i64,
    effort_hours: Option<f64>,
    rate: Option<f64>,
    lower: Option<f64>,
    upper: Option<f64>,
}

/// ISO 2025-W10 (from Monday 3 March): one Robin, an hour of listening.
/// 2025-W11 (from Monday 10 March): two hours of listening, no Robin.
/// 2025-W12 (from Monday 17 March): no listening and no Robin.
/// 2025-W13 (from Monday 24 March): one Robin and no listening.
fn store() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.conn()
        .execute_batch(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
             VALUES
               ('2025-03-03','07:00:00','Erithacus rubecula','Robin',0.9,'r-1.wav',
                epoch(TIMESTAMP '2025-03-03 07:00:00')),
               ('2025-03-24','07:00:00','Erithacus rubecula','Robin',0.9,'r-2.wav',
                epoch(TIMESTAMP '2025-03-24 07:00:00'));
             CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES
                 ('2025-03-03','local',3600),
                 ('2025-03-11','local',3600), ('2025-03-12','local',3600);",
        )
        .expect("seed");
    (db, dir)
}

fn weeks(db: &AnalyticsDb, params: &AbundanceParams) -> Vec<RateRow> {
    let sql = abundance::effort_corrected_abundance_sql(params);
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    stmt.query_map([], |r| {
        Ok(RateRow {
            iso_week: r.get("iso_week")?,
            raw_count: r.get("raw_count")?,
            effort_hours: r.get("effort_hours")?,
            rate: r.get("detections_per_hour")?,
            // Read leniently so the gate below reports the missing week, which
            // is the defect, rather than the missing column; the counterpart
            // test requires both bounds on a detected week, so an absent column
            // still fails there.
            lower: r.get::<_, Option<f64>>("rate_ci_lower").ok().flatten(),
            upper: r.get::<_, Option<f64>>("rate_ci_upper").ok().flatten(),
        })
    })
    .unwrap_or_else(|e| panic!("query: {e}"))
    .map(|r| r.unwrap_or_else(|e| panic!("row: {e}")))
    .collect()
}

fn week(out: &[RateRow], n: i64) -> Option<RateRow> {
    out.iter().find(|w| w.iso_week == n).copied()
}

/// The acceptance gate: the listened week with no Robin is a zero with an
/// upper bound above zero.
#[test]
fn a_listened_week_with_no_detections_is_a_rate_of_zero_with_an_upper_bound() {
    let (db, _dir) = store();
    let out = weeks(&db, &AbundanceParams::for_year(2025));
    let w11 = week(&out, 11).unwrap_or_else(|| {
        panic!(
            "ISO week 11 was listened to for two hours and the Robin was not \
             heard: that is a rate of 0, not a missing row: {out:?}"
        )
    });
    assert_eq!(w11.raw_count, 0, "{w11:?}");
    assert_eq!(w11.effort_hours, Some(2.0), "{w11:?}");
    assert_eq!(w11.rate, Some(0.0), "{w11:?}");
    assert_eq!(w11.lower, Some(0.0), "{w11:?}");
    // Exact (Garwood) upper bound for k = 0: -ln(0.025) = 3.6889 detections,
    // over two hours.
    let upper = w11
        .upper
        .unwrap_or_else(|| panic!("no upper bound: {w11:?}"));
    assert!(
        upper > 0.0 && (upper - 3.688_879 / 2.0).abs() < 1e-3,
        "two silent hours cannot rule out a rate up to ~1.84/h: {w11:?}"
    );
}

/// Counterparts: a week with neither listening nor detections is not invented,
/// a detected week with no listening keeps its NULL rate, and the detected,
/// listened week carries the exact interval for one detection in one hour.
#[test]
fn only_listened_weeks_are_filled_and_detected_weeks_keep_their_interval() {
    let (db, _dir) = store();
    let out = weeks(&db, &AbundanceParams::for_year(2025));
    assert!(
        week(&out, 12).is_none(),
        "nobody listened in week 12, so there is no rate to report: {out:?}"
    );
    let w13 = week(&out, 13).unwrap_or_else(|| panic!("week 13: {out:?}"));
    assert_eq!(w13.raw_count, 1, "{w13:?}");
    assert!(
        w13.rate.is_none() && w13.upper.is_none() && w13.lower.is_none(),
        "a week with no listening has no rate and no interval: {w13:?}"
    );
    let w10 = week(&out, 10).unwrap_or_else(|| panic!("week 10: {out:?}"));
    assert_eq!(w10.rate, Some(1.0), "{w10:?}");
    // Garwood 95 % for k = 1: [0.025318, 5.571643].
    let (lo, hi) = (
        w10.lower.expect("lower bound"),
        w10.upper.expect("upper bound"),
    );
    assert!((lo - 0.025_318).abs() < 1e-4, "{w10:?}");
    assert!((hi - 5.571_643).abs() < 1e-3, "{w10:?}");
}

/// A named species is filled over the listened weeks even in a year it was
/// never detected: "listened all year, never heard" is the answer.
#[test]
fn a_named_species_never_heard_is_zero_in_every_listened_week() {
    let (db, _dir) = store();
    let params = AbundanceParams {
        species: Some("Nightingale".into()),
        year: 2025,
        min_weekly_count: 1,
    };
    let out = weeks(&db, &params);
    let listened: Vec<i64> = out.iter().map(|w| w.iso_week).collect();
    assert_eq!(listened, vec![10, 11], "{out:?}");
    assert!(
        out.iter().all(|w| w.rate == Some(0.0) && w.raw_count == 0),
        "{out:?}"
    );
}
