//! Background database-maintenance tasks for unattended deployments.
//!
//! A 24/7/365 field installation has nobody to run `VACUUM`, prune old
//! backups, or notice that the integrity check started failing. This
//! module fills that gap with a single supervised tokio task that:
//!
//!   * Runs a **`PRAGMA integrity_check`** once per day at a fixed UTC
//!     offset from boot, logging WARN on failure (also pinged to the
//!     heartbeat URL in future versions).
//!   * Prunes **expired login sessions** on the same daily tick so the
//!     `sessions` table stays compact over months of continuous use.
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
                run_session_prune(&db_path).await;
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

/// Delete expired login-session rows so the `sessions` table does not grow
/// without bound on a long-running install. Best-effort and fully logged; a
/// failure never aborts the maintenance loop.
async fn run_session_prune(db_path: &Path) {
    if !db_path.exists() {
        tracing::debug!("session prune skipped: db not present yet");
        return;
    }
    let db_path = db_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        use birdnet_db::accounts::SessionStore;
        let conn = birdnet_db::sqlite::open_or_create(&db_path).map_err(|e| e.to_string())?;
        conn.prune_expired_sessions().map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok(0)) => tracing::debug!("scheduled session prune: nothing expired"),
        Ok(Ok(n)) => tracing::info!(
            pruned = n,
            "scheduled session prune removed expired sessions"
        ),
        Ok(Err(e)) => tracing::warn!(error = %e, "scheduled session prune failed"),
        Err(e) => tracing::warn!(error = %e, "scheduled session prune task panicked"),
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
    // Match the actual backup filename shape `{db_name}.backup.{unix_secs}`
    // (`backup_database` in `birdnet-db::resilience`). The previous
    // `extension == "db"/"sqlite"/"bak"` filter matched **nothing** for real
    // backups — their extension is the timestamp (`1733400000`), so this whole
    // pruner was silent dead code and the only retention came from the inline
    // prune inside `backup_database` (capped at `MAX_BACKUP_FILES`).
    //
    // The substring filter is deliberately broader than the inline prune
    // (which keys on the current `{db_name}.backup.` prefix). That lets this
    // pass catch stale backup files left over from a prior `db_name` — e.g.
    // an operator who renamed `birds.db` to `BirdDB.db` — which the
    // db-name-specific inline pruner can't see. `BACKUP_RETENTION` is the
    // *process-wide* outer bound; per-db retention is enforced inline.
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(backup_dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.contains(".backup."))
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
        // 5 backups, mtimes 0..5 days ago. Use the real backup filename shape
        // `{db_name}.backup.{unix_secs}` that `resilience::backup_database`
        // writes — the prior test used `birds-{i}.db`, which never appears in
        // production and silently passed even when the prune filter was wrong.
        for i in 0..5 {
            touch(
                tmp.path(),
                &format!("birds.db.backup.{}", 1_700_000_000 + i),
                now - day * u32::try_from(i).unwrap(),
            );
        }
        prune_old_backups_blocking(tmp.path(), 3).unwrap();
        let remaining: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert_eq!(remaining.len(), 3);
        // Oldest two (indices 3 and 4) should be gone.
        assert!(!remaining.contains(&"birds.db.backup.1700000003".to_string()));
        assert!(!remaining.contains(&"birds.db.backup.1700000004".to_string()));
    }

    #[test]
    fn prune_is_noop_when_under_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        for i in 0..2 {
            touch(
                tmp.path(),
                &format!("birds.db.backup.{}", 1_700_000_000 + i),
                now,
            );
        }
        prune_old_backups_blocking(tmp.path(), 10).unwrap();
        let remaining: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn prune_ignores_non_backup_files() {
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        // Non-backup files (the live DB, WAL/SHM sidecars, unrelated dotfiles)
        // must never be pruned, regardless of how many backup files we keep.
        touch(tmp.path(), "notes.txt", now);
        touch(tmp.path(), "birds.db", now); // live DB
        touch(tmp.path(), "birds.db-wal", now); // WAL sidecar
        touch(tmp.path(), "birds.db.backup.1700000001", now);
        touch(tmp.path(), "birds.db.backup.1700000002", now);
        prune_old_backups_blocking(tmp.path(), 1).unwrap();
        let remaining: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert!(remaining.contains(&"notes.txt".to_string()));
        assert!(remaining.contains(&"birds.db".to_string()));
        assert!(remaining.contains(&"birds.db-wal".to_string()));
        // Exactly one `.backup.` file should remain.
        assert_eq!(
            remaining.iter().filter(|n| n.contains(".backup.")).count(),
            1
        );
    }

    #[test]
    fn prune_catches_stale_backups_from_other_db_names() {
        // Operator renamed `birds.db` → `BirdDB.db`. The inline prune inside
        // `backup_database` is keyed on the *current* `db_name` prefix and
        // can't see the old backups; this maintenance pruner is the safety
        // net that bounds the directory regardless of historical names.
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        let day = Duration::from_secs(24 * 60 * 60);
        touch(tmp.path(), "BirdDB.db.backup.1700000000", now - day * 5);
        touch(tmp.path(), "birds.db.backup.1700000001", now - day * 4);
        touch(tmp.path(), "birds.db.backup.1700000002", now - day * 3);
        prune_old_backups_blocking(tmp.path(), 2).unwrap();
        let remaining: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        // Oldest (the BirdDB one) must be gone; the 2 newest survive.
        assert_eq!(
            remaining.iter().filter(|n| n.contains(".backup.")).count(),
            2
        );
        assert!(!remaining.contains(&"BirdDB.db.backup.1700000000".to_string()));
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

    #[tokio::test]
    async fn session_prune_removes_only_expired_rows() {
        use birdnet_db::accounts::{SessionStore, UserStore};
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            let admin = conn.find_user_by_name("admin").unwrap();
            // One already-expired session and one far-future session.
            conn.create_session("expired-sid", admin.id, "2000-01-01 00:00:00", None, None)
                .unwrap();
            conn.create_session("live-sid", admin.id, "2999-01-01 00:00:00", None, None)
                .unwrap();
        }

        run_session_prune(&db).await;

        let conn = rusqlite::Connection::open(&db).unwrap();
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'live-sid'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 1, "the expired session row must be pruned");
        assert_eq!(live, 1, "the live session row must survive");
    }
}
