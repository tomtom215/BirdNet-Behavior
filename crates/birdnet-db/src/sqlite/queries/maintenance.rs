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
/// Job key for the daily prune of the append-only operational logs.
pub const JOB_LOG_RETENTION: &str = "log_retention";
/// Job key for the daily `species_summary` drift check.
pub const JOB_SUMMARY_AUDIT: &str = "summary_audit";
/// Job key for the offsite upload of the weekly snapshot.
///
/// Separate from [`JOB_BACKUP_VACUUM`] because the two fail independently and
/// mean different things: a local backup that fails means the station has no
/// recoverable snapshot at all, and an offsite upload that fails means the only
/// copy is on the card the scheme exists to survive. Recorded under its own key
/// so a health check can tell an operator which one it is — before this, an
/// offsite failure produced one `warn!` and reached no counter, no
/// `maintenance_runs` row, no health field and no alert, so a station whose
/// only off-card copy had failed for twelve months looked identical to one
/// whose uploads all succeeded.
pub const JOB_OFFSITE_BACKUP: &str = "offsite_backup";

/// How long an `audit_log` row is kept.
///
/// 180 days is what O-15 documented as the retention default; until this
/// constant had a scheduler behind it, `AuditLog::prune` had no production
/// caller at all and the table grew for the life of the station.
pub const AUDIT_RETENTION_DAYS: u32 = 180;

/// How long a `sound_levels` / `sound_level_broadband` bucket is kept.
///
/// 400 days, matching `audio_levels`' own retention, so a full year of
/// soundscape plus a margin survives and a two-year station does not carry
/// four years of ⅓-octave buckets on an SD card.
///
/// Like `AUDIT_RETENTION_DAYS` before it, this constant had a pruner and no
/// caller: `sound_levels::prune` was reachable from nowhere, so both tables
/// grew for the life of the station despite a doc comment saying otherwise.
pub const SOUND_LEVEL_RETENTION_DAYS: u32 = 400;

/// How long a **reviewed** quarantine row is kept.
///
/// Unreviewed rows are never pruned: they are the operator's queue, and
/// deleting a decision nobody has made yet is the one thing this must not do.
/// 90 days after review matches `NOTIFICATION_RETENTION_DAYS`, and the same
/// "there was a pruner and no caller" applies — `prune_quarantine`'s own doc
/// comment says "This prevents the table from growing unbounded on long-running
/// stations", which was not true of any station.
pub const QUARANTINE_RETENTION_DAYS: u32 = 90;

/// How long a `notification_log` row is kept.
///
/// 90 days, matching the number `/admin/notifications` already passes — which
/// was the *only* thing that pruned this table, so on a headless station (the
/// deployment this project is for) it never ran at all.
pub const NOTIFICATION_RETENTION_DAYS: u32 = 90;

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
    record_run_result(conn, job, unix_secs, None)
}

/// Record that `job` completed at `unix_secs`, and whether it passed.
///
/// `ok` is `None` for jobs with no pass/fail to report — the session prune
/// either ran or errored, it has no verdict — and `Some` for the ones that do.
/// Storing the verdict is what lets a reader learn the database is sound
/// without checking it again: [`crate::sqlite::quick_check`] reads every page
/// of the file, which is far too expensive to put behind a badge that refreshes
/// every 30 s on every page.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn record_run_result(
    conn: &Connection,
    job: &str,
    unix_secs: i64,
    ok: Option<bool>,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO maintenance_runs (job, last_run_unix, ok) VALUES (?1, ?2, ?3)
         ON CONFLICT(job) DO UPDATE SET last_run_unix = excluded.last_run_unix,
                                        ok            = excluded.ok",
        params![job, unix_secs, ok],
    )?;
    Ok(())
}

/// The verdict `job` last recorded, and when.
///
/// Three distinct answers, and the caller must not collapse them:
///
/// * `None` — the job has never completed on this database. Nothing is known.
/// * `Some((when, None))` — it ran, and reported no verdict.
/// * `Some((when, Some(ok)))` — it ran and passed (`true`) or failed (`false`).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn last_run_result(
    conn: &Connection,
    job: &str,
) -> Result<Option<(i64, Option<bool>)>, DbError> {
    let row = conn
        .query_row(
            "SELECT last_run_unix, ok FROM maintenance_runs WHERE job = ?1",
            params![job],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<bool>>(1)?)),
        )
        .optional()?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// The job catalogue (`G-32`)
