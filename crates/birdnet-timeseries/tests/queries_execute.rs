//! Every time-series query must actually run against `DuckDB`.
//!
//! This file exists because the crate's own tests could not have caught a
//! defect that took out every dashboard these queries back. They are `*_sql_*`
//! tests: each builds a query string and asserts it *contains* the right
//! substrings, so a query DuckDB refuses to bind passes them exactly as well as
//! one that works. Sixteen public queries had no execution coverage at all.
//!
//! What shipped past them was an unbound ICU extension — every one of these
//! queries filters on `CURRENT_DATE - INTERVAL n DAYS`, whose operator lives in
//! ICU, and on a freshly opened store the first such query failed to bind (see
//! `birdnet_behavioral::connection::load_icu`). No amount of asserting on SQL
//! text could see that, because the text was correct.
//!
//! So the gate here is deliberately not "does the SQL look right". It executes
//! every public query against a real DuckDB holding real rows and requires
//! results back. The store is opened the way the application opens it —
//! ICU loaded up front — so what is under test is the queries themselves.

#![cfg(feature = "analytics")]

use duckdb::Connection;

use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::{
    AnomalyParams, DailyParams, DiversityParams, HourlyParams, PeakParams, SessionParams,
    TrendParams, WeeklyParams,
};

/// Days-since-epoch for a civil date (Howard Hinnant's algorithm).
const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date from days-since-epoch — the inverse of [`days_from_civil`].
const fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A DuckDB holding 90 days of detections ending today.
///
/// The dates are computed in Rust and inserted as literals. That is not
/// squeamishness: DuckDB 1.5.5 binds `DATE - x` only when it can constant-fold
/// the right-hand side, and *whether it folds depends on the surrounding
/// statement* — `generate_series(CURRENT_DATE - 89, …)` binds in a bare
/// `SELECT` and fails inside `INSERT … SELECT`, with the same
/// `age(DATE, INTEGER_LITERAL)` error this file exists to catch. A fixture that
/// used SQL date arithmetic would therefore fail for reasons that have nothing
/// to do with the code under test.
///
/// The window still ends *today* rather than at fixed calendar dates, because
/// every query filters on a look-back from `CURRENT_DATE`; a pinned fixture
/// would drift out of range and start returning empty results that look like
/// passes.
fn seeded_db() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory duckdb");
    // Mirror `AnalyticsDb::open`: ICU carries the date arithmetic every query
    // below depends on, and DuckDB will not load it until a query needs it —
    // by which point that query has already failed to bind. Loading it here
    // keeps this file testing the queries rather than re-testing the loader,
    // which `birdnet-behavioral` gates directly.
    conn.execute_batch("LOAD icu;").expect("load icu");
    conn.execute_batch(
        "CREATE TABLE detections (
            Date VARCHAR, Time VARCHAR, Sci_Name VARCHAR, Com_Name VARCHAR,
            Confidence DOUBLE, Lat DOUBLE, Lon DOUBLE, Cutoff DOUBLE,
            Week INTEGER, Sens DOUBLE, Overlap DOUBLE, File_Name VARCHAR);",
    )
    .expect("create detections");

    let today: String = conn
        .query_row("SELECT CAST(CURRENT_DATE AS VARCHAR)", [], |r| r.get(0))
        .expect("current date");
    let parts: Vec<i64> = today.split('-').map(|p| p.parse().unwrap()).collect();
    let today_days = days_from_civil(parts[0], parts[1], parts[2]);

    // 90 days x 4 hours x 3 species, walked back from today.
    let mut sql = String::from(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence,
             Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name) VALUES ",
    );
    let species = [
        ("Turdus merula", "Eurasian Blackbird"),
        ("Erithacus rubecula", "European Robin"),
        ("Parus major", "Great Tit"),
    ];
    let mut first = true;
    for back in 0..90 {
        let (y, m, d) = civil_from_days(today_days - back);
        for hour in [5, 6, 7, 18] {
            for (offset, (sci, com)) in species.iter().enumerate() {
                if !first {
                    sql.push(',');
                }
                first = false;
                let confidence = 0.60 + f64::from(u32::try_from(back % 30).unwrap()) / 100.0;
                let _ = std::fmt::Write::write_fmt(
                    &mut sql,
                    format_args!(
                        "('{y:04}-{m:02}-{d:02}','{hour:02}:{min:02}:00','{sci}','{com}',\
                          {confidence},51.5,-0.1,0.7,1,1.0,0.0,'rec.wav')",
                        min = (back * 7 + i64::try_from(offset).unwrap_or(0)) % 60,
                    ),
                );
            }
        }
    }
    conn.execute_batch(&sql).expect("seed detections");

    // The same view the application builds (see birdnet-behavioral's
    // CREATE_DETECTIONS_TS_VIEW): TRY_CAST so a row that names no point in time
    // drops out of the results instead of aborting the query.
    conn.execute_batch(
        "CREATE OR REPLACE VIEW detections_ts AS
         SELECT *,
             TRY_CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp,
             TRY_CAST(Date AS DATE)                     AS detection_date
         FROM detections;",
    )
    .expect("create view");
    conn
}

