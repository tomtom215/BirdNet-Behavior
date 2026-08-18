//! Every phenology query builder must produce SQL `DuckDB` can actually run.
//!
//! The builders in `birdnet_behavioral::phenology` are a public API with tests
//! that only ever asserted on the *text* they generate (`sql.contains("month")`
//! and so on). Text assertions cannot tell a query DuckDB will run from one it
//! refuses to bind, and this crate is the DuckDB crate — its `detections` table
//! *is* the OLAP copy — so nothing anywhere established that these queries work
//! against the engine they target.
//!
//! This file executes all nine against a real store with real rows.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{AbundanceParams, PhenologyParams, abundance, timing};
use tempfile::TempDir;

/// A store holding two full years of detections plus a `recordings` table for
/// the effort-corrected query.
fn seeded(year: i32) -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();

    let mut values = Vec::new();
    for y in [year - 1, year] {
        for month in 1..=12 {
            for day in [3, 11, 19, 27] {
                for (sci, com) in [
                    ("Turdus merula", "Eurasian Blackbird"),
                    ("Erithacus rubecula", "European Robin"),
                ] {
                    // Trailing NULLs are `import_batch_id` (migration 25) and
                    // `review_verdict` (migration 26): the station's own
                    // recordings, not yet reviewed.
                    values.push(format!(
                        "('{y:04}-{month:02}-{day:02}','06:30:00','{sci}','{com}',0.85,\
                          NULL,NULL,NULL,NULL,NULL,NULL,'rec.wav',NULL,NULL)"
                    ));
                }
            }
        }
    }
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections VALUES {};",
            values.join(",")
        ))
        .expect("seed detections");

    // The effort-corrected query joins the station's recording-effort table.
    db.conn()
        .execute_batch(&format!(
            "CREATE TABLE recording_effort (date VARCHAR, source VARCHAR, seconds DOUBLE);
             INSERT INTO recording_effort VALUES ('{year:04}-01-03', 'local', 14400.0),
                                                 ('{year:04}-06-11', 'local', 19800.0);"
        ))
        .expect("seed recordings");

    (db, dir)
}

/// Build every query the module exposes, paired with a name for reporting.
fn all_queries(year: u32) -> Vec<(&'static str, String)> {
    let plain = PhenologyParams::default();
    let ranged = PhenologyParams {
        species: None,
        year_start: Some(year - 1),
        year_end: Some(year),
        ..PhenologyParams::default()
    };
    let with_species = PhenologyParams {
        species: Some("Eurasian Blackbird".into()),
        year_start: Some(year - 1),
        year_end: Some(year),
        ..PhenologyParams::default()
    };
    let abundance_params = AbundanceParams::for_year(year);

    vec![
        ("phenology_timing", timing::phenology_timing_sql(&plain)),
        // No species filter *and* a year range: the species clause is empty
        // here, so anything appending `AND …` after it has to cope.
        (
            "phenology_timing (year range, no species)",
            timing::phenology_timing_sql(&ranged),
        ),
        (
            "phenology_timing (year range + species)",
            timing::phenology_timing_sql(&with_species),
        ),
        ("migration_window", timing::migration_window_sql(2, &ranged)),
        ("first_detection", timing::first_detection_sql(&ranged)),
        ("interannual_trend", timing::interannual_trend_sql(&ranged)),
        (
            "weekly_abundance",
            abundance::weekly_abundance_sql(&abundance_params),
        ),
        (
            "peak_weeks",
            abundance::peak_weeks_sql(&abundance_params, 10),
        ),
        (
            "monthly_totals",
            abundance::monthly_totals_sql(&abundance_params),
        ),
        ("weekly_richness", abundance::weekly_richness_sql(year)),
        (
            "effort_corrected_abundance",
            abundance::effort_corrected_abundance_sql(&abundance_params),
        ),
    ]
}

#[test]
fn every_phenology_query_binds_and_runs() {
    let year: i32 = 2026;
    let (db, _tmp) = seeded(year);
    let year_u32 = u32::try_from(year).unwrap();

    let mut failures = Vec::new();
    for (name, sql) in all_queries(year_u32) {
        // `prepare` is enough to surface a binder or parser error, and keeps a
        // query that legitimately returns no rows from looking like a failure.
        if let Err(e) = db.conn().prepare(&sql) {
            let first = format!("{e}")
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            failures.push(format!("  {name}: {first}"));
        }
    }

    assert!(
        failures.is_empty(),
        "phenology SQL that DuckDB will not run:\n{}",
        failures.join("\n")
    );
}

