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

// ---------------------------------------------------------------------------
// The second entry point.
//
// Everything above goes through `check_integrity`. That is the function
// `check_and_recover` calls at boot, and it is the only one the header guard
// was ever applied to. The *runtime* paths call `full_integrity_check`
// instead — the daily scheduled check (`src/maintenance.rs`), `--check-db`
// (`src/helpers/db.rs`) and `--doctor` (`src/doctor/database.rs`) — and it ran
// `PRAGMA integrity_check` with no guard at all.
//
// So a `birds.db` truncated to zero *while the station was running* was
// reported healthy by the daily check every day for the rest of the year, the
// ingest halt that a failed verdict is supposed to trip never tripped, and an
// operator who followed the manual's advice to run `--check-db` was told the
// database was fine. Only the next reboot could notice.
//
// Probed directly against SQLite before writing this, on a zero-length file
// opened read-only: `PRAGMA integrity_check` -> `ok`, `PRAGMA quick_check` ->
// `ok`. A file with the right length and the wrong magic raises "file is not a
// database" instead, so the hole is specifically the truncated case — which is
// exactly what an SD card produces.
// ---------------------------------------------------------------------------

/// The same three not-a-database shapes, through the entry point the running
/// station actually uses.
#[test]
fn full_integrity_check_refuses_what_is_not_a_database() {
    let dir = tempfile::tempdir().expect("tempdir");

    let empty = dir.path().join("empty.db");
    std::fs::write(&empty, b"").expect("write empty");
    assert!(
        !birdnet_db::resilience::full_integrity_check(&empty).expect("check"),
        "a zero-length file is not a database; SQLite opens it as a brand-new \
         empty one and integrity_check said \"ok\""
    );

    let short = dir.path().join("short.db");
    std::fs::write(&short, b"SQLite f").expect("write short");
    assert!(
        !birdnet_db::resilience::full_integrity_check(&short).expect("check"),
        "a file too short to hold SQLite's 16-byte magic is not a database"
    );

    let junk = dir.path().join("junk.db");
    let mut bytes = b"NOT A DATABASE!!".to_vec();
    bytes.extend(std::iter::repeat_n(0_u8, 4_096));
    std::fs::write(&junk, &bytes).expect("write junk");
    assert!(
        !birdnet_db::resilience::full_integrity_check(&junk).expect("check"),
        "a file whose first sixteen bytes are not SQLite's magic is not a database"
    );
}

/// The discrimination for the second entry point, and the reason the fix
/// cannot be "return false". A `full_integrity_check` that refused everything
/// would satisfy the test above, and would then halt detection writes on every
/// healthy station at its next daily tick.
#[test]
fn full_integrity_check_still_passes_an_ordinary_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (db, _backups) = station(dir.path(), 25);

    assert!(
        birdnet_db::resilience::full_integrity_check(&db).expect("check"),
        "a real database must still pass the daily check"
    );
    assert_eq!(rows_in(&db), 25, "and must not be touched by checking it");
}

/// A set expressed only as scattered call sites cannot be checked.
///
/// The behavioural gates above pin the two entry points that exist today. This
/// one pins the *rule*: every public function in `resilience.rs` whose name
/// says it checks integrity must consult the header first. A third one added
/// later — a `check_integrity_of_backup`, say — inherits the guard or turns
/// this red, rather than quietly reopening the hole in a new place, which is
/// how the hole got there the first time.
///
/// Parsed by brace depth from column zero rather than by reading nearby lines,
/// so `cargo fmt` reshaping a body cannot move the call out of the window.
#[test]
fn every_public_integrity_entry_point_consults_the_header() {
    let src = include_str!("../src/resilience.rs");

    let mut checked: Vec<&str> = Vec::new();
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let name = rest.split('(').next().unwrap_or_default();
        if !name.contains("integrity") {
            continue;
        }
        // Collect the body: from here to the first line that is exactly `}`
        // at column zero, which for a top-level item is its closing brace
        // whatever rustfmt does inside it.
        let mut body = String::new();
        for l in lines.by_ref() {
            if l == "}" {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        assert!(
            body.contains("has_sqlite_header"),
            "`{name}` reports on database integrity and never asks whether the \
             file is a database. SQLite opens a zero-length file as a healthy \
             empty one, so this function answers \"ok\" for a truncated \
             birds.db. Call `has_sqlite_header` first, as `check_integrity` \
             does."
        );
        checked.push(name);
    }

    assert!(
        checked.len() >= 2,
        "expected to find at least the two known integrity entry points in \
         resilience.rs, found {checked:?} — if they were renamed this gate is \
         no longer reading anything and must be updated, not deleted"
    );
}
