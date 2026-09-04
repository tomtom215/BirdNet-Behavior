//! `quick_check` cannot see an index that disagrees with its table, and two
//! recovery decisions were made on it.
//!
//! # What SQLite actually promises
//!
//! `PRAGMA quick_check` is not a cheaper `integrity_check` with the same
//! answer. It checks page structure and skips, in SQLite's own words,
//! verifying "that index content matches table content" and the UNIQUE
//! constraints. So it catches a torn page and misses a database whose indexes
//! have quietly stopped agreeing with the rows they point at — which is the
//! shape that matters here, because a query that uses such an index silently
//! returns *fewer rows*, with nothing to see afterwards.
//!
//! Measured on the SQLite this workspace bundles (3.53.2), on a `detections`
//! table with one indexed value patched in the table b-tree and every page left
//! structurally valid:
//!
//! ```text
//!   20 000 rows,  9.6 MB   quick_check -> "ok"        34 ms
//!                          integrity_check(1) -> "row 10001 missing from
//!                                                 index idx_detections_sci_first_cover"
//!  200 000 rows, 95.1 MB   quick_check -> "ok"       314 ms
//!                          integrity_check(1) -> "row 100001 missing from index …"
//! ```
//!
//! An earlier reading of this had a *random* byte flip in an index page
//! reproducing it. It does not: that produces structurally invalid cells, which
//! `quick_check` does catch. The corruption has to leave the structure intact
//! and make the index disagree, which is what `desync_index_from_table` below
//! does.
//!
//! # Why it mattered here
//!
//! Two decisions used `check_integrity`, which is `quick_check`:
//!
//! * `backup_database` refuses to snapshot a corrupt source. Its own comment
//!   says why — "the rolling backup ring would otherwise overwrite the last
//!   good backup with a copy of the damaged DB, eventually leaving zero
//!   recoverable backups". That is precisely what happened, because the guard
//!   could not see this corruption: five weekly snapshots later, every backup
//!   in the ring was a copy of the damaged database.
//! * `restore_from_backup` walks the ring newest-first and restores the first
//!   candidate that passes. It could therefore restore a backup whose indexes
//!   disagree with its tables, over a live database, and report success.
//!
//! # What is deliberately still `quick_check`
//!
//! `check_and_recover`'s verdict on the *live* database at boot. The deep check
//! costs 7.7 s per 95 MB here against 314 ms — 24× — and that path runs before
//! the listener binds, on a station that brownouts several times a month;
//! `PS-17` is already about how long that path takes. A false "healthy" there
//! is inaction rather than destruction, and the daily `full_integrity_check`
//! covers it within the day. The ring being protected is what makes that
//! trade safe, and it is why these two moved and that one did not.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;

/// A distinctive value in an indexed column, so the patch can be located.
const MARKER: &str = "Zzzmarkerus zzzmarkerii";

/// A database of `rows` detections, one of them carrying [`MARKER`].
fn station(path: &Path, rows: i64) {
    let conn = rusqlite::Connection::open(path).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    // Journal mode DELETE so the whole database is in the one file the patch
    // below edits; in WAL the marker could still be sitting in the -wal.
    conn.execute_batch("PRAGMA journal_mode=DELETE;")
        .expect("journal mode");
    let tx = conn.unchecked_transaction().expect("tx");
    for i in 0..rows {
        tx.execute(
            "INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap, File_Name)
             VALUES (?1, ?2, ?3, ?4, 0.9, 0.7, 19, 1.25, 0.0, ?5)",
            rusqlite::params![
                format!("2026-{:02}-{:02}", 1 + (i % 12), 1 + (i % 28)),
                format!("{:02}:{:02}:{:02}", i % 24, i % 60, i % 60),
                if i == rows / 2 {
                    MARKER.to_string()
                } else {
                    format!("Genus{} species{}", i % 300, i % 300)
                },
                format!("Common Bird {}", i % 300),
                format!("clip-{i}.wav"),
            ],
        )
        .expect("insert");
    }
    tx.commit().expect("commit");
    conn.execute_batch("VACUUM; ANALYZE;").expect("vacuum");
}