/// The timing query has to return the rows it describes, not merely bind.
///
/// `phenology_timing_sql` is the module's headline query and the one with a
/// documented result shape, so pin that the columns it promises come back with
/// plausible values rather than trusting a successful `prepare`.
#[test]
fn phenology_timing_returns_usable_rows() {
    let year: i32 = 2026;
    let (db, _tmp) = seeded(year);
    let params = PhenologyParams {
        species: Some("Eurasian Blackbird".into()),
        year_start: Some(u32::try_from(year).unwrap()),
        year_end: Some(u32::try_from(year).unwrap()),
        ..PhenologyParams::default()
    };

    let sql = timing::phenology_timing_sql(&params);
    let mut stmt = db.conn().prepare(&sql).expect("timing query binds");
    let rows: Vec<(String, i32, String, String, i64, i32, i32)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .expect("timing query runs")
        .map(Result::unwrap)
        .collect();

    assert_eq!(rows.len(), 1, "one species x one year: {rows:?}");
    let (species, yr, first, last, count, first_doy, last_doy) = &rows[0];
    assert_eq!(species, "Eurasian Blackbird");
    assert_eq!(*yr, year);
    assert_eq!(first, &format!("{year}-01-03"));
    assert_eq!(last, &format!("{year}-12-27"));
    assert_eq!(*count, 48, "4 days x 12 months");
    assert_eq!(*first_doy, 3, "3 January is day 3");
    // Pinned rather than given a range. The range was here because a raw
    // day-of-year moved with the year's length; on the common-year basis the
    // answer is the same number every year, and a range would hide a
    // regression back to the raw ordinal.
    assert_eq!(*last_doy, 361, "27 December is day 361 of a common year");
}

// ---------------------------------------------------------------------------
// Leap-year stability of day-of-year
// ---------------------------------------------------------------------------

/// Seed detections from explicit `(date, common name)` pairs.
///
/// The `seeded` helper above walks whole years on fixed day numbers, which is
/// exactly the shape that cannot express the question here: whether the *same
/// calendar date* is given the same day-of-year in a leap year and a common
/// one.
fn seeded_dates(rows: &[(&str, &str)]) -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let db = AnalyticsDb::open(&dir.path().join("analytics.duckdb")).unwrap();
    let values: Vec<String> = rows
        .iter()
        .map(|(date, com)| {
            format!(
                "('{date}','06:30:00','Sci name','{com}',0.85,\
                  NULL,NULL,NULL,NULL,NULL,NULL,'rec.wav',NULL,NULL)"
            )
        })
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections VALUES {};",
            values.join(",")
        ))
        .expect("seed detections");
    (db, dir)
}

/// Every `(species, year)` row the timing query returns, as
/// `(species, year, first_doy, last_doy)`.
fn timing_doys(db: &AnalyticsDb, sql: &str) -> Vec<(String, i32, i32, i32)> {
    let mut stmt = db.conn().prepare(sql).expect("timing query binds");
    let mut rows: Vec<(String, i32, i32, i32)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(5)?, r.get(6)?)))
        .expect("timing query runs")
        .map(Result::unwrap)
        .collect();
    rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    rows
}

