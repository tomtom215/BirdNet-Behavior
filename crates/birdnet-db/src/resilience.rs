//! Database resilience: WAL enforcement, backup, integrity, and recovery.
//!
//! Rust equivalent of `scripts/web/db_resilience.py`.
//! Uses the SQLite backup API for safe hot backups and provides
//! automatic corruption detection with recovery from backups.

use rusqlite::Connection;
use std::fmt;
use std::path::{Path, PathBuf};

/// Maximum number of backup files to retain.
const MAX_BACKUP_FILES: usize = 5;

/// Resilience operation errors.
#[derive(Debug)]
pub enum ResilienceError {
    /// SQLite error during resilience operation.
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

/// Enforce WAL journal mode on a database file.
///
/// WAL (Write-Ahead Logging) provides crash resilience: incomplete
/// transactions are rolled back on recovery rather than corrupting the database.
///
/// # Errors
///
/// Returns `ResilienceError` if the database cannot be opened or WAL cannot be set.
pub fn enforce_wal_mode(db_path: &Path) -> Result<(), ResilienceError> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA wal_autocheckpoint=1000;",
    )?;
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
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Run full integrity check (slower but more thorough).
///
/// # Errors
///
/// Returns `ResilienceError` on check failure.
pub fn full_integrity_check(db_path: &Path) -> Result<bool, ResilienceError> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let result: String =
        conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Create a backup of the database using the SQLite backup API.
///
/// This is safe to call while the database is in use (hot backup).
/// The backup is created at `{backup_dir}/{db_name}.backup.{timestamp}`.
///
/// # Errors
///
/// Returns `ResilienceError` on backup failure.
pub fn backup_database(db_path: &Path, backup_dir: &Path) -> Result<PathBuf, ResilienceError> {
    std::fs::create_dir_all(backup_dir)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let db_name = db_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("birds.db");
    let backup_path = backup_dir.join(format!("{db_name}.backup.{timestamp}"));

    let source = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut dest = Connection::open(&backup_path)?;

    let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
    backup
        .run_to_completion(100, std::time::Duration::from_millis(50), None)
        .map_err(ResilienceError::Sqlite)?;

    tracing::info!(
        path = %backup_path.display(),
        "database backup created"
    );

    // Prune old backups
    prune_backups(backup_dir, db_name, MAX_BACKUP_FILES)?;

    Ok(backup_path)
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
        // Also remove WAL/SHM journal files if present
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");
        let _ = std::fs::remove_file(wal_path);
        let _ = std::fs::remove_file(shm_path);
    }

    let source = Connection::open_with_flags(
        backup_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut dest = Connection::open(db_path)?;

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

    let Some(backup_path) = find_latest_backup(backup_dir, db_name) else {
        return Err(ResilienceError::NoBackup);
    };

    // Verify backup is also healthy before restoring
    match check_integrity(&backup_path) {
        Ok(true) => {}
        Ok(false) => {
            return Err(ResilienceError::Unrecoverable(
                "latest backup is also corrupt".into(),
            ));
        }
        Err(e) => {
            return Err(ResilienceError::Unrecoverable(format!(
                "cannot verify backup: {e}"
            )));
        }
    }

    restore_from_backup(&backup_path, db_path)?;

    Ok(RecoveryResult {
        healthy: true,
        action: RecoveryAction::Recovered,
        details: format!("restored from {}", backup_path.display()),
    })
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
            "INSERT INTO detections VALUES ('2026-03-11', '08:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.87, 42.36, -71.06, 0.7, 10, 1.25, 0.0, 'test.wav')",
            [],
        )
        .unwrap();
        drop(conn);
        let backup_dir = tempfile::tempdir().unwrap();
        (tmp, backup_dir.into_path())
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
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("birds.db.backup.")
            })
            .collect();

        assert_eq!(remaining.len(), 3);
    }
}