// ---------------------------------------------------------------------------

/// One scheduled maintenance job: what it is called, what it does, and how
/// often it should run.
///
/// This exists because `maintenance_runs` is not a list of jobs. It is a list
/// of jobs that have *completed at least once*, and those are different sets.
/// A job that has never run has no row, which migration 28's own note calls
/// out as "a third state the badge must not confuse with a failure" — so
/// anything reporting the schedule to an operator has to start from the
/// catalogue and look the rows up, never the other way round. Enumerating the
/// table would silently omit exactly the jobs worth asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSpec {
    /// The key it is recorded under in `maintenance_runs`.
    pub job: &'static str,
    /// What it does, in words an operator can act on.
    pub title: &'static str,
    /// How often it should run, in seconds.
    pub interval_secs: i64,
    /// Whether this job records a pass/fail verdict.
    ///
    /// A session prune either ran or errored; it has no verdict, and a reader
    /// must not render its `None` as a failure. See [`record_run_result`].
    pub reports_verdict: bool,
}

/// Every job the maintenance loop schedules.
///
/// A unit test in this module scans this file's own source for `pub const
/// JOB_` declarations and fails if one of them is missing here, so a job
/// cannot be added to the scheduler and stay invisible to anything that
/// reports the schedule.
pub const JOBS: &[JobSpec] = &[
    JobSpec {
        job: JOB_INTEGRITY_CHECK,
        title: "Database integrity check",
        interval_secs: DAILY_INTERVAL_SECS,
        reports_verdict: true,
    },
    JobSpec {
        job: JOB_BACKUP_VACUUM,
        title: "Backup and VACUUM",
        interval_secs: BACKUP_VACUUM_INTERVAL_SECS,
        reports_verdict: true,
    },
    JobSpec {
        job: JOB_OFFSITE_BACKUP,
        title: "Offsite backup upload",
        // Recorded during the backup pass, so it shares that cadence rather
        // than having one of its own.
        interval_secs: BACKUP_VACUUM_INTERVAL_SECS,
        reports_verdict: true,
    },
    JobSpec {
        job: JOB_SPECIES_CAP,
        title: "Recording cap and clip retention",
        interval_secs: DAILY_INTERVAL_SECS,
        reports_verdict: false,
    },
    JobSpec {
        job: JOB_LOG_RETENTION,
        title: "Operational log retention",
        interval_secs: DAILY_INTERVAL_SECS,
        reports_verdict: false,
    },
    JobSpec {
        job: JOB_SUMMARY_AUDIT,
        title: "Species summary drift check",
        interval_secs: DAILY_INTERVAL_SECS,
        reports_verdict: false,
    },
    JobSpec {
        job: JOB_SESSION_PRUNE,
        title: "Expired login session prune",
        interval_secs: DAILY_INTERVAL_SECS,
        reports_verdict: false,
    },
];

/// Why a job is due to run.
///
/// Named rather than a bare `bool` because the scheduler logs one of these
/// cases and not the others, and because a reader deserves to know that
/// "overdue" and "the clock moved" are different situations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueReason {
    /// It has never completed on this database.
    ///
    /// Due immediately rather than one full period from now: a fresh install
    /// or an upgrade is precisely when a first integrity check and a
    /// recoverable snapshot are most valuable.
    NeverRun,
    /// Its recorded last run is in the future.
    ///
    /// A Pi with no real-time clock boots at the epoch and jumps forward when
    /// NTP lands; a correction can also move the clock backwards, leaving a
    /// stored timestamp ahead of now. Suppressing the job until real time
    /// caught up could mean years, so it runs and re-anchors the schedule.
    ClockWentBackwards,
    /// A full interval has passed since it last ran.
    IntervalElapsed,
}

