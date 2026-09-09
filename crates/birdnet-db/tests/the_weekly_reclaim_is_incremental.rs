//! The weekly space reclaim moves free pages, not the whole file (PS-3).
//!
//! The job used to be `VACUUM`: a rewrite of the database through the temp
//! directory and back — 3.0× the file size written, measured at 274.7 MB for
//! 91.3 MB — staged in the unit's `PrivateTmp` tmpfs inside `MemoryMax=1G`,
//! under one exclusive lock past which a detection insert timed out and was
//! logged lost. Five gates:
//!
//! 1. A database this binary creates is in `auto_vacuum=INCREMENTAL` from its
//!    first table.
//! 2. A database from before is converted once, with the rewrite's copy staged
//!    beside the file rather than in the temp directory, and then left alone.
//! 3. The weekly reclaim returns every free page and issues no `VACUUM` —
//!    read from SQLite's statement trace, not inferred from the file size,
//!    because a `VACUUM` shrinks the file too.
//! 4. On a file that is not incremental the reclaim refuses rather than
//!    falling back to the rewrite it replaces.
//! 5. The live writer waits fifteen seconds for a lock, not five.

use std::sync::{Mutex, MutexGuard};

use birdnet_db::resilience::{
    self, AUTO_VACUUM_INCREMENTAL, ResilienceError, VacuumMode, auto_vacuum_mode,
};
use rusqlite::Connection;
use rusqlite::trace::{TraceEvent, TraceEventCodes};

/// Statements SQLite ran on a traced connection. One log for the binary, so
/// the tests that read it take `TRACE` for their whole body.
static STATEMENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static TRACE: Mutex<()> = Mutex::new(());

fn record(event: TraceEvent<'_>) {
    if let TraceEvent::Stmt(stmt, _) = event {
        STATEMENTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(stmt.sql().into_owned());
    }
}

fn trace(conn: &Connection, on: bool) {
    conn.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, on.then_some(record));
}

fn traced() -> MutexGuard<'static, ()> {
    let guard = TRACE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    STATEMENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    guard
}

fn statements() -> Vec<String> {
    STATEMENTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn freelist(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .expect("freelist_count")
}

/// A table of 200 4 KiB rows, then deleted: ~200 free pages.
fn fill_and_delete(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bulk(i INTEGER PRIMARY KEY, pad BLOB); \
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 200) \
         INSERT INTO bulk(pad) SELECT zeroblob(4000) FROM n; \
         DELETE FROM bulk;",
    )
    .expect("fill and delete");
}

/// A database made the way a station from before this change made it:
/// tables created with the default `auto_vacuum=NONE`.
fn legacy_database(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).expect("open");
    conn.execute_batch("CREATE TABLE detections(x); INSERT INTO detections VALUES (1);")
        .expect("legacy schema");
    assert_eq!(
        auto_vacuum_mode(&conn).unwrap(),
        0,
        "the fixture must start non-incremental"
    );
    conn
}

#[test]
fn a_database_this_binary_creates_is_incremental_from_its_first_table() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("birds.db");
    let conn = Connection::open(&path).unwrap();
    birdnet_db::migration::migrate(&conn).expect("migrate");
    assert_eq!(
        auto_vacuum_mode(&conn).unwrap(),
        AUTO_VACUUM_INCREMENTAL,
        "migrate must set auto_vacuum=INCREMENTAL before the first table exists; \
         afterwards only a full VACUUM can"
    );
}