/// The same calendar date must get the same day-of-year in every year.
///
/// After 28 February a leap year runs one day ahead: 1 May is the 122nd day of
/// 2024 and the 121st day of 2025. `first_doy` and `last_doy` are the numbers
/// the multi-year percentiles in `migration_window_sql` are taken over, so an
/// uncorrected day-of-year puts a systematic one-day error into every arrival
/// and departure estimate that spans a leap year — the one thing this module
/// exists to measure.
///
/// The counterpart in `real_arrival_shifts_survive_normalisation` checks that
/// the correction has not simply flattened the signal.
#[test]
fn day_of_year_is_leap_year_stable() {
    // Identical calendar dates in a leap year (2024) and a common year (2025).
    let (db, _tmp) = seeded_dates(&[
        ("2024-05-01", "Common Swift"),
        ("2024-05-05", "Common Swift"),
        ("2024-05-09", "Common Swift"),
        ("2025-05-01", "Common Swift"),
        ("2025-05-05", "Common Swift"),
        ("2025-05-09", "Common Swift"),
    ]);

    let params = PhenologyParams {
        species: Some("Common Swift".into()),
        ..PhenologyParams::default()
    };

    let rows = timing_doys(&db, &timing::phenology_timing_sql(&params));
    assert_eq!(rows.len(), 2, "one row per year: {rows:?}");
    assert_eq!(
        (rows[0].2, rows[0].3),
        (rows[1].2, rows[1].3),
        "1-9 May is the same day-of-year in 2024 and 2025: {rows:?}"
    );
    assert_eq!(rows[0].2, 121, "1 May is day 121 of a common year");
    assert_eq!(rows[1].3, 129, "9 May is day 129 of a common year");

    // The percentiles over those values must land on the day itself, not
    // between the leap-year and common-year readings of it.
    let sql = timing::migration_window_sql(2, &params);
    let mut stmt = db.conn().prepare(&sql).expect("window query binds");
    let windows: Vec<(String, i64, f64, f64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?, r.get(6)?)))
        .expect("window query runs")
        .map(Result::unwrap)
        .collect();
    assert_eq!(windows.len(), 1, "one species: {windows:?}");
    let (_, years, arrival_median, departure_median) = &windows[0];
    assert_eq!(*years, 2);
    assert!(
        (arrival_median - 121.0).abs() < f64::EPSILON,
        "median arrival smeared off 1 May: {arrival_median}"
    );
    assert!(
        (departure_median - 129.0).abs() < f64::EPSILON,
        "median departure smeared off 9 May: {departure_median}"
    );
}

/// The counterpart: a genuine one-day shift must still read as one day.
///
/// A correction that collapsed every arrival onto the same number would satisfy
/// the gate above while destroying the measurement. This species really does
/// arrive a day later in 2025 than in 2024, and that day has to survive.
#[test]
fn real_arrival_shifts_survive_normalisation() {
    let (db, _tmp) = seeded_dates(&[
        ("2024-05-01", "European Robin"),
        ("2024-05-05", "European Robin"),
        ("2024-05-09", "European Robin"),
        ("2025-05-02", "European Robin"),
        ("2025-05-06", "European Robin"),
        ("2025-05-10", "European Robin"),
    ]);

    let params = PhenologyParams {
        species: Some("European Robin".into()),
        ..PhenologyParams::default()
    };
    let rows = timing_doys(&db, &timing::phenology_timing_sql(&params));
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        rows[1].2 - rows[0].2,
        1,
        "2 May 2025 is one day later than 1 May 2024: {rows:?}"
    );
}

/// 29 February shares 28 February's slot, and 1 March is day 60 in every year.
///
/// A common-year day-of-year has no 29 February to offer, so the leap day has
/// to fold onto one of its neighbours. Folding it backwards onto 28 February
/// keeps 1 March at 60 in leap and common years alike; that is the convention
/// callers are entitled to rely on, so it is pinned rather than left to the
/// shape of the expression.
#[test]
fn leap_day_folds_onto_28_february() {
    let (db, _tmp) = seeded_dates(&[
        ("2024-02-27", "Leap Folder"),
        ("2024-02-28", "Leap Folder"),
        ("2024-02-29", "Leap Folder"),
        ("2024-03-01", "March Anchor"),
        ("2024-03-05", "March Anchor"),
        ("2024-03-09", "March Anchor"),
        ("2025-03-01", "March Anchor"),
        ("2025-03-05", "March Anchor"),
        ("2025-03-09", "March Anchor"),
    ]);

    let rows = timing_doys(
        &db,
        &timing::phenology_timing_sql(&PhenologyParams::default()),
    );
    let folder = rows.iter().find(|r| r.0 == "Leap Folder").expect("seeded");
    assert_eq!(folder.2, 58, "27 February is day 58");
    assert_eq!(
        folder.3, 59,
        "29 February folds onto 28 February's day 59: {folder:?}"
    );

    for anchor in rows.iter().filter(|r| r.0 == "March Anchor") {
        assert_eq!(
            anchor.2, 60,
            "1 March is day 60 in {}: {anchor:?}",
            anchor.1
        );
    }
}
