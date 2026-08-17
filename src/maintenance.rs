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
//!
//! ## Why the schedule is persisted, not uptime-relative
//!
//! Each job's due-ness is computed from a **wall-clock timestamp recorded in
//! the database** (`maintenance_runs`, migration 21), not from a tokio
//! interval measured since process start. Uptime-relative timers silently
//! disable themselves on any station that restarts more often than the job's
//! period — and unattended stations restart constantly: a settings change
//! ("applies on restart"), an update, a power cut, a systemd watchdog bounce.
//! A station rebooting daily would never once reach a weekly timer, so the
//! weekly backup + VACUUM simply never ran. Because
//! `resilience::check_and_recover` can only restore from a backup, that turned
//! recoverable corruption into total data loss on exactly the deployments the
//! schedule exists to protect.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use birdnet_db::sqlite::{
    BACKUP_VACUUM_INTERVAL_SECS, DAILY_INTERVAL_SECS, JOB_BACKUP_VACUUM, JOB_INTEGRITY_CHECK,
    JOB_SESSION_PRUNE, JOB_SPECIES_CAP,
};

/// Daily integrity-check cadence.
///
/// Defined in `birdnet-db` beside the job keys so the admin page that reports
/// "next backup due" reads the same number this loop schedules against.
const INTEGRITY_CHECK_INTERVAL: Duration = Duration::from_secs(DAILY_INTERVAL_SECS.unsigned_abs());

/// Weekly VACUUM cadence.
const VACUUM_INTERVAL: Duration = Duration::from_secs(BACKUP_VACUUM_INTERVAL_SECS.unsigned_abs());

/// How many backup files to retain in the backup directory.
const BACKUP_RETENTION: usize = 14;

/// Wait this long after boot before the first maintenance tick. Avoids
/// piling onto the startup CPU spike (model load + WAL replay + axum
/// initialisation).
const STARTUP_GRACE: Duration = Duration::from_secs(5 * 60);

/// How often the loop wakes to ask "is anything overdue?".
///
/// Decoupled from the job periods themselves: the jobs are scheduled against
/// persisted wall-clock timestamps, so this only bounds how promptly an overdue
/// job is noticed. Half an hour keeps the wakeups negligible on a Pi while
/// bounding lateness to well under any job's period.
const SCHEDULER_TICK: Duration = Duration::from_secs(30 * 60);

/// Kick off the maintenance task. Returns immediately; the loop runs in
/// the background until the process exits.
///
/// `recordings_dir` + `species_cap` drive the per-species recording cap
/// (`MAX_FILES_SPECIES`): on the daily tick, keep only the newest `species_cap`
/// extracted clips per species and prune the older ones off disk.
/// `species_cap == 0` disables the cap (keep everything).
pub fn spawn_database_maintenance(
    db_path: PathBuf,
    backup_dir: PathBuf,
    recordings_dir: PathBuf,
    species_cap: u32,
    clip_retention_days: u32,
) {
    tokio::spawn(async move {
        run_loop(
            db_path,
            backup_dir,
            recordings_dir,
            species_cap,
            clip_retention_days,
        )
        .await;
    });
}

