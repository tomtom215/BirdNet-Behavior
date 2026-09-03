//! The weekly backup has to finish on a station that is still recording.
//!
//! # What was wrong
//!
//! `backup_database` drove SQLite's online backup API with
//! `run_to_completion(100, 50 ms)`. That is a loop of `sqlite3_backup_step(100)`
//! with a 50 ms sleep after **every** step that does not return `Done` — so
//! even with nothing else touching the database, an N-page file costs at least
//! `N/100 × 50 ms`.
//!
//! The fatal part is the other half. The source is opened on its own read-only
//! connection, so every write from the daemon's connection is an *external*
//! write, and SQLite documents that an external write causes the **next call to
//! `sqlite3_backup_step()` to restart the copy from page 0**. A station
//! recording a detection every twenty seconds — an ordinary dawn — writes far
//! more often than the copy can finish, so the backup restarts for ever.
//!
//! It never returned, and `run_backup_and_vacuum` is **awaited inline** in the
//! single sequential maintenance loop (`src/maintenance.rs`). So the daily
//! `PRAGMA integrity_check`, `VACUUM`, clip retention, the per-species cap and
//! log retention all stopped too, for the life of the process, with no error
//! path taken and therefore nothing logged. A station in that state keeps
//! recording birds — the right priority — and quietly stops taking the
//! snapshots that make a corrupt card recoverable, turning "recoverable
//! corruption" into "total data loss".
//!
//! # What this gate holds
//!
//! A real database, a real concurrent writer, and a deadline. The backup runs
//! on its own thread and the test waits on a channel, so the previous code
//! **fails** the test rather than hanging the suite for ever.
//!
//! Observed failing against `run_to_completion(100, Duration::from_millis(50))`
//! with the writer running: the channel times out at 30 s with the backup
//! still going. With the writer stopped it passes, which is why the writer is
//! the discrimination and not decoration — a gate that only timed the backup
//! would have been green on the code that shipped.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long the backup gets before this test calls it broken.
///
/// Generous by design. On this machine the fixed version takes well under a
/// second; the point of the number is to be far above any plausible honest
/// duration and far below "for ever", which is what the previous code did.
const BUDGET: Duration = Duration::from_secs(30);

/// Rows in the fixture. Enough that the file is comfortably more than the 100
/// pages one `run_to_completion` step copies, so the old code needed many steps
/// and therefore many chances to be restarted.
const ROWS: i64 = 4_000;

/// Interval between the concurrent writer's commits.
///
/// Faster than a real station (a detection every 20 s) so the test runs in
/// seconds rather than hours, and slow enough that it is a realistic *write*
/// pattern rather than a lock-starvation test.
const WRITE_EVERY: Duration = Duration::from_millis(20);

fn seed(path: &Path, rows: i64) {
    let conn = rusqlite::Connection::open(path).expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch("PRAGMA journal_mode=WAL;").expect("wal");
    let tx = conn.unchecked_transaction().expect("tx");
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO detections
                   (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0.7, 19, 1.25, 0.0, ?6)",
            )
            .expect("prepare");
        for i in 0..rows {
            stmt.execute(rusqlite::params![
                "2026-05-19",
                format!("{:02}:{:02}:{:02}", i % 24, (i / 24) % 60, i % 60),
                "Pica pica",
                "Eurasian Magpie",
                0.9_f64,
                // A long-ish filename so the fixture reaches a useful page count.
                format!("2026-05-19-birdnet-{i:06}-06:30:00.wav"),
            ])
            .expect("insert");
        }
    }
    tx.commit().expect("commit");
}