/// Run every public query and require each to bind, execute and return rows.
///
/// One test rather than sixteen: the failure mode this guards against takes
/// out every query at once, and a single list is harder to add a method to
/// without noticing.
#[test]
fn every_query_binds_executes_and_returns_rows() {
    let conn = seeded_db();
    let db = TimeSeriesDb::new(&conn).expect("executor");

    let hourly = HourlyParams {
        lookback_days: 30,
        species: None,
    };
    let daily = DailyParams {
        lookback_days: 30,
        species: None,
    };
    let weekly = WeeklyParams { lookback_weeks: 8 };
    let trend = TrendParams::default();
    let peak = PeakParams {
        window_minutes: 60,
        hop_minutes: 30,
        lookback_days: 30,
        limit: 10,
    };
    let session = SessionParams {
        gap_minutes: 30,
        date_filter: None,
        lookback_days: 30,
        limit: 50,
    };
    let diversity = DiversityParams {
        lookback_days: 30,
        include_shannon: true,
    };
    let anomaly = AnomalyParams {
        z_threshold: 2.0,
        window_days: 7,
        lookback_days: 60,
    };
    // Ask DuckDB for today rather than computing it in Rust, so the fixture and
    // the query agree on the date even across a midnight boundary.
    let today: String = conn
        .query_row("SELECT CAST(CURRENT_DATE AS VARCHAR)", [], |r| r.get(0))
        .expect("current date");

    // (name, row count) — every one of these previously raised a binder error.
    let results: Vec<(&str, usize)> = vec![
        (
            "hourly_activity",
            db.hourly_activity(&hourly).expect("hourly_activity").len(),
        ),
        (
            "daily_activity",
            db.daily_activity(&daily).expect("daily_activity").len(),
        ),
        (
            "weekly_activity",
            db.weekly_activity(&weekly).expect("weekly_activity").len(),
        ),
        (
            "hourly_heatmap",
            db.hourly_heatmap(&hourly).expect("hourly_heatmap").len(),
        ),
        (
            "top_species",
            db.top_species(30, 10).expect("top_species").len(),
        ),
        (
            "moving_average",
            db.moving_average(&trend).expect("moving_average").len(),
        ),
        (
            "year_over_year",
            db.year_over_year(&weekly).expect("year_over_year").len(),
        ),
        (
            "anomalies",
            db.anomalies(&anomaly).expect("anomalies").len(),
        ),
        (
            "daily_richness",
            db.daily_richness(&diversity).expect("daily_richness").len(),
        ),
        (
            "accumulation_curve",
            db.accumulation_curve(None, None)
                .expect("accumulation_curve")
                .len(),
        ),
        (
            "peak_windows",
            db.peak_windows(&peak).expect("peak_windows").len(),
        ),
        (
            "species_peak_hours",
            db.species_peak_hours("Eurasian Blackbird", 30)
                .expect("species_peak_hours")
                .len(),
        ),
        (
            "activity_sessions",
            db.activity_sessions(&session)
                .expect("activity_sessions")
                .len(),
        ),
        (
            "daily_max_gaps",
            db.daily_max_gaps(30, 0).expect("daily_max_gaps").len(),
        ),
        (
            "intraday_gaps",
            db.intraday_gaps(&today, 0).expect("intraday_gaps").len(),
        ),
        (
            "quiet_days",
            db.quiet_days(1, 30).expect("quiet_days").len(),
        ),
    ];

    // `quiet_days` is legitimately empty against a fixture with no gaps in it;
    // everything else has to find the 90 days of detections that were seeded.
    for (name, rows) in results {
        if name == "quiet_days" {
            continue;
        }
        assert!(
            rows > 0,
            "{name} bound and ran but returned no rows — the fixture spans 90 \
             days of detections, so an empty result means the query is not \
             seeing them"
        );
    }
}

/// The look-back expressions callers can pass must bind.
///
/// `moving_average` takes its range as a raw DuckDB expression string
/// (`TrendParams::from_date`), so it is the one place a caller hands the query
/// a fragment the crate never validates. The default is exercised above; these
/// are the other spellings in use across the codebase, pinned so a future
/// caller has a worked example rather than a binder error.
#[test]
fn interval_lookback_expressions_bind() {
    let conn = seeded_db();
    let db = TimeSeriesDb::new(&conn).expect("executor");

    for expr in [
        "CURRENT_DATE - INTERVAL 60 DAYS",
        "CURRENT_DATE - INTERVAL 60 DAY",
        "CURRENT_DATE - INTERVAL 8 WEEKS",
        "CURRENT_DATE - INTERVAL 2 MONTHS",
        "(CURRENT_DATE - INTERVAL 60 DAYS)::DATE",
    ] {
        let params = TrendParams {
            window_days: 7,
            from_date: Some(expr.to_string()),
            to_date: None,
            species: None,
        };
        assert!(
            db.moving_average(&params).is_ok(),
            "look-back expression should bind: {expr}"
        );
    }
}
