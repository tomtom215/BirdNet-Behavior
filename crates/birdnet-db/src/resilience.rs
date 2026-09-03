//! Database resilience: WAL enforcement, backup, integrity, and recovery.
//!
//! Rust equivalent of `scripts/web/db_resilience.py`.
//! Uses the `SQLite` backup API for safe hot backups and provides
//! automatic corruption detection with recovery from backups.

use rusqlite::Connection;
use std::fmt;
use std::path::{Path, PathBuf};

/// Maximum number of backup files to retain.
const MAX_BACKUP_FILES: usize = 5;

/// Resilience operation errors.
#[derive(Debug)]
pub enum ResilienceError {
    /// `SQLite` error during resilience operation.
    Sqlite(rusqlite::Error),
    /// I/O error during backup/restore.
    Io(std::io::Error),
    /// No backup available for recovery.
    NoBackup,
    /// Database is corrupt and unrecoverable.
    Unrecoverable(String),
}

impl fmt::Display for ResilienceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::NoBackup => write!(f, "no backup available for recovery"),
            Self::Unrecoverable(msg) => write!(f, "unrecoverable: {msg}"),
        }
    }
}

impl std::error::Error for ResilienceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::NoBackup | Self::Unrecoverable(_) => None,
        }
    }
}

impl From<rusqlite::Error> for ResilienceError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<std::io::Error> for ResilienceError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Busy-timeout (ms) applied to every maintenance connection in this module.
///
/// Matches the per-connection `PRAGMA busy_timeout=5000` set by
/// `sqlite::open_or_create`. Without it the maintenance helpers open with a 0 ms
/// busy handler and return `SQLITE_BUSY` on the first contended lock, so a
/// scheduled VACUUM on a busy station would frequently no-op for a week even
/// though the lock would have been free in milliseconds.
const MAINTENANCE_BUSY_TIMEOUT_MS: u32 = 5_000;

/// Open a writer connection with the standard busy-timeout applied. Internal
/// helper that the maintenance entry points (`enforce_wal_mode`,
/// `vacuum_database`, `checkpoint_wal`, the backup paths) use so they all
/// honour the same wait-on-contention policy as the live writer.
fn open_with_busy_timeout(path: &Path) -> Result<Connection, ResilienceError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_millis(u64::from(
        MAINTENANCE_BUSY_TIMEOUT_MS,
    )))?;
    Ok(conn)
}

/// Open a read-only connection with the standard busy-timeout applied.
fn open_readonly_with_busy_timeout(path: &Path) -> Result<Connection, ResilienceError> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_millis(u64::from(
        MAINTENANCE_BUSY_TIMEOUT_MS,
    )))?;
    Ok(conn)
}

/// Enforce WAL journal mode on a database file.
///
/// WAL (Write-Ahead Logging) provides crash resilience: incomplete
/// transactions are rolled back on recovery rather than corrupting the database.
///
/// # Errors
///
/// Returns `ResilienceError` if the database cannot be opened or WAL cannot be set.
pub fn enforce_wal_mode(db_path: &Path) -> Result<(), ResilienceError> {
    let conn = open_with_busy_timeout(db_path)?;
    // `PRAGMA journal_mode=WAL` returns the *resulting* mode and silently stays
    // in the previous mode (e.g. `delete`) on filesystems that can't back WAL's
    // shared-memory index — some network mounts and container overlay/tmpfs
    // combos. The whole crash-recovery design assumes WAL, so verify it took.
    // Warn rather than error: a degraded journal mode is still usable, and
    // failing here would block startup on those filesystems.
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        tracing::warn!(
            journal_mode = %mode,
            path = %db_path.display(),
            "could not enable WAL journal mode (filesystem may not support it); \
             crash resilience is reduced — incomplete writes may not roll back cleanly"
        );
    }
    conn.execute_batch(
        "PRAGMA synchronous=NORMAL;
         PRAGMA wal_autocheckpoint=1000;",
    )?;
    Ok(())
}

/// Reclaim space and defragment the on-disk layout via `VACUUM`.
///
/// Intended to be called from a low-frequency background task (the binary
/// schedules this weekly). Returns when the operation finishes — VACUUM
/// holds an exclusive lock, so callers should make sure no other writer
/// is active. The operation is idempotent and safe to run on a healthy
/// database; on a corrupted one it returns an error rather than masking it.
///
/// # Errors
///
/// Returns `ResilienceError` if the database cannot be opened or `VACUUM`
/// fails.
pub fn vacuum_database(db_path: &Path) -> Result<(), ResilienceError> {
    let conn = open_with_busy_timeout(db_path)?;
    conn.execute_batch("VACUUM;")?;
    Ok(())
}

