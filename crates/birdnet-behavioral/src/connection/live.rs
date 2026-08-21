//! Live verification against the real `behavioral` community extension.
//!
//! These run only under `cargo test --features analytics` and require network
//! access to the DuckDB community extension CDN on first run (to
//! `INSTALL behavioral FROM community`). When the extension cannot be loaded
//! (offline, or no matching build), each test skips rather than fails.
//!
//! Purpose: validate the SQL produced by `queries.rs` against the real
//! function signatures of the published extension, and lock in the
//! domain-specific result semantics with a deterministic fixture.

use super::AnalyticsDb;
use crate::{queries, types};
use tempfile::TempDir;

/// Open a DB and load the extension; return `None` (skip) if it can't load.
fn loaded_db() -> Option<(AnalyticsDb, TempDir)> {
    let dir = TempDir::new().ok()?;
    let mut db = AnalyticsDb::open(&dir.path().join("probe.duckdb")).ok()?;
    match db.load_extension() {
        Ok(()) => {
            eprintln!(
                "[live] duckdb={:?} behavioral={:?}",
                db.duckdb_version(),
                db.extension_version()
            );
            Some((db, dir))
        }
        Err(e) => {
            eprintln!("[live] SKIP: behavioral extension unavailable: {e}");
            None
        }
    }
}

/// Insert a dawn-chorus fixture: Robin -> Blackbird -> Wren across 3 mornings.
///
/// Detection days per species: Robin {01,02,03}, Blackbird {01,02,03},
/// Wren {01,03}. Day 02 is missing its Wren.
fn seed(db: &AnalyticsDb) {
    let rows = [
        (
            "2024-05-01",
            "05:00:00",
            "Erithacus rubecula",
            "European Robin",
        ),
        (
            "2024-05-01",
            "05:10:00",
            "Turdus merula",
            "Eurasian Blackbird",
        ),
        (
            "2024-05-01",
            "05:20:00",
            "Troglodytes troglodytes",
            "Eurasian Wren",
        ),
        (
            "2024-05-01",
            "05:50:00",
            "Erithacus rubecula",
            "European Robin",
        ),
        (
            "2024-05-02",
            "05:05:00",
            "Erithacus rubecula",
            "European Robin",
        ),
        (
            "2024-05-02",
            "05:12:00",
            "Turdus merula",
            "Eurasian Blackbird",
        ),
        (
            "2024-05-03",
            "05:00:00",
            "Erithacus rubecula",
            "European Robin",
        ),
        (
            "2024-05-03",
            "05:15:00",
            "Turdus merula",
            "Eurasian Blackbird",
        ),
        (
            "2024-05-03",
            "05:25:00",
            "Troglodytes troglodytes",
            "Eurasian Wren",
        ),
    ];
    // `detected_at_utc` is stamped, not left to default, because every temporal
    // function under test now takes `detection_instant` — which is derived from
    // it. A fixture that omitted the column would hand `sessionize`,
    // `window_funnel` and `sequence_match` a column of NULLs and assert against
    // whatever they return for that, which is not what any of them do in
    // production. The seed is in UTC, so `epoch(TIMESTAMP)` over the same wall
    // clock is the instant.
    let mut sql = String::from(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) \
         VALUES ",
    );
    let values: Vec<String> = rows
        .iter()
        .map(|(d, t, s, c)| {
            format!("('{d}', '{t}', '{s}', '{c}', 0.9, epoch(TIMESTAMP '{d} {t}'))")
        })
        .collect();
    sql.push_str(&values.join(", "));
    sql.push(';');
    db.conn().execute_batch(&sql).expect("seed insert");
}

#[test]
fn live_sessionize() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let sessions = db.sessionize(&types::SessionizeParams::default()).unwrap();
    eprintln!("[live] sessions: {sessions:?}");
    // Robin: detections at 05:00 & 05:50 (gap 50 > 30) on day 1 -> 2 sessions,
    // plus day 2 and day 3 -> 4 sessions total. sessionize partitions by
    // Sci_Name across the whole ordered timeline.
    let robin = sessions
        .iter()
        .filter(|s| s.species == "European Robin")
        .count();
    assert_eq!(robin, 4, "Robin should split into 4 sessions");
    // Every session carries a parseable ISO start timestamp and a duration.
    assert!(sessions.iter().all(|s| s.start_time.contains("2024-05")));
}