/// Whether `job` is due, given when it last ran.
///
/// **This is the scheduler's own rule.** `src/maintenance.rs::due` calls this
/// for the decision and adds only the things a pure function cannot do — read
/// the timestamp from disk, and apply its in-process floor. Reporting surfaces
/// call it too, so "overdue" on a page and "run it now" in the loop cannot
/// drift apart; before this they were two copies of three branches.
///
/// The one thing a reporting caller does **not** see is that in-process floor,
/// which exists so a station whose disk is full cannot re-run a weekly VACUUM
/// every half hour. It only ever *delays* a job within one process lifetime,
/// so a job this reports as due may already have run since the last recorded
/// completion. That is the honest answer from persisted state, which is the
/// only state an operator can inspect.
#[must_use]
pub const fn due_state(
    last_run_unix: Option<i64>,
    interval_secs: i64,
    now_unix: i64,
) -> Option<DueReason> {
    let Some(last) = last_run_unix else {
        return Some(DueReason::NeverRun);
    };
    if last > now_unix {
        return Some(DueReason::ClockWentBackwards);
    }
    if now_unix.saturating_sub(last) >= interval_secs {
        return Some(DueReason::IntervalElapsed);
    }
    None
}

/// What is known about one scheduled job right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStatus {
    /// Its key in `maintenance_runs`.
    pub job: &'static str,
    /// What it does.
    pub title: &'static str,
    /// How often it should run, in seconds.
    pub interval_secs: i64,
    /// When it last completed, or `None` if it never has.
    pub last_run_unix: Option<i64>,
    /// Its last verdict: `None` when it has never run **or** when it reports
    /// no verdict. [`JobSpec::reports_verdict`] tells the two apart.
    pub ok: Option<bool>,
    /// Why it is due, or `None` if it is not.
    pub due: Option<DueReason>,
    /// When it next falls due, or `None` if it has never run.
    pub next_due_unix: Option<i64>,
}