/// Force a WAL checkpoint to flush pending writes back into the main
/// database file. Useful before backups, before unmounting, or on
/// scheduled maintenance windows.
///
/// # Errors
///
/// Returns `ResilienceError` if the database cannot be opened or the
/// checkpoint fails.
pub fn checkpoint_wal(db_path: &Path) -> Result<(), ResilienceError> {
    let conn = open_with_busy_timeout(db_path)?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

/// Run integrity check on a database.
///
/// Uses `PRAGMA quick_check` for speed. For full check, use `full_integrity_check`.
///
/// # Errors
///
/// Returns `ResilienceError` on check failure.
pub fn check_integrity(db_path: &Path) -> Result<bool, ResilienceError> {
    let conn = open_readonly_with_busy_timeout(db_path)?;
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Run full integrity check (slower but more thorough).
///
/// # Errors
///
/// Returns `ResilienceError` on check failure.
pub fn full_integrity_check(db_path: &Path) -> Result<bool, ResilienceError> {
    let conn = open_readonly_with_busy_timeout(db_path)?;
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Create a backup of the database using the `SQLite` backup API.
///
/// This is safe to call while the database is in use (hot backup).
/// The backup is created at `{backup_dir}/{db_name}.backup.{timestamp}`.
///
/// # Errors
///
/// Returns `ResilienceError` on backup failure.
pub fn backup_database(db_path: &Path, backup_dir: &Path) -> Result<PathBuf, ResilienceError> {
    std::fs::create_dir_all(backup_dir)?;

    // Refuse to snapshot a corrupt source — the rolling backup ring (capped at
    // `MAX_BACKUP_FILES`) would otherwise overwrite the last good backup with
    // a copy of the damaged DB, eventually leaving zero recoverable backups
    // for `check_and_recover` to restore from. A failed quick_check is rare on
    // a healthy station, so this is cheap defense-in-depth.
    if !check_integrity(db_path)? {
        return Err(ResilienceError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::DatabaseCorrupt,
                extended_code: 0,
            },
            Some(format!(
                "refusing to back up corrupt source database at {}",
                db_path.display()
            )),
        )));
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let db_name = db_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("birds.db");
    let backup_path = backup_dir.join(format!("{db_name}.backup.{timestamp}"));

    // A backup that fails part-way — most likely a full disk, which is exactly
    // when this runs on a struggling station — used to leave its truncated file
    // behind. That file is worthless (it cannot pass an integrity check) but it
    // sorts newest, so it became the first thing recovery reached for, and it
    // consumed space on the disk that was already full. Remove it on any error
    // so the ring only ever contains complete snapshots.
    let result = (|| -> Result<(), ResilienceError> {
        let source = open_readonly_with_busy_timeout(db_path)?;
        let mut dest = open_with_busy_timeout(&backup_path)?;
        let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
        copy_whole_database(&backup)
    })();
    if let Err(e) = result {
        if let Err(rm) = std::fs::remove_file(&backup_path)
            && rm.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %backup_path.display(),
                error = %rm,
                "could not remove the incomplete backup"
            );
        }
        return Err(e);
    }

    tracing::info!(
        path = %backup_path.display(),
        "database backup created"
    );

    // Prune old backups
    prune_backups(backup_dir, db_name, MAX_BACKUP_FILES)?;

    Ok(backup_path)
}

/// How long the copy may spend being refused before it is called a failure.
///
/// A deadline is not decoration here: the defect this function exists to fix
/// was a copy that never returned, and a `SQLITE_BUSY` retried without a bound
/// would reproduce it in a new shape. Ten minutes is far above any honest
/// duration for a station-sized database and far below "for ever".
const BACKUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(600);

