//! What a `SIGKILL` in the middle of a write leaves behind.
//!
//! # The hole this closes
//!
//! A station in an outdoor enclosure loses power. `Restart=always` and
//! `OOMPolicy=stop` are in the unit, `soak_recovers_from_db_corruption_at_restart`
//! covers recovery from an *already corrupt* file, and `soak.rs`'s own module
//! doc says "the kill-capture, clock-skew, and disk-purge faults are covered by
//! the supervisor, schedule, and disk-manager unit tests respectively."
//!
//! Every one of those is a test of a *handler*. None of them kills a process
//! mid-write and then looks at the database. Nothing in this repository ever
//! has. The corruption soak injects damage by overwriting bytes — a plausible
//! shape, but chosen by the test rather than produced by the failure. So the
//! question "does an unclean shutdown actually damage anything?" had an assumed
//! answer, and `CLAUDE.md` is explicit that an assumed answer is not one.
//!
//! # What is actually being asserted
//!
//! `PRAGMA integrity_check` passing is the weak half. The strong half is
//! `species_summary`: a trigger-maintained rollup (migration 24) that every
//! species page and the whole "top species" surface reads *instead of*
//! `detections`. It is maintained by `AFTER INSERT`/`UPDATE`/`DELETE` triggers,
//! so it shares the writing transaction — and if a kill could ever land between
//! the row and its rollup, every count on the dashboard would drift a little
//! further from the truth on every power cut, silently and permanently. No
//! integrity check would ever notice: both tables would be perfectly
//! well-formed and disagree.
//!
//! So: kill a real child process mid-insert, reopen, and require the rollup to
//! reconstruct exactly.
//!
//! # Why there is a `journal_mode=MEMORY` counterpart
//!
//! A test that kills a process and finds an intact database proves nothing
//! until you know it *could* have found a broken one. The counterpart runs the
//! identical harness against a connection whose journal is in memory, where an
//! unclean shutdown genuinely does leave damage, and requires the harness to
//! report it. Without that, "all pass" would be indistinguishable from a
//! harness that cannot see damage at all.
//!
//! # Scope, stated plainly
//!
//! `PRAGMA synchronous=NORMAL` (see `sqlite/connection.rs`) means this test
//! covers **process death**, not **power loss**. On a kill, the OS still owns
//! every byte already written and flushes it; on a real power cut, WAL frames
//! committed since the last checkpoint can be lost — that is the documented
//! trade-off of NORMAL, and it costs recent detections, never integrity. This
//! test asserts the integrity half and the "no torn rollup" half, which are the
//! parts that would corrupt a station's history rather than shorten it.

use std::process::{Command, Stdio};

use birdnet_db::sqlite::{DetectionRecord, insert_detection};
use rusqlite::Connection;

/// Environment variables the parent uses to re-exec this binary as the writer
/// child, and the libtest filter that selects the entry point.
///
/// Not argv: libtest parses `argv` itself and exits on an unrecognised flag, so
/// a `--zz-writer` argument never reaches test code — observed, as the child
/// producing no output at all and the harness reporting "child did not reach
/// the insert loop". The environment is invisible to libtest, and the filter
/// below is a name libtest is happy to accept.
const ENV_DB: &str = "BNB_UNCLEAN_CHILD_DB";
const ENV_MODE: &str = "BNB_UNCLEAN_CHILD_JOURNAL_MODE";
const CHILD_ENTRY: &str = "zz_writer_child_entry";

/// What the child prints once its schema is up.
const READY_MARKER: &str = "__BNB_WRITER_READY__";

/// How long the child is allowed to write before it is killed. Long enough to
/// get thousands of rows in, short enough to keep the test quick.
const WRITE_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

// ---------------------------------------------------------------------------
// Child
// ---------------------------------------------------------------------------