/// Make the indexes disagree with the table, leaving every page valid.
///
/// Patches [`MARKER`] inside the `detections` b-tree pages only — same byte
/// length, still UTF-8, still a well-formed cell — so the index entries keep
/// the old value and no page is structurally damaged. `dbstat` is what tells
/// the table's pages from the indexes'; patching both would leave them
/// agreeing again and prove nothing.
fn desync_index_from_table(path: &Path) {
    let conn = rusqlite::Connection::open(path).expect("open");
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .expect("page_size");
    let table_pages: Vec<i64> = conn
        .prepare("SELECT pageno FROM dbstat WHERE name = 'detections' AND pgsize > 0")
        .expect("prepare dbstat")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("pages");
    drop(conn);
    assert!(!table_pages.is_empty(), "dbstat found no detections pages");

    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open for patch");
    let mut patched = 0_usize;
    for page in table_pages {
        let off = u64::try_from((page - 1) * page_size).expect("offset");
        f.seek(SeekFrom::Start(off)).expect("seek");
        let mut buf = vec![0_u8; usize::try_from(page_size).expect("page size")];
        if f.read_exact(&mut buf).is_err() {
            continue;
        }
        if let Some(pos) = buf
            .windows(MARKER.len())
            .position(|w| w == MARKER.as_bytes())
        {
            buf[pos] = b'Y';
            f.seek(SeekFrom::Start(off)).expect("seek back");
            f.write_all(&buf).expect("write");
            patched += 1;
        }
    }
    f.sync_all().expect("sync");
    assert_eq!(
        patched, 1,
        "the marker must be patched in exactly one table page, or this fixture \
         is not producing the corruption it claims to"
    );
}

/// The premise, pinned. Everything below exists because this is true, and if a
/// future SQLite makes `quick_check` catch this, this gate says so rather than
/// the deep check quietly becoming pointless.
#[test]
fn quick_check_cannot_see_an_index_that_disagrees_with_its_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    station(&db, 20_000);
    desync_index_from_table(&db);

    let conn =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open");
    let quick: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .expect("quick_check");
    let deep: String = conn
        .query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))
        .expect("integrity_check");

    assert_eq!(
        quick, "ok",
        "if quick_check has learned to catch this, the deep check below is no \
         longer buying anything and this file should be revisited"
    );
    assert!(
        deep.contains("missing from index"),
        "integrity_check must see what quick_check cannot, or the fixture is \
         not corrupting anything: {deep}"
    );
}

/// The reproduction. A corrupt source must not enter the backup ring.
#[test]
fn a_database_whose_indexes_disagree_is_not_backed_up() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let backups = dir.path().join("backups");
    station(&db, 20_000);
    desync_index_from_table(&db);

    let err = birdnet_db::resilience::backup_database(&db, &backups)
        .expect_err("a corrupt source must not be snapshotted");
    assert!(
        err.to_string().contains("corrupt"),
        "and must say so: {err}"
    );
    assert!(
        std::fs::read_dir(&backups).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            || std::fs::read_dir(&backups)
                .expect("backup dir")
                .next()
                .is_none(),
        "nothing may be written into the ring"
    );
}

/// The other recovery decision: a corrupt backup must not be restored from.
#[test]
fn a_backup_whose_indexes_disagree_is_not_restored_from() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).expect("backup dir");

    // The good backup, taken first so it sorts as the older of the two.
    let good = dir.path().join("good.db");
    station(&good, 5_000);
    std::fs::copy(&good, backups.join("birds.db.backup.20260101-000000")).expect("good backup");

    // The newer backup, corrupt in the way quick_check cannot see.
    let bad = dir.path().join("bad.db");
    station(&bad, 20_000);
    desync_index_from_table(&bad);
    std::fs::copy(&bad, backups.join("birds.db.backup.20260201-000000")).expect("bad backup");

    // A live database that is unambiguously gone, so recovery must run.
    let db = dir.path().join("birds.db");
    std::fs::write(&db, b"").expect("truncate live db");

    let result = birdnet_db::resilience::check_and_recover(&db, &backups).expect("recover");
    assert!(
        result.healthy,
        "the older good backup must be found: {result:?}"
    );

    let conn = rusqlite::Connection::open(&db).expect("open restored");
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        rows, 5_000,
        "the restored database must be the older *good* one, not the newer \
         corrupt one — got {rows} rows"
    );
}

