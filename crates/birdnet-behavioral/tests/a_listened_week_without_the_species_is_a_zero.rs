//! A week the station listened to, in which a species was not detected, is a
//! detection rate of zero, with an interval, not a missing row.
//!
//! `effort_corrected_abundance_sql` built its weekly rows from detections only,
//! so a species-week with listening and no detections was simply absent. A
//! chart or a downstream model reads an absent week as "no data", and a zero
//! as "listened and heard nothing"; for a rate those are different claims, and
//! only the second one is true here. Nor did the rate carry any statement of
//! its own uncertainty: two detections in two hours and two hundred in two
//! hundred hours both read as 1.0.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{AbundanceParams, abundance};
use tempfile::TempDir;

/// ISO 2025-W10 is Monday 3 to Sunday 9 March; W11 is 10 to 16 March; W12 is
/// 17 to 23 March.
///
/// Effort: two hours on the 3rd (W10), four on the 11th (W11), one on the
/// 18th (W12).
///
/// Great Tit: three detections on the 3rd, none in W11, none in W12.
/// Robin: one detection on the 11th, so that with a minimum of two it is not
/// reported at all, and must not acquire zero rows either.
/// Wren: one detection on the 3rd and three on the 18th: with a minimum of two
/// its W10 is dropped for being under the minimum. A dropped week had
/// detections, so it must not come back as a zero.
fn store() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    let det = |date: &str, time: &str, com: &str| {
        format!(
            "('{date}','{time}','Sci name','{com}',0.9,\
              epoch(TIMESTAMP '{date} {time}'))"
        )
    };
    let values = [
        det("2025-03-03", "06:00:00", "Great Tit"),
        det("2025-03-03", "06:10:00", "Great Tit"),
        det("2025-03-03", "06:20:00", "Great Tit"),
        det("2025-03-11", "06:00:00", "Robin"),
        det("2025-03-03", "07:00:00", "Wren"),
        det("2025-03-18", "07:00:00", "Wren"),
        det("2025-03-18", "07:10:00", "Wren"),
        det("2025-03-18", "07:20:00", "Wren"),
    ];
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             VALUES {};
             CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES
                 ('2025-03-03','mic-a',7200),
                 ('2025-03-11','mic-a',14400),
                 ('2025-03-18','mic-a',3600);",
            values.join(",")
        ))
        .expect("seed");
    (db, dir)
}

#[derive(Debug)]
struct Row {
    species: String,
    iso_week: i64,
    raw_count: i64,
    effort_count: i64,
    effort_hours: Option<f64>,
    rate: Option<f64>,
    lower: Option<f64>,
    upper: Option<f64>,
}

fn rows(db: &AnalyticsDb, params: &AbundanceParams) -> Vec<Row> {
    let sql = abundance::effort_corrected_abundance_sql(params);
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    stmt.query_map([], |r| {
        Ok(Row {
            species: r.get("species")?,
            iso_week: r.get("iso_week")?,
            raw_count: r.get("raw_count")?,
            effort_count: r.get("effort_count")?,
            effort_hours: r.get("effort_hours")?,
            rate: r.get("detections_per_hour")?,
            // Read leniently so that the unfixed query, which has no interval,
            // still reports which row it got wrong; every test below demands
            // `Some` of these, so a missing column still fails.
            lower: r.get("detections_per_hour_lower95").ok().flatten(),
            upper: r.get("detections_per_hour_upper95").ok().flatten(),
        })
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

fn find<'a>(out: &'a [Row], species: &str, week: i64) -> Option<&'a Row> {
    out.iter()
        .find(|r| r.species == species && r.iso_week == week)
}

/// The gate: W11 was listened to for four hours and no Great Tit was detected.
/// That is a rate of 0 per hour, and the upper bound is the exact Poisson
/// (Garwood) bound for a count of zero, ln(40) = 3.689, over four hours.
#[test]
fn a_listened_week_with_no_detections_is_a_rate_of_zero_with_an_upper_bound() {
    let (db, _dir) = store();
    let out = rows(&db, &AbundanceParams::for_year(2025));
    let w11 = find(&out, "Great Tit", 11)
        .unwrap_or_else(|| panic!("W11 was listened to; the Great Tit row is missing: {out:?}"));
    assert_eq!(w11.raw_count, 0, "{w11:?}");
    assert_eq!(w11.effort_count, 0, "{w11:?}");
    assert_eq!(w11.effort_hours, Some(4.0), "{w11:?}");
    assert_eq!(w11.rate, Some(0.0), "{w11:?}");
    assert_eq!(w11.lower, Some(0.0), "{w11:?}");
    let upper = w11.upper.expect("an upper bound");
    assert!(
        upper > 0.0,
        "an absence over four hours does not prove a rate of zero: {w11:?}"
    );
    assert!(
        (upper - 40f64.ln() / 4.0).abs() < 1e-4,
        "exact 95% upper bound for 0 events, over 4 h: {w11:?}"
    );
    // W12 too: listened to, no Great Tit.
    let w12 = find(&out, "Great Tit", 12).unwrap_or_else(|| panic!("W12: {out:?}"));
    assert_eq!(w12.rate, Some(0.0), "{w12:?}");
}

/// The counterpart: a week with detections has the rate it had before, and an
/// interval that contains it.
#[test]
fn a_week_with_detections_keeps_its_rate() {
    let (db, _dir) = store();
    let out = rows(&db, &AbundanceParams::for_year(2025));
    let w10 = find(&out, "Great Tit", 10).unwrap_or_else(|| panic!("W10: {out:?}"));
    assert_eq!(w10.raw_count, 3, "{w10:?}");
    assert_eq!(w10.effort_count, 3, "{w10:?}");
    assert_eq!(w10.effort_hours, Some(2.0), "{w10:?}");
    assert_eq!(
        w10.rate,
        Some(1.5),
        "three detections in two hours: {w10:?}"
    );
    let (lo, hi) = (w10.lower.expect("lower"), w10.upper.expect("upper"));
    // Garwood bounds for k = 3: 0.6187 and 8.7673, over two hours.
    assert!((lo - 0.618_672 / 2.0).abs() < 1e-4, "{w10:?}");
    assert!((hi - 8.767_273 / 2.0).abs() < 1e-4, "{w10:?}");
    // No weeks were invented outside the effort spine: W10-W12 only.
    assert!(
        out.iter().all(|r| (10..=12).contains(&r.iso_week)),
        "{out:?}"
    );
}

/// With a minimum of two, a week dropped for being *under* the minimum had
/// detections in it: it must not reappear as a zero, and a species with no
/// reported week must not be given a column of zeros.
#[test]
fn a_week_under_the_minimum_is_not_a_zero() {
    let (db, _dir) = store();
    let params = AbundanceParams {
        species: None,
        year: 2025,
        min_weekly_count: 2,
    };
    let out = rows(&db, &params);
    assert!(
        out.iter().all(|r| r.species != "Robin"),
        "Robin has no week at the minimum, so it is not reported: {out:?}"
    );
    assert!(
        find(&out, "Wren", 10).is_none(),
        "the Wren was detected in W10, once; that is not a zero: {out:?}"
    );
    let w11 = find(&out, "Wren", 11).unwrap_or_else(|| panic!("Wren W11: {out:?}"));
    assert_eq!(w11.rate, Some(0.0), "{w11:?}");
    let w12 = find(&out, "Wren", 12).unwrap_or_else(|| panic!("Wren W12: {out:?}"));
    assert_eq!(w12.raw_count, 3, "{w12:?}");
    assert_eq!(w12.rate, Some(3.0), "{w12:?}");
}
