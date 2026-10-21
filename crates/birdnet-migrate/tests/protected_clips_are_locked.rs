//! BirdNET-Pi's protected clips arrive locked.
//!
//! `disk_check_exclude.txt` keeps a clip out of BirdNET-Pi's purge and its
//! per-species cap. Here the same promise is a **locked** detection, which the
//! disk manager never removes. These tests hold the half that lives in this
//! crate: which rows get locked and which do not. That the disk manager then
//! keeps a locked clip is held by `crates/birdnet-web/tests/
//! a_protected_clip_survives_the_purge.rs`.

use std::fs;

use birdnet_migrate::clips::{
    find_protected_list, protect_imported, protected_names, read_protected_list,
};

const PROTECTED: &str = "Eurasian_Blackbird-85-2026-03-01-birdnet-06:30:15.mp3";
const ORDINARY: &str = "Eurasian_Blackbird-85-2026-03-01-birdnet-06:31:15.mp3";
const OWN: &str = "Eurasian_Blackbird-85-2026-03-02-birdnet-06:30:15.mp3";
const NOT_YET: &str = "Pica_pica-91-2026-03-01-birdnet-07:00:00.mp3";

fn db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute(
        "INSERT INTO import_batches (source_kind) VALUES ('birdnet-pi')",
        [],
    )
    .expect("batch");
    for (time, file, batch) in [
        ("06:30:15", PROTECTED, Some(1)),
        ("06:31:15", ORDINARY, Some(1)),
        // This station's own recording of a clip with a listed name: not an
        // import, so not BirdNET-Pi's to protect.
        ("06:32:15", OWN, None),
    ] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
             Sens, Overlap, File_Name, chunk_offset_secs, import_batch_id) \
             VALUES ('2026-03-01', ?1, 'Turdus merula', 'Eurasian Blackbird', 0.9, 0.7, 9, 1.25, 0.0, ?2, 0, ?3)",
            rusqlite::params![time, file, batch],
        )
        .expect("insert");
    }
    conn
}

fn list() -> Vec<String> {
    protected_names(&format!(
        "##start\n2026-03-01/Eurasian_Blackbird/{PROTECTED}\n\
         2026-03-01/Eurasian_Blackbird/{PROTECTED}.png\n\
         2026-03-02/Eurasian_Blackbird/{OWN}\n\
         2026-03-01/Pica_pica/{NOT_YET}\n##end\n"
    ))
}

fn locked(conn: &rusqlite::Connection) -> Vec<String> {
    let mut v = birdnet_db::sqlite::locked_file_names(conn).expect("locked");
    v.sort();
    v
}

#[test]
fn an_imported_protected_clip_is_locked_and_nothing_else_is() {
    let conn = db();
    let names = list();
    assert_eq!(names.len(), 3, "{names:?}");
    let p = protect_imported(&conn, &names, true).expect("protect");
    assert_eq!(p.listed, 3);
    assert_eq!(p.matched, 1, "only the imported row with a listed name");
    assert_eq!(p.newly_locked, 1);
    assert_eq!(
        locked(&conn),
        [PROTECTED],
        "the ordinary and the own stay unlocked"
    );
}

#[test]
fn a_preview_counts_and_changes_nothing() {
    let conn = db();
    let p = protect_imported(&conn, &list(), false).expect("preview");
    assert_eq!((p.matched, p.newly_locked), (1, 1));
    assert!(locked(&conn).is_empty(), "a preview locked something");
}

#[test]
fn running_it_again_locks_nothing_new_and_a_late_import_is_caught() {
    let conn = db();
    let names = list();
    protect_imported(&conn, &names, true).expect("first");
    let again = protect_imported(&conn, &names, true).expect("again");
    assert_eq!((again.matched, again.newly_locked), (1, 0));
    // The history for the listed Pica pica clip arrives later; running the same
    // list again finds it.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, \
         Sens, Overlap, File_Name, chunk_offset_secs, import_batch_id) \
         VALUES ('2026-03-01', '07:00:00', 'Pica pica', 'Eurasian Magpie', 0.9, 0.7, 9, 1.25, 0.0, ?1, 0, 1)",
        [NOT_YET],
    )
    .expect("late import");
    let late = protect_imported(&conn, &names, true).expect("late");
    assert_eq!((late.matched, late.newly_locked), (2, 1));
    assert_eq!(locked(&conn), [PROTECTED, NOT_YET]);
}

#[test]
fn the_list_is_found_beside_the_standard_install_and_read_from_a_folder() {
    let home = tempfile::tempdir().expect("home");
    let by_date = home.path().join("BirdSongs/Extracted/By_Date");
    let scripts = home.path().join("BirdNET-Pi/scripts");
    fs::create_dir_all(&by_date).expect("tree");
    fs::create_dir_all(&scripts).expect("scripts");
    assert_eq!(find_protected_list(&home.path().join("BirdSongs")), None);
    let file = scripts.join("disk_check_exclude.txt");
    fs::write(
        &file,
        format!("##start\n2026-03-01/Eurasian_Blackbird/{PROTECTED}\n##end\n"),
    )
    .expect("list");
    for pointed_at in [
        home.path().join("BirdSongs"),
        home.path().join("BirdSongs/Extracted"),
        by_date,
    ] {
        assert_eq!(find_protected_list(&pointed_at), Some(file.clone()));
    }
    // Read from the file, or from the BirdNET-Pi folder that holds it.
    assert_eq!(read_protected_list(&file).expect("file"), [PROTECTED]);
    assert_eq!(
        read_protected_list(&home.path().join("BirdNET-Pi")).expect("folder"),
        [PROTECTED]
    );
    assert!(read_protected_list(&home.path().join("nothing")).is_err());
}
