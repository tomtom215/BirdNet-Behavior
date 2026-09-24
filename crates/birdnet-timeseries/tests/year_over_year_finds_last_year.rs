//! Year-over-year finds last year.
//!
//! The `weekly` CTE was limited to the look-back window (`CURRENT_DATE - N
//! WEEKS`), and each week was joined to the one 52 weeks before it — which lay
//! outside that CTE for every week compared. A probe over 2.7 years of daily
//! data returned 39 rows and one non-null `prior_year_count`; `yoy_delta` was
//! then `current - COALESCE(NULL, 0)`, a delta against a year the query never
//! read. It also compared only the current *calendar* year's weeks, where the
//! parameter says "counting back from today".

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::WeeklyParams;
use tempfile::TempDir;

/// One detection every day for the last 800 days.
fn two_years_daily() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("ts.duckdb")).expect("open analytics db");
    db.conn()
        .execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence,
                 Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, detected_at_utc)
             SELECT strftime(d, '%Y-%m-%d'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
                    0.9, 51.5, -0.1, 0.7, 1, 1.0, 0.0, 'rec.wav',
                    epoch(d + INTERVAL 6 HOUR)
               FROM range(CAST(CURRENT_DATE - INTERVAL 800 DAY AS TIMESTAMP),
                          CAST(CURRENT_DATE + INTERVAL 1 DAY AS TIMESTAMP),
                          INTERVAL 1 DAY) t(d)",
        )
        .expect("seed");
    (db, dir)
}

#[test]
fn every_week_compared_finds_the_same_week_last_year() {
    let (store, _tmp) = two_years_daily();
    let ts = TimeSeriesDb::new(store.conn()).expect("executor");
    let rows = ts
        .year_over_year(&WeeklyParams { lookback_weeks: 12 })
        .expect("year_over_year");
    assert!(
        (12..=13).contains(&rows.len()),
        "the last 12 weeks, counted back from today: {} rows",
        rows.len()
    );
    // Every week but the partial current one is a full seven days in both
    // years, since the station detected every day of both.
    let full: Vec<_> = rows.iter().filter(|r| r.current_year_count == 7).collect();
    assert!(full.len() >= 11, "{rows:?}");
    for r in full {
        assert_eq!(r.prior_year_count, Some(7), "week {}: {r:?}", r.week_start);
        assert_eq!(r.yoy_delta, Some(0), "week {}", r.week_start);
    }
}

/// The counterpart: a week with no prior year has no delta, rather than a
/// delta against zero.
#[test]
fn a_week_with_no_prior_year_has_no_delta() {
    let dir = TempDir::new().expect("temp dir");
    let store = AnalyticsDb::open(&dir.path().join("ts.duckdb")).expect("open analytics db");
    store
        .conn()
        .execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence,
                 Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, detected_at_utc)
             SELECT strftime(d, '%Y-%m-%d'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
                    0.9, 51.5, -0.1, 0.7, 1, 1.0, 0.0, 'rec.wav', epoch(d + INTERVAL 6 HOUR)
               FROM range(CAST(CURRENT_DATE - INTERVAL 20 DAY AS TIMESTAMP),
                          CAST(CURRENT_DATE + INTERVAL 1 DAY AS TIMESTAMP),
                          INTERVAL 1 DAY) t(d)",
        )
        .expect("seed");
    let ts = TimeSeriesDb::new(store.conn()).expect("executor");
    let rows = ts
        .year_over_year(&WeeklyParams { lookback_weeks: 4 })
        .expect("year_over_year");
    assert!(!rows.is_empty());
    for r in rows {
        assert_eq!(r.prior_year_count, None);
        assert_eq!(
            r.yoy_delta, None,
            "a delta against a year with no data: {r:?}"
        );
    }
}
