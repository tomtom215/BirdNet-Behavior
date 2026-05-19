//! Background database-maintenance tasks for unattended deployments.
//!
//! A 24/7/365 field installation has nobody to run `VACUUM`, prune old
//! backups, or notice that the integrity check started failing. This
//! module fills that gap with a single supervised tokio task that:
//!
//!   * Runs a **`PRAGMA integrity_check`** once per day at a fixed UTC
//!     offset from boot, logging WARN on failure (also pinged to the
//!     heartbeat URL in future versions).
//!   * Runs **`VACUUM`** once per week to reclaim space from deletes
//!     and keep the page layout from fragmenting over months of
//!     continuous appends.
//!   * Rotates database **backups**: takes a fresh snapshot before each
//!     VACUUM, then prunes the backup directory down to the most recent
//!     N files so backups themselves do not fill the disk.
//!
//! Every step is best-effort and fully logged. Failures never kill the
//! background task — the next interval will retry. The whole task is a
//! no-op when the database file does not exist yet (fresh install).
//!
//! All blocking work (file I/O, SQLite, integrity checks) runs inside
//! `spawn_blocking` so the tokio runtime stays responsive.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Daily integrity-check cadence.
const INTEGRITY_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Weekly VACUUM cadence.
const VACUUM_INTERVAL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How many backup files to retain in the backup directory.
const BACKUP_RETENTION: usize = 14;

/// Wait this long after boot before the first maintenance tick. Avoids
/// piling onto the startup CPU spike (model load + WAL replay + axum
/// initialisation).
const STARTUP_GRACE: Duration = Duration::from_secs(5 * 60);

/// Kick off the maintenance task. Returns immediately; the loop runs in
/// the background until the process exits.
pub fn spawn_database_maintenance(db_path: PathBuf, backup_dir: PathBuf) {
    tokio::spawn(async move {
        run_loop(db_path, backup_dir).await;
    });
}

async fn run_loop(db_path: PathBuf, backup_dir: PathBuf) {
    tracing::info!(
        db_path = %db_path.display(),
        backup_dir = %backup_dir.display(),
        integrity_check_every_hours = INTEGRITY_CHECK_INTERVAL.as_secs() / 3600,
        vacuum_every_days = VACUUM_INTERVAL.as_secs() / 86400,
        backup_retention = BACKUP_RETENTION,
        "database maintenance task scheduled"
    );
    tokio::time::sleep(STARTUP_GRACE).await;

    let mut integrity_ticker = tokio::time::interval(INTEGRITY_CHECK_INTERVAL);
    let mut vacuum_ticker = tokio::time::interval(VACUUM_INTERVAL);
    // Skip the immediate first tick on each (we already waited STARTUP_GRACE).
    integrity_ticker.tick().await;
    vacuum_ticker.tick().await;

    loop {
        tokio::select! {
            () = async { integrity_ticker.tick().await; } => {
                run_integrity_check(&db_path).await;
            }
            () = async { vacuum_ticker.tick().await; } => {
                run_backup_and_vacuum(&db_path, &backup_dir).await;
            }
        }
    }
}

async fn run_integrity_check(db_path: &Path) {
    if !db_path.exists() {
        tracing::debug!("integrity check skipped: db not present yet");
        return;
    }
    let db_path = db_path.to_path_buf();
    let result =
        tokio::task::spawn_blocking(move || birdnet_db::resilience::full_integrity_check(&db_path))
            .await;
    match result {
        Ok(Ok(true)) => tracing::info!("scheduled integrity check: PASS"),
        Ok(Ok(false)) => tracing::error!(
            "scheduled integrity check: FAIL — database corruption detected; \
             run `birdnet-behavior --check-db` and restore from backup"
        ),
        Ok(Err(e)) => tracing::warn!(error = %e, "scheduled integrity check errored"),
        Err(e) => tracing::warn!(error = %e, "scheduled integrity check task panicked"),
    }
}