/// Delay between retries when the copy could not start at all.
const BACKUP_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// Copy the whole source database in **one** `sqlite3_backup_step()` call.
///
/// # Why one call and not a loop
///
/// SQLite restarts an online backup from page 0 whenever the source database is
/// written by a connection other than the backup's own — and the source here is
/// opened on its own read-only connection, so *every* detection the daemon
/// stores is such a write. The restart happens on the **next call to
/// `sqlite3_backup_step()`**, which is what makes a loop of small steps the
/// wrong shape: each sleep between steps is another window for a write to
/// arrive and undo everything copied so far.
///
/// This function used to be `run_to_completion(100, 50 ms)`, a loop of
/// 100-page steps with a 50 ms sleep after each. On a 209 MB database taking a
/// detection every twenty seconds — an ordinary dawn — it was measured still
/// running after 300 seconds, having restarted eight times, reaching 77 % and
/// dropping to 0 each time. It never finished, and because
/// `run_backup_and_vacuum` is awaited inline in the single sequential
/// maintenance loop, the daily integrity check, `VACUUM`, clip retention, the
/// per-species cap and log retention all stopped with it, for the life of the
/// process, with no error path taken and so nothing logged.
///
/// A negative page count copies every remaining page inside one step, holding
/// one read transaction on the source, so there is no next call for a write to
/// restart. In WAL mode — which this database uses — that read transaction is a
/// snapshot and does not block the writer, so the station keeps recording
/// through the copy.
///
/// `Busy` and `Locked` mean the step could not begin, so no work is lost by
/// retrying; `More` cannot be returned for a negative page count, and is
/// handled by continuing rather than by asserting, because "copy the rest" is
/// the right response to it either way.
fn copy_whole_database(backup: &rusqlite::backup::Backup<'_, '_>) -> Result<(), ResilienceError> {
    use rusqlite::backup::StepResult;

    let deadline = std::time::Instant::now() + BACKUP_DEADLINE;
    loop {
        // Anything that is not `Done` means keep going: `Busy` and `Locked`
        // both mean the step did not begin, and `StepResult` is
        // `#[non_exhaustive]`, so a future variant meaning anything other than
        // "finished" must also keep going rather than return a half-copied
        // file as a success.
        if backup.step(-1).map_err(ResilienceError::Sqlite)? == StepResult::Done {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(ResilienceError::Unrecoverable(format!(
                "database backup did not complete within {}s",
                BACKUP_DEADLINE.as_secs()
            )));
        }
        std::thread::sleep(BACKUP_RETRY_DELAY);
    }
}

/// Remove old backup files, keeping only the N most recent.
fn prune_backups(backup_dir: &Path, db_name: &str, keep: usize) -> Result<(), ResilienceError> {
    let prefix = format!("{db_name}.backup.");
    let mut backups: Vec<PathBuf> = std::fs::read_dir(backup_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();

    backups.sort();

    if backups.len() > keep {
        for old in &backups[..backups.len() - keep] {
            tracing::debug!(path = %old.display(), "pruning old backup");
            std::fs::remove_file(old)?;
        }
    }

    Ok(())
}

/// Find the most recent backup file for a database.
pub fn find_latest_backup(backup_dir: &Path, db_name: &str) -> Option<PathBuf> {
    let prefix = format!("{db_name}.backup.");
    let mut backups: Vec<PathBuf> = std::fs::read_dir(backup_dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) {
                Some(entry.path())
            } else {
                None
            }
        })
        .collect();

    backups.sort();
    backups.pop()
}

/// Every backup for `db_name`, newest first.
///
/// The names embed a zero-padded-by-magnitude Unix timestamp, so a lexical
/// sort is chronological for any timestamp of the same digit count — which
/// holds for every backup this decade — and the reverse gives newest-first.
///
/// # Errors
///
/// Never errors; an unreadable directory yields an empty list.
#[must_use]
pub fn backups_newest_first(backup_dir: &Path, db_name: &str) -> Vec<PathBuf> {
    let prefix = format!("{db_name}.backup.");
    let mut backups: Vec<PathBuf> = std::fs::read_dir(backup_dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix).then(|| entry.path())
        })
        .collect();
    backups.sort_unstable();
    backups.reverse();
    backups
}

