//! The locked-clip set is read every 60 seconds, so it must not scan the table.
//!
//! # What this is defending
//!
//! `locked_file_names` — `SELECT DISTINCT File_Name FROM detections WHERE
//! is_locked = 1 AND File_Name IS NOT NULL` — is re-read by the disk manager on
//! every purge cycle (`check_interval_secs: 60`) so that locking a clip in
//! `/admin/recordings` takes effect without a restart. That design is right.
//!
//! Through migration 32 the supporting index was a plain `detections(is_locked)`.
//! `is_locked` is 0 for essentially every row, so `ANALYZE` tells the planner the
//! column has one or two distinct values and a seek buys little — and on a real
//! history it concludes a seek buys nothing at all. Measured on a three-year,
//! 3 285 000-row fixture with forty clips locked, the plan was
//! `SCAN detections | USE TEMP B-TREE FOR DISTINCT` at **267.6 ms per run,
//! 1 440 times a day**, holding the connection the detection writer also needs.
//! The partial index in migration 33 takes it to **0.16 ms** and shrinks the
//! index from 29.6 MB to 4.1 kB.
//!
//! On the small fixture these tests can build, migration 32's index is still
//! *chosen* — the degradation to an outright scan needs the statistics a real
//! history produces. What holds at both sizes is that the index does not cover
//! the query, so that is the assertion this leans on; see
//! [`the_locked_clip_read_uses_a_covering_index_not_a_scan`] for both readings.
//!
//! A timing assertion would be flaky, so this asserts the *plan* instead: the
//! planner must choose the index, and it must cover the query. That is the
//! property that produced the speed, and it is the one a future migration could
//! silently take away.

use rusqlite::Connection;

fn station(locked: usize, unlocked: usize) -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let insert = |i: usize, is_locked: i64| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, is_locked)
             VALUES ('2026-06-15', ?1, 'Erithacus rubecula', 'European Robin', 0.9, ?2, ?3)",
            rusqlite::params![
                format!("{:02}:{:02}:{:02}", i / 3600, (i / 60) % 60, i % 60),
                format!("clip-{i}.wav"),
                is_locked
            ],
        )
        .expect("insert");
    };
    for i in 0..unlocked {
        insert(i, 0);
    }
    for i in unlocked..(unlocked + locked) {
        insert(i, 1);
    }
    conn.execute_batch("ANALYZE;").expect("analyze");
    conn
}

const LOCKED_QUERY: &str =
    "SELECT DISTINCT File_Name FROM detections WHERE is_locked = 1 AND File_Name IS NOT NULL";

fn plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare");
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    rows.join(" | ")
}

/// Observed failing against migration 32's `CREATE INDEX idx_detections_locked
/// ON detections(is_locked)`. What it reports depends on the size of the
/// database, and both readings are worth recording because only one of them is
/// reproducible in a unit test:
///
/// * on this 4 040-row fixture:
///   ```text
///   the index must carry File_Name so the query needs no table lookup:
///   SEARCH detections USING INDEX idx_detections_locked (is_locked=?)
///     | USE TEMP B-TREE FOR DISTINCT
///   ```
///   The index is chosen, but every match costs a table lookup and the
///   `DISTINCT` needs a temporary b-tree.
/// * on the three-year, 3 285 000-row fixture the plan degrades further, to
///   `SCAN detections | USE TEMP B-TREE FOR DISTINCT` at **267.6 ms**, because
///   `ANALYZE` reports one or two distinct values in `is_locked` and the
///   planner concludes a seek buys nothing. That is the version that actually
///   ran on stations, once a minute.
///
/// The covering assertion is the one that holds at every size, so it is the one
/// this gate leans on; the `SCAN` assertion catches the large-database form.
#[test]
fn the_locked_clip_read_uses_a_covering_index_not_a_scan() {
    let conn = station(40, 4_000);
    let p = plan(&conn, LOCKED_QUERY);

    assert!(
        !p.contains("SCAN detections"),
        "the locked-clip read must not scan the detections table — it runs \
         every 60 s: {p}"
    );
    assert!(
        p.contains("idx_detections_locked"),
        "the planner must choose the locked index: {p}"
    );
    assert!(
        p.contains("COVERING INDEX"),
        "the index must carry File_Name so the query needs no table lookup: {p}"
    );
}

/// The counterpart: the index must still return the right rows, not merely be
/// chosen. A partial index whose predicate did not match the query would be
/// ignored (slow but correct); one that matched too loosely would be fast and
/// wrong, and only this notices that.
#[test]
fn the_partial_index_returns_exactly_the_locked_clips() {
    let conn = station(40, 4_000);
    let names = birdnet_db::sqlite::locked_file_names(&conn).expect("read");
    assert_eq!(names.len(), 40, "every locked clip, and only those");
    assert!(
        names.iter().all(|n| {
            let i: usize = n
                .trim_start_matches("clip-")
                .trim_end_matches(".wav")
                .parse()
                .expect("name");
            i >= 4_000
        }),
        "an unlocked clip leaked into the locked set"
    );
}

/// A station where nothing is locked — the common case — must also not scan,
/// and must come back empty rather than with everything.
#[test]
fn a_station_with_nothing_locked_still_does_not_scan() {
    let conn = station(0, 4_000);
    let p = plan(&conn, LOCKED_QUERY);
    assert!(!p.contains("SCAN detections"), "{p}");
    assert!(
        birdnet_db::sqlite::locked_file_names(&conn)
            .expect("read")
            .is_empty()
    );
}

/// The three indexes migration 33 retires must be gone, and the ones that earn
/// their keep must stay. Without this, a future migration could re-add a
/// 46 MB index nothing reads and nothing would say so.
#[test]
fn the_retired_indexes_are_gone_and_the_earning_ones_remain() {
    let conn = station(1, 10);
    let names: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'index' AND name LIKE 'idx_detections_%'")
            .expect("prepare");
        stmt.query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    };

    for retired in [
        "idx_detections_chunk_offset",
        "idx_detections_correlation_id",
        "idx_detections_source",
    ] {
        assert!(
            !names.iter().any(|n| n == retired),
            "{retired} is read by no production query and must not come back \
             without one: {names:?}"
        );
    }
    for kept in [
        "idx_detections_date",
        "idx_detections_datetime",
        "idx_detections_date_species",
        "idx_detections_locked",
        "idx_detections_import_batch",
        "idx_detections_utc",
    ] {
        assert!(
            names.iter().any(|n| n == kept),
            "{kept} is load-bearing and disappeared: {names:?}"
        );
    }
}