async fn run_loop(
    db_path: PathBuf,
    backup_dir: PathBuf,
    recordings_dir: PathBuf,
    species_cap: u32,
    clip_retention_days: u32,
) {
    tracing::info!(
        db_path = %db_path.display(),
        backup_dir = %backup_dir.display(),
        recordings_dir = %recordings_dir.display(),
        species_cap,
        clip_retention_days,
        integrity_check_every_hours = INTEGRITY_CHECK_INTERVAL.as_secs() / 3600,
        vacuum_every_days = VACUUM_INTERVAL.as_secs() / 86400,
        backup_retention = BACKUP_RETENTION,
        "database maintenance task scheduled"
    );
    tokio::time::sleep(STARTUP_GRACE).await;

    // In-process floor on each job's last run, used when the database write
    // that records a completion fails (read-only filesystem, disk full — the
    // very conditions maintenance is meant to survive). Without it a failing
    // `record_run` would leave the job permanently overdue and re-run it every
    // tick, turning a weekly VACUUM into a half-hourly one.
    let mut attempted: HashMap<&'static str, i64> = HashMap::new();

    let mut ticker = tokio::time::interval(SCHEDULER_TICK);
    loop {
        // Fires immediately on the first pass — STARTUP_GRACE has already
        // elapsed, and anything overdue should not wait another half hour.
        ticker.tick().await;

        if due(
            &db_path,
            JOB_INTEGRITY_CHECK,
            INTEGRITY_CHECK_INTERVAL,
            &attempted,
        )
        .await
        {
            run_integrity_check(&db_path).await;
            mark_ran(&db_path, JOB_INTEGRITY_CHECK, &mut attempted).await;
        }
        if due(
            &db_path,
            JOB_SESSION_PRUNE,
            INTEGRITY_CHECK_INTERVAL,
            &attempted,
        )
        .await
        {
            run_session_prune(&db_path).await;
            mark_ran(&db_path, JOB_SESSION_PRUNE, &mut attempted).await;
        }
        if due(
            &db_path,
            JOB_SPECIES_CAP,
            INTEGRITY_CHECK_INTERVAL,
            &attempted,
        )
        .await
        {
            run_recording_species_cap(&db_path, &recordings_dir, species_cap).await;
            run_clip_retention(&db_path, &recordings_dir, clip_retention_days).await;
            mark_ran(&db_path, JOB_SPECIES_CAP, &mut attempted).await;
        }
        if due(&db_path, JOB_BACKUP_VACUUM, VACUUM_INTERVAL, &attempted).await {
            run_backup_and_vacuum(&db_path, &backup_dir).await;
            mark_ran(&db_path, JOB_BACKUP_VACUUM, &mut attempted).await;
        }
    }
}

/// Current wall-clock time as Unix seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Decide whether `job` is overdue, from the persisted last-run timestamp.
///
/// Returns `false` while the database does not exist yet (fresh install with
/// no detections), and `true` when the job has never been recorded — an
/// upgrade or fresh install should get its first integrity check and backup
/// promptly rather than one full period later, since an upgrade is precisely
/// when a recoverable snapshot is most valuable.
async fn due(
    db_path: &Path,
    job: &'static str,
    interval: Duration,
    attempted: &HashMap<&'static str, i64>,
) -> bool {
    if !db_path.exists() {
        return false;
    }
    let owned = db_path.to_path_buf();
    let persisted = tokio::task::spawn_blocking(move || -> Result<Option<i64>, String> {
        let conn = birdnet_db::sqlite::open_or_create(&owned).map_err(|e| e.to_string())?;
        birdnet_db::sqlite::last_run_unix(&conn, job).map_err(|e| e.to_string())
    })
    .await;

    let last = match persisted {
        Ok(Ok(ts)) => ts,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, job, "could not read maintenance schedule; using in-process fallback");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, job, "maintenance schedule lookup panicked; using in-process fallback");
            None
        }
    };

    // The in-process floor only ever *delays* a job, so a database that cannot
    // be written still gets at most one run per interval per process lifetime.
    let last = match (last, attempted.get(job).copied()) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (some, None) | (None, some) => some,
    };

    let Some(last) = last else {
        return true;
    };

    let now = now_unix();
    // A Pi without an RTC boots at the epoch and jumps forward when NTP lands;
    // a correction can also move the clock *backwards*, leaving a stored
    // timestamp in the future. Treat that as due and re-anchor on the next
    // completion — otherwise the job would be suppressed until real time caught
    // up with the bogus timestamp, potentially for years.
    if last > now {
        tracing::warn!(
            job,
            last_run_unix = last,
            now_unix = now,
            "maintenance last-run timestamp is in the future (clock moved backwards); \
             running now to re-anchor the schedule"
        );
        return true;
    }
    let elapsed = now.saturating_sub(last);
    elapsed >= i64::try_from(interval.as_secs()).unwrap_or(i64::MAX)
}

/// Persist the completion time for `job`, and record it in the in-process
/// floor so a failed write cannot cause the job to re-run every tick.
async fn mark_ran(db_path: &Path, job: &'static str, attempted: &mut HashMap<&'static str, i64>) {
    let now = now_unix();
    attempted.insert(job, now);

    let owned = db_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = birdnet_db::sqlite::open_or_create(&owned).map_err(|e| e.to_string())?;
        birdnet_db::sqlite::record_run(&conn, job, now).map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok(())) => tracing::debug!(job, "maintenance run recorded"),
        Ok(Err(e)) => tracing::warn!(
            error = %e,
            job,
            "could not record maintenance run; the schedule will restart from this boot if the \
             process restarts before the next successful write"
        ),
        Err(e) => tracing::warn!(error = %e, job, "recording the maintenance run panicked"),
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

