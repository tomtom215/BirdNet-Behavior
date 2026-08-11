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

/// Count of open file descriptors pointing **inside `dir`** (Linux only;
/// `None` elsewhere).
///
/// Deliberately scoped to one directory rather than counting `/proc/self/fd`
/// wholesale. These soak tests are threads in a single test binary and run
/// concurrently, so a process-wide count answers "how many files does this
/// *process* have open", which a sibling test opening its own database changes
/// underneath you. That produced a spurious "open fds grew from 4 to 13 —
/// possible leak" failure in a full workspace run while the same test passed
/// alone and passed on re-run: a flaky assertion, not a leak.
///
/// Scoping to the test's own temp directory measures what the assertion
/// actually means — did *this* cycle leave its own files open — and is immune
/// to what other tests are doing.
fn open_fd_count(dir: &std::path::Path) -> Option<usize> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    Some(
        entries
            .filter_map(Result::ok)
            .filter_map(|e| std::fs::read_link(e.path()).ok())
            .filter(|target| target.starts_with(dir))
            .count(),
    )
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
    let fd_before = open_fd_count(dir.path());
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
            source: None,
            duration_secs: None,
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
    if let (Some(before), Some(after)) = (fd_before, open_fd_count(dir.path())) {
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
            source: None,
            duration_secs: None,
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

    let fd_before = open_fd_count(dir.path());

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
    if let (Some(before), Some(after)) = (fd_before, open_fd_count(dir.path())) {
        assert!(
            after <= before + 8,
            "open fds grew from {before} to {after} across the recovery cycle — possible leak"
        );
    }
}

/// The initial SQLite → DuckDB analytics sync must not scale its memory with
/// the station's history.
///
/// This is the regression test for the defect that made a long-running station
/// unable to start. `read_sqlite_detections` collected the *entire* detections
/// table into a `Vec` before appending a single row, so peak RSS grew with the
/// row count: measured at **541 MiB for 1 M rows and 967 MiB for 2 M** against
/// the `MemoryMax=1G` the systemd unit sets. A station crossed the ceiling at
/// roughly 2.1 M detections and then could not start at all — and with
/// `Restart=always`, looped. A multi-year BirdNET-Pi database, which is exactly
/// what the migration importer brings in, is that size on arrival.
///
/// The sync now streams into the appender in batches, so the bound below is a
/// function of the batch and not of `n`. It is deliberately generous: the point
/// is to catch a return to *linear* growth, not to pin an allocator's exact
/// behaviour.
///
/// The default row count is chosen from measurement, not taste. Growth during
/// the sync, on this repo's CI-equivalent hardware:
///
/// | rows | buffering (old) | streaming (new) |
/// |---------|-----------------|-----------------|
/// | 200 000 | 87 MiB          | 51 MiB          |
/// | 400 000 | 162 MiB         | 56 MiB          |
/// | 1 000 000 | ~420 MiB (extrapolated) | 62 MiB |
///
/// The old path is linear; the new one is flat to within the DuckDB buffer
/// pool's own growth. 200 000 rows was *not* enough to separate them against a
/// sensible bound — the first version of this test passed on the broken code —
/// so the default is 400 000, where the two differ by ~3x.
///
/// `BIRDNET_SOAK_SYNC_N` raises the row count for a heavier local run.
#[cfg(feature = "analytics")]
#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn soak_analytics_sync_memory_is_bounded_by_batch_not_history() {
    use birdnet_behavioral::connection::AnalyticsDb;

    // Set between the two measured curves at the default row count: streaming
    // needs ~56 MiB there and buffering needed ~162 MiB. That leaves the passing
    // path 2x headroom for a different allocator while still failing decisively
    // the moment growth goes linear again.
    const MAX_GROWTH_KB: u64 = 112 * 1024;

    // Process-wide RSS reading — must not race the other soak tests.
    let _serial = soak_serial();

    let n: usize = std::env::var("BIRDNET_SOAK_SYNC_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400_000);

    let dir = tempfile::tempdir().unwrap();
    let sqlite_path = dir.path().join("station.db");
    let duck_path = dir.path().join("analytics.duckdb");

    // Build a station history the way a real one accumulates it: many rows,
    // realistic string widths (the `Vec<SyncRow>` that used to be built held
    // five `String`s per row, which is where the memory went).
    {
        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                 Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
                 Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
        conn.execute_batch("BEGIN").unwrap();
        {
            let mut stmt = conn
                .prepare("INSERT INTO detections VALUES (?,?,?,?,?,?,?,?,?,?,?,?)")
                .unwrap();
            for i in 0..n {
                let day = 1 + (i % 28);
                let date = format!("2026-03-{day:02}");
                let time = format!("{:02}:{:02}:{:02}", (i / 3600) % 24, (i / 60) % 60, i % 60);
                let sci = format!("Genus{} species{}", i % 300, i % 300);
                let com = format!("Common Bird Name {}", i % 300);
                let file = format!("Common_Bird_Name_{}-{i}-{date}-birdnet-RTSP_1.wav", i % 300);
                stmt.execute(rusqlite::params![
                    date,
                    time,
                    sci,
                    com,
                    0.5 + (i % 50) as f64 / 100.0,
                    42.3601_f64,
                    -71.0589_f64,
                    0.7_f64,
                    i32::try_from((i % 52) + 1).unwrap_or(1),
                    1.25_f64,
                    0.0_f64,
                    file,
                ])
                .unwrap();
            }
        }
        conn.execute_batch("COMMIT").unwrap();
    }

    let sqlite_conn = rusqlite::Connection::open(&sqlite_path).expect("reopen sqlite");
    let analytics = AnalyticsDb::open(&duck_path).expect("open duckdb");

    let rss_before = vmrss_kb();
    let synced = analytics
        .sync_from_sqlite(&sqlite_conn)
        .expect("initial sync must succeed");
    let rss_after = vmrss_kb();

    assert_eq!(synced as usize, n, "every row must reach DuckDB");
    assert_eq!(
        analytics.detection_count().expect("count") as usize,
        n,
        "the DuckDB copy must match SQLite exactly"
    );

    if let (Some(before), Some(after)) = (rss_before, rss_after) {
        let grew_kb = after.saturating_sub(before);
        // Printed unconditionally: when this test fails the actual number is
        // the whole diagnosis, and when it passes it is the trend line a future
        // change would be judged against.
        eprintln!(
            "analytics sync: {n} rows grew RSS by {} MiB ({grew_kb} KiB)",
            grew_kb / 1024
        );
        assert!(
            grew_kb < MAX_GROWTH_KB,
            "syncing {n} rows grew RSS by {} MiB ({grew_kb} KiB) — the sync is buffering the \
             whole history again rather than streaming it",
            grew_kb / 1024
        );
    }
}