#[test]
fn live_window_funnel() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let params = types::FunnelParams {
        species_sequence: vec![
            "European Robin".into(),
            "Eurasian Blackbird".into(),
            "Eurasian Wren".into(),
        ],
        window_minutes: 120,
        hour_start: 4,
        hour_end: 8,
    };
    let funnel = db.funnel(&params).unwrap();
    eprintln!("[live] funnel: {funnel:?}");
    let by_date = |d: &str| {
        funnel
            .iter()
            .find(|f| f.date.starts_with(d))
            .unwrap()
            .steps_completed
    };
    assert_eq!(by_date("2024-05-01"), 3, "day1 full sequence");
    assert_eq!(by_date("2024-05-02"), 2, "day2 Robin->Blackbird only");
    assert_eq!(by_date("2024-05-03"), 3, "day3 full sequence");
}

#[test]
fn live_sequence_match() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let params = types::PatternParams {
        species_sequence: vec![
            "European Robin".into(),
            "Eurasian Blackbird".into(),
            "Eurasian Wren".into(),
        ],
        max_gap_minutes: None,
        hour_start: 4,
        hour_end: 8,
    };
    let results = db.sequence_match(&params).unwrap();
    eprintln!("[live] sequence_match: {results:?}");
    let matched = |d: &str| {
        results
            .iter()
            .find(|m| m.date.starts_with(d))
            .unwrap()
            .matched
    };
    assert!(matched("2024-05-01"), "day1 R->B->W in order");
    assert!(!matched("2024-05-02"), "day2 missing Wren");
    assert!(matched("2024-05-03"), "day3 R->B->W in order");
}

#[test]
fn live_sequence_count() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let params = types::PatternParams {
        species_sequence: vec![
            "European Robin".into(),
            "Eurasian Blackbird".into(),
            "Eurasian Wren".into(),
        ],
        max_gap_minutes: None,
        hour_start: 4,
        hour_end: 8,
    };
    let results = db.sequence_count(&params).unwrap();
    eprintln!("[live] sequence_count: {results:?}");
    let count = |d: &str| {
        results
            .iter()
            .find(|m| m.date.starts_with(d))
            .unwrap()
            .count
    };
    // Same fixture as live_sequence_match, but counted: day1 and day3 each
    // complete R->B->W exactly once; day2 has no Wren so the sequence count is 0.
    assert_eq!(count("2024-05-01"), 1, "day1 R->B->W once");
    assert_eq!(count("2024-05-02"), 0, "day2 missing Wren -> 0");
    assert_eq!(count("2024-05-03"), 1, "day3 R->B->W once");
}

#[test]
fn live_funnel_events() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let params = types::FunnelParams {
        species_sequence: vec![
            "European Robin".into(),
            "Eurasian Blackbird".into(),
            "Eurasian Wren".into(),
        ],
        window_minutes: 120,
        hour_start: 4,
        hour_end: 8,
    };
    let results = db.funnel_events(&params).unwrap();
    eprintln!("[live] funnel_events: {results:?}");
    let steps = |d: &str| {
        results
            .iter()
            .find(|f| f.date.starts_with(d))
            .unwrap()
            .step_times
            .len()
    };
    // One timestamp per completed funnel step (cf. live_window_funnel):
    // day1 and day3 complete all three; day2 reaches Robin->Blackbird only.
    assert_eq!(steps("2024-05-01"), 3, "day1 full sequence -> 3 step times");
    assert_eq!(
        steps("2024-05-02"),
        2,
        "day2 Robin->Blackbird -> 2 step times"
    );
    assert_eq!(steps("2024-05-03"), 3, "day3 full sequence -> 3 step times");
    // The step timestamps are real, date-stamped values.
    let day1 = results
        .iter()
        .find(|f| f.date.starts_with("2024-05-01"))
        .unwrap();
    assert!(day1.step_times.iter().all(|t| t.contains("2024-05-01")));
}

