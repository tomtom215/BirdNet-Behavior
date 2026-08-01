//! Persisted run times for the background maintenance schedule.
//!
//! The maintenance loop (integrity check, session prune, per-species recording
//! cap, backup + VACUUM) must be scheduled against the wall clock, not process
//! uptime. An unattended station restarts for all sorts of ordinary reasons —
//! a settings change that "applies on restart", an update, a power cut, a
//! systemd watchdog bounce — and a uptime-relative timer resets on every one of
//! them. A station that reboots daily would never reach a weekly timer.
//!
//! These two functions are the whole persistence layer: record when a job last
//! finished, and ask how long ago that was.

use rusqlite::{Connection, OptionalExtension, params};

use crate::sqlite::connection::DbError;

/// Job key for the daily `PRAGMA integrity_check`.
pub const JOB_INTEGRITY_CHECK: &str = "integrity_check";
/// Job key for the daily expired-login-session prune.
pub const JOB_SESSION_PRUNE: &str = "session_prune";
/// Job key for the daily per-species recording cap (`MAX_FILES_SPECIES`).
pub const JOB_SPECIES_CAP: &str = "species_cap";
/// Job key for the weekly backup + VACUUM pass.
pub const JOB_BACKUP_VACUUM: &str = "backup_vacuum";

/// Period of the daily jobs (integrity check, session prune, species cap).
///
/// Lives here, beside the job keys, so the scheduler that *runs* the job and
/// the admin page that *reports* when it is next due read one definition
/// instead of each carrying its own copy to drift out of sync.
pub const DAILY_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Period of the backup + VACUUM job.
pub const BACKUP_VACUUM_INTERVAL_SECS: i64 = 7 * 24 * 60 * 60;

/// Read the Unix-seconds timestamp at which `job` last completed.
///
/// Returns `None` when the job has never completed on this database — the
/// caller decides whether that means "run immediately" or "start the clock
/// now" (see [`record_run`]).
///
/// # Errors
///
/// Returns `DbError` on query failure. A missing `maintenance_runs` table is
/// a query failure, not `None`: the table is created by migration 21, so its
/// absence means migrations did not run and the caller should not silently
/// treat every job as never-run.
pub fn last_run_unix(conn: &Connection, job: &str) -> Result<Option<i64>, DbError> {
    let ts = conn
        .query_row(
            "SELECT last_run_unix FROM maintenance_runs WHERE job = ?1",
            params![job],
            |row| row.get(0),
        )
        .optional()?;
    Ok(ts)
}

/// Record that `job` completed at `unix_secs`.
///
/// Upserts, so the row is created on first completion and overwritten
/// thereafter.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn record_run(conn: &Connection, job: &str, unix_secs: i64) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO maintenance_runs (job, last_run_unix) VALUES (?1, ?2)
         ON CONFLICT(job) DO UPDATE SET last_run_unix = excluded.last_run_unix",
        params![job, unix_secs],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_or_create;

    #[test]
    fn unrecorded_job_reads_as_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        assert_eq!(last_run_unix(&conn, JOB_BACKUP_VACUUM).unwrap(), None);
    }

    #[test]
    fn record_then_read_round_trips() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        record_run(&conn, JOB_INTEGRITY_CHECK, 1_700_000_000).unwrap();
        assert_eq!(
            last_run_unix(&conn, JOB_INTEGRITY_CHECK).unwrap(),
            Some(1_700_000_000)
        );
    }

    #[test]
    fn record_upserts_rather_than_duplicating() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        record_run(&conn, JOB_SESSION_PRUNE, 1_700_000_000).unwrap();
        record_run(&conn, JOB_SESSION_PRUNE, 1_700_009_999).unwrap();
        assert_eq!(
            last_run_unix(&conn, JOB_SESSION_PRUNE).unwrap(),
            Some(1_700_009_999)
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM maintenance_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "the PRIMARY KEY must collapse repeats to one row");
    }

    #[test]
    fn jobs_are_tracked_independently() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        record_run(&conn, JOB_INTEGRITY_CHECK, 100).unwrap();
        record_run(&conn, JOB_BACKUP_VACUUM, 200).unwrap();
        record_run(&conn, JOB_SPECIES_CAP, 300).unwrap();
        assert_eq!(
            last_run_unix(&conn, JOB_INTEGRITY_CHECK).unwrap(),
            Some(100)
        );
        assert_eq!(last_run_unix(&conn, JOB_BACKUP_VACUUM).unwrap(), Some(200));
        assert_eq!(last_run_unix(&conn, JOB_SPECIES_CAP).unwrap(), Some(300));
        // A job nobody recorded stays unrecorded.
        assert_eq!(last_run_unix(&conn, JOB_SESSION_PRUNE).unwrap(), None);
    }

    #[test]
    fn timestamps_survive_reopening_the_database() {
        // The whole point of the table: a restart must not reset the schedule.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        {
            let conn = open_or_create(tmp.path()).unwrap();
            record_run(&conn, JOB_BACKUP_VACUUM, 1_700_000_000).unwrap();
        }
        let conn = open_or_create(tmp.path()).unwrap();
        assert_eq!(
            last_run_unix(&conn, JOB_BACKUP_VACUUM).unwrap(),
            Some(1_700_000_000),
            "a reboot must not lose the last-run timestamp"
        );
    }
}
