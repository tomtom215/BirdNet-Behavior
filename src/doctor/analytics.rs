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

/// The analytics preflight check.
pub(super) fn check_analytics(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let dir = analytics_dir(cli, config);
    vec![verdict(
        cfg!(feature = "analytics"),
        analytics_request(cli),
        dir.as_deref(),
    )]
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
    fn check_analytics_returns_exactly_one_verdict() {
        // Whatever the build's feature set, the check yields one Check and never
        // panics (it opens no DuckDB).
        assert_eq!(check_analytics(&cli(), None).len(), 1);
    }
}
