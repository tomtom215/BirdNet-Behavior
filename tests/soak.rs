//! Compressed soak / longevity test.
//!
//! A station runs 24/7/365: over a year it ingests millions of detections, and
//! it must do so without leaking memory or file descriptors or growing the
//! database without bound. A true multi-day soak can't live in CI, so this is a
//! fast proxy — it drives a large batch of detections through the *production*
//! insertion path on an on-disk database, then asserts that resident memory,
//! open file descriptors, and on-disk size all stay bounded.
//!
//! Raise the volume for a heavier local run with `BIRDNET_SOAK_N=200000`.

use birdnet_db::sqlite::{DetectionRecord, insert_detection};
use birdnet_web::state::AppState;

/// Default detection count — large enough to surface a per-insert leak, small
/// enough to stay a few seconds in CI.
const DEFAULT_N: usize = 20_000;

/// Resident set size in KiB from `/proc/self/status` (Linux only; `None` elsewhere).
fn vmrss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kb| kb.parse().ok())
}

/// Count of open file descriptors (Linux only; `None` elsewhere).
fn open_fd_count() -> Option<usize> {
    Some(std::fs::read_dir("/proc/self/fd").ok()?.count())
}

/// Total on-disk size of a SQLite database including its `-wal` / `-shm`
/// sidecars, so WAL-buffered data is not under-counted.
fn total_db_bytes(db_path: &std::path::Path) -> u64 {
    let main = db_path.to_path_buf();
    let wal = db_path.with_extension("db-wal");
    let shm = db_path.with_extension("db-shm");
    [main, wal, shm]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
}

/// Both soak tests read **process-wide** counters (`VmRSS` and the
/// `/proc/self/fd` count), so they must not run concurrently: cargo runs the
/// tests in a file in parallel by default, and one test's allocations / open
/// descriptors would pollute the other's measurement (which is exactly what made
/// the bounded-RSS assertion flap once a second resource-measuring test joined
/// the file). Serialise them on a shared lock, recovering from poisoning so a
/// panic in one surfaces as that test's failure instead of cascading.
static SOAK_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn soak_serial() -> std::sync::MutexGuard<'static, ()> {
    SOAK_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[tokio::test]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
async fn soak_insertions_stay_bounded() {
    // Held for the whole test so the process-wide RSS/fd readings below aren't
    // polluted by the other soak test running in parallel.
    let _serial = soak_serial();
    let n: usize = std::env::var("BIRDNET_SOAK_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_N);

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("soak.db");
    let conn = birdnet_db::sqlite::open_or_create(&db_path).expect("open db");
    let state = AppState::from_connection(conn, db_path.clone());

    let rss_before = vmrss_kb();
    let fd_before = open_fd_count();
    let start = std::time::Instant::now();

    for i in 0..n {
        // Distinct (Time, File_Name, chunk_offset) per row so every detection is
        // a genuinely new row (no INSERT OR IGNORE dedupe collapsing the count).
        let time = format!("{:02}:{:02}:{:02}", (i / 3600) % 24, (i / 60) % 60, i % 60);
        let file_name = format!("soak-{i:08}.wav");
        let record = DetectionRecord {
            date: "2026-03-15",
            time: &time,
            sci_name: "Turdus merula",
            com_name: "Eurasian Blackbird",
            confidence: 0.5 + (i % 50) as f64 / 100.0,
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(11),
            sensitivity: None,
            overlap: None,
            file_name: &file_name,
            chunk_offset_secs: Some(i as f64),
            correlation_id: None,
        };
        state
            .with_db(|conn| insert_detection(conn, &record))
            .expect("insert failed");
    }

    let elapsed = start.elapsed();

    // Every detection persisted (distinct keys → no silent dedupe).
    let count: i64 = state.with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap()
    });
    assert_eq!(
        usize::try_from(count).unwrap(),
        n,
        "every detection must persist"
    );

    // Checkpoint so the bytes settle into the main file before we size it.
    state.with_db(|c| {
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    });

    let db_bytes = total_db_bytes(&db_path);
    eprintln!(
        "soak: N={n} in {:.2}s ({:.0} ins/s), db={} KiB",
        elapsed.as_secs_f64(),
        n as f64 / elapsed.as_secs_f64().max(f64::EPSILON),
        db_bytes / 1024
    );

    // DB grows ~linearly; cap generously (≈1 KiB/row covers row + indexes + slack).
    assert!(
        db_bytes < n as u64 * 1024 + 8 * 1024 * 1024,
        "DB size {db_bytes} bytes is larger than expected for {n} rows"
    );

    // Resident memory must not balloon — the insertion path retains nothing per row.
    if let (Some(before), Some(after)) = (rss_before, vmrss_kb()) {
        let growth_kb = after.saturating_sub(before);
        eprintln!("soak: RSS grew {growth_kb} KiB ({} MiB)", growth_kb / 1024);
        assert!(
            growth_kb < 128 * 1024,
            "RSS grew {growth_kb} KiB (>128 MiB) over {n} inserts — possible leak"
        );
    }

    // No file-descriptor leak — the loop opens nothing new.
    if let (Some(before), Some(after)) = (fd_before, open_fd_count()) {
        assert!(
            after <= before + 8,
            "open fds grew from {before} to {after} over {n} inserts — possible descriptor leak"
        );
    }
}