/// The status of every job in [`JOBS`], as of `now_unix`.
///
/// Driven by the catalogue and not by the table, so a job that has never run
/// appears with `last_run_unix: None` rather than not appearing at all.
///
/// # Errors
///
/// Returns `DbError` on query failure. A missing `maintenance_runs` table is
/// an error rather than an empty answer, for the reason [`last_run_unix`]
/// gives: it would otherwise report every job as never-run on a database whose
/// migrations did not run.
pub fn job_statuses(conn: &Connection, now_unix: i64) -> Result<Vec<JobStatus>, DbError> {
    let mut out = Vec::with_capacity(JOBS.len());
    for spec in JOBS {
        let row = last_run_result(conn, spec.job)?;
        let (last_run_unix, ok) = match row {
            Some((when, ok)) => (Some(when), ok),
            None => (None, None),
        };
        out.push(JobStatus {
            job: spec.job,
            title: spec.title,
            interval_secs: spec.interval_secs,
            last_run_unix,
            ok,
            due: due_state(last_run_unix, spec.interval_secs, now_unix),
            next_due_unix: last_run_unix.map(|t| t.saturating_add(spec.interval_secs)),
        });
    }
    Ok(out)
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

#[cfg(test)]
mod job_catalogue_tests {
    use super::{
        BACKUP_VACUUM_INTERVAL_SECS, DAILY_INTERVAL_SECS, DueReason, JOBS, JobSpec, due_state,
        job_statuses,
    };
    use crate::sqlite::open_or_create;

    /// This file's own source, so the gate below reads the declarations
    /// rather than a hand-kept copy of them.
    const THIS_FILE: &str = include_str!("maintenance.rs");

    /// Every `pub const JOB_…` declared in this file, by its string value.
    fn declared_job_keys() -> Vec<String> {
        let mut out = Vec::new();
        for line in THIS_FILE.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("pub const JOB_") else {
                continue;
            };
            // `NAME: &str = "value";` — take what is between the quotes.
            let Some(open) = rest.find('"') else { continue };
            let Some(close) = rest[open + 1..].find('"') else {
                continue;
            };
            out.push(rest[open + 1..open + 1 + close].to_owned());
        }
        out
    }

    /// A job the scheduler runs and the catalogue does not list is invisible
    /// to everything that reports the schedule — which is the whole failure
    /// this catalogue exists to prevent.
    ///
    /// Observed failing with the `JOB_SESSION_PRUNE` entry removed from
    /// `JOBS`; the assertion named `session_prune` as declared but missing
    /// from the catalogue.
    #[test]
    fn every_declared_job_is_in_the_catalogue() {
        let declared = declared_job_keys();
        assert!(
            declared.len() >= 7,
            "the scan found only {declared:?}; it has stopped matching the declarations"
        );
        let listed: Vec<&str> = JOBS.iter().map(|j| j.job).collect();
        let missing: Vec<&String> = declared
            .iter()
            .filter(|d| !listed.contains(&d.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "these jobs are declared but missing from JOBS: {missing:?}"
        );
        let unknown: Vec<&&str> = listed
            .iter()
            .filter(|l| !declared.iter().any(|d| d == *l))
            .collect();
        assert!(
            unknown.is_empty(),
            "these JOBS entries match no declared constant: {unknown:?}"
        );
    }

    /// The catalogue is a set, and two entries sharing a key would make one of
    /// them unreachable.
    #[test]
    fn the_catalogue_has_no_duplicate_keys() {
        let mut keys: Vec<&str> = JOBS.iter().map(|j| j.job).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "duplicate job keys in JOBS");
    }

    /// Every interval is one the scheduler actually uses. A catalogue entry
    /// claiming a cadence nothing schedules would report a next-due time that
    /// never arrives.
    #[test]
    fn every_interval_is_one_the_scheduler_runs() {
        for JobSpec {
            job, interval_secs, ..
        } in JOBS
        {
            assert!(
                *interval_secs == DAILY_INTERVAL_SECS
                    || *interval_secs == BACKUP_VACUUM_INTERVAL_SECS,
                "{job} claims a {interval_secs}s cadence, which nothing schedules"
            );
        }
    }

    // ── the due rule ────────────────────────────────────────────────────

    /// A job that has never run is due now, not one full period from now: an
    /// upgrade is exactly when a first integrity check and a recoverable
    /// snapshot matter most.
    #[test]
    fn a_job_that_has_never_run_is_due_immediately() {
        assert_eq!(
            due_state(None, DAILY_INTERVAL_SECS, 0),
            Some(DueReason::NeverRun)
        );
    }

    /// A timestamp in the future means the clock moved backwards. Waiting for
    /// real time to catch up could mean years on a Pi that booted at the
    /// epoch and then had NTP land.
    ///
    /// Observed failing with the `last > now_unix` branch removed: the state
    /// came back `None` and the job would have been suppressed.
    #[test]
    fn a_last_run_in_the_future_is_due_rather_than_suppressed() {
        assert_eq!(
            due_state(Some(2_000), DAILY_INTERVAL_SECS, 1_000),
            Some(DueReason::ClockWentBackwards)
        );
    }

    /// The boundary itself, both sides. A gate that only checked "an hour ago
    /// is not due" would pass against an off-by-one that never fires.
    ///
    /// Observed failing with `>` for `>=`: the job was not due at exactly one
    /// interval, so each run drifted a tick later than the last.
    #[test]
    fn a_job_is_due_at_exactly_one_interval_and_not_a_second_before() {
        let last = 1_000_000;
        let now_at = last + DAILY_INTERVAL_SECS;
        assert_eq!(
            due_state(Some(last), DAILY_INTERVAL_SECS, now_at),
            Some(DueReason::IntervalElapsed)
        );
        assert_eq!(due_state(Some(last), DAILY_INTERVAL_SECS, now_at - 1), None);
    }

    // ── the status query ────────────────────────────────────────────────

    /// The point of the catalogue: an empty `maintenance_runs` still reports
    /// every job, as never-run and due.
    ///
    /// Observed failing with `job_statuses` rewritten to `SELECT job FROM
    /// maintenance_runs`: it returned nothing at all on a fresh database.
    #[test]
    fn a_fresh_database_reports_every_job_as_never_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_or_create(&dir.path().join("t.db")).expect("db");
        let statuses = job_statuses(&conn, 1_000_000).expect("statuses");
        assert_eq!(statuses.len(), JOBS.len());
        for s in &statuses {
            assert_eq!(s.last_run_unix, None, "{}", s.job);
            assert_eq!(s.due, Some(DueReason::NeverRun), "{}", s.job);
            assert_eq!(s.next_due_unix, None, "{}", s.job);
            assert!(!s.title.is_empty(), "{}", s.job);
        }
    }

    /// A job that ran recently is not due, carries its verdict, and knows
    /// when it next falls due.
    #[test]
    fn a_job_that_just_ran_reports_its_verdict_and_its_next_due_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_or_create(&dir.path().join("t.db")).expect("db");
        let now = 1_000_000;
        super::record_run_result(&conn, super::JOB_INTEGRITY_CHECK, now, Some(true)).expect("rec");

        let statuses = job_statuses(&conn, now + 60).expect("statuses");
        let it = statuses
            .iter()
            .find(|s| s.job == super::JOB_INTEGRITY_CHECK)
            .expect("integrity check is in the catalogue");
        assert_eq!(it.last_run_unix, Some(now));
        assert_eq!(it.ok, Some(true));
        assert_eq!(it.due, None);
        assert_eq!(it.next_due_unix, Some(now + DAILY_INTERVAL_SECS));

        // Its neighbours are untouched — one job running does not mark others.
        let prune = statuses
            .iter()
            .find(|s| s.job == super::JOB_SESSION_PRUNE)
            .expect("session prune is in the catalogue");
        assert_eq!(prune.last_run_unix, None);
    }

    /// A failed job and a job with nothing to report both store `ok = NULL`
    /// or `ok = 0`, and a reader must not collapse them. `reports_verdict` is
    /// what separates "it failed" from "it has no verdict".
    ///
    /// Observed failing with `reports_verdict` hard-coded to `true`: the
    /// session prune's absent verdict became indistinguishable from the
    /// integrity check's recorded failure.
    #[test]
    fn a_job_with_no_verdict_is_not_a_failed_job() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_or_create(&dir.path().join("t.db")).expect("db");
        let now = 1_000_000;
        super::record_run_result(&conn, super::JOB_INTEGRITY_CHECK, now, Some(false)).expect("a");
        super::record_run(&conn, super::JOB_SESSION_PRUNE, now).expect("b");

        let statuses = job_statuses(&conn, now).expect("statuses");
        let failed = statuses
            .iter()
            .find(|s| s.job == super::JOB_INTEGRITY_CHECK)
            .expect("a");
        let quiet = statuses
            .iter()
            .find(|s| s.job == super::JOB_SESSION_PRUNE)
            .expect("b");
        assert_eq!(failed.ok, Some(false), "a recorded failure must survive");
        assert_eq!(quiet.ok, None, "a job with no verdict must not invent one");

        let spec_failed = JOBS
            .iter()
            .find(|j| j.job == super::JOB_INTEGRITY_CHECK)
            .expect("a");
        let spec_quiet = JOBS
            .iter()
            .find(|j| j.job == super::JOB_SESSION_PRUNE)
            .expect("b");
        assert!(spec_failed.reports_verdict);
        assert!(
            !spec_quiet.reports_verdict,
            "the session prune has no verdict to report, so its None is not a failure"
        );
    }

    /// A job overdue by more than its interval is still reported as due
    /// rather than wrapping or saturating into a nonsense next-due time.
    #[test]
    fn a_long_overdue_job_reports_as_due() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_or_create(&dir.path().join("t.db")).expect("db");
        let last = 1_000;
        super::record_run(&conn, super::JOB_BACKUP_VACUUM, last).expect("rec");
        let statuses = job_statuses(&conn, last + BACKUP_VACUUM_INTERVAL_SECS * 10).expect("s");
        let it = statuses
            .iter()
            .find(|s| s.job == super::JOB_BACKUP_VACUUM)
            .expect("f");
        assert_eq!(it.due, Some(DueReason::IntervalElapsed));
        assert_eq!(it.next_due_unix, Some(last + BACKUP_VACUUM_INTERVAL_SECS));
    }
}