/// Restore a database from a backup file.
///
/// If the destination is corrupt, it is removed first and recreated.
///
/// # Errors
///
/// Returns `ResilienceError` on restore failure.
pub fn restore_from_backup(backup_path: &Path, db_path: &Path) -> Result<(), ResilienceError> {
    // Remove corrupt destination if it exists (cannot open corrupt files with SQLite)
    if db_path.exists() {
        std::fs::remove_file(db_path)?;
        // Also remove WAL/SHM journal files if present. Use `with_suffix` (raw
        // append) rather than `with_extension`: the WAL/SHM sidecars are
        // `<db_path>-wal` / `-shm`, and `with_extension("db-wal")` only produces
        // that for a path literally ending in `.db`. For any other name (e.g.
        // `/data/station` or `my.archive.db`) it would target the wrong file and
        // leave the real sidecars attached to the freshly restored DB, risking
        // re-corruption — the same bug `quarantine_corrupt_database` already
        // avoids with `with_suffix`.
        let wal_path = with_suffix(db_path, "-wal");
        let shm_path = with_suffix(db_path, "-shm");
        if let Err(e) = std::fs::remove_file(&wal_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %wal_path.display(), error = %e, "failed to remove WAL file during restore");
        }
        if let Err(e) = std::fs::remove_file(&shm_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %shm_path.display(), error = %e, "failed to remove SHM file during restore");
        }
    }

    let source = open_readonly_with_busy_timeout(backup_path)?;
    let mut dest = open_with_busy_timeout(db_path)?;

    let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
    backup
        .run_to_completion(100, std::time::Duration::from_millis(50), None)
        .map_err(ResilienceError::Sqlite)?;

    // Close the dest connection before enforcing WAL
    drop(backup);
    drop(dest);
    drop(source);

    // Enforce WAL mode on restored database
    enforce_wal_mode(db_path)?;

    tracing::warn!(
        backup = %backup_path.display(),
        target = %db_path.display(),
        "database restored from backup"
    );

    Ok(())
}

/// Append a raw suffix to a whole path (not just the file stem), so
/// `birds.db` + `-wal` → `birds.db-wal` and `birds.db` + `.corrupt.5` →
/// `birds.db.corrupt.5`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Move a corrupt database aside so the daemon can start fresh without ever
/// writing to corruption.
///
/// The main file and its `-wal` / `-shm` sidecars are renamed to
/// `<name>.corrupt.<unix_secs>` (with matching `-wal` / `-shm`) and preserved
/// for offline recovery. Returns the path the main database was moved to.
///
/// This is the policy when [`check_and_recover`] reports the database is
/// corrupt and there is no good backup to restore: refusing to boot would
/// leave a remote, unattended station recording nothing, and writing to a
/// corrupt file risks worsening it and losing more data. Quarantining lets the
/// station resume on a fresh database immediately while the corrupt copy is
/// kept for the operator to recover offline.
///
/// # Errors
///
/// Returns `ResilienceError::Io` if the main database file cannot be renamed.
/// The caller should then refuse to start rather than write to the corrupt
/// file. Sidecar moves are best-effort (their loss is harmless).
pub fn quarantine_corrupt_database(db_path: &Path) -> Result<PathBuf, ResilienceError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let quarantine = with_suffix(db_path, &format!(".corrupt.{timestamp}"));

    std::fs::rename(db_path, &quarantine)?;

    // Best-effort: move the WAL/SHM sidecars too so they don't attach to the
    // fresh database created at the original path.
    for sidecar in ["-wal", "-shm"] {
        let from = with_suffix(db_path, sidecar);
        if from.exists() {
            let _ = std::fs::rename(&from, with_suffix(&quarantine, sidecar));
        }
    }

    Ok(quarantine)
}

/// Check database health and attempt recovery if corrupt.
///
/// # Errors
///
/// Returns `ResilienceError` if recovery fails.
pub fn check_and_recover(
    db_path: &Path,
    backup_dir: &Path,
) -> Result<RecoveryResult, ResilienceError> {
    // Check integrity
    match check_integrity(db_path) {
        Ok(true) => {
            return Ok(RecoveryResult {
                healthy: true,
                action: RecoveryAction::None,
                details: "database integrity check passed".into(),
            });
        }
        Ok(false) => {
            tracing::error!(path = %db_path.display(), "database corruption detected");
        }
        Err(e) => {
            tracing::error!(path = %db_path.display(), error = %e, "integrity check failed");
        }
    }

    // Try to restore from backup
    let db_name = db_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("birds.db");

    // Walk the backups newest-first and restore from the first one that
    // verifies. Trying only the newest — and giving up if it failed — threw
    // away the entire point of keeping a ring of fourteen: one bad snapshot at
    // the head (a write interrupted by a full disk or a power cut, or a copy
    // taken from an already-degrading card) made the caller quarantine the
    // database and start fresh while thirteen good backups sat beside it. That
    // is a season of records lost to the one file that happened to be newest.
    let candidates = backups_newest_first(backup_dir, db_name);
    if candidates.is_empty() {
        return Err(ResilienceError::NoBackup);
    }

    let mut rejected = 0_usize;
    for backup_path in &candidates {
        match check_integrity(backup_path) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    path = %backup_path.display(),
                    "backup is corrupt; trying the next oldest"
                );
                rejected += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    path = %backup_path.display(),
                    error = %e,
                    "cannot verify backup; trying the next oldest"
                );
                rejected += 1;
                continue;
            }
        }

        restore_from_backup(backup_path, db_path)?;
        if rejected > 0 {
            tracing::warn!(
                restored_from = %backup_path.display(),
                skipped = rejected,
                "recovered from an older backup after newer ones failed verification"
            );
        }
        return Ok(RecoveryResult {
            healthy: true,
            action: RecoveryAction::Recovered,
            details: format!("restored from {}", backup_path.display()),
        });
    }

    Err(ResilienceError::Unrecoverable(format!(
        "all {} backup(s) failed verification",
        candidates.len()
    )))
}

