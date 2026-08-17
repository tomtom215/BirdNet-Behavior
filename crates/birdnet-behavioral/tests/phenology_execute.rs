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
                    // Trailing NULL is `import_batch_id` (migration 25): these
                    // are the station's own recordings, not an import.
                    values.push(format!(
                        "('{y:04}-{month:02}-{day:02}','06:30:00','{sci}','{com}',0.85,\
                          NULL,NULL,NULL,NULL,NULL,NULL,'rec.wav',NULL)"
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

    // The effort-corrected query joins a caller-supplied recordings table.
    db.conn()
        .execute_batch(&format!(
            "CREATE TABLE recordings (date VARCHAR, duration_hours DOUBLE);
             INSERT INTO recordings VALUES ('{year:04}-01-03', 4.0), ('{year:04}-06-11', 5.5);"
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
    assert!(
        (361..=362).contains(last_doy),
        "27 December is day 361 in a common year: {last_doy}"
    );
}
