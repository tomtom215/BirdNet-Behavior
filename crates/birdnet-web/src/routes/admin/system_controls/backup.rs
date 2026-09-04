//! Full tar.gz backup download and restore upload.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};

use crate::state::AppState;

/// Set while a restore is unpacking, so a second upload cannot run concurrently.
///
/// A restore streams an archive over the live database and recordings directory.
/// Two of them interleaving would corrupt both, and the UI makes that easy to
/// trigger: the upload is multi-gigabyte and silent while it runs, which is
/// exactly the condition that gets a button clicked twice. htmx does not dedupe
/// in-flight requests unless told to (`hx-sync`), and a UI guard cannot bind a
/// client that simply POSTs twice, so the invariant is enforced here as well.
static RESTORE_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Clears [`RESTORE_IN_PROGRESS`] however the handler leaves — including an
/// early return on a malformed upload, which is most of its exit paths.
struct RestoreGuard;

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        RESTORE_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Removes the staging directory however the backup closure leaves — including
/// the several `?` returns between creating it and streaming the archive.
struct StagingDir(std::path::PathBuf);

impl Drop for StagingDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.0.display(), error = %e, "could not remove backup staging directory");
        }
    }
}

/// A scratch directory beside the database, for the snapshot and the archive.
///
/// **Not** `std::env::temp_dir()`, which is where both used to go. The shipped
/// unit sets `PrivateTmp=yes`, so `/tmp` is a systemd-managed **tmpfs** and its
/// pages are charged to the service's cgroup — against `MemoryMax=1G`. A station
/// with a few gigabytes of clips could therefore OOM-kill itself by pressing
/// "download backup". The database's own directory is real disk, and it is the
/// one the operator has already sized for this data.
fn scratch_dir(db_path: &std::path::Path) -> std::path::PathBuf {
    let parent = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    parent.join(format!(
        ".bnb-backup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    ))
}

/// Take a consistent snapshot of `db_path` into `staging`, named exactly as the
/// live database is, and return its path.
///
/// # Why this is not `tar czf birds.db`
///
/// That is what this handler used to do, and a copy of the main database file is
/// not a copy of the database. In WAL mode the most recent committed
/// transactions live only in `birds.db-wal` until a checkpoint moves them
/// across; `-wal` was not in the archive and no checkpoint was taken, so **every
/// transaction still in the WAL was missing from the backup**. `tar` also read
/// the file while the daemon wrote to it, so the copy was not even a consistent
/// point in time.
///
/// `rusqlite::backup::Backup` — which is what the *scheduled* backup has always
/// used (`birdnet_db::resilience::backup_database`) — is the SQLite online-backup
/// API. It reads through a real connection, so it sees WAL content, and it holds
/// a read transaction, so what it produces is one instant. The two backup paths
/// in this codebase now have the same correctness properties, which is the actual
/// fix: the operator-facing button was the unreliable one.
///
/// `backup_database` additionally refuses to snapshot a source that fails
/// `quick_check`, which is the right answer here too — handing someone an archive
/// of a corrupt database is worse than telling them it is corrupt.
fn stage_backup_snapshot(
    db_path: &std::path::Path,
    staging: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(staging).map_err(|e| format!("could not create staging dir: {e}"))?;
    let produced = birdnet_db::resilience::backup_database(db_path, staging)
        .map_err(|e| format!("could not snapshot the database: {e}"))?;
    // `backup_database` names its output `<db_name>.backup.<unix_secs>`. The
    // archive member has to be named exactly as the live database is, or the
    // restore drops a file the daemon never opens.
    let name = db_path
        .file_name()
        .ok_or_else(|| "database path has no file name".to_string())?;
    let target = staging.join(name);
    if produced != target {
        std::fs::rename(&produced, &target).map_err(|e| {
            format!(
                "could not name the snapshot {}: {e}",
                name.to_string_lossy()
            )
        })?;
    }
    Ok(target)
}

pub(super) async fn full_backup(State(state): State<AppState>) -> axum::response::Response {
    let db_path = state.db_path().to_path_buf();
    let rec_dir = state.recording_dir();
    let base_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let staging = scratch_dir(&db_path);
        let cleanup = StagingDir(staging.clone());
        let tmp = staging.join("birdnet-backup.tar.gz");

        let mut args = vec!["czf".to_string(), tmp.to_string_lossy().to_string()];

        if db_path.exists() {
            let snapshot = stage_backup_snapshot(&db_path, &staging)?;
            let name = snapshot
                .file_name()
                .ok_or_else(|| "snapshot has no file name".to_string())?;
            args.push("-C".to_string());
            args.push(staging.to_string_lossy().to_string());
            args.push(name.to_string_lossy().to_string());
        }

        let conf_path = base_dir.join("birdnet.conf");
        if conf_path.exists() {
            args.push("-C".to_string());
            args.push(base_dir.to_string_lossy().to_string());
            args.push("birdnet.conf".to_string());
        }

        if rec_dir.exists()
            && let Some(name) = rec_dir.file_name()
        {
            args.push("-C".to_string());
            args.push(
                rec_dir
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_string_lossy()
                    .to_string(),
            );
            args.push(name.to_string_lossy().to_string());
        }

        let status = std::process::Command::new("tar").args(&args).status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => return Err(format!("tar exited with status {s}")),
            Err(e) => return Err(format!("failed to run tar: {e}")),
        }

        // Open the archive and hand back the *handle*, then let `cleanup` unlink
        // the whole staging directory on the way out.
        //
        // The previous code returned a path and scheduled a detached task to
        // delete it after sleeping 300 s. That is a race in both directions: a
        // multi-gigabyte download over a field LTE link outlives the timer, and a
        // restart inside the window leaks the archive — which, when it lived in
        // `PrivateTmp`, meant leaking it into RAM. An open descriptor to an
        // unlinked file keeps the bytes readable for exactly as long as the
        // download needs them and not one byte longer, with no timer to guess at.
        let file = std::fs::File::open(&tmp).map_err(|e| format!("failed to open backup: {e}"))?;
        let size = file.metadata().map(|m| m.len()).unwrap_or_default();
        drop(cleanup);
        Ok((file, size))
    })
    .await;

    match result {
        Ok(Ok((file, size))) => {
            let stream = tokio_util::io::ReaderStream::new(tokio::fs::File::from_std(file));

            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/gzip")
                .header(
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"birdnet-backup.tar.gz\"",
                )
                .header(header::CONTENT_LENGTH, size)
                .body(axum::body::Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("backup failed: {e}"),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("internal error: {e}"),
        )
            .into_response(),
    }
}