#[test]
fn a_database_from_before_is_converted_once_beside_itself_and_then_left_alone() {
    let _t = traced();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("birds.db");
    drop(legacy_database(&path));

    let first = resilience::ensure_incremental_vacuum(&path).expect("convert");
    assert!(
        matches!(first, VacuumMode::Converted { .. }),
        "a non-incremental file must be converted, got {first:?}"
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(auto_vacuum_mode(&conn).unwrap(), AUTO_VACUUM_INCREMENTAL);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM detections", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "the conversion must keep the rows");

    let second = resilience::ensure_incremental_vacuum(&path).expect("second look");
    assert_eq!(
        second,
        VacuumMode::AlreadyIncremental,
        "a converted file must not be rewritten again on the next start"
    );

    // The one-time rewrite stages its copy beside the database, not in the
    // temp directory that is a memory-charged tmpfs under the unit. Traced on
    // the conversion itself, on a fresh legacy file.
    let path2 = tmp.path().join("other.db");
    let conn2 = legacy_database(&path2);
    trace(&conn2, true);
    resilience::convert_to_incremental(&conn2, tmp.path()).expect("convert traced");
    let dir = tmp.path().to_string_lossy().into_owned();
    let staged_beside = statements()
        .iter()
        .any(|sql| sql.to_ascii_lowercase().contains("temp_store_directory") && sql.contains(&dir));
    assert!(
        staged_beside,
        "the conversion must set temp_store_directory to the database's directory; \
         statements: {:?}",
        statements()
    );
}

#[test]
fn the_weekly_reclaim_returns_every_free_page_without_a_vacuum() {
    let _t = traced();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("birds.db");
    let conn = Connection::open(&path).unwrap();
    birdnet_db::migration::migrate(&conn).expect("migrate");
    fill_and_delete(&conn);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let free_before = freelist(&conn);
    assert!(
        free_before >= 100,
        "the fixture must leave free pages to reclaim, has {free_before}"
    );
    let size_before = std::fs::metadata(&path).unwrap().len();

    trace(&conn, true);
    let freed = resilience::reclaim_free_pages_on(&conn).expect("reclaim");
    trace(&conn, false);

    assert_eq!(freelist(&conn), 0, "every free page must be returned");
    assert_eq!(
        freed,
        u64::try_from(free_before).unwrap(),
        "the report must count the pages it freed"
    );
    let size_after = std::fs::metadata(&path).unwrap().len();
    assert!(
        size_after < size_before,
        "the file must shrink: {size_before} -> {size_after}"
    );
    let vacuumed: Vec<String> = statements()
        .into_iter()
        .filter(|sql| sql.trim_start().to_ascii_uppercase().starts_with("VACUUM"))
        .collect();
    assert!(
        vacuumed.is_empty(),
        "the weekly reclaim issued a full VACUUM — the rewrite this exists to replace: {vacuumed:?}"
    );
    let stepped = statements()
        .iter()
        .any(|sql| sql.to_ascii_lowercase().contains("incremental_vacuum"));
    assert!(
        stepped,
        "no incremental_vacuum step was issued; statements: {:?}",
        statements()
    );
}

#[test]
fn a_file_that_is_not_incremental_is_refused_rather_than_rewritten() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("birds.db");
    let conn = legacy_database(&path);
    fill_and_delete(&conn);
    drop(conn);
    let err =
        resilience::reclaim_free_pages(&path).expect_err("must refuse a non-incremental file");
    assert!(
        matches!(err, ResilienceError::AutoVacuum(_)),
        "the refusal must name the mode, got {err}"
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        auto_vacuum_mode(&conn).unwrap(),
        0,
        "the refusal must not have converted it"
    );
}

#[test]
fn the_live_writer_waits_fifteen_seconds_for_a_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("birds.db");
    let writer = birdnet_db::sqlite::open_or_create(&path).expect("open");
    let ms: i64 = writer
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        ms,
        i64::from(birdnet_db::sqlite::connection::WRITER_BUSY_TIMEOUT_MS),
        "the writer's busy_timeout"
    );
    assert_eq!(
        ms, 15_000,
        "a detection is worth a fifteen-second wait, not five"
    );
    let reader = birdnet_db::sqlite::open_readonly(&path).expect("open read-only");
    let reader_ms: i64 = reader
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        reader_ms, 5_000,
        "the readers keep the five seconds a page load can afford"
    );
}