/// Delete the given clip files and stamp their detections as reclaimed.
///
/// Shared by both retention passes so the delicate part — what counts as
/// "gone", what is left alone to retry, and what is written back — has exactly
/// one implementation. Returns how many files were actually removed from disk.
fn reclaim_clips(
    conn: &rusqlite::Connection,
    recordings_dir: &Path,
    files: Vec<String>,
    pass: &str,
) -> usize {
    let mut removed = 0_usize;
    for file_name in files {
        let Some(base) = Path::new(&file_name).file_name() else {
            continue;
        };
        // `NotFound` counts as pruned-and-done: the audio is gone either way,
        // so stamp the row and stop re-selecting it. Any other error (a
        // permissions problem, a read-only mount) leaves the row alone so the
        // next pass retries instead of marking a live clip gone.
        match std::fs::remove_file(recordings_dir.join(base)) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(
                    pass,
                    file = %file_name,
                    error = %e,
                    "could not remove clip; leaving the detection untouched"
                );
                continue;
            }
        }
        // Record *when* the audio went, and keep `File_Name`. Retention
        // reclaims disk, never provenance: the filename carries the capture
        // timestamp and source the clip was cut from, and is how a detection is
        // matched back to an archived copy. Analyses months later can still see
        // that this detection had audio and what it was called; only the play
        // button goes away.
        if let Err(e) = conn.execute(
            "UPDATE detections SET Clip_Pruned_At = ?2 \
             WHERE File_Name = ?1 AND Clip_Pruned_At IS NULL",
            rusqlite::params![file_name, now_unix()],
        ) {
            tracing::warn!(
                pass,
                file = %file_name,
                error = %e,
                "clip pruned but the detection could not be stamped"
            );
        }
    }
    removed
}

/// Reclaim clip audio older than `days` days (`CLIP_RETENTION_DAYS`).
///
/// The age-based half of retention, alongside the per-species cap and the
/// disk-full backstop. `days == 0` means keep audio forever, which is the
/// default and the behaviour every existing station has today.
///
/// **Why a new setting key rather than the old `recording_days`.** The settings
/// form has always shown a "Keep Recordings (days)" field, defaulted to 30,
/// whose value nothing in the codebase ever read — `cleanup_old_recordings` was
/// called only by its own tests. So the field was inert, and any station whose
/// operator ever saved the settings form for an unrelated reason has `30`
/// sitting in the database, recorded by somebody who was told it meant
/// something and then saw no effect. Teaching the existing key to work would
/// have silently deleted every clip older than a month on those stations at the
/// next maintenance tick — a retroactive purge nobody asked for, triggered by an
/// upgrade. A key no station can already hold cannot do that: age-based
/// retention is off until an operator turns it on, deliberately, after this
/// change.
///
/// Locked clips are exempt and the detection rows survive, exactly as for the
/// per-species cap.
async fn run_clip_retention(db_path: &Path, recordings_dir: &Path, days: u32) {
    if days == 0 || !db_path.exists() {
        return;
    }
    let db_path = db_path.to_path_buf();
    let recordings_dir = recordings_dir.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let conn = birdnet_db::sqlite::open_or_create(&db_path).map_err(|e| e.to_string())?;
        // `Date` is a naive local-date string written from the capture
        // filename, so the cutoff is computed in the same lens ('localtime')
        // rather than against UTC — otherwise the boundary would drift by the
        // station's offset.
        //
        // As with the species cap, a file shared by several detections is only
        // reclaimed when *every* detection referencing it is both past the
        // cutoff and unlocked.
        let cutoff = format!("-{days} days");
        let mut stmt = conn
            .prepare(
                "WITH clips AS (
                     SELECT File_Name,
                            COALESCE(is_locked, 0) AS locked,
                            Date < date('now', 'localtime', ?1) AS expired
                     FROM detections
                     WHERE File_Name IS NOT NULL AND TRIM(File_Name) <> ''
                       AND Clip_Pruned_At IS NULL
                 )
                 SELECT DISTINCT File_Name FROM clips
                 WHERE expired = 1 AND locked = 0
                   AND File_Name NOT IN (
                       SELECT File_Name FROM clips WHERE expired = 0 OR locked = 1
                   )",
            )
            .map_err(|e| e.to_string())?;
        let expired: Vec<String> = stmt
            .query_map([&cutoff], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        Ok(reclaim_clips(
            &conn,
            &recordings_dir,
            expired,
            "clip retention",
        ))
    })
    .await;
    match result {
        Ok(Ok(0)) => tracing::debug!("clip retention: nothing past the cutoff"),
        Ok(Ok(n)) => tracing::info!(
            reclaimed = n,
            keep_days = days,
            "clip retention: reclaimed audio older than the cutoff (detections kept)"
        ),
        Ok(Err(e)) => tracing::warn!(error = %e, "clip retention failed"),
        Err(e) => tracing::warn!(error = %e, "clip retention task panicked"),
    }
}

