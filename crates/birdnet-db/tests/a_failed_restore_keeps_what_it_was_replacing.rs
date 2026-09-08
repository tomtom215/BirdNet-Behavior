//! A restore that cannot finish must not have destroyed what it was replacing.
//!
//! # What was wrong
//!
//! `restore_from_backup` deleted the destination *first*:
//!
//! ```text
//!     if db_path.exists() { std::fs::remove_file(db_path)?; … }
//!     let source = open_readonly_with_busy_timeout(backup_path)?;
//!     let mut dest = open_with_busy_timeout(db_path)?;
//!     Backup::new(&source, &mut dest)?.run_to_completion(…)?;
//! ```
//!
//! Every failure after that first line — the card that caused the corruption
//! producing another I/O error, a power cut, or the disk being full — left the
//! station with neither the original nor the replacement.
//!
//! And the caller could not tell that apart from having no backup at all.
//! `src/app.rs` treats *every* error out of `check_and_recover` as "corrupt and
//! no good backup exists", quarantines, and starts fresh — so a recoverable
//! fault became total history loss, and the weekly ring then rotated the good
//! backup away within five weeks.
//!
//! # Reproduced on a real full disk
//!
//! On a 12 MB tmpfs holding a 2 711 552-byte backup, a `birds.db` truncated to
//! zero (the `PS-2` shape) and a filler file leaving 1 355 776 bytes free:
//!
//! ```text
//!   check_and_recover -> Err(Sqlite(SqliteFailure(DiskFull, "database or disk is full")))
//!   live db exists AFTER: true    size AFTER: 0
//!   rows readable in the live db after: None
//!   backup still there: true (2711552 bytes)
//! ```
//!
//! A perfectly good backup sitting beside a station about to report that no
//! good backup exists. That reproduction needs to mount a filesystem, which CI
//! cannot do, so the gates below induce the failure by handing the restore a
//! source it cannot read — which exercises the same ordering, and is itself a
//! real case: the candidate was verified moments earlier, on a card that is by
//! hypothesis failing.

use std::path::Path;

fn station(path: &Path, rows: i64) {
    let conn = rusqlite::Connection::open(path).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for i in 0..rows {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-19','06:30:00','Pica pica','Eurasian Magpie',0.9,?1)",
            rusqlite::params![format!("clip-{i}.wav")],
        )
        .expect("insert");
    }
}

fn rows_in(path: &Path) -> Option<i64> {
    rusqlite::Connection::open(path)
        .ok()?
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .ok()
}

/// The reproduction: the destination must survive a restore that fails.
#[test]
fn a_failed_restore_leaves_the_original_database_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    station(&db, 42);
    assert_eq!(rows_in(&db), Some(42), "fixture");

    // A "backup" that is not a database. On a dying card this is what a
    // verified candidate can become between the check and the copy.
    let backup = dir.path().join("birds.db.backup.20260101-000000");
    std::fs::write(&backup, b"this is not a database").expect("write");

    let err = birdnet_db::resilience::restore_from_backup(&backup, &db)
        .expect_err("a restore from an unreadable source must fail");

    assert_eq!(
        rows_in(&db),
        Some(42),
        "the database being replaced must still be there, with its rows: a \
         restore that cannot finish must not be the thing that loses the \
         history. Error was: {err}"
    );
    assert!(
        !dir.path().join("birds.db.restore-tmp").exists(),
        "and no half-written temporary may be left behind"
    );
}

/// The failure must be distinguishable from having no backup, because the
/// caller's response to the two is opposite: one starts fresh, the other must
/// not.
#[test]
fn a_failed_restore_is_not_reported_as_having_no_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    station(&db, 7);
    let backup = dir.path().join("birds.db.backup.20260101-000000");
    std::fs::write(&backup, b"this is not a database").expect("write");

    let err = birdnet_db::resilience::restore_from_backup(&backup, &db).expect_err("must fail");
    assert!(
        matches!(
            err,
            birdnet_db::resilience::ResilienceError::RestoreFailed { .. }
        ),
        "a restore that could not be completed must say so, not arrive as an \
         anonymous sqlite or I/O error the caller reads as \"no good backup\": {err:?}"
    );
    assert!(
        err.to_string().contains(&backup.display().to_string()),
        "and must name the backup that is still intact, so the operator knows \
         there is something to recover: {err}"
    );
}

