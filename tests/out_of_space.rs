//! What a full disk does to a station that is still recording.
//!
//! # The hole this closes
//!
//! An SD card in an outdoor enclosure fills. That is not an edge case, it is
//! the schedule: audio clips accumulate until the disk manager's purge cannot
//! keep up, or the purge is off, or the operator raised the retention. The
//! repository had a `DiskManager` with a full-disk backstop, a doctor check
//! that grades free space, and a `station_health` alert — three layers of
//! *prediction* — and not one test of what happens when a write actually
//! fails. Every claim about that path was a comment.
//!
//! Two of those comments are load-bearing:
//!
//! * `resilience::backup_database` says a part-finished backup "used to leave
//!   its truncated file behind ... it sorts newest, so it became the first
//!   thing recovery reached for". A worthless newest backup on a station whose
//!   database has just been damaged is how a power cut becomes data loss.
//! * `maintenance.rs` says the in-process floor exists for "read-only
//!   filesystem, disk full — the very conditions maintenance is meant to
//!   survive". That half is tested; the write itself was not.
//!
//! # How a full disk is produced here
//!
//! `/dev/full` returns `ENOSPC` on every write and reads as zeroes, and a
//! symlink pointing at it is a destination SQLite will open as a new database
//! and then fail to write. Probed as root in this container, that is a genuine
//! `errno 28` from the kernel, not a simulated error:
//! `SqliteFailure(DiskFull, "database or disk is full")`.
//!
//! It is not `ENOSPC` everywhere, and assuming it was is what made the first
//! version of this test fail in CI. Three observations, all first-hand:
//!
//! ```text
//! as root, this container        sqlite error: database or disk is full
//! as uid 65534, this container   sqlite error: database or disk is full
//! GitHub runner                  sqlite error: attempt to write a readonly database
//! ```
//!
//! The first guess was that the runner's non-root user could not create
//! SQLite's auxiliary journal beside a `/dev` path. Running the same test
//! binary under `setpriv --reuid=65534` here disproved it — still `ENOSPC`. So
//! the runner's cause is **not established**; the likeliest remaining
//! explanation is that its environment does not present a writable `/dev/full`
//! at all, but that is a guess and is labelled as one.
//!
//! What is established is that the scenario is the same in every case — a
//! backup destination that cannot be written — and so is the invariant this
//! file exists to check. Only the errno differs, so the assertion is on the
//! invariant, and the reported error is printed so a fourth mode arrives in
//! the log rather than silently widening what this tolerates.
//!
//! It needs no root and no mount, which matters: a `mount -t tmpfs` harness
//! would be skipped on any CI runner without `CAP_SYS_ADMIN`, and a gate that
//! skips where it counts is the defect this repository closed in D13.
//!
//! For the *source* database the equivalent is `PRAGMA max_page_count`, which
//! makes SQLite return the same `SQLITE_FULL` it returns for `ENOSPC` on the
//! main file. That covers the application's error path exactly; what it does
//! not cover is the WAL file failing to grow, which is stated here rather than
//! left to be assumed.

use birdnet_db::sqlite::{DetectionRecord, insert_detection};
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A migrated, checkpointed database with a handful of detections in it.
fn seeded_station() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = dir.path().join("birds.db");
    let conn = Connection::open(&db).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for i in 0..20 {
        insert_detection(&conn, &detection(i)).expect("seed insert");
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
    drop(conn);
    (dir, db)
}

/// One detection, distinct per `i`.
fn detection(i: usize) -> DetectionRecord<'static> {
    // Leaked so the record can borrow for 'static without threading lifetimes
    // through every call site; a handful of small strings in a test process.
    let time: &'static str = Box::leak(
        format!("{:02}:{:02}:{:02}", (i / 3600) % 24, (i / 60) % 60, i % 60).into_boxed_str(),
    );
    let file_name: &'static str = Box::leak(format!("full-{i:08}.wav").into_boxed_str());
    DetectionRecord {
        date: "2026-03-16",
        time,
        sci_name: "Turdus merula",
        com_name: "Eurasian Blackbird",
        confidence: 0.8,
        lat: None,
        lon: None,
        cutoff: None,
        week: Some(11),
        sensitivity: None,
        overlap: None,
        file_name,
        chunk_offset_secs: Some(0.0),
        correlation_id: None,
        source: None,
        duration_secs: None,
        detected_at_utc: None,
    }
}