/// Vet an archive listing before anything is written to disk.
///
/// The only check this used to make was "some member ends in `.db`", and then it
/// ran `tar xzf` straight into the data directory. GNU tar happens to strip a
/// leading `/` and refuse `..` members, so the traversal was covered by the
/// implementation rather than by us — and `tar` here is whatever is on `PATH`,
/// which on a minimal image or in some containers is busybox, whose guarantees
/// differ. A check we make ourselves does not depend on which `tar` answered.
///
/// # Errors
///
/// Rejects absolute paths, any `..` component, and an archive with no database in
/// it at all.
fn check_archive_members(listing: &str) -> Result<(), String> {
    let mut has_db = false;
    for line in listing.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with('/') {
            return Err(format!("archive contains an absolute path: {line}"));
        }
        let path = std::path::Path::new(line);
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("archive escapes the data directory: {line}"));
        }
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("db"))
        {
            has_db = true;
        }
    }
    if has_db {
        Ok(())
    } else {
        Err("archive does not contain a database file".to_string())
    }
}

/// Make the just-extracted database file the one the daemon will actually open.
///
/// # The sidecars
///
/// Extracting `birds.db` over the live one leaves `birds.db-wal` and
/// `birds.db-shm` from the *running* daemon sitting beside it, and SQLite will
/// replay that WAL onto the restored file on the next open. Proven, not reasoned
/// about: a backup taken after a `VACUUM` (1 000 rows) restored over a database
/// that had since grown to 9 000 rows plus a new table, with the WAL left
/// uncheckpointed and `-shm` removed the way a reboot removes it, reopens
/// reporting `integrity_check: ok` and **9 000 rows** — the restore silently did
/// nothing, and said so in green. Leaving `-shm` in place instead restored
/// correctly, so the failure is intermittent, which is worse than deterministic.
///
/// `birdnet_db::resilience::restore_from_backup` has always removed both; this
/// path never called it. The removal is what that function knows and this one
/// did not.
///
/// # The check
///
/// An archive is operator-supplied. Nothing upstream of here has opened the file
/// it contains, so "the extract succeeded" says only that `tar` was happy.
/// `quick_check` is what tells the operator whether the thing they just restored
/// is a database at all — while they are still standing in front of it, rather
/// than at the next restart.
///
/// # Errors
///
/// Returns the reason the restored database is not usable. A sidecar that cannot
/// be removed is fatal here: continuing would hand back exactly the silent
/// non-restore described above.
fn finalize_restore(db_path: &std::path::Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm"] {
        let mut name = db_path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = std::path::PathBuf::from(name);
        match std::fs::remove_file(&sidecar) {
            Ok(()) => {
                tracing::info!(path = %sidecar.display(), "removed stale sidecar after restore");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "restored the database but could not remove {} ({e}) — \
                     the old write-ahead log would be replayed over it on the next start",
                    sidecar.display()
                ));
            }
        }
    }

    match birdnet_db::resilience::full_integrity_check(db_path) {
        Ok(true) => Ok(()),
        Ok(false) => Err("the restored database fails an integrity check".to_string()),
        Err(e) => Err(format!("the restored database could not be opened: {e}")),
    }
}