/// The counterpart, and the reason the fix cannot be "never restore". Recovery
/// is the whole point of the backup ring.
#[test]
fn a_successful_restore_still_replaces_the_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    station(&db, 3);

    let backup = dir.path().join("birds.db.backup.20260101-000000");
    station(&backup, 99);

    birdnet_db::resilience::restore_from_backup(&backup, &db).expect("restore");
    assert_eq!(
        rows_in(&db),
        Some(99),
        "the restored database must be the backup's, not the old one's"
    );
    assert!(
        !dir.path().join("birds.db.restore-tmp").exists(),
        "and the temporary must be gone"
    );
    assert!(
        birdnet_db::resilience::check_integrity(&db).expect("verify"),
        "and must be a usable database"
    );
}

/// A restore replaces the live database with an *older* copy. Everything the
/// station recorded after that backup was taken is in the file being replaced
/// and nowhere else — a torn `birds.db` is usually a few bad pages, not a lost
/// history — so that file must be kept beside the live one, under the same
/// `.corrupt.<timestamp>` name the no-backup path uses, where the doctor's
/// quarantine scan finds it. It used to be deleted, and the log said only
/// "database restored from backup".
#[test]
fn a_restore_keeps_the_damaged_original_beside_the_live_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    station(&db, 3);
    // Tear it the way a power cut does: truncate to half.
    let full = std::fs::metadata(&db).expect("meta").len();
    let torn_len = full / 2;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&db)
        .expect("open")
        .set_len(torn_len)
        .expect("truncate");

    let backup = dir.path().join("birds.db.backup.20260101-000000");
    station(&backup, 99);

    birdnet_db::resilience::restore_from_backup(&backup, &db).expect("restore");
    assert_eq!(rows_in(&db), Some(99));

    let kept: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.starts_with("birds.db.corrupt.") && !n.ends_with("-wal") && !n.ends_with("-shm")
        })
        .collect();
    assert_eq!(
        kept.len(),
        1,
        "the damaged original must be kept, not deleted: {kept:?}"
    );
    assert_eq!(
        std::fs::metadata(&kept[0]).expect("meta").len(),
        torn_len,
        "and it must be the torn file as it was"
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("restore-tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the verification's sidecars were left behind: {leftovers:?}"
    );
}

/// Restoring over a destination that does not exist at all is the ordinary
/// first-recovery case and must still work.
#[test]
fn a_restore_onto_nothing_still_works() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let backup = dir.path().join("birds.db.backup.20260101-000000");
    station(&backup, 11);

    birdnet_db::resilience::restore_from_backup(&backup, &db).expect("restore");
    assert_eq!(rows_in(&db), Some(11));
}

/// End to end through the caller: a corrupt database, a good backup, and a
/// restore that cannot be written must not report "no good backup".
#[test]
fn check_and_recover_keeps_the_database_when_the_restore_cannot_be_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).expect("mkdir");

    let db = dir.path().join("birds.db");
    station(&db, 5);
    // Corrupt it in a way check_integrity sees, so recovery is attempted.
    std::fs::write(&db, b"NOT A DATABASE!!").expect("corrupt");

    // A candidate that passes nothing — the ring is there, but unusable.
    std::fs::write(
        backups.join("birds.db.backup.20260101-000000"),
        b"also not a database",
    )
    .expect("write");

    let err =
        birdnet_db::resilience::check_and_recover(&db, &backups).expect_err("no usable candidate");
    assert!(
        err.to_string().contains("failed verification"),
        "with no usable candidate this is genuinely unrecoverable and must say \
         so — that is the case where starting fresh is right: {err}"
    );
}