/// The counterpart, and the reason the fix cannot be "refuse everything".
/// A healthy station takes a backup every week and must keep doing so.
#[test]
fn a_healthy_database_is_still_backed_up() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let backups = dir.path().join("backups");
    station(&db, 20_000);

    let path = birdnet_db::resilience::backup_database(&db, &backups)
        .expect("a healthy database must still be backed up");
    assert!(path.exists(), "and the snapshot must be there");
    assert!(
        birdnet_db::resilience::check_integrity(&path).expect("verify"),
        "and must itself verify"
    );
}

/// The second counterpart: the cheap check still does its own job. Structural
/// damage — a torn page — is what `quick_check` is for, and moving these two
/// call sites to the deep check must not have removed that.
#[test]
fn structural_damage_is_still_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let backups = dir.path().join("backups");
    station(&db, 2_000);

    // Overwrite a page in the middle of the file with garbage.
    let page_size: u64 = 4096;
    let len = std::fs::metadata(&db).expect("stat").len();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(&db)
        .expect("open");
    f.seek(SeekFrom::Start(len / 2 / page_size * page_size))
        .expect("seek");
    f.write_all(&vec![0x5A_u8; usize::try_from(page_size).expect("size")])
        .expect("write");
    f.sync_all().expect("sync");
    drop(f);

    assert!(
        birdnet_db::resilience::backup_database(&db, &backups).is_err(),
        "a structurally damaged database must still be refused"
    );
}

/// A set expressed only as scattered call sites cannot be checked.
///
/// The behavioural gates above pin the two decisions that exist today. This one
/// pins the *rule*: the cheap check lives in exactly one place, and the
/// functions that overwrite or discard data go through the deep one. A third
/// decision added later inherits the rule or turns this red, rather than
/// quietly reintroducing the hole — which is how the hole got here, since
/// `backup_database`'s comment described the exact failure it was suffering
/// from while naming the instrument that could not see it.
///
/// Parsed by brace depth from column zero rather than by reading nearby lines,
/// so `cargo fmt` reshaping a body cannot move a call out of the window.
#[test]
fn the_decisions_that_overwrite_data_use_the_deep_check() {
    let src = include_str!("../src/resilience.rs");

    // Executable lines only. The first draft of this counted the string
    // anywhere and found three: two of them are this module's own doc comments
    // explaining what `quick_check` skips, which is prose the gate has no
    // business policing.
    let quick = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| l.contains("\"PRAGMA quick_check\""))
        .count();
    assert_eq!(
        quick, 1,
        "`PRAGMA quick_check` must be issued in exactly one place in \
         resilience.rs, inside check_integrity. A second one is a second place \
         for a decision to be made on a check that cannot see an index \
         disagreeing with its table."
    );

    let mut bodies: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("pub fn ") else {
            continue;
        };
        let name = rest.split('(').next().unwrap_or_default();
        let mut body = String::new();
        for l in lines.by_ref() {
            if l == "}" {
                break;
            }
            // Comments are excluded, and this is not fussiness. The first draft
            // kept them, and the mutation that reverts the candidate walk to the
            // cheap check stayed *green*: `check_and_recover`'s own comment
            // explains that the daily `full_integrity_check` covers the live
            // verdict, and `contains` was satisfied by that sentence. A gate
            // that a comment can satisfy is not checking the code.
            if l.trim_start().starts_with("//") {
                continue;
            }
            body.push_str(l);
            body.push('\n');
        }
        bodies.insert(name, body);
    }

    let backup = bodies
        .get("backup_database")
        .expect("backup_database must exist in resilience.rs");
    assert!(
        backup.contains("full_integrity_check"),
        "`backup_database` decides what enters the rolling ring, and the ring is \
         what `check_and_recover` restores from. It must use the deep check."
    );
    assert!(
        !backup.contains("check_integrity(db_path)"),
        "`backup_database` must not fall back to the cheap check: an index that \
         disagrees with its table passes it, and five weekly snapshots later \
         every backup in the ring is a copy of the damaged database."
    );

    let recover = bodies
        .get("check_and_recover")
        .expect("check_and_recover must exist in resilience.rs");
    assert!(
        recover.contains("full_integrity_check"),
        "`check_and_recover` chooses which backup is written over the live \
         database. That candidate must pass the deep check, whatever the live \
         database's own verdict was checked with."
    );
}