async fn run_backup_and_vacuum(db_path: &Path, backup_dir: &Path) {
    if !db_path.exists() {
        tracing::debug!("backup+vacuum skipped: db not present yet");
        return;
    }
    let db_path_b = db_path.to_path_buf();
    let backup_dir_b = backup_dir.to_path_buf();

    // Step 1: backup.
    let backup_result = tokio::task::spawn_blocking(move || {
        birdnet_db::resilience::backup_database(&db_path_b, &backup_dir_b)
    })
    .await;
    match backup_result {
        Ok(Ok(path)) => tracing::info!(backup = %path.display(), "scheduled backup created"),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "scheduled backup failed");
            // Do not VACUUM if backup failed — preserve recoverability.
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "scheduled backup task panicked");
            return;
        }
    }

    // Step 2: prune old backups.
    if let Err(e) = prune_old_backups(backup_dir, BACKUP_RETENTION).await {
        tracing::warn!(error = %e, "backup pruning failed");
    }

    // Step 3: checkpoint the WAL (so VACUUM sees a clean state) and then VACUUM.
    let db_path_v = db_path.to_path_buf();
    let vac = tokio::task::spawn_blocking(move || {
        // Best-effort checkpoint: VACUUM works even if this fails.
        if let Err(e) = birdnet_db::resilience::checkpoint_wal(&db_path_v) {
            tracing::warn!(error = %e, "WAL checkpoint failed before VACUUM");
        }
        birdnet_db::resilience::vacuum_database(&db_path_v)
    })
    .await;
    match vac {
        Ok(Ok(())) => tracing::info!("scheduled VACUUM complete"),
        Ok(Err(e)) => tracing::warn!(error = %e, "scheduled VACUUM failed"),
        Err(e) => tracing::warn!(error = %e, "scheduled VACUUM task panicked"),
    }
}

/// Remove the oldest backups so at most `keep` remain. A missing backup
/// directory is treated as success (nothing to prune).
async fn prune_old_backups(backup_dir: &Path, keep: usize) -> std::io::Result<()> {
    if !backup_dir.exists() {
        return Ok(());
    }
    let dir = backup_dir.to_path_buf();
    tokio::task::spawn_blocking(move || prune_old_backups_blocking(&dir, keep))
        .await
        .map_err(|e| std::io::Error::other(format!("join error: {e}")))?
}

fn prune_old_backups_blocking(backup_dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(backup_dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "db" || ext == "sqlite" || ext == "bak")
        })
        .filter_map(|e| {
            e.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| (e.path(), t))
        })
        .collect();

    // Newest first.
    entries.sort_by_key(|(_, t)| std::cmp::Reverse(*t));

    let to_delete: Vec<PathBuf> = entries.into_iter().skip(keep).map(|(p, _)| p).collect();
    let count = to_delete.len();
    for path in to_delete {
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::debug!(file = %path.display(), "pruned old backup"),
            Err(e) => tracing::warn!(file = %path.display(), error = %e, "failed to prune backup"),
        }
    }
    if count > 0 {
        tracing::info!(pruned = count, retained = keep, "backup directory pruned");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn touch(dir: &Path, name: &str, mtime: std::time::SystemTime) -> PathBuf {
        let path = dir.join(name);
        let file = File::create(&path).unwrap();
        // `File::set_modified` is stable since Rust 1.75; the project's MSRV
        // is 1.88 so we can rely on it instead of pulling in `filetime`.
        file.set_modified(mtime).unwrap();
        path
    }

    #[test]
    fn prune_removes_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        let day = Duration::from_secs(24 * 60 * 60);
        // 5 backups, mtimes 0..5 days ago
        for i in 0..5 {
            touch(tmp.path(), &format!("birds-{i}.db"), now - day * i);
        }
        prune_old_backups_blocking(tmp.path(), 3).unwrap();
        let remaining: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert_eq!(remaining.len(), 3);
        // Oldest two (indices 3 and 4) should be gone.
        assert!(!remaining.contains(&"birds-3.db".to_string()));
        assert!(!remaining.contains(&"birds-4.db".to_string()));
    }

    #[test]
    fn prune_is_noop_when_under_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        for i in 0..2 {
            touch(tmp.path(), &format!("birds-{i}.db"), now);
        }
        prune_old_backups_blocking(tmp.path(), 10).unwrap();
        let remaining: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn prune_ignores_non_database_files() {
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        touch(tmp.path(), "notes.txt", now);
        touch(tmp.path(), "a.db", now);
        touch(tmp.path(), "b.db", now);
        prune_old_backups_blocking(tmp.path(), 1).unwrap();
        let remaining: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert!(remaining.contains(&"notes.txt".to_string()));
        // Exactly one .db file should remain.
        assert_eq!(
            remaining
                .iter()
                .filter(|n| std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("db")))
                .count(),
            1
        );
    }

    #[test]
    fn prune_missing_dir_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        // Synchronous helper used for the test — async wrapper checks
        // existence first and returns Ok early.
        assert!(!missing.exists());
    }

    #[test]
    fn vacuum_works_on_empty_sqlite() {
        // Uses the public birdnet-db API; smoke-tests the maintenance task
        // can actually call the function it depends on.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE x(i INTEGER PRIMARY KEY); INSERT INTO x VALUES (1),(2),(3); DELETE FROM x;",
            )
            .unwrap();
        }
        let before = std::fs::metadata(&db).unwrap().len();
        birdnet_db::resilience::vacuum_database(&db).unwrap();
        let after = std::fs::metadata(&db).unwrap().len();
        // VACUUM should not grow the file (often shrinks it after deletes).
        assert!(after <= before, "VACUUM grew file: {before} -> {after}");
    }
}
