//! A file that is not a database must not pass for a healthy one.
//!
//! # What was wrong
//!
//! `check_integrity` ran `PRAGMA quick_check` and nothing else. SQLite opens a
//! **zero-length file as a brand-new empty database** — that is by design, and
//! it is how every database in this project gets created — so `quick_check`
//! returns `"ok"` on a `birds.db` that has been truncated to nothing.
//!
//! `check_and_recover` therefore took its `Ok(true)` branch, logged *"database
//! healthy"*, returned `RecoveryAction::None`, and `src/app.rs` carried on.
//! `migrate()` then built a fresh schema into the empty file and the station
//! started recording into it — with five good backups sitting beside it that
//! the ring rotates out in about 35 days.
//!
//! Truncation to zero is not exotic on the hardware this runs on. It is what a
//! power cut during an SD card's own wear-levelling relocation produces, what a
//! filesystem repair does to a file whose inode survived and whose extents did
//! not, and what a partly-restored backup leaves behind.
//!
//! # What this gate holds
//!
//! Three shapes of "this is not a database" — empty, too short to hold the
//! header, and header-shaped-but-wrong — must all be refused, and the ordinary
//! healthy database must still pass. The last is the discrimination: a
//! `check_integrity` that simply returned `false` would satisfy the first three
//! and destroy every station on earth.
//!
//! Observed failing before the fix: `a_zero_length_database_is_not_healthy` and
//! `recovery_restores_a_zero_length_database_from_its_backup` both fail, the
//! first with `quick_check said "ok"`.

use std::path::Path;

/// A database with a known number of rows, and a verified backup beside it.
fn station(dir: &Path, rows: i64) -> (std::path::PathBuf, std::path::PathBuf) {
    let db = dir.join("birds.db");
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).expect("backup dir");

    let conn = rusqlite::Connection::open(&db).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for i in 0..rows {
        conn.execute(
            "INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap, File_Name)
             VALUES ('2026-05-19', '06:30:00', 'Pica pica', 'Eurasian Magpie',
                     0.9, 0.7, 19, 1.25, 0.0, ?1)",
            rusqlite::params![format!("clip-{i}.wav")],
        )
        .expect("insert");
    }
    drop(conn);

    let backup = birdnet_db::resilience::backup_database(&db, &backups).expect("backup");
    assert!(
        birdnet_db::resilience::check_integrity(&backup).expect("verify backup"),
        "the fixture's backup must be good, or the recovery half proves nothing"
    );
    (db, backups)
}

fn rows_in(db: &Path) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open");
    conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count")
}

#[test]
fn a_zero_length_database_is_not_healthy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    std::fs::write(&db, b"").expect("write empty");
    assert_eq!(std::fs::metadata(&db).expect("stat").len(), 0);

    assert!(
        !birdnet_db::resilience::check_integrity(&db).expect("check"),
        "a zero-length file is not a database; SQLite opens it as a brand-new \
         empty one and quick_check said \"ok\""
    );
}

#[test]
fn a_file_too_short_for_the_header_is_not_healthy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    // The first eight bytes of a real header, and then nothing — a torn write
    // that landed inside the magic string itself.
    std::fs::write(&db, b"SQLite f").expect("write short");

    assert!(
        !birdnet_db::resilience::check_integrity(&db).expect("check"),
        "a file too short to hold SQLite's 16-byte magic is not a database"
    );
}

#[test]
fn a_file_with_the_wrong_magic_is_not_healthy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    // Right length, wrong contents: what a filesystem repair leaves when it
    // reallocates a block from somewhere else.
    let mut junk = b"NOT A DATABASE!!".to_vec();
    junk.extend(std::iter::repeat_n(0_u8, 4_096));
    std::fs::write(&db, &junk).expect("write junk");

    assert!(
        !birdnet_db::resilience::check_integrity(&db).expect("check"),
        "a file whose first sixteen bytes are not SQLite's magic is not a database"
    );
}

/// The discrimination. A `check_integrity` that returned `false` for everything
/// would pass all three tests above and quarantine every healthy station on
/// earth at its next boot.
#[test]
fn an_ordinary_healthy_database_still_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db, backups) = station(dir.path(), 25);

    assert!(
        birdnet_db::resilience::check_integrity(&db).expect("check"),
        "a real database must still pass"
    );

    let result = birdnet_db::resilience::check_and_recover(&db, &backups).expect("recover");
    assert!(result.healthy, "a real database must be reported healthy");
    assert_eq!(
        result.action,
        birdnet_db::resilience::RecoveryAction::None,
        "a healthy database must not be restored over"
    );
    assert_eq!(rows_in(&db), 25, "and must not be touched");
}

/// The whole point: the station must come back with its history, not with a
/// fresh schema and five good backups it will rotate away.
#[test]
fn recovery_restores_a_zero_length_database_from_its_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db, backups) = station(dir.path(), 25);

    // Truncate to nothing, the way a power cut during a wear-levelling
    // relocation does.
    std::fs::write(&db, b"").expect("truncate");
    assert_eq!(std::fs::metadata(&db).expect("stat").len(), 0);

    let result = birdnet_db::resilience::check_and_recover(&db, &backups)
        .expect("a good backup exists, so recovery must succeed");

    assert_eq!(
        result.action,
        birdnet_db::resilience::RecoveryAction::Recovered,
        "an empty database with a good backup beside it must be restored, not \
         reported healthy: {}",
        result.details
    );
    assert_eq!(
        rows_in(&db),
        25,
        "the restored database must carry the history back"
    );
}