/// Insert one row every [`WRITE_EVERY`] until told to stop. Returns the number
/// of rows it managed to write, so the test can assert the writer was real.
fn spawn_writer(
    path: &Path,
    stop: Arc<AtomicBool>,
    written: Arc<AtomicU64>,
) -> std::thread::JoinHandle<()> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(&path).expect("writer open");
        conn.busy_timeout(Duration::from_secs(30)).expect("busy");
        let mut i = 0_u64;
        while !stop.load(Ordering::SeqCst) {
            let ok = conn.execute(
                "INSERT INTO detections
                   (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES ('2026-05-20', '07:00:00', 'Turdus merula', 'Eurasian Blackbird',
                         0.8, 0.7, 20, 1.25, 0.0, ?1)",
                rusqlite::params![format!("live-{i}.wav")],
            );
            if ok.is_ok() {
                written.fetch_add(1, Ordering::SeqCst);
            }
            i += 1;
            std::thread::sleep(WRITE_EVERY);
        }
    })
}

#[test]
fn the_backup_finishes_while_another_connection_keeps_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).expect("backup dir");
    seed(&db, ROWS);

    let size = std::fs::metadata(&db).expect("stat").len();
    assert!(
        size > 400 * 1024,
        "the fixture must exceed the 100 pages one backup step copies, or the \
         old code would have finished on the first step and this gate would \
         prove nothing; got {size} bytes"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicU64::new(0));
    let writer = spawn_writer(&db, Arc::clone(&stop), Arc::clone(&written));

    // Let the writer get going, so the backup starts into an already-moving
    // database rather than racing it.
    std::thread::sleep(Duration::from_millis(200));

    let (tx, rx) = std::sync::mpsc::channel();
    let db_for_backup = db;
    let backups_for_backup = backups;
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = birdnet_db::resilience::backup_database(&db_for_backup, &backups_for_backup);
        let _ = tx.send((result, started.elapsed()));
    });

    let outcome = rx.recv_timeout(BUDGET);
    stop.store(true, Ordering::SeqCst);
    let _ = writer.join();

    let rows_written = written.load(Ordering::SeqCst);
    assert!(
        rows_written > 5,
        "the concurrent writer must actually have written during the backup, or \
         this gate is testing the quiet case the old code also passed; wrote \
         {rows_written}"
    );

    let (result, elapsed) = outcome.unwrap_or_else(|_| {
        panic!(
            "backup_database did not return within {BUDGET:?} while another \
             connection was writing ({rows_written} rows written meanwhile). \
             This is the defect: SQLite restarts an incremental backup from \
             page 0 on every external write."
        )
    });
    let path = result.expect("the backup must succeed");

    assert!(
        path.exists(),
        "the backup file must exist at {}",
        path.display()
    );
    assert!(
        birdnet_db::resilience::check_integrity(&path).expect("integrity check"),
        "the snapshot must be a valid database, not merely a file"
    );

    // A snapshot of a moving database must hold at least what was there when it
    // started. Asserting the *exact* count would be wrong — the writer is still
    // going — and asserting nothing would let an empty file pass.
    let conn = rusqlite::Connection::open(&path).expect("open backup");
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .expect("count");
    assert!(
        n >= ROWS,
        "the snapshot holds {n} rows; it must hold at least the {ROWS} that were \
         committed before it started"
    );

    eprintln!(
        "backup of {size} bytes completed in {elapsed:?} with {rows_written} concurrent writes"
    );
}

/// The counterpart, so the gate above is known to be measuring contention and
/// not merely file size: with no writer, the same fixture backs up fine.
///
/// This one passed against the previous code too. It is here so that a future
/// change which makes the *quiet* case slow is also caught, and so the reason
/// the first test needs its writer is written down next to it.
#[test]
fn the_backup_of_a_quiet_database_is_not_slow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("birds.db");
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).expect("backup dir");
    seed(&db, ROWS);

    let started = Instant::now();
    let path = birdnet_db::resilience::backup_database(&db, &backups).expect("backup");
    let elapsed = started.elapsed();

    assert!(
        birdnet_db::resilience::check_integrity(&path).expect("integrity check"),
        "the snapshot must be a valid database"
    );
    assert!(
        elapsed < BUDGET,
        "backing up a quiet database took {elapsed:?}"
    );
}