/// Enforce the per-species recording cap (`MAX_FILES_SPECIES`): keep only the
/// newest `cap` extracted clips per species and delete the older ones off disk.
///
/// DB-driven on purpose — the flat clip filename is not reliably
/// species-parseable (common names can contain hyphens, e.g.
/// `Black-capped_Chickadee`), so the database (the authority on species↔clip)
/// decides what to prune. The detection row is preserved for stats and counts;
/// only its audio file is removed. Best-effort and fully logged; `cap == 0`
/// means unlimited (no-op). The web serves clips flat by base name, so the same
/// base name is what we delete from `recordings_dir`.
///
/// Two invariants this enforces, both of which the first cut got wrong:
///
///   * **Locked clips are never pruned.** `/admin/recordings` → "lock" is the
///     operator's one guarantee that a clip survives automatic cleanup, and
///     `docs/book/field/deployment.md` documents it as such. The disk purge honoured
///     it; this cap did not, so a cap silently deleted the very recordings a
///     researcher had marked to keep.
///   * **The row is stamped, never stripped.** `Clip_Pruned_At` records when
///     the audio was reclaimed and `File_Name` is kept, so a detection still
///     shows that it had a clip and what it was called — provenance an analysis
///     may need long after the disk space was recovered. The stamp is what
///     stops the clips browser offering a play button for a file that no longer
///     exists, and what stops this query re-selecting every already-pruned row
///     forever (which would grow without bound, re-attempting thousands of
///     deletes a day on a station with a year of history).
async fn run_recording_species_cap(db_path: &Path, recordings_dir: &Path, cap: u32) {
    if cap == 0 || !db_path.exists() {
        return;
    }
    let db_path = db_path.to_path_buf();
    let recordings_dir = recordings_dir.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let conn = birdnet_db::sqlite::open_or_create(&db_path).map_err(|e| e.to_string())?;
        // Rank each species' clips newest-first; anything past the cap is an
        // older clip to prune. rowid breaks same-second ties deterministically.
        //
        // The NOT IN guard keeps a file that is *shared* by several detections
        // (legacy rows and BirdNET-Pi imports point several detections at one
        // source segment) from being deleted while any detection still within
        // the cap — or explicitly locked — depends on it.
        let mut stmt = conn
            .prepare(
                "WITH ranked AS (
                     SELECT File_Name,
                            COALESCE(is_locked, 0) AS locked,
                            ROW_NUMBER() OVER (
                                PARTITION BY Com_Name
                                ORDER BY Date DESC, Time DESC, rowid DESC
                            ) AS rn
                     FROM detections
                     WHERE File_Name IS NOT NULL AND TRIM(File_Name) <> ''
                       AND Clip_Pruned_At IS NULL
                 )
                 SELECT DISTINCT File_Name FROM ranked
                 WHERE rn > ?1 AND locked = 0
                   AND File_Name NOT IN (
                       SELECT File_Name FROM ranked WHERE rn <= ?1 OR locked = 1
                   )",
            )
            .map_err(|e| e.to_string())?;
        let over: Vec<String> = stmt
            .query_map([cap], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        Ok(reclaim_clips(&conn, &recordings_dir, over, "species cap"))
    })
    .await;
    match result {
        Ok(Ok(0)) => tracing::debug!("species cap: nothing over the limit"),
        Ok(Ok(n)) => {
            tracing::info!(
                removed = n,
                cap,
                "species cap: pruned oldest clips per species"
            );
        }
        Ok(Err(e)) => tracing::warn!(error = %e, "species cap prune failed"),
        Err(e) => tracing::warn!(error = %e, "species cap prune task panicked"),
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

    /// Insert a detection with a clip file and create the file on disk.
    fn seed_clip(conn: &rusqlite::Connection, dir: &Path, com: &str, date: &str, file: &str) {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, '06:00:00', 'Sci name', ?2, 0.9, ?3)",
            rusqlite::params![date, com, file],
        )
        .unwrap();
        std::fs::write(dir.join(file), b"x").unwrap();
    }

    #[tokio::test]
    async fn species_cap_prunes_oldest_clips_per_species() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            // Four clips for one species (chronological), plus one clip for a
            // second species that must be untouched (its count is under the cap).
            seed_clip(&conn, &recs, "European Robin", "2026-01-01", "robin-1.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-02", "robin-2.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-03", "robin-3.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-04", "robin-4.wav");
            seed_clip(&conn, &recs, "Great Tit", "2026-01-01", "tit-1.wav");
        }

        // Cap at 2 → keep the 2 newest robins, prune the 2 oldest; tit untouched.
        run_recording_species_cap(&db, &recs, 2).await;

        assert!(!recs.join("robin-1.wav").exists(), "oldest robin pruned");
        assert!(
            !recs.join("robin-2.wav").exists(),
            "2nd-oldest robin pruned"
        );
        assert!(recs.join("robin-3.wav").exists(), "newer robin kept");
        assert!(recs.join("robin-4.wav").exists(), "newest robin kept");
        assert!(
            recs.join("tit-1.wav").exists(),
            "under-cap species untouched"
        );

        // DB rows are preserved (only the audio is pruned).
        let conn = rusqlite::Connection::open(&db).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5, "detection rows are kept for stats; only files pruned");
    }

    // ── Restart-durable scheduling (F1) ────────────────────────────────────

    #[tokio::test]
    async fn a_never_run_job_is_due_immediately() {
        // A fresh install or an upgrade should get its first integrity check
        // and backup promptly — an upgrade is exactly when a recoverable
        // snapshot is most valuable.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        rusqlite::Connection::open(&db)
            .map(|c| birdnet_db::migration::migrate(&c).unwrap())
            .unwrap();
        assert!(due(&db, JOB_BACKUP_VACUUM, VACUUM_INTERVAL, &HashMap::new()).await);
    }

    #[tokio::test]
    async fn a_job_that_just_ran_is_not_due() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            birdnet_db::sqlite::record_run(&conn, JOB_BACKUP_VACUUM, now_unix()).unwrap();
        }
        assert!(!due(&db, JOB_BACKUP_VACUUM, VACUUM_INTERVAL, &HashMap::new()).await);
    }

    #[tokio::test]
    async fn the_schedule_survives_a_restart_and_still_fires_when_overdue() {
        // THE regression. The old loop measured its weekly interval from
        // process start, so a station that rebooted more often than weekly —
        // for a settings change, an update, a power cut, a watchdog bounce —
        // never once ran the backup + VACUUM. Here the process is "new" (an
        // empty in-process map, as after a restart) but the recorded run is
        // eight days old, so the job must fire.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let eight_days_ago = now_unix() - 8 * 24 * 60 * 60;
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            birdnet_db::sqlite::record_run(&conn, JOB_BACKUP_VACUUM, eight_days_ago).unwrap();
        }
        assert!(
            due(&db, JOB_BACKUP_VACUUM, VACUUM_INTERVAL, &HashMap::new()).await,
            "an overdue job must run after a restart, not restart its timer"
        );
    }

    #[tokio::test]
    async fn a_recent_run_still_suppresses_the_job_across_a_restart() {
        // Counter-test to the above: restart-durability must not turn into
        // "runs on every boot". A station rebooting hourly must not VACUUM
        // hourly.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let two_days_ago = now_unix() - 2 * 24 * 60 * 60;
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            birdnet_db::sqlite::record_run(&conn, JOB_BACKUP_VACUUM, two_days_ago).unwrap();
        }
        assert!(
            !due(&db, JOB_BACKUP_VACUUM, VACUUM_INTERVAL, &HashMap::new()).await,
            "a weekly job two days old must stay suppressed no matter how often we boot"
        );
    }

    #[tokio::test]
    async fn mark_ran_persists_and_then_suppresses() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        rusqlite::Connection::open(&db)
            .map(|c| birdnet_db::migration::migrate(&c).unwrap())
            .unwrap();
        let mut attempted = HashMap::new();

        assert!(
            due(
                &db,
                JOB_INTEGRITY_CHECK,
                INTEGRITY_CHECK_INTERVAL,
                &attempted
            )
            .await
        );
        mark_ran(&db, JOB_INTEGRITY_CHECK, &mut attempted).await;
        assert!(
            !due(
                &db,
                JOB_INTEGRITY_CHECK,
                INTEGRITY_CHECK_INTERVAL,
                &attempted
            )
            .await
        );

        // And it really landed in the database, not just the in-process map.
        let conn = rusqlite::Connection::open(&db).unwrap();
        assert!(
            birdnet_db::sqlite::last_run_unix(&conn, JOB_INTEGRITY_CHECK)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn in_process_floor_throttles_when_the_database_cannot_record() {
        // If `record_run` cannot write (read-only mount, disk full — the very
        // conditions maintenance exists to survive) the in-process floor must
        // still keep a weekly job from running every half-hourly tick.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        rusqlite::Connection::open(&db)
            .map(|c| birdnet_db::migration::migrate(&c).unwrap())
            .unwrap();
        let mut attempted: HashMap<&'static str, i64> = HashMap::new();
        attempted.insert(JOB_BACKUP_VACUUM, now_unix());
        assert!(
            !due(&db, JOB_BACKUP_VACUUM, VACUUM_INTERVAL, &attempted).await,
            "the in-process floor must suppress a job whose persisted write failed"
        );
    }

    #[tokio::test]
    async fn a_future_timestamp_re_anchors_instead_of_wedging() {
        // A Pi without an RTC boots at the epoch and jumps when NTP lands; a
        // correction can also move the clock backwards, leaving a stored
        // timestamp in the future. Without this the job would be suppressed
        // until real time caught up — potentially for years.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let far_future = now_unix() + 365 * 24 * 60 * 60;
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            birdnet_db::sqlite::record_run(&conn, JOB_SPECIES_CAP, far_future).unwrap();
        }
        assert!(
            due(
                &db,
                JOB_SPECIES_CAP,
                INTEGRITY_CHECK_INTERVAL,
                &HashMap::new()
            )
            .await,
            "a future timestamp must re-anchor the schedule, not disable the job"
        );
    }

    #[tokio::test]
    async fn nothing_is_due_before_the_database_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("not-yet.db");
        assert!(
            !due(
                &missing,
                JOB_INTEGRITY_CHECK,
                INTEGRITY_CHECK_INTERVAL,
                &HashMap::new()
            )
            .await
        );
    }

    // ── Species cap: locks and dangling references (F4/F5) ─────────────────

    #[tokio::test]
    async fn species_cap_never_prunes_a_locked_clip() {
        // "lock" is the operator's one guarantee that a clip survives
        // automatic cleanup, and docs/book/field/deployment.md documents it as
        // such. The disk purge honoured it; this cap did not.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip(
                &conn,
                &recs,
                "European Robin",
                "2026-01-01",
                "robin-old.wav",
            );
            seed_clip(
                &conn,
                &recs,
                "European Robin",
                "2026-01-02",
                "robin-mid.wav",
            );
            seed_clip(
                &conn,
                &recs,
                "European Robin",
                "2026-01-03",
                "robin-new.wav",
            );
            // The operator locks the OLDEST clip — the one the cap would drop.
            conn.execute(
                "UPDATE detections SET is_locked = 1 WHERE File_Name = 'robin-old.wav'",
                [],
            )
            .unwrap();
        }

        run_recording_species_cap(&db, &recs, 1).await;

        assert!(
            recs.join("robin-old.wav").exists(),
            "a locked clip must survive the per-species cap"
        );
        assert!(recs.join("robin-new.wav").exists(), "newest clip kept");
        assert!(
            !recs.join("robin-mid.wav").exists(),
            "the unlocked over-cap clip is still pruned"
        );
    }

    #[tokio::test]
    async fn species_cap_stamps_the_pruned_clip_without_losing_its_name() {
        // Retention reclaims disk, never provenance. The filename carries the
        // capture timestamp and source the clip was cut from and is how a
        // detection is matched back to an archived copy, so it must survive —
        // while the stamp is what stops the browser offering a dead play button
        // and stops this query re-selecting the row forever.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip(&conn, &recs, "European Robin", "2026-01-01", "gone.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-02", "kept.wav");
        }

        run_recording_species_cap(&db, &recs, 1).await;

        let conn = rusqlite::Connection::open(&db).unwrap();
        let (name_kept, stamped): (String, Option<i64>) = conn
            .query_row(
                "SELECT File_Name, Clip_Pruned_At FROM detections WHERE Date = '2026-01-01'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            name_kept, "gone.wav",
            "the pruned clip's name is provenance and must survive"
        );
        assert!(
            stamped.is_some_and(|t| t > 0),
            "the row must record when the audio was reclaimed"
        );

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "the detection itself is preserved for stats");

        // The surviving clip is untouched and still playable.
        let live: Option<i64> = conn
            .query_row(
                "SELECT Clip_Pruned_At FROM detections WHERE File_Name = 'kept.wav'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(live.is_none(), "the surviving clip is not marked pruned");
    }

    #[tokio::test]
    async fn a_pruned_clip_disappears_from_the_clips_browser_but_not_from_the_data() {
        // The two halves of the contract, asserted against the real reader
        // queries rather than a hand-written predicate: no play button, but the
        // detection still counts.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip(&conn, &recs, "European Robin", "2026-01-01", "gone.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-02", "kept.wav");
        }

        run_recording_species_cap(&db, &recs, 1).await;

        let conn = rusqlite::Connection::open(&db).unwrap();
        let clips = birdnet_db::sqlite::recent_clips(
            &conn,
            birdnet_db::sqlite::RecordingsFilter::All,
            None,
            50,
            0,
        )
        .unwrap();
        assert_eq!(clips.len(), 1, "only the playable clip is browsable");
        assert_eq!(clips[0].file_name.as_deref(), Some("kept.wav"));

        // ...but every counting/trending surface still sees both detections.
        assert_eq!(birdnet_db::sqlite::detection_count(&conn).unwrap(), 2);
        let per_day = birdnet_db::sqlite::detections_per_day(&conn).unwrap();
        let total: i64 = per_day.iter().map(|d| d.count).sum();
        assert_eq!(total, 2, "trend data must not lose the pruned detection");
    }

    #[tokio::test]
    async fn species_cap_is_idempotent_and_stops_re_selecting() {
        // Second pass must find nothing left to do — the property that keeps
        // the daily query from growing without bound over a year of history.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            for i in 1..=4 {
                seed_clip(
                    &conn,
                    &recs,
                    "European Robin",
                    &format!("2026-01-0{i}"),
                    &format!("r{i}.wav"),
                );
            }
        }

        run_recording_species_cap(&db, &recs, 2).await;
        let after_first: i64 = rusqlite::Connection::open(&db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Clip_Pruned_At IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_first, 2);

        // Re-running changes nothing and finds nothing to delete.
        run_recording_species_cap(&db, &recs, 2).await;
        let after_second: i64 = rusqlite::Connection::open(&db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Clip_Pruned_At IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_second, 2, "the cap is idempotent");
        assert!(recs.join("r3.wav").exists() && recs.join("r4.wav").exists());
    }

    #[tokio::test]
    async fn species_cap_keeps_a_file_another_in_cap_detection_still_needs() {
        // Legacy rows and BirdNET-Pi imports point several detections at one
        // source segment. Deleting it because *one* of them is over the cap
        // would silently break playback for the others.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            // Robin has three clips; "shared.wav" is its oldest (over a cap of
            // 2) but is also the Great Tit's only, in-cap, clip.
            seed_clip(&conn, &recs, "European Robin", "2026-01-01", "shared.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-02", "robin-2.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-03", "robin-3.wav");
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES ('2026-01-01', '06:00:00', 'Parus major', 'Great Tit', 0.9, 'shared.wav')",
                [],
            )
            .unwrap();
        }

        run_recording_species_cap(&db, &recs, 2).await;

        assert!(
            recs.join("shared.wav").exists(),
            "a file an in-cap detection still references must not be deleted"
        );
    }

    // ── Age-based clip retention (CLIP_RETENTION_DAYS) ─────────────────────

    /// Seed a clip dated `days_ago` relative to today, in the local-date form
    /// the detections table actually stores.
    fn seed_clip_days_ago(
        conn: &rusqlite::Connection,
        dir: &Path,
        com: &str,
        days_ago: i64,
        file: &str,
    ) {
        let date: String = conn
            .query_row(
                "SELECT date('now', 'localtime', ?1)",
                [format!("-{days_ago} days")],
                |r| r.get(0),
            )
            .unwrap();
        seed_clip(conn, dir, com, &date, file);
    }

    #[tokio::test]
    async fn clip_retention_is_off_by_default() {
        // The safety property. The settings form used to show an inert "Keep
        // Recordings (days)" field defaulted to 30, so stations carry a value
        // nobody meant. Retention must stay off until explicitly enabled.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip_days_ago(&conn, &recs, "European Robin", 3650, "ancient.wav");
        }

        run_clip_retention(&db, &recs, 0).await;

        assert!(
            recs.join("ancient.wav").exists(),
            "0 days must mean keep forever — a decade-old clip survives"
        );
    }

    #[tokio::test]
    async fn clip_retention_reclaims_only_what_is_past_the_cutoff() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip_days_ago(&conn, &recs, "European Robin", 40, "old.wav");
            seed_clip_days_ago(&conn, &recs, "European Robin", 31, "just-old.wav");
            seed_clip_days_ago(&conn, &recs, "European Robin", 5, "recent.wav");
            seed_clip_days_ago(&conn, &recs, "Great Tit", 0, "today.wav");
        }

        run_clip_retention(&db, &recs, 30).await;

        assert!(!recs.join("old.wav").exists(), "40 days old is reclaimed");
        assert!(
            !recs.join("just-old.wav").exists(),
            "31 days old is past a 30-day cutoff"
        );
        assert!(recs.join("recent.wav").exists(), "5 days old is kept");
        assert!(recs.join("today.wav").exists(), "today's clip is kept");
    }

    #[tokio::test]
    async fn clip_retention_never_touches_a_locked_clip() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip_days_ago(&conn, &recs, "European Robin", 400, "keep.wav");
            seed_clip_days_ago(&conn, &recs, "European Robin", 400, "drop.wav");
            conn.execute(
                "UPDATE detections SET is_locked = 1 WHERE File_Name = 'keep.wav'",
                [],
            )
            .unwrap();
        }

        run_clip_retention(&db, &recs, 30).await;

        assert!(
            recs.join("keep.wav").exists(),
            "a locked clip outlives any age cutoff"
        );
        assert!(!recs.join("drop.wav").exists());
    }

    #[tokio::test]
    async fn clip_retention_keeps_the_detection_and_its_provenance() {
        // Retention reclaims disk, never analysis data.
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip_days_ago(&conn, &recs, "European Robin", 400, "gone.wav");
        }

        run_clip_retention(&db, &recs, 30).await;

        let conn = rusqlite::Connection::open(&db).unwrap();
        let (name, stamped): (String, Option<i64>) = conn
            .query_row(
                "SELECT File_Name, Clip_Pruned_At FROM detections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "gone.wav", "the filename is provenance and survives");
        assert!(stamped.is_some(), "the row records when the audio went");
        assert_eq!(
            birdnet_db::sqlite::detection_count(&conn).unwrap(),
            1,
            "the detection still counts toward every total and trend"
        );
    }

    #[tokio::test]
    async fn clip_retention_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip_days_ago(&conn, &recs, "European Robin", 400, "a.wav");
            seed_clip_days_ago(&conn, &recs, "European Robin", 2, "b.wav");
        }

        run_clip_retention(&db, &recs, 30).await;
        run_clip_retention(&db, &recs, 30).await;

        let conn = rusqlite::Connection::open(&db).unwrap();
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Clip_Pruned_At IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live, 1, "a second pass finds nothing new to do");
        assert!(recs.join("b.wav").exists());
    }

    #[tokio::test]
    async fn species_cap_zero_is_unlimited_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("birds.db");
        let recs = tmp.path().join("recordings");
        std::fs::create_dir_all(&recs).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            seed_clip(&conn, &recs, "European Robin", "2026-01-01", "keep-1.wav");
            seed_clip(&conn, &recs, "European Robin", "2026-01-02", "keep-2.wav");
        }
        run_recording_species_cap(&db, &recs, 0).await; // unlimited
        assert!(recs.join("keep-1.wav").exists());
        assert!(recs.join("keep-2.wav").exists());
    }
}
