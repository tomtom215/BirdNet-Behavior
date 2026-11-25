//! RES-26. The listened-day rule has two implementations of one number: the SQL
//! `ledger` CTE (`MEDIAN(seconds) FILTER (WHERE seconds > 0)`, which the trend
//! queries and the reporting frequency read from the analytics copy) and
//! [`typical_day`], which the pages that read the `SQLite` ledger use. This holds
//! them to the same answer, an even count of days included, where a median is a
//! choice (`DuckDB`'s is the mean of the middle two).

use birdnet_timeseries::queries::trend::typical_day;

fn duckdb_median(days: &[f64]) -> Option<f64> {
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch("CREATE TABLE effort (seconds DOUBLE)")
        .expect("table");
    for s in days {
        conn.execute("INSERT INTO effort VALUES (?)", [s])
            .expect("row");
    }
    conn.query_row(
        "SELECT MEDIAN(seconds) FILTER (WHERE seconds > 0) FROM effort",
        [],
        |r| r.get::<_, Option<f64>>(0),
    )
    .expect("median")
}

#[test]
fn the_rust_typical_day_is_the_sql_one() {
    for days in [
        // Odd count, zeros left out: [100, 300, 900] → 300.
        vec![0.0, 900.0, 100.0, 300.0, 0.0],
        // Even count: (200 + 300) / 2 = 250.
        vec![400.0, 100.0, 300.0, 200.0],
        // A station of whole days with one 61-second day: [61, 86400 × 4] → 86400.
        vec![86_400.0, 61.0, 86_400.0, 86_400.0, 86_400.0],
        // Only zeros: no typical day.
        vec![0.0, 0.0],
    ] {
        assert_eq!(
            typical_day(days.iter().copied()),
            duckdb_median(&days),
            "{days:?}"
        );
    }
    // The even case is the one that discriminates: a lower or upper median would say 200 or 300.
    assert_eq!(duckdb_median(&[400.0, 100.0, 300.0, 200.0]), Some(250.0));
}
