//! The column called `iso_week` has to be the ISO week.
//!
//! # What this is defending
//!
//! Every weekly phenology query aliases its bucket `iso_week`, and the
//! expression behind it was `%W` — the Monday-based week number, which is not
//! the ISO week. Probed against the real bundled engines (SQLite 3.53.2 and
//! DuckDB agree exactly):
//!
//! ```text
//!                %W    %V (ISO)   %Y     %G (ISO year)
//! 2024-12-30     53       01      2024      2025
//! 2024-12-31     53       01      2024      2025
//! 2025-01-01     00       01      2025      2025
//! 2025-01-05     00       01      2025      2025
//! 2025-01-06     01       02      2025      2025
//! ```
//!
//! Two things follow for a chart whose purpose is comparing week-of-year across
//! years. `%W` has a **week 00** of one to six days, and often a week 53 stub,
//! both drawn at full height beside seven-day weeks. And the *same real week*
//! landed in two different years' charts: 30–31 December under `%Y=2024,
//! %W=53`, 1–5 January under `%Y=2025, %W=00`.
//!
//! # Why these execute rather than assert on the SQL text
//!
//! The module's own unit tests check the generated string
//! (`sql.contains("relative_abundance")` and so on), and a text assertion cannot
//! tell `%V` from `%W` in any way that means something — it would just be the
//! same mistake written twice. These run the builders against a real DuckDB
//! store seeded across a year boundary, which is the only place the two
//! definitions disagree.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{AbundanceParams, abundance};
use tempfile::TempDir;

/// One detection per given date, all the same species.
fn seeded(dates: &[&str]) -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).expect("open");
    let values: Vec<String> = dates
        .iter()
        .map(|d| {
            format!(
                "('{d}','06:30:00','Turdus merula','Eurasian Blackbird',0.9,\
                  NULL,NULL,NULL,NULL,NULL,NULL,'rec.wav',NULL,NULL)"
            )
        })
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections \
              (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, \
               Sens, Overlap, File_Name, import_batch_id, review_verdict) \
             VALUES {};",
            values.join(",")
        ))
        .expect("seed");
    (db, dir)
}

/// `(iso_week, detection_count)` for a year, from the real builder.
fn weeks(db: &AnalyticsDb, year: u32) -> Vec<(i64, i64)> {
    let sql = abundance::weekly_abundance_sql(&AbundanceParams::for_year(year));
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    let mut out: Vec<(i64, i64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>("iso_week")?,
                r.get::<_, i64>("detection_count")?,
            ))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    out.sort_unstable();
    out
}

/// The regression, stated as the thing the chart shows.
///
/// Observed failing with `WEEK_EXPR` back on `%W` and the year filter back on
/// `%Y`: 2025 came back as `[(0, 5), (1, 2)]` — a five-day "week 00" beside a
/// seven-day week — and 2024 carried a `(53, 2)` stub holding the other half of
/// the same real week.
#[test]
fn the_turn_of_the_year_is_one_seven_day_week_not_two_stubs() {
    // ISO week 2025-W01 runs Mon 30 Dec 2024 to Sun 5 Jan 2025. Seed all seven
    // days, plus one day of the following week so the bucket has a neighbour.
    let (db, _dir) = seeded(&[
        "2024-12-30",
        "2024-12-31",
        "2025-01-01",
        "2025-01-02",
        "2025-01-03",
        "2025-01-04",
        "2025-01-05",
        "2025-01-06",
    ]);

    assert_eq!(
        weeks(&db, 2025),
        vec![(1, 7), (2, 1)],
        "all seven days of ISO week 2025-W01 must land in one bucket, and the \
         Monday after it in the next"
    );

    // And none of it leaks into 2024, whose ISO year ends on 29 December.
    assert!(
        weeks(&db, 2024).is_empty(),
        "30–31 December 2024 belong to ISO year 2025; filing them under 2024 is \
         what split one real week across two charts"
    );
}

/// The counterpart, so the gate above is a discrimination rather than "some
/// grouping happened": an ordinary mid-year week is unaffected, and consecutive
/// weeks stay consecutive.
#[test]
fn a_mid_year_week_is_unchanged() {
    // ISO 2025-W24 is Mon 9 June to Sun 15 June.
    let (db, _dir) = seeded(&[
        "2025-06-09",
        "2025-06-11",
        "2025-06-15",
        "2025-06-16", // the Monday of W25
    ]);
    assert_eq!(weeks(&db, 2025), vec![(24, 3), (25, 1)]);
}

/// Every bucket the builder produces is a real ISO week number. `%W` can emit
/// `0`, which no ISO week is, and a `0` bucket is the visible symptom of the
/// defect.
#[test]
fn no_bucket_is_week_zero() {
    let (db, _dir) = seeded(&[
        "2025-01-01",
        "2025-01-02",
        "2025-06-11",
        "2025-12-29",
        "2025-12-31",
    ]);
    for (week, _) in weeks(&db, 2025) {
        assert!(
            (1..=53).contains(&week),
            "ISO weeks run 1..=53; got {week}, which is `%W`'s week 00 by another name"
        );
    }
}

/// A 53-week ISO year really does have a week 53, so the range above is not
/// quietly excluding real data. 2026 is such a year: it ends Sun 3 January 2027.
#[test]
fn a_fifty_three_week_year_keeps_its_fifty_third_week() {
    let (db, _dir) = seeded(&["2026-12-28", "2026-12-31", "2027-01-03"]);
    assert_eq!(
        weeks(&db, 2026),
        vec![(53, 3)],
        "ISO 2026-W53 runs 28 Dec 2026 to 3 Jan 2027 — all three days are one week"
    );
}

/// The effort join has to bucket weeks the same way the detections do, or a
/// week's counts get divided by a different week's listening hours. The error
/// would be largest exactly at the year boundary, where the two definitions
/// differ most.
#[test]
fn effort_and_detections_agree_on_what_a_week_is() {
    let (db, _dir) = seeded(&["2024-12-30", "2025-01-02", "2025-01-05"]);
    db.conn()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS recording_effort (
                 date TEXT, source TEXT, seconds BIGINT);
             INSERT INTO recording_effort VALUES
                 ('2024-12-30','local',3600),
                 ('2025-01-02','local',3600),
                 ('2025-01-05','local',3600);",
        )
        .expect("effort");

    let sql = abundance::effort_corrected_abundance_sql(&AbundanceParams::for_year(2025));
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    let rows: Vec<(i64, i64, Option<f64>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>("iso_week")?,
                r.get::<_, i64>("raw_count")?,
                r.get::<_, Option<f64>>("effort_hours")?,
            ))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();

    assert_eq!(
        rows.len(),
        1,
        "three days of one ISO week are one row: {rows:?}"
    );
    let (week, count, hours) = &rows[0];
    assert_eq!(*week, 1);
    assert_eq!(*count, 3);
    assert_eq!(
        *hours,
        Some(3.0),
        "all three hours of effort must reach the same bucket as the three \
         detections — a mismatched week definition shows up here as a NULL or a \
         partial denominator"
    );
}