#[test]
fn live_sequence_match_events() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let params = types::PatternParams {
        species_sequence: vec![
            "European Robin".into(),
            "Eurasian Blackbird".into(),
            "Eurasian Wren".into(),
        ],
        max_gap_minutes: None,
        hour_start: 4,
        hour_end: 8,
    };
    let results = db.sequence_match_events(&params).unwrap();
    eprintln!("[live] sequence_match_events: {results:?}");
    let steps = |d: &str| {
        results
            .iter()
            .find(|f| f.date.starts_with(d))
            .unwrap()
            .step_times
            .len()
    };
    // Verified against the real extension: sequence_match_events returns the
    // timestamps of the longest in-order prefix reached — like
    // window_funnel_events (cf. live_funnel_events), a partial run still yields
    // its steps. Day2 reaches Robin->Blackbird before the missing Wren stops it.
    assert_eq!(
        steps("2024-05-01"),
        3,
        "day1 full R->B->W -> 3 matched times"
    );
    assert_eq!(
        steps("2024-05-02"),
        2,
        "day2 Robin->Blackbird, no Wren -> 2-step prefix"
    );
    assert_eq!(
        steps("2024-05-03"),
        3,
        "day3 full R->B->W -> 3 matched times"
    );
    // The matched timestamps are real, date-stamped values.
    let day1 = results
        .iter()
        .find(|f| f.date.starts_with("2024-05-01"))
        .unwrap();
    assert!(day1.step_times.iter().all(|t| t.contains("2024-05-01")));
}

#[test]
fn live_next_species() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let preds = db.next_species("European Robin", 60, 5).unwrap();
    eprintln!("[live] next_species: {preds:?}");
    // In each daily activity session the first Robin is followed by a Blackbird.
    assert_eq!(
        preds.first().map(|p| p.predicted_species.as_str()),
        Some("Eurasian Blackbird")
    );
    assert_eq!(preds[0].frequency, 3);
    assert!((preds[0].probability - 1.0).abs() < 1e-9);
}

#[test]
fn live_retention() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    let params = types::RetentionParams {
        intervals: vec![1, 2, 3],
        min_detections: 1,
    };
    let ret = db.retention(&params).unwrap();
    eprintln!("[live] retention: {ret:?}");
    let rate = |sp: &str, days: u32| {
        ret.iter()
            .find(|r| r.species == sp)
            .and_then(|r| r.retention_rates.iter().find(|x| x.days == days))
            .map(|x| x.rate)
            .unwrap()
    };
    // Robin anchors {01,02,03}: returns within 1 day for 01 & 02, not 03 -> 2/3.
    assert!((rate("European Robin", 1) - 2.0 / 3.0).abs() < 1e-6);
    // Wren anchors {01,03}: day1 has no return within 1 day (no Wren on 02),
    // but does within 2 days (Wren on 03); day3 never returns.
    assert!((rate("Eurasian Wren", 1) - 0.0).abs() < 1e-6);
    assert!((rate("Eurasian Wren", 2) - 0.5).abs() < 1e-6);
}

/// Confirm the raw SQL the builders emit parses and runs (defends against
/// SQL-shape regressions even if the typed wrappers change).
#[test]
fn live_raw_queries_execute() {
    let Some((db, _tmp)) = loaded_db() else {
        return;
    };
    seed(&db);
    for sql in [
        queries::sessionize_sql(&types::SessionizeParams::default()),
        queries::retention_sql(&types::RetentionParams::default()),
        queries::funnel_sql(&types::FunnelParams::default()),
        queries::sequence_match_sql(&types::PatternParams::default()),
        queries::sequence_match_events_sql(&types::PatternParams::default()),
        queries::next_species_sql("European Robin", 60, 10),
    ] {
        // execute_batch runs the SELECT to completion and surfaces any
        // unknown-function / type / syntax error from the real engine.
        db.conn()
            .execute_batch(&sql)
            .unwrap_or_else(|e| panic!("query failed to execute: {e}\n--- SQL ---\n{sql}"));
    }
}