/// Result of a check-and-recover operation.
#[derive(Debug)]
pub struct RecoveryResult {
    /// Whether the database is healthy after the operation.
    pub healthy: bool,
    /// What action was taken.
    pub action: RecoveryAction,
    /// Human-readable details.
    pub details: String,
}

/// Action taken during recovery.
#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// No action needed; database was healthy.
    None,
    /// Database was recovered from backup.
    Recovered,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_or_create;

    fn temp_db_with_data() -> (tempfile::NamedTempFile, PathBuf) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        conn.execute(
            "INSERT INTO detections \
             (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name) \
             VALUES ('2026-03-11', '08:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.87, 42.36, -71.06, 0.7, 10, 1.25, 0.0, 'test.wav')",
            [],
        )
        .unwrap();
        drop(conn);
        let backup_dir = tempfile::tempdir().unwrap();
        (tmp, backup_dir.keep())
    }

    #[test]
    fn recovery_falls_back_to_an_older_backup_when_the_newest_is_corrupt() {
        // THE data-loss path this guards. Trying only the newest backup meant
        // one bad snapshot at the head made the caller quarantine the database
        // and start fresh — discarding a season of records while good backups
        // sat beside it.
        let (tmp, backup_dir) = temp_db_with_data();

        // A good backup, then a newer one that is garbage.
        let good = backup_database(tmp.path(), &backup_dir).unwrap();
        let db_name = tmp.path().file_name().unwrap().to_string_lossy();
        let newer = backup_dir.join(format!("{db_name}.backup.9999999999"));
        std::fs::write(&newer, b"this is not a sqlite database").unwrap();
        assert!(
            backups_newest_first(&backup_dir, &db_name)[0] == newer,
            "the corrupt one must sort newest for this test to mean anything"
        );

        // Corrupt the live database so recovery has to restore.
        std::fs::write(tmp.path(), b"corrupted beyond repair").unwrap();

        let result = check_and_recover(tmp.path(), &backup_dir).unwrap();
        assert_eq!(result.action, RecoveryAction::Recovered);
        assert!(
            result
                .details
                .contains(&*good.file_name().unwrap().to_string_lossy()),
            "must restore from the older good backup, got: {}",
            result.details
        );

        // And the restored database really carries the original row.
        let conn = open_or_create(tmp.path()).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "the detection history survived the recovery");
    }

    #[test]
    fn recovery_reports_unrecoverable_only_when_every_backup_fails() {
        // Counter-test: the fallback must not paper over a genuinely
        // unrecoverable station by "succeeding" on junk.
        let (tmp, backup_dir) = temp_db_with_data();
        let db_name = tmp.path().file_name().unwrap().to_string_lossy();
        for ts in ["1700000001", "1700000002"] {
            std::fs::write(backup_dir.join(format!("{db_name}.backup.{ts}")), b"junk").unwrap();
        }
        std::fs::write(tmp.path(), b"corrupted").unwrap();

        let err = check_and_recover(tmp.path(), &backup_dir).unwrap_err();
        assert!(
            matches!(err, ResilienceError::Unrecoverable(_)),
            "expected Unrecoverable, got {err:?}"
        );
    }

    #[test]
    fn a_failed_backup_leaves_no_partial_file_behind() {
        // A truncated snapshot is worthless, sorts newest, and eats space on
        // the disk that was already full. It must not survive the failure.
        let (tmp, backup_dir) = temp_db_with_data();
        let db_name = tmp.path().file_name().unwrap().to_string_lossy();

        // Corrupt the source so `backup_database` refuses part-way through.
        std::fs::write(tmp.path(), b"not a database").unwrap();
        assert!(backup_database(tmp.path(), &backup_dir).is_err());

        assert!(
            backups_newest_first(&backup_dir, &db_name).is_empty(),
            "a failed backup must not leave a file in the ring"
        );
    }

    #[test]
    fn backups_newest_first_orders_by_timestamp() {
        let (tmp, backup_dir) = temp_db_with_data();
        let db_name = tmp.path().file_name().unwrap().to_string_lossy();
        for ts in ["1700000001", "1700000003", "1700000002"] {
            std::fs::write(backup_dir.join(format!("{db_name}.backup.{ts}")), b"x").unwrap();
        }
        let ordered = backups_newest_first(&backup_dir, &db_name);
        let names: Vec<String> = ordered
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                format!("{db_name}.backup.1700000003"),
                format!("{db_name}.backup.1700000002"),
                format!("{db_name}.backup.1700000001"),
            ]
        );
    }

    #[test]
    fn enforce_wal_sets_journal_mode() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        drop(conn);

        enforce_wal_mode(tmp.path()).unwrap();

        let conn = Connection::open(tmp.path()).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn integrity_check_passes() {
        let (tmp, _backup_dir) = temp_db_with_data();
        assert!(check_integrity(tmp.path()).unwrap());
    }

    #[test]
    fn backup_and_restore() {
        let (tmp, backup_dir) = temp_db_with_data();

        let backup_path = backup_database(tmp.path(), &backup_dir).unwrap();
        assert!(backup_path.exists());

        // Corrupt the original by overwriting with garbage
        std::fs::write(tmp.path(), b"corrupted data").unwrap();

        // Restore (handles corrupt destination by removing it first)
        restore_from_backup(&backup_path, tmp.path()).unwrap();

        // Verify restored data
        let conn = Connection::open(tmp.path()).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn check_and_recover_healthy() {
        let (tmp, backup_dir) = temp_db_with_data();
        let result = check_and_recover(tmp.path(), &backup_dir).unwrap();
        assert!(result.healthy);
        assert_eq!(result.action, RecoveryAction::None);
    }

    #[test]
    fn prune_keeps_only_n_backups() {
        let (tmp, backup_dir) = temp_db_with_data();

        // Create 7 backups with distinct timestamps
        for i in 0..7 {
            let path = backup_dir.join(format!("birds.db.backup.{i}"));
            std::fs::copy(tmp.path(), &path).unwrap();
        }

        prune_backups(&backup_dir, "birds.db", 3).unwrap();

        let remaining: Vec<_> = std::fs::read_dir(&backup_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("birds.db.backup.")
            })
            .collect();

        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn with_suffix_appends_to_whole_path() {
        assert_eq!(
            with_suffix(Path::new("/data/birds.db"), "-wal"),
            PathBuf::from("/data/birds.db-wal")
        );
        assert_eq!(
            with_suffix(Path::new("/data/birds.db"), ".corrupt.5"),
            PathBuf::from("/data/birds.db.corrupt.5")
        );
    }

    #[test]
    fn quarantine_moves_corrupt_db_and_sidecars_aside() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("birds.db");
        std::fs::write(&db, b"corrupt").unwrap();
        std::fs::write(with_suffix(&db, "-wal"), b"wal").unwrap();
        std::fs::write(with_suffix(&db, "-shm"), b"shm").unwrap();

        let quarantined = quarantine_corrupt_database(&db).unwrap();

        // Original paths are cleared so a fresh database can be created there,
        // and no stale sidecar attaches to it.
        assert!(!db.exists());
        assert!(!with_suffix(&db, "-wal").exists());
        assert!(!with_suffix(&db, "-shm").exists());

        // The corrupt data is preserved for offline recovery, not deleted.
        assert!(quarantined.exists());
        assert_eq!(std::fs::read(&quarantined).unwrap(), b"corrupt");
        assert_eq!(
            std::fs::read(with_suffix(&quarantined, "-wal")).unwrap(),
            b"wal"
        );
        assert!(
            quarantined
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".corrupt.")
        );
    }

    #[test]
    fn check_and_recover_errors_when_corrupt_without_backup() {
        // Pins the contract the daemon relies on: a corrupt DB with no backup
        // returns an error (the daemon then quarantines and starts fresh
        // rather than writing to the corrupt file).
        let (tmp, backup_dir) = temp_db_with_data();
        std::fs::write(tmp.path(), b"not a database").unwrap();
        assert!(matches!(
            check_and_recover(tmp.path(), &backup_dir),
            Err(ResilienceError::NoBackup)
        ));
    }
}