/// A corrupt analytics database must be quarantined and rebuilt at the next
/// start, not silently disable analytics for ever.
///
/// The `DuckDB` store is purely derived from `SQLite`, so throwing it away is
/// always safe — yet a failed open used to be logged once as "not available
/// (non-fatal)" and then ignored on every subsequent start. Every analytics
/// page stayed empty until a human noticed and deleted the file by hand, which
/// an unattended field station never gets. This is the DuckDB counterpart of
/// `soak_recovers_from_db_corruption_at_restart`.
#[cfg(feature = "analytics")]
#[test]
fn soak_analytics_recovers_from_a_corrupt_duckdb_at_restart() {
    use birdnet_behavioral::connection::{AnalyticsDb, OpenOutcome};

    let dir = tempfile::tempdir().unwrap();
    let sqlite_path = dir.path().join("station.db");
    let duck_path = dir.path().join("analytics.duckdb");

    // A station with some history in SQLite — the source of truth.
    let sqlite_conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    sqlite_conn
        .execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                 Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
                 Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
    {
        let mut stmt = sqlite_conn
            .prepare("INSERT INTO detections VALUES (?,?,?,?,?,NULL,NULL,NULL,NULL,NULL,NULL,?)")
            .unwrap();
        for i in 0..250 {
            let time = format!("{:02}:{:02}:{:02}", (i / 3600) % 24, (i / 60) % 60, i % 60);
            stmt.execute(rusqlite::params![
                "2026-03-15",
                time,
                "Turdus merula",
                "Eurasian Blackbird",
                0.8_f64,
                format!("clip-{i}.wav"),
            ])
            .unwrap();
        }
    }

    // 1. Normal operation: analytics opens clean and syncs.
    {
        let (analytics, outcome) = AnalyticsDb::open_or_quarantine(&duck_path).expect("first open");
        assert_eq!(outcome, OpenOutcome::Opened);
        assert_eq!(analytics.sync_from_sqlite(&sqlite_conn).unwrap(), 250);
        assert_eq!(analytics.detection_count().unwrap(), 250);
    }

    // 2. The card develops a bad block / a DuckDB upgrade renders the file
    //    unreadable. Either way the bytes are no longer a database.
    std::fs::write(&duck_path, b"\x00\x01 not a duckdb file at all \xff\xfe").unwrap();

    // 3. Restart: the file is moved aside and a fresh one created in its place.
    let quarantined = {
        let (analytics, outcome) =
            AnalyticsDb::open_or_quarantine(&duck_path).expect("recovery must succeed");
        let OpenOutcome::Rebuilt { quarantined } = outcome else {
            panic!("a corrupt analytics database must be rebuilt, not opened as-is");
        };
        assert!(quarantined.exists(), "the bad file must be kept for triage");
        assert_eq!(
            analytics.detection_count().unwrap(),
            0,
            "the replacement starts empty"
        );

        // 4. The station's usual startup sync repopulates it from SQLite, which
        //    is the whole reason discarding it is safe.
        assert_eq!(analytics.sync_from_sqlite(&sqlite_conn).unwrap(), 250);
        assert_eq!(
            analytics.detection_count().unwrap(),
            250,
            "analytics must be whole again without operator action"
        );
        quarantined
    };

    // 5. The next start is uneventful — recovery is not a loop.
    {
        let (analytics, outcome) =
            AnalyticsDb::open_or_quarantine(&duck_path).expect("subsequent open");
        assert_eq!(outcome, OpenOutcome::Opened);
        assert_eq!(analytics.detection_count().unwrap(), 250);
    }

    // 6. And the leftover is discoverable, so `--doctor` can report it rather
    //    than leaving a silent file on the disk.
    assert!(
        quarantined
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(".duckdb.corrupt.")),
        "quarantine name must be recognisable to the doctor scan: {quarantined:?}"
    );
}