/// One detection row for `db_path`'s connection, keyed distinctly by `i`.
fn insert_n(conn: &rusqlite::Connection, base: usize, count: usize) {
    for i in base..base + count {
        let time = format!("{:02}:{:02}:{:02}", (i / 3600) % 24, (i / 60) % 60, i % 60);
        let file_name = format!("fault-{i:08}.wav");
        let record = DetectionRecord {
            date: "2026-03-16",
            time: &time,
            sci_name: "Turdus merula",
            com_name: "Eurasian Blackbird",
            confidence: 0.8,
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(11),
            sensitivity: None,
            overlap: None,
            file_name: &file_name,
            #[allow(clippy::cast_precision_loss)]
            chunk_offset_secs: Some(i as f64),
            correlation_id: None,
        };
        insert_detection(conn, &record).expect("insert failed");
    }
}

/// Fault-injection soak: a station must survive an unclean-shutdown / power-loss
/// database corruption and resume recording with no operator action.
///
/// Models the real recovery cycle: run → snapshot backup → (power loss corrupts
/// the file) → restart detects corruption and recovers from the backup → keep
/// recording. Asserts the recovered data is intact, post-recovery writes persist,
/// and the process doesn't leak file descriptors across the fault. This is the
/// "recovers from an injected fault" half of the 24/7 soak acceptance; the
/// kill-capture, clock-skew, and disk-purge faults are covered by the supervisor,
/// schedule, and disk-manager unit tests respectively.
#[test]
fn soak_recovers_from_db_corruption_at_restart() {
    use birdnet_db::resilience::{RecoveryAction, backup_database, check_and_recover};

    // Serialise against the bounded-RSS soak test so neither pollutes the
    // other's process-wide fd / memory measurement (see `soak_serial`).
    let _serial = soak_serial();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("station.db");
    let backup_dir = dir.path().join("backups");

    let fd_before = open_fd_count();

    // 1. Normal operation: record a batch, then take a hot backup (as the
    //    scheduled maintenance task does before VACUUM).
    {
        let conn = birdnet_db::sqlite::open_or_create(&db_path).expect("open db");
        insert_n(&conn, 0, 500);
        backup_database(&db_path, &backup_dir).expect("backup");
        // Connection dropped here — the station "shuts down".
    }

    // 2. Power loss mid-write corrupts the main database file.
    std::fs::write(&db_path, b"\x00\x01 not a sqlite file at all \xff\xfe").unwrap();

    // 3. Restart: the resilience layer detects the corruption and restores the
    //    most recent good backup — no operator action, no data loss back to the
    //    snapshot, and crucially the station does NOT write to the corrupt file.
    let recovery = check_and_recover(&db_path, &backup_dir).expect("recovery should succeed");
    assert!(recovery.healthy, "database must be healthy after recovery");
    assert_eq!(
        recovery.action,
        RecoveryAction::Recovered,
        "a corrupt DB with a good backup must be recovered, not left as-is"
    );

    // 4. The station resumes: the recovered rows are intact and new detections
    //    persist on the same path the daemon would reopen.
    {
        let conn = birdnet_db::sqlite::open_or_create(&db_path).expect("reopen after recovery");
        let recovered: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(recovered, 500, "all pre-fault detections recovered");

        insert_n(&conn, 500, 500);
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1000, "station keeps recording after recovery");
    }

    // No descriptor leak across the corrupt → recover → resume cycle.
    if let (Some(before), Some(after)) = (fd_before, open_fd_count()) {
        assert!(
            after <= before + 8,
            "open fds grew from {before} to {after} across the recovery cycle — possible leak"
        );
    }
}
