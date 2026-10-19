//! The retention query looks only as far ahead as its longest interval, and
//! answers exactly what the all-pairs query answered.
//!
//! The query joined every pair of a species' active days and let the
//! `retention()` aggregate discard the pairs no interval looks at, so its cost
//! grew with the square of a species' active days: 300 000 detections over 700
//! days and 150 species took 74 s on the machine this was written on (6.7 s
//! with the join bounded; identical rows). A public, unauthenticated endpoint
//! serves it, and the connection it runs on is shared.
//!
//! A faster query that answers differently is worse than a slow one, so this
//! holds the old query as the definition and compares raw result rows, over
//! interval sets chosen so that a reach that is too short, or taken from the
//! wrong end of an unsorted list, changes an answer.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::queries::retention_sql;
use birdnet_behavioral::types::RetentionParams;

/// The retention query as it was written before the join was bounded: every
/// pair of a species' active days, however far apart. Kept here as the
/// definition the bounded query must agree with.
fn reference_sql(params: &RetentionParams) -> String {
    let conditions: Vec<String> = std::iter::once("b.d = a.d".to_string())
        .chain(
            params
                .intervals
                .iter()
                .map(|days| format!("b.d > a.d AND b.d <= a.d + INTERVAL '{days} day'")),
        )
        .collect();
    let rate_exprs: Vec<String> = (0..params.intervals.len())
        .map(|i| format!("AVG(CASE WHEN r[{}] THEN 1.0 ELSE 0.0 END)", i + 2))
        .collect();
    format!(
        "WITH sd AS (
            SELECT DISTINCT Com_Name, detection_date AS d FROM detections_ts
        ),
        cohort AS (
            SELECT a.Com_Name AS species, a.d AS anchor,
                   retention({conditions}) AS r
            FROM sd a JOIN sd b ON a.Com_Name = b.Com_Name
            GROUP BY a.Com_Name, a.d
        ),
        detections AS (
            SELECT Com_Name AS species, COUNT(*) AS n
            FROM detections_ts
            GROUP BY Com_Name
        ),
        rates AS (
            SELECT species, [{rates}] AS retention_rates
            FROM cohort
            GROUP BY species
        ),
        presence AS (
            SELECT Com_Name AS species,
                   COUNT(DISTINCT date_trunc('week', d)) AS weeks_present,
                   date_diff('day', MIN(d), MAX(d)) AS span_days
            FROM sd
            GROUP BY Com_Name
        ),
        station AS (
            SELECT COUNT(DISTINCT date_trunc('week', d)) AS weeks FROM sd
        )
        SELECT r.species, r.retention_rates, p.weeks_present, s.weeks, p.span_days
        FROM rates r
        JOIN presence p ON p.species = r.species
        JOIN detections n ON n.species = r.species
        CROSS JOIN station s
        WHERE n.n >= {min}
        ORDER BY p.weeks_present DESC, r.species",
        conditions = conditions.join(", "),
        rates = rate_exprs.join(", "),
        min = params.min_detections,
    )
}

fn store() -> (tempfile::TempDir, AnalyticsDb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.load_extension().expect("the behavioral extension loads");
    // Twelve species over 400 days. Species `s` is present on about one day in
    // `s + 2`, so the dense ones are seen again within days and the sparse ones
    // only after gaps of weeks or months: every interval below separates some.
    db.conn()
        .execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             SELECT strftime(DATE '2025-01-01' + CAST(d AS INTEGER), '%Y-%m-%d'),
                    '06:30:00', 'Genus ' || s, 'Bird ' || s, 0.9
             FROM range(400) a(d), range(12) b(s)
             WHERE hash(d * 31 + s) % (s + 2) = 0;",
        )
        .expect("seed");
    (dir, db)
}

type Row = (String, String, i64, i64, i64);

fn rows(db: &AnalyticsDb, sql: &str) -> Vec<Row> {
    let mut stmt = db.conn().prepare(sql).expect("the query is valid");
    stmt.query_map([], |r| {
        let rates: duckdb::types::Value = r.get(1)?;
        Ok((
            r.get(0)?,
            format!("{rates:?}"),
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
        ))
    })
    .expect("runs")
    .map(|r| r.expect("row"))
    .collect()
}

fn params(intervals: &[u32]) -> RetentionParams {
    RetentionParams {
        intervals: intervals.to_vec(),
        min_detections: 5,
    }
}

#[test]
fn the_bounded_query_answers_what_the_all_pairs_query_answered() {
    let (_dir, db) = store();
    for intervals in [
        &[1, 2, 3, 7, 14, 30][..],
        &[3, 45, 200],
        &[1],
        // The longest is not last.
        &[30, 1, 7],
        // Longer than the data.
        &[400],
        &[0, 5],
    ] {
        let p = params(intervals);
        let reference = rows(&db, &reference_sql(&p));
        let bounded = rows(&db, &retention_sql(&p));
        assert!(
            reference.len() >= 8,
            "{intervals:?}: only {} species in the reference; the fixture is too thin",
            reference.len()
        );
        assert_eq!(bounded, reference, "intervals {intervals:?}");
    }
}

#[test]
fn the_fixture_makes_a_too_short_reach_visible() {
    // The precondition of the comparison above: for the interval set that
    // reaches furthest, the rate at the end differs from the rate at the start
    // for some species, so a join that stopped short would change a row.
    let (_dir, db) = store();
    let reference = rows(&db, &reference_sql(&params(&[3, 45, 200])));
    let differs = reference.iter().any(|(_, rates, ..)| {
        let nums: Vec<f64> = rates
            .trim_start_matches("List([")
            .trim_end_matches("])")
            .split(", ")
            .filter_map(|v| {
                v.trim_start_matches("Double(")
                    .trim_end_matches(')')
                    .parse()
                    .ok()
            })
            .collect();
        nums.len() == 3 && nums[2] > nums[0] + 0.05
    });
    assert!(differs, "{reference:?}");
}
