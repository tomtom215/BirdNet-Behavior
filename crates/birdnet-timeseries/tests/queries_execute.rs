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
//! results back. The store is a real `AnalyticsDb`, opened exactly as the
//! application opens it, so what is under test is the queries themselves and
//! not a fixture's idea of how a store gets set up.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use tempfile::TempDir;

use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::{
    AnomalyParams, DailyParams, DiversityParams, HourlyParams, PeakParams, SessionParams,
    TrendParams, WeeklyParams,
};

/// Days-since-epoch for a civil date, and back.
///
/// Thin wrappers over `birdnet_core::civil`, which owns the one implementation
/// of this arithmetic in the workspace. This file used to carry its own copy;
/// a test fixture computing dates a different way from the code under test is
/// exactly how a date bug hides.
const fn days_from_civil(y: u32, m: u32, d: u32) -> i64 {
    birdnet_core::civil::days_from_civil(y, m, d)
}

const fn civil_from_days(z: i64) -> (u32, u32, u32) {
    birdnet_core::civil::civil_from_days(z)
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
fn seeded_db() -> (AnalyticsDb, TempDir) {
    // A real `AnalyticsDb`, not an approximation of one. It loads ICU, creates
    // `detections`, and creates the `detections_ts` view — and it is exactly
    // what reaches these queries in production, where `birdnet-web` does
    // `TimeSeriesDb::new(db.conn())`.
    //
    // This used to open a bare in-memory connection and issue `LOAD icu;`
    // itself. That is not what `AnalyticsDb::open` does, and it only ever
    // worked when `~/.duckdb` already held ICU: a bare `LOAD` does not
    // autoinstall (DuckDB only does that while binding a query that needs the
    // extension), so the gate passed or failed on whether some *other* test
    // binary in the same `cargo test` run had populated that cache first.
    // Moving the cache aside made it fail, which is how it was found.
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("timeseries.duckdb")).expect("open analytics db");
    let conn = db.conn();

    let today: String = conn
        .query_row("SELECT CAST(CURRENT_DATE AS VARCHAR)", [], |r| r.get(0))
        .expect("current date");
    let parts: Vec<u32> = today.split('-').map(|p| p.parse().unwrap()).collect();
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

    // `detections_ts` is already there — `AnalyticsDb::open` creates it, and it
    // is a view, so it sees the rows just inserted. Rebuilding a copy of it
    // here would let the two definitions drift, which is the same class of
    // mistake as re-implementing the ICU load above.
    (db, dir)
}

/// Run every public query and require each to bind, execute and return rows.
///
/// One test rather than sixteen: the failure mode this guards against takes
/// out every query at once, and a single list is harder to add a method to
/// without noticing.
#[test]
fn every_query_binds_executes_and_returns_rows() {
    let (store, _tmp) = seeded_db();
    let db = TimeSeriesDb::new(store.conn()).expect("executor");

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
    let today: String = store
        .conn()
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
    let (store, _tmp) = seeded_db();
    let db = TimeSeriesDb::new(store.conn()).expect("executor");

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
