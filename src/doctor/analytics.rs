//! Analytics / behavioral-extension preflight.
//!
//! Answers "will the behavioral-analytics dashboards work on this install?"
//! without opening DuckDB (the daemon does that, and a preflight DuckDB open
//! during `ExecStartPre` could contend with a running service). The verdict
//! turns on three preconditions:
//!   1. was this binary built with the `analytics` feature? (release binaries
//!      are; a `--no-default-features` dev build is not);
//!   2. is analytics enabled, or explicitly opted out via an empty
//!      `--analytics-db`?;
//!   3. is the directory that will hold the DuckDB file writable?
//!
//! The behavioral extension itself ships embedded in release binaries and loads
//! offline (see `crates/birdnet-behavioral`), so a correctly-built, correctly-
//! pathed install needs no network — which is exactly what this reports.

use std::path::{Path, PathBuf};

use birdnet_core::config::Config;

use super::{Check, writable};
use crate::cli::Cli;

const NAME: &str = "Analytics (behavioral)";

/// Whether analytics is enabled or explicitly opted out.
///
/// Mirrors `helpers::build_state_with_analytics`: an empty `--analytics-db` is a
/// deliberate opt-out; an explicit non-empty path, the `ANALYTICS_DB_PATH`
/// config key, or the default all mean "enabled". Pure so it is unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    /// Operator turned analytics off with an empty `--analytics-db`.
    Disabled,
    /// Analytics is on (explicitly, via config, or by default).
    Enabled,
}

fn analytics_request(cli: &Cli) -> Request {
    match cli.analytics_db.as_ref() {
        Some(p) if p.as_os_str().is_empty() => Request::Disabled,
        _ => Request::Enabled,
    }
}

/// The directory that will hold the DuckDB analytics file, if it can be
/// resolved — the parent of the configured (or default) analytics DB path. The
/// default sits beside the operational SQLite database, so we fall back to that
/// database's directory.
fn analytics_dir(cli: &Cli, config: Option<&Config>) -> Option<PathBuf> {
    // An explicit (non-empty) --analytics-db or ANALYTICS_DB_PATH wins; otherwise
    // the default analytics DB lives beside the operational SQLite database, so
    // fall back to DB_PATH's directory.
    let path = cli
        .analytics_db
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned()
        .or_else(|| config.and_then(|c| c.get("ANALYTICS_DB_PATH").map(PathBuf::from)))
        .or_else(|| config.and_then(|c| c.get("DB_PATH").map(PathBuf::from)))?;
    path.parent().map(Path::to_path_buf)
}

/// Pure verdict from the three preconditions, so every branch is unit-testable
/// without a DuckDB build. `compiled` is `cfg!(feature = "analytics")` at the
/// call site; `dir` is the analytics directory if known.
fn verdict(compiled: bool, req: Request, dir: Option<&Path>) -> Check {
    if !compiled {
        return match req {
            // The operator pointed at an analytics DB but this binary can't
            // provide analytics — the dashboards would silently stay empty.
            // `dir.is_some()` means analytics was actually configured (explicit
            // path or a DB path to default beside), not just a bare dev run.
            Request::Enabled if dir.is_some() => Check::warn(
                NAME,
                "an analytics database is configured but this binary was built WITHOUT analytics support",
                "Install a release binary (analytics is on by default), or rebuild with `--features analytics`.",
            ),
            _ => Check::skip(
                NAME,
                "this is a slim build (no DuckDB analytics compiled in)",
            ),
        };
    }

    match req {
        Request::Disabled => Check::skip(
            NAME,
            "analytics explicitly disabled via an empty --analytics-db",
        ),
        Request::Enabled => match dir {
            // The directory exists but the daemon couldn't write the DuckDB file
            // there — analytics would fail to open at runtime.
            Some(d) if d.exists() && !writable(d) => Check::warn(
                NAME,
                format!("analytics is enabled but {} is not writable", d.display()),
                "Fix the directory's ownership/permissions (try: install.sh repair); the DuckDB file is created there on first run.",
            ),
            _ => Check::pass(
                NAME,
                "enabled — the behavioral extension is embedded in release binaries and loads with no network",
            ),
        },
    }
}

const QUARANTINE_NAME: &str = "Analytics (quarantined files)";

/// Quarantined analytics databases found beside the live one.
///
/// A corrupt or version-incompatible DuckDB file is moved aside on start and
/// rebuilt from SQLite (see `AnalyticsDb::open_or_quarantine`). That recovery is
/// automatic and correct, but it is only announced in the journal — where an
/// unattended station's operator will never see it. Surfacing the leftover file
/// here means the doctor and `/admin/doctor` report that something went wrong
/// and that a file is sitting there using disk.
fn quarantined_files(dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".duckdb.corrupt.") && !n.contains(".wal"))
        })
        .collect();
    found.sort();
    found
}