/// Buckets where `species_summary` disagrees with a recount of `detections`.
///
/// The bucket expression and the `review_verdict` filter are copied from the
/// triggers in migration 24, not re-derived.
fn rollup_disagreements(conn: &Connection) -> i64 {
    conn.query_row(
        "WITH recomputed AS (
             SELECT Com_Name, Sci_Name, SUBSTR(Time, 1, 2) AS hour,
                    COUNT(*) AS detections, SUM(Confidence) AS confidence_sum
             FROM detections
             WHERE review_verdict IS NOT 'rejected'
             GROUP BY Com_Name, Sci_Name, hour
         )
         SELECT COUNT(*) FROM (
             SELECT r.Com_Name FROM recomputed r
             FULL OUTER JOIN species_summary s
               ON s.Com_Name = r.Com_Name AND s.Sci_Name = r.Sci_Name AND s.hour = r.hour
             WHERE r.Com_Name IS NULL OR s.Com_Name IS NULL
                OR s.detections <> r.detections
                OR ABS(s.confidence_sum - r.confidence_sum) > 1e-9
         )",
        [],
        |r| r.get::<_, i64>(0),
    )
    .expect("rollup comparison")
}

/// Fill `backup_dir` with symlinks to `/dev/full` covering the timestamps
/// `backup_database` might pick, so wherever it writes, the write fails.
///
/// Returns the names created.
fn pave_with_full_device(backup_dir: &std::path::Path) -> Vec<String> {
    std::fs::create_dir_all(backup_dir).expect("create backup dir");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    // A generous window: the call takes milliseconds, but a loaded runner can
    // stall, and a missed second would silently turn this into a test of a
    // successful backup.
    (now.saturating_sub(2)..now + 30)
        .map(|t| {
            let name = format!("birds.db.backup.{t}");
            std::os::unix::fs::symlink("/dev/full", backup_dir.join(&name))
                .expect("symlink to /dev/full");
            name
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The backup ring on a full disk
// ---------------------------------------------------------------------------

/// A backup that cannot be written must leave nothing behind.
///
/// The danger is specific: backups are found by sorting names, which sort by
/// timestamp, so a truncated file from a failed backup is the *newest* one and
/// therefore the first candidate `check_and_recover` reaches for. On a station
/// whose database was just damaged by the same power event that filled the
/// disk, restoring from it would replace a damaged database with an empty one.
#[test]
fn a_backup_that_runs_out_of_space_leaves_no_file_behind() {
    let (dir, db) = seeded_station();
    let backup_dir = dir.path().join("backups");
    let paved = pave_with_full_device(&backup_dir);

    let err = birdnet_db::resilience::backup_database(&db, &backup_dir)
        .expect_err("a backup onto an unwritable device must fail");
    let msg = err.to_string().to_ascii_lowercase();

    // How the write fails is not the same everywhere, and asserting one
    // spelling of it was wrong — see the module doc for the three readings and
    // for what is and is not established about why they differ. Both known
    // modes are the case under test: a backup that cannot be written. The
    // invariant below is what actually matters, and it is identical either
    // way.
    //
    // Printed rather than silently accepted, so a *third* mode shows up in the
    // log instead of quietly widening what this test tolerates.
    eprintln!("[out_of_space] the failed backup reported: {msg}");
    assert!(
        msg.contains("full") || msg.contains("readonly") || msg.contains("read-only"),
        "the backup should have failed because the destination could not be \
         written, but it failed for some other reason: {msg}"
    );

    // Exactly one destination was consumed — the one it chose — and it was
    // removed rather than left as a zero-length newest-backup.
    let remaining: Vec<String> = std::fs::read_dir(&backup_dir)
        .expect("read backup dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        remaining.len(),
        paved.len() - 1,
        "exactly one destination should have been used and removed"
    );

    // And nothing that survived is a real file: a partial snapshot would be.
    for name in &remaining {
        let meta = std::fs::symlink_metadata(backup_dir.join(name)).expect("stat");
        assert!(
            meta.file_type().is_symlink(),
            "{name} is a regular file — a partial backup was left behind"
        );
    }

    // The recovery path must therefore find nothing real. The paving symlinks
    // are this harness's own scaffolding and would themselves be found, so they
    // come out first — leaving whatever the failed backup actually left, which
    // must be nothing.
    for name in &remaining {
        std::fs::remove_file(backup_dir.join(name)).expect("remove scaffolding");
    }
    assert!(
        birdnet_db::resilience::find_latest_backup(&backup_dir, "birds.db").is_none(),
        "recovery must not be offered a backup that was never completed"
    );
}

/// The counterpart, so the test above is not satisfied by "backups never
/// work": with a writable directory a backup is produced, it is a real
/// database, and recovery finds it.
#[test]
fn a_backup_with_space_available_is_written_and_is_valid() {
    let (dir, db) = seeded_station();
    let backup_dir = dir.path().join("backups");

    let path = birdnet_db::resilience::backup_database(&db, &backup_dir).expect("backup");
    assert!(path.is_file(), "the backup is a real file");

    let restored = Connection::open(&path).expect("open the backup");
    let ok: String = restored
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(ok, "ok", "the backup is a valid database");
    let rows: i64 = restored
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 20, "and it carries the station's detections");

    assert_eq!(
        birdnet_db::resilience::find_latest_backup(&backup_dir, "birds.db"),
        Some(path),
        "recovery finds a completed backup"
    );
}

// ---------------------------------------------------------------------------
// The detection path on a full database
// ---------------------------------------------------------------------------

/// A full database must refuse the write, not lose it quietly.
///
/// This is the difference between an operator who sees an error and one whose
/// station appears to be running while recording nothing.
#[test]
fn a_full_database_refuses_the_insert_rather_than_dropping_it() {
    let (_dir, db) = seeded_station();
    let conn = birdnet_db::sqlite::open_connection(&db).expect("open");

    // Cap the file at its current size: the next page allocation is SQLITE_FULL,
    // which is exactly what ENOSPC on the main database file produces.
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .expect("page_count");
    conn.execute_batch(&format!("PRAGMA max_page_count = {pages};"))
        .expect("cap");

    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");

    // Enough inserts that page allocation is certainly required.
    let mut errors: i64 = 0;
    for i in 1000..1500 {
        if insert_detection(&conn, &detection(i)).is_err() {
            errors += 1;
        }
    }
    assert!(
        errors > 0,
        "a database that cannot grow must report failures; it reported none, so \
         either the cap did not apply or errors are being swallowed"
    );

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        after - before,
        500 - errors,
        "{errors} of 500 inserts reported an error, so exactly {} rows should \
         have landed — a mismatch means a write that reported failure was \
         committed anyway, or one that reported success was not",
        500 - errors
    );

    // The important part: a refused write leaves no half-applied state. The
    // row and its rollup update share a transaction, so either both happened
    // or neither did.
    assert_eq!(
        rollup_disagreements(&conn),
        0,
        "species_summary drifted from detections when the database filled — no \
         integrity check would ever report this, and every count on the \
         dashboard would be wrong from here on"
    );
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(integrity, "ok", "a full database must not be a corrupt one");
}

/// The counterpart: the same 500 inserts must all succeed when the cap is not
/// in place, so the test above is measuring the cap and not a broken fixture.
#[test]
fn the_same_inserts_succeed_when_there_is_room() {
    let (_dir, db) = seeded_station();
    let conn = birdnet_db::sqlite::open_connection(&db).expect("open");
    for i in 1000..1500 {
        insert_detection(&conn, &detection(i)).expect("insert with room to spare");
    }
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 520);
    assert_eq!(rollup_disagreements(&conn), 0);
}

/// Freeing space must be enough — no restart, no manual repair.
///
/// This is what the disk manager's purge does in production: it deletes clips
/// and the station carries on. If the connection were left wedged after a
/// `SQLITE_FULL`, the purge would free space to no effect and the station would
/// stay silent until someone noticed.
#[test]
fn the_station_resumes_recording_once_space_is_freed() {
    let (_dir, db) = seeded_station();
    let conn = birdnet_db::sqlite::open_connection(&db).expect("open");

    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .expect("page_count");
    conn.execute_batch(&format!("PRAGMA max_page_count = {pages};"))
        .expect("cap");
    let mut hit_the_wall = false;
    for i in 1000..1500 {
        if insert_detection(&conn, &detection(i)).is_err() {
            hit_the_wall = true;
            break;
        }
    }
    assert!(hit_the_wall, "the cap never bit; this tested nothing");

    // The purge frees space.
    conn.execute_batch("PRAGMA max_page_count = 1000000;")
        .expect("raise the cap");

    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    insert_detection(&conn, &detection(999_999))
        .expect("the same connection must record again once there is room");
    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(after, before + 1);
    assert_eq!(
        rollup_disagreements(&conn),
        0,
        "the first write after recovery must keep the rollup exact"
    );
}