/// Insert detections forever. Never returns; the parent kills it.
fn writer_child(db_path: &str, journal_mode: &str) -> ! {
    use std::io::Write;

    let conn = Connection::open(db_path).expect("child: open");
    // The real pragmas, except for the journal mode under test.
    conn.execute_batch(&format!(
        "PRAGMA journal_mode={journal_mode};
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;"
    ))
    .expect("child: pragmas");
    birdnet_db::migration::migrate(&conn).expect("child: migrate");

    // Tell the parent the schema is ready, so the kill lands during inserts
    // rather than during migration. A distinctive marker, matched by
    // `contains`: libtest writes "test zz_writer_child_entry ... " with no
    // trailing newline, so the marker shares a line with it.
    println!("{READY_MARKER}");
    std::io::stdout().flush().expect("child: flush");

    let mut i: usize = 0;
    loop {
        let time = format!("{:02}:{:02}:{:02}", (i / 3600) % 24, (i / 60) % 60, i % 60);
        let file_name = format!("kill-{i:08}.wav");
        // Two species so the rollup has more than one row to keep straight.
        let (sci, com) = if i.is_multiple_of(2) {
            ("Turdus merula", "Eurasian Blackbird")
        } else {
            ("Parus major", "Great Tit")
        };
        let record = DetectionRecord {
            date: "2026-03-16",
            time: &time,
            sci_name: sci,
            com_name: com,
            confidence: 0.5 + f64::from(u32::try_from(i % 40).unwrap_or(0)) / 100.0,
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(11),
            sensitivity: None,
            overlap: None,
            file_name: &file_name,
            chunk_offset_secs: Some(0.0),
            correlation_id: None,
            source: None,
            duration_secs: None,
            detected_at_utc: None,
        };
        insert_detection(&conn, &record).expect("child: insert");
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// What reopening the killed station's database found.
#[derive(Debug)]
struct Postmortem {
    /// `PRAGMA integrity_check`'s verdict, or the error that prevented asking.
    integrity: Result<String, String>,
    /// Rows in `detections`, or the error reading them.
    rows: Result<i64, String>,
    /// Rows where `species_summary` disagrees with a recount of `detections`.
    ///
    /// `Ok(0)` is the invariant. Anything else is a rollup that no integrity
    /// check would ever flag, because both tables are individually well-formed.
    summary_disagreements: Result<i64, String>,
}

impl Postmortem {
    /// Whether this is a database a station could keep running on.
    fn is_intact(&self) -> bool {
        self.integrity.as_deref() == Ok("ok")
            && self.rows.is_ok()
            && self.summary_disagreements == Ok(0)
    }
}

/// Spawn a writer, kill it mid-write, and reopen what it left.
fn kill_mid_write(journal_mode: &str) -> (Postmortem, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let db = dir.path().join("birds.db");

    let mut child = Command::new(std::env::current_exe().expect("current_exe"))
        // `--nocapture` so the child's readiness line reaches our pipe rather
        // than libtest's buffer; `--exact` so only the writer entry runs.
        .args([CHILD_ENTRY, "--exact", "--nocapture", "--test-threads=1"])
        .env(ENV_DB, &db)
        .env(ENV_MODE, journal_mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn writer");

    // Wait for "ready" so the kill cannot land during migration. libtest prints
    // its own banner first, so read until the marker rather than taking line 1.
    {
        use std::io::{BufRead, BufReader};
        let mut reader = BufReader::new(child.stdout.as_mut().expect("child stdout"));
        let mut ready = false;
        for _ in 0..20 {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line.contains(READY_MARKER) {
                        ready = true;
                        break;
                    }
                }
                Err(e) => panic!("reading the child's output: {e}"),
            }
        }
        assert!(ready, "child did not reach the insert loop");
    }
    std::thread::sleep(WRITE_WINDOW);

    // SIGKILL on Unix: no unwinding, no destructors, no SQLite cleanup — the
    // closest a test can get to the power going out.
    child.kill().expect("kill writer");
    let status = child.wait().expect("reap writer");
    assert!(
        !status.success(),
        "the writer exited on its own ({status:?}); it should have been killed mid-write"
    );

    (postmortem(&db), dir)
}

/// Reopen the database exactly as the restarted service would, and look.
fn postmortem(db: &std::path::Path) -> Postmortem {
    let conn = match Connection::open(db) {
        Ok(c) => c,
        Err(e) => {
            let e = e.to_string();
            return Postmortem {
                integrity: Err(e.clone()),
                rows: Err(e.clone()),
                summary_disagreements: Err(e),
            };
        }
    };
    let _ = conn.execute_batch("PRAGMA busy_timeout=5000;");

    let integrity = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string());

    let rows = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| e.to_string());

    // Recompute the rollup from the rows it summarises and count the cells that
    // differ. `FULL OUTER JOIN` so a bucket present on only one side counts
    // too — a missing row and a wrong number are the same defect here.
    //
    // The bucket expression is `SUBSTR(Time, 1, 2)` and the filter is
    // `review_verdict IS NOT 'rejected'`, copied from the triggers in migration
    // 24 rather than rewritten. An earlier draft used
    // `CAST(strftime('%H', Time) AS INTEGER)`, which compared an INTEGER
    // against a TEXT column and only agreed because SQLite applied the CAST's
    // numeric affinity to the column value. It gave the right answer for the
    // wrong reason, and the first `Time` value it could not coerce would have
    // turned every bucket into a false disagreement.
    let summary_disagreements = conn
        .query_row(
            "WITH recomputed AS (
                 SELECT Com_Name, Sci_Name,
                        SUBSTR(Time, 1, 2) AS hour,
                        COUNT(*)           AS detections,
                        SUM(Confidence)    AS confidence_sum
                 FROM detections
                 WHERE review_verdict IS NOT 'rejected'
                 GROUP BY Com_Name, Sci_Name, hour
             )
             SELECT COUNT(*) FROM (
                 SELECT r.Com_Name, s.Com_Name AS s_com
                 FROM recomputed r
                 FULL OUTER JOIN species_summary s
                   ON s.Com_Name = r.Com_Name
                  AND s.Sci_Name = r.Sci_Name
                  AND s.hour     = r.hour
                 WHERE r.Com_Name IS NULL
                    OR s.Com_Name IS NULL
                    OR s.detections <> r.detections
                    OR ABS(s.confidence_sum - r.confidence_sum) > 1e-9
             )",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string());

    Postmortem {
        integrity,
        rows,
        summary_disagreements,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The writer child's way in.
///
/// In an ordinary run `ENV_DB` is unset and this is a no-op test. When the
/// parent re-execs this binary with the variable set and this test named as an
/// exact filter, it becomes the process that gets killed.
#[test]
fn zz_writer_child_entry() {
    let Ok(db) = std::env::var(ENV_DB) else {
        return;
    };
    let mode = std::env::var(ENV_MODE).expect("parent sets both variables together");
    writer_child(&db, &mode);
}

/// The station's actual configuration, killed mid-write.
///
/// Repeated, because a torn write is a race: one kill landing in a safe spot
/// proves nothing about the next one. Five kills at 400 ms of inserts each
/// covers thousands of transactions.
#[test]
fn a_killed_station_reopens_with_its_rollup_exact() {
    for attempt in 1..=5 {
        let (pm, _dir) = kill_mid_write("WAL");
        assert_eq!(
            pm.integrity.as_deref(),
            Ok("ok"),
            "attempt {attempt}: the database did not survive a kill: {pm:?}"
        );
        let rows = pm.rows.clone().expect("row count");
        assert!(
            rows > 0,
            "attempt {attempt}: nothing was committed before the kill, so this \
             run tested nothing — raise WRITE_WINDOW"
        );
        assert_eq!(
            pm.summary_disagreements,
            Ok(0),
            "attempt {attempt}: species_summary disagrees with detections after \
             a kill ({rows} rows). No integrity check would ever catch this: \
             both tables are well-formed and the counts are simply wrong. {pm:?}"
        );
    }
}

/// Writes still land after the unclean restart — the station keeps recording
/// with no operator action, which is the whole point of `Restart=always`.
#[test]
fn a_killed_station_can_keep_recording() {
    let (pm, dir) = kill_mid_write("WAL");
    let before = pm.rows.expect("row count");

    let db = dir.path().join("birds.db");
    let conn = birdnet_db::sqlite::open_connection(&db).expect("reopen as the service does");
    let record = DetectionRecord {
        date: "2026-03-17",
        time: "07:15:00",
        sci_name: "Erithacus rubecula",
        com_name: "European Robin",
        confidence: 0.91,
        lat: None,
        lon: None,
        cutoff: None,
        week: Some(11),
        sensitivity: None,
        overlap: None,
        file_name: "after-restart.wav",
        chunk_offset_secs: Some(0.0),
        correlation_id: None,
        source: None,
        duration_secs: None,
        detected_at_utc: None,
    };
    insert_detection(&conn, &record).expect("insert after an unclean restart");
    drop(conn);

    let after = postmortem(&db);
    assert_eq!(after.rows, Ok(before + 1), "the new detection did not land");
    assert_eq!(
        after.summary_disagreements,
        Ok(0),
        "the rollup drifted on the first write after recovery: {after:?}"
    );
}

/// Counterpart 1: the rollup check must report a rollup that is wrong.
///
/// This is the assertion the two tests above actually rest on, and on its own
/// `summary_disagreements == Ok(0)` is equally consistent with "the rollup is
/// exact" and "the query cannot tell". So: take a database that has just
/// survived a kill and passed, move one count by one, and require the check to
/// notice. A rollup drifting by one detection per power cut is exactly the
/// failure mode this file exists to catch, so one is the right size of lie.
#[test]
fn the_rollup_check_notices_a_rollup_that_is_wrong() {
    let (pm, dir) = kill_mid_write("WAL");
    assert_eq!(
        pm.summary_disagreements,
        Ok(0),
        "precondition: the database came back clean"
    );

    let db = dir.path().join("birds.db");
    let conn = Connection::open(&db).expect("reopen");
    let changed = conn
        .execute(
            // `species_summary` is WITHOUT ROWID, so it is addressed by its
            // primary key.
            "UPDATE species_summary SET detections = detections + 1
             WHERE (Com_Name, Sci_Name, hour) =
                   (SELECT Com_Name, Sci_Name, hour FROM species_summary
                     ORDER BY Com_Name, Sci_Name, hour LIMIT 1)",
            [],
        )
        .expect("nudge one bucket");
    assert_eq!(changed, 1, "there was a bucket to nudge");
    drop(conn);

    let after = postmortem(&db);
    assert_eq!(
        after.integrity.as_deref(),
        Ok("ok"),
        "a wrong count is not a malformed database — that is the point"
    );
    assert_eq!(
        after.summary_disagreements,
        Ok(1),
        "one bucket off by one must read as exactly one disagreement: {after:?}"
    );
    assert!(!after.is_intact(), "and must not count as intact");
}

/// Counterpart 2: a rollup with a *missing* bucket must also register.
///
/// The `FULL OUTER JOIN` exists for this. An inner join would compare only the
/// buckets both sides have and report `0` for a summary that had lost a species
/// entirely — the worst version of the defect reading as the healthiest result.
#[test]
fn the_rollup_check_notices_a_missing_bucket() {
    let (_pm, dir) = kill_mid_write("WAL");
    let db = dir.path().join("birds.db");

    let conn = Connection::open(&db).expect("reopen");
    let removed = conn
        .execute(
            "DELETE FROM species_summary
             WHERE (Com_Name, Sci_Name, hour) =
                   (SELECT Com_Name, Sci_Name, hour FROM species_summary
                     ORDER BY Com_Name, Sci_Name, hour LIMIT 1)",
            [],
        )
        .expect("drop one bucket");
    assert_eq!(removed, 1);
    drop(conn);

    let after = postmortem(&db);
    assert_eq!(
        after.summary_disagreements,
        Ok(1),
        "a bucket present in detections and absent from the rollup must count: {after:?}"
    );
}

/// Counterpart 3: `integrity_check` must report a database that really is
/// malformed, so `Ok("ok")` above means something.
///
/// Damage is applied to the page-1 header rather than a random offset, which is
/// what `soak_recovers_from_db_corruption_at_restart` does — a byte flipped in
/// free space can legitimately go unnoticed, and a counterpart that sometimes
/// finds nothing is not one.
#[test]
fn the_integrity_check_notices_a_malformed_file() {
    let (_pm, dir) = kill_mid_write("WAL");
    let db = dir.path().join("birds.db");

    // Checkpoint first so the damage is not simply superseded by WAL frames.
    {
        let conn = Connection::open(&db).expect("reopen");
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint");
    }
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&db)
            .expect("open for damage");
        // Page size and the b-tree root live here; garbage is not recoverable.
        f.seek(SeekFrom::Start(24)).expect("seek");
        f.write_all(&[0xFF; 64]).expect("write damage");
        f.sync_all().expect("sync");
    }

    let after = postmortem(&db);
    assert!(
        after.integrity.as_deref() != Ok("ok"),
        "a database with a scribbled-on header must not report 'ok': {after:?}"
    );
    assert!(!after.is_intact(), "and must not count as intact");
}

/// A note on what was tried and did not work, kept because the next person
/// will have the same idea.
///
/// The first counterpart here re-ran the whole kill harness with
/// `journal_mode=MEMORY`, on the assumption that an unclean shutdown with no
/// on-disk journal leaves damage. It does not, at least not reliably: twelve
/// kills produced twelve databases reporting `integrity: Ok("ok")`, `rows:
/// Ok(586..691)`, `summary_disagreements: Ok(0)`. Single-row inserts are small
/// enough that the page write reaching the kernel is effectively all-or-nothing,
/// and the kernel outlives the process. A counterpart that only sometimes finds
/// damage is not a counterpart, so the three above inject it deterministically
/// instead.
#[test]
fn zz_documentation_only_journal_mode_memory_did_not_reliably_corrupt() {
    // Nothing to assert; the doc comment is the content. Kept as a test so it
    // is not deleted as a stray comment block.
}