/// Pure verdict for the quarantine check, so both branches are unit-testable
/// without touching a filesystem.
fn quarantine_verdict(found: &[PathBuf]) -> Check {
    let Some(newest) = found.last() else {
        return Check::pass(
            QUARANTINE_NAME,
            "no quarantined analytics databases — the analytics store has not had to be rebuilt",
        );
    };
    Check::warn(
        QUARANTINE_NAME,
        format!(
            "{} quarantined analytics database(s) found; the most recent is {}",
            found.len(),
            newest.display()
        ),
        "Analytics was rebuilt automatically from SQLite, so no detections were lost. \
         Delete the quarantined file(s) once you are satisfied nothing else is wrong; \
         repeated quarantines point at failing storage.",
    )
}

/// The analytics preflight checks.
pub(super) fn check_analytics(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let dir = analytics_dir(cli, config);
    vec![
        verdict(
            cfg!(feature = "analytics"),
            analytics_request(cli),
            dir.as_deref(),
        ),
        quarantine_verdict(&quarantined_files(dir.as_deref())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser as _;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn quarantine_check_passes_when_nothing_was_rebuilt() {
        let c = quarantine_verdict(&[]);
        assert_eq!(c.status, Status::Pass);
    }

    #[test]
    fn quarantine_check_warns_and_names_the_newest_file() {
        let found = vec![
            PathBuf::from("/var/lib/birdnet/birds.duckdb.corrupt.1700000000"),
            PathBuf::from("/var/lib/birdnet/birds.duckdb.corrupt.1800000000"),
        ];
        let c = quarantine_verdict(&found);
        assert_eq!(c.status, Status::Warn);
        assert!(c.message.contains('2'), "the count should be reported");
        assert!(
            c.message.contains("1800000000"),
            "the most recent quarantine should be named: {}",
            c.message
        );
    }

    #[test]
    fn quarantine_scan_finds_only_analytics_corpses() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("birds.duckdb"), b"live").unwrap();
        std::fs::write(d.join("birds.duckdb.corrupt.1700000000"), b"old").unwrap();
        // The sidecar moved alongside it must not be counted as a second one.
        std::fs::write(d.join("birds.duckdb.corrupt.1700000000.wal"), b"wal").unwrap();
        // A quarantined *SQLite* database belongs to the other check.
        std::fs::write(d.join("birds.db.corrupt.1700000000"), b"sqlite").unwrap();

        let found = quarantined_files(Some(d));
        assert_eq!(found.len(), 1, "found: {found:?}");
        assert!(
            found[0]
                .to_string_lossy()
                .ends_with(".duckdb.corrupt.1700000000")
        );
    }

    #[test]
    fn quarantine_scan_is_quiet_on_a_missing_directory() {
        assert!(quarantined_files(Some(Path::new("/nonexistent/xyzzy"))).is_empty());
        assert!(quarantined_files(None).is_empty());
    }

    #[test]
    fn request_reads_the_opt_out() {
        let mut c = cli();
        assert_eq!(analytics_request(&c), Request::Enabled); // default on
        c.analytics_db = Some(PathBuf::from("/data/birds.duckdb"));
        assert_eq!(analytics_request(&c), Request::Enabled); // explicit path
        c.analytics_db = Some(PathBuf::new()); // empty == opt-out
        assert_eq!(analytics_request(&c), Request::Disabled);
    }

    #[test]
    fn slim_build_warns_only_when_analytics_was_requested() {
        // !compiled + a resolvable (explicit) dir + Enabled → warn.
        let warn = verdict(false, Request::Enabled, Some(Path::new("/data")));
        assert_eq!(warn.status, Status::Warn);
        assert!(warn.remediation.is_some());
        // !compiled + nothing requested → a quiet skip (slim dev build).
        let skip = verdict(false, Request::Enabled, None);
        assert_eq!(skip.status, Status::Skip);
        let skip_disabled = verdict(false, Request::Disabled, None);
        assert_eq!(skip_disabled.status, Status::Skip);
    }

    #[test]
    fn analytics_build_passes_when_enabled_and_writable() {
        // compiled + enabled + a writable dir (tempdir) → pass.
        let tmp = std::env::temp_dir();
        let pass = verdict(true, Request::Enabled, Some(&tmp));
        assert_eq!(pass.status, Status::Pass);
        // compiled + opted out → skip.
        let skip = verdict(true, Request::Disabled, None);
        assert_eq!(skip.status, Status::Skip);
    }

    #[test]
    fn analytics_build_warns_when_dir_unwritable() {
        // An existing-but-unwritable analytics dir is a real, actionable fault.
        let unwritable = Path::new("/proc"); // exists, not writable
        let v = verdict(true, Request::Enabled, Some(unwritable));
        assert_eq!(v.status, Status::Warn);
        assert!(v.remediation.is_some());
    }

    #[test]
    fn check_analytics_returns_both_verdicts() {
        // Whatever the build's feature set, the check yields its two Checks —
        // the capability verdict and the quarantine scan — and never panics (it
        // opens no DuckDB).
        let checks = check_analytics(&cli(), None);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].name, NAME);
        assert_eq!(checks[1].name, QUARANTINE_NAME);
    }
}
