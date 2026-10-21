//! A clip BirdNET-Pi's owner protected is still there after this station's
//! disk purge, and an unprotected one of the same age is not.
//!
//! `disk_check_exclude.txt` is BirdNET-Pi's "never purge this". The import turns
//! its entries into locked detections (`birdnet_migrate::clips::protect_imported`)
//! and the disk manager re-reads the locked file names from the database on every
//! cycle. This drives both ends: the lock from the list, then a real purge pass
//! over a real directory with the locked names read back from the database the
//! way the running station reads them.

use std::path::Path;
use std::sync::{Arc, Mutex};

use birdnet_core::audio::capture::{DiskManager, DiskManagerConfig, FullDiskAction, PurgePolicy};
use birdnet_migrate::clips::{protect_imported, protected_names};

const PROTECTED: &str = "Eurasian_Blackbird-85-2026-03-01-birdnet-06:30:15.wav";
const UNPROTECTED: &str = "Eurasian_Blackbird-85-2026-03-01-birdnet-06:31:15.wav";

fn db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO import_batches (source_kind) VALUES ('birdnet-pi')",
        [],
    )
    .expect("batch");
    for (time, file) in [("06:30:15", PROTECTED), ("06:31:15", UNPROTECTED)] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
             Sens, Overlap, File_Name, chunk_offset_secs, import_batch_id) \
             VALUES ('2026-03-01', ?1, 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7, 9, 1.25, 0.0, ?2, 0, 1)",
            rusqlite::params![time, file],
        )
        .expect("insert");
    }
    conn
}

/// Twelve old clips, so the oldest tenth the flat purge takes is exactly one:
/// the protected one is the oldest of all, so an unprotected station takes it
/// first, and the unprotected one is a second older than the rest.
fn recordings(dir: &Path) {
    let aged = |name: &str, secs_ago: u64| {
        let path = dir.join(name);
        std::fs::write(&path, vec![0_u8; 64]).expect("clip");
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(when))
            .expect("age");
    };
    let long_ago = 400 * 86_400;
    aged(PROTECTED, long_ago + 2);
    aged(UNPROTECTED, long_ago + 1);
    for i in 0..10 {
        aged(&format!("other-{i}.wav"), long_ago - 1_000 - i);
    }
}

/// One purge pass at a threshold every disk is over, with the locked names read
/// from `conn` as the disk manager's cycle reads them (it refreshes
/// `locked_file_names` from its provider before `check_and_purge`; that step is
/// private, so it is repeated here with the same query).
fn purge(dir: &Path, conn: &Arc<Mutex<rusqlite::Connection>>) -> u32 {
    let locked = birdnet_db::sqlite::locked_file_names(&conn.lock().expect("db")).expect("locked");
    DiskManager::new(DiskManagerConfig {
        monitored_dir: dir.to_path_buf(),
        purge_threshold: 0,
        full_disk_action: FullDiskAction::Purge,
        purge_policy: PurgePolicy::OldestFirst,
        locked_file_names: locked,
        ..DiskManagerConfig::default()
    })
    .check_and_purge()
    .expect("a purge pass")
}

#[test]
fn a_protected_clip_survives_and_its_unprotected_twin_does_not() {
    let dir = tempfile::tempdir().expect("dir");
    recordings(dir.path());
    let conn = Arc::new(Mutex::new(db()));
    let names = protected_names(&format!(
        "##start\n2026-03-01/Eurasian_Blackbird/{PROTECTED}\n\
         2026-03-01/Eurasian_Blackbird/{PROTECTED}.png\n##end\n"
    ));
    let p = protect_imported(&conn.lock().expect("db"), &names, true).expect("protect");
    assert_eq!((p.matched, p.newly_locked), (1, 1));

    assert_eq!(
        purge(dir.path(), &conn),
        1,
        "the oldest tenth of twelve is one"
    );
    assert!(
        dir.path().join(PROTECTED).exists(),
        "the protected clip was purged"
    );
    assert!(
        !dir.path().join(UNPROTECTED).exists(),
        "the purge took neither; the test proves nothing"
    );
}

/// The control: the same directory, the same purge, no lock. The protected
/// clip is the oldest and goes. Without this the test above would pass for a
/// purge that removed nothing.
#[test]
fn without_the_lock_the_same_clip_is_the_one_purged() {
    let dir = tempfile::tempdir().expect("dir");
    recordings(dir.path());
    let conn = Arc::new(Mutex::new(db()));
    assert_eq!(purge(dir.path(), &conn), 1);
    assert!(!dir.path().join(PROTECTED).exists());
    assert!(dir.path().join(UNPROTECTED).exists());
}