pub(super) async fn restore_backup(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    mut multipart: axum::extract::Multipart,
) -> Html<String> {
    use tokio::io::AsyncWriteExt as _;

    // Before the upload is even read: a restore replaces the detection history
    // wholesale, and the row has to exist whether or not the restore finishes.
    crate::audit::audit(
        &state,
        Some(&request_user),
        "data.database.restore",
        None,
        None,
    );

    if RESTORE_IN_PROGRESS
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        tracing::warn!("refused a second restore while one was already running");
        return Html(
            r#"<p class="ctl-err">A restore is already running. Wait for it to finish before starting another.</p>"#
                .to_string(),
        );
    }
    let _restore_guard = RestoreGuard;

    // Stream the uploaded archive straight to a temp file. A full backup
    // (database + recordings) routinely runs to many GB — far past axum's 2 MiB
    // default body limit — and the previous code buffered the whole thing in
    // memory (twice, via `field.bytes()` + `to_vec()`), which rejected real
    // backups and would OOM a Pi. Streaming keeps memory flat regardless of
    // archive size. NamedTempFile auto-removes the file on drop (even on an
    // early return), replacing the previous manual cleanup.
    let Ok(Ok(tmp)) =
        tokio::task::spawn_blocking(|| tempfile::Builder::new().suffix(".tar.gz").tempfile()).await
    else {
        return Html(
            r#"<p class="ctl-err">Internal error: could not allocate a temp file.</p>"#.to_string(),
        );
    };
    let tmp_path = tmp.path().to_path_buf();

    let mut bytes_written: u64 = 0;
    let mut found = false;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Html(format!(r#"<p class="ctl-err">Upload failed: {e}</p>"#)),
        };
        if field.name() != Some("backup") {
            continue;
        }
        let mut out = match tokio::fs::File::create(&tmp_path).await {
            Ok(f) => f,
            Err(e) => return Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#)),
        };
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = out.write_all(&chunk).await {
                        return Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#));
                    }
                    bytes_written += chunk.len() as u64;
                }
                Ok(None) => break,
                Err(e) => return Html(format!(r#"<p class="ctl-err">Upload failed: {e}</p>"#)),
            }
        }
        if let Err(e) = out.flush().await {
            return Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#));
        }
        found = true;
        break;
    }

    if !found || bytes_written == 0 {
        return Html(r#"<p class="ctl-err">No backup file uploaded.</p>"#.to_string());
    }

    let db_path = state.db_path().to_path_buf();
    let target_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        // Keep the NamedTempFile alive for the duration of the tar operations;
        // it unlinks the archive automatically when this closure returns.
        let _archive = tmp;
        let tmp_str = tmp_path.to_string_lossy().to_string();

        let list_output = std::process::Command::new("tar")
            .args(["tzf", &tmp_str])
            .output()
            .map_err(|e| format!("failed to list archive: {e}"))?;

        if !list_output.status.success() {
            return Err("invalid archive (tar returned error)".to_string());
        }

        let listing = String::from_utf8_lossy(&list_output.stdout);
        check_archive_members(&listing)?;

        let status = std::process::Command::new("tar")
            .args(["xzf", &tmp_str, "-C", &target_dir.to_string_lossy()])
            .status()
            .map_err(|e| format!("failed to extract: {e}"))?;

        if !status.success() {
            return Err(format!("tar extract failed with status {status}"));
        }

        finalize_restore(&db_path)?;

        Ok(
            "Backup restored successfully. Restart the server to load the restored data."
                .to_string(),
        )
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(r#"<p class="ctl-ok">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(r#"<p class="ctl-err">Restore failed: {e}</p>"#)),
        Err(e) => Html(format!(r#"<p class="ctl-err">Internal error: {e}</p>"#)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RESTORE_IN_PROGRESS, RestoreGuard, check_archive_members, finalize_restore,
        stage_backup_snapshot,
    };
    use std::sync::atomic::Ordering;

    /// A WAL-mode database with `n` rows committed and **not** checkpointed, so
    /// the newest rows exist only in `birds.db-wal`.
    ///
    /// This is the ordinary steady state of a running station, not a contrived
    /// one: `wal_autocheckpoint` defaults to 1000 pages, so up to ~4 MB of
    /// committed work is normally WAL-resident at any instant.
    fn wal_dirty_db(dir: &std::path::Path, n: usize) -> std::path::PathBuf {
        let db = dir.join("birds.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
             CREATE TABLE detections(id INTEGER PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for i in 0..n {
            tx.execute(
                "INSERT INTO detections(name) VALUES(?1)",
                [format!("row-{i}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        // The precondition this whole test rests on: the rows are in the WAL and
        // not in the main file. Asserted rather than assumed — if a future
        // rusqlite checkpointed on commit, the gate below would pass for a
        // reason that has nothing to do with what it claims to assert.
        let wal = dir.join("birds.db-wal");
        assert!(
            wal.metadata().is_ok_and(|m| m.len() > 0),
            "fixture is not WAL-dirty, so this test would prove nothing"
        );
        std::mem::forget(conn); // leave the WAL un-checkpointed by never closing
        db
    }

    fn row_count(db: &std::path::Path) -> i64 {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap()
    }

    /// The download button's archive has to carry the transactions that are still
    /// in the write-ahead log.
    ///
    /// Observed failing against the previous implementation: replacing
    /// `stage_backup_snapshot`'s body with the `std::fs::copy` of the main file
    /// that `tar czf birds.db` amounted to yields `snapshot has 0 rows, live
    /// database has 500`. The scheduled backup never had this bug; only the
    /// operator-facing one did.
    #[test]
    fn the_snapshot_carries_transactions_still_in_the_wal() {
        let dir = tempfile::tempdir().unwrap();
        let db = wal_dirty_db(dir.path(), 500);
        let staging = dir.path().join("staging");

        let snapshot = stage_backup_snapshot(&db, &staging).expect("snapshot");

        assert_eq!(
            snapshot.file_name().unwrap(),
            db.file_name().unwrap(),
            "the archive member must be named as the live database is, or a \
             restore drops a file the daemon never opens"
        );
        assert_eq!(
            row_count(&snapshot),
            500,
            "the snapshot lost the WAL-resident rows"
        );
    }

    /// The counterpart, so the gate above is a discrimination and not a blanket
    /// alarm: a database whose WAL *has* been checkpointed also round-trips.
    #[test]
    fn a_checkpointed_database_snapshots_too() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("birds.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE detections(id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO detections(name) VALUES('a'),('b'),('c');",
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        drop(conn);

        let snapshot = stage_backup_snapshot(&db, &dir.path().join("staging")).expect("snapshot");
        assert_eq!(row_count(&snapshot), 3);
    }

    /// A stale `-wal` beside a freshly restored file is replayed onto it, so the
    /// restore has to remove the sidecars — which is the whole defect.
    ///
    /// Observed failing with the sidecar removal taken back out of
    /// `finalize_restore` (the 0.14.0 behaviour, where nothing removed them at
    /// all): `the restore must yield the backup's rows, not the live
    /// database's — left: 9, right: 3`. The daemon comes back on the database
    /// the operator was trying to replace.
    #[test]
    fn a_restore_removes_the_sidecars_the_old_database_left() {
        let dir = tempfile::tempdir().unwrap();
        let db = wal_dirty_db(dir.path(), 9);
        assert_eq!(row_count(&db), 9, "the live database has the newer rows");

        // The archive's copy lands over the main file; the running daemon's
        // sidecars are still there.
        let backup = dir.path().join("backup.db");
        {
            let conn = rusqlite::Connection::open(&backup).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode=DELETE;
                 CREATE TABLE detections(id INTEGER PRIMARY KEY, name TEXT);
                 INSERT INTO detections(name) VALUES('x'),('y'),('z');",
            )
            .unwrap();
        }
        std::fs::copy(&backup, &db).unwrap();
        assert!(
            dir.path().join("birds.db-wal").exists(),
            "the stale WAL is there"
        );

        finalize_restore(&db).expect("finalize");

        // The consequence first, because it is the one the operator sees: if the
        // sidecars survive, SQLite replays the old WAL and hands back the
        // database they were trying to replace.
        assert_eq!(
            row_count(&db),
            3,
            "the restore must yield the backup's rows, not the live database's"
        );
        assert!(
            !dir.path().join("birds.db-wal").exists(),
            "-wal must be gone"
        );
        assert!(
            !dir.path().join("birds.db-shm").exists(),
            "-shm must be gone"
        );
    }

    #[test]
    fn a_restored_file_that_is_not_a_database_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("birds.db");
        std::fs::write(&db, b"this is not a database").unwrap();
        assert!(
            finalize_restore(&db).is_err(),
            "a corrupt restore must be reported while the operator is watching"
        );
    }

    #[test]
    fn archive_members_that_escape_the_data_directory_are_refused() {
        assert!(check_archive_members("birds.db\nrecordings/a.wav\n").is_ok());
        assert!(
            check_archive_members("recordings/a.wav\n").is_err(),
            "no db"
        );
        assert!(
            check_archive_members("/etc/passwd\nbirds.db\n").is_err(),
            "absolute"
        );
        assert!(
            check_archive_members("../../etc/cron.d/x\nbirds.db\n").is_err(),
            "parent-dir traversal"
        );
        assert!(
            check_archive_members("recordings/../../../x\nbirds.db\n").is_err(),
            "traversal in the middle of a path"
        );
    }

    fn claim() -> bool {
        RESTORE_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// The invariant the handler relies on: one restore at a time, and the slot
    /// is released however the handler leaves — including the early returns on a
    /// malformed upload, which is what a `Drop` guard buys over a manual reset.
    #[test]
    fn a_second_restore_cannot_start_while_one_is_running() {
        assert!(claim(), "the first restore claims the slot");
        let guard = RestoreGuard;
        assert!(
            !claim(),
            "a concurrent restore must be refused, not run over the live database"
        );
        drop(guard);
        assert!(claim(), "the slot is released when the handler returns");
        RESTORE_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}
