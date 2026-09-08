//! A lock held by a live sibling is not corruption (DD-25).
//!
//! `server.log 01:12:03`: `ERROR analytics database is unusable; quarantining
//! it and rebuilding from SQLite` — from a *new* process that then died with
//! `AddrInUse`, because the first, SIGTERM'd a second earlier, was still up.
//! `DuckDB`'s "Could not set lock on file" was taken for a damaged file, and
//! the first process's live store was moved aside under it.
//!
//! The lock is a file lock, so the holder has to be another process: this
//! test re-runs its own binary as a worker that opens the store and keeps it
//! open, and asks `open_or_quarantine_with_grace` for the same file.

use std::path::Path;
use std::time::Duration;

use birdnet_behavioral::connection::{AnalyticsDb, AnalyticsError, OpenOutcome};

const WORKER_TARGET: &str = "BNB_DUCKDB_WORKER_TARGET";
const WORKER_HOLD_SECS: &str = "BNB_DUCKDB_WORKER_HOLD_SECS";

/// Open the store named in the environment and hold it for the given time.
#[test]
#[ignore = "worker for a_lock_held_by_a_live_sibling_is_waited_out_not_quarantined"]
fn hold_the_store_open_worker() {
    let Ok(target) = std::env::var(WORKER_TARGET) else {
        return;
    };
    let hold: u64 = std::env::var(WORKER_HOLD_SECS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let db = AnalyticsDb::open(Path::new(&target)).expect("worker opens the store");
    // Say so on stdout, so the parent knows the lock is held before it tries.
    println!("HOLDING");
    std::thread::sleep(Duration::from_secs(hold));
    drop(db);
}

fn quarantined_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".corrupt."))
        .collect()
}

#[test]
fn a_lock_held_by_a_live_sibling_is_waited_out_not_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("analytics.duckdb");
    // Create the store first, so the worker holds an existing file.
    drop(AnalyticsDb::open(&path).expect("create"));

    let exe = std::env::current_exe().unwrap();
    let mut child = std::process::Command::new(&exe)
        .args([
            "--exact",
            "hold_the_store_open_worker",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(WORKER_TARGET, &path)
        .env(WORKER_HOLD_SECS, "6")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the worker");
    {
        use std::io::BufRead as _;
        let stdout = child.stdout.take().unwrap();
        let mut lines = std::io::BufReader::new(stdout).lines();
        let mut holding = false;
        for line in lines.by_ref() {
            if line.unwrap().contains("HOLDING") {
                holding = true;
                break;
            }
        }
        assert!(holding, "the worker never reported holding the store");
        // Keep draining so the worker never blocks on a full pipe.
        std::thread::spawn(move || for _ in lines {});
    }

    // Short grace: the sibling is still there at its end.
    let started = std::time::Instant::now();
    let outcome = AnalyticsDb::open_or_quarantine_with_grace(&path, Duration::from_secs(1));
    let elapsed = started.elapsed();
    match outcome {
        Err(AnalyticsError::Locked(msg)) => {
            assert!(
                msg.contains("lock"),
                "the error names the lock, not the file's health: {msg}"
            );
        }
        Ok((_, OpenOutcome::Rebuilt { quarantined })) => {
            panic!(
                "the live sibling's store was quarantined to {}",
                quarantined.display()
            )
        }
        other => panic!("expected Locked, got {:?}", other.map(|(_, o)| o)),
    }
    assert!(
        elapsed >= Duration::from_secs(1),
        "the grace was waited out ({elapsed:?})"
    );
    assert!(
        quarantined_files(dir.path()).is_empty(),
        "nothing was moved aside: {:?}",
        quarantined_files(dir.path())
    );

    // Once the sibling is gone, the same call opens the same file — which is
    // the restart-overlapping-a-slow-shutdown case, resolved inside the grace.
    let _ = child.wait();
    let (db, outcome) =
        AnalyticsDb::open_or_quarantine_with_grace(&path, Duration::from_secs(10)).unwrap();
    assert!(matches!(outcome, OpenOutcome::Opened), "{outcome:?}");
    drop(db);
    assert!(quarantined_files(dir.path()).is_empty());
}
