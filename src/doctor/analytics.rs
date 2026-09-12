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

const QUARANTINE_NAME: &str = "Quarantined databases";

/// Quarantined databases found beside the live ones — **either** store.
///
/// A corrupt or version-incompatible DuckDB file is moved aside on start and
/// rebuilt from SQLite (see `AnalyticsDb::open_or_quarantine`). That recovery is
/// automatic and correct, but it is only announced in the journal — where an
/// unattended station's operator will never see it.
///
/// # Why this also matches the SQLite store now
///
/// It used to match `.duckdb.corrupt.` only, and its test asserted that a
/// quarantined `birds.db.corrupt.<ts>` was *not* counted, on the stated grounds
/// that it "belongs to the other check". There was no other check. Nothing in
/// the product looked for it: not this scan, not a station-health condition,
/// not a prune. So the one quarantine that means **the entire detection history
/// is gone** — `src/app.rs` moves the database aside and starts fresh when no
/// backup verifies — was the one nothing reported, and the file sat on the card
/// for ever.
///
/// A DuckDB quarantine costs the behavioural dashboards until a rebuild
/// finishes. A SQLite quarantine costs everything the station has ever heard.
/// Both belong here; the message distinguishes them.
pub fn quarantined_files(dir: Option<&Path>) -> Vec<PathBuf> {
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
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                (n.contains(".duckdb.corrupt.") || n.contains(".db.corrupt."))
                        // The `-wal` / `-shm` sidecars are moved alongside the
                        // file they belong to; counting them would report one
                        // incident as three.
                        && !n.contains(".wal")
                        && !n.ends_with("-wal")
                        && !n.ends_with("-shm")
            })
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
            "no quarantined databases — neither store has had to be set aside",
        );
    };
    // The two stores mean opposite things when quarantined. An analytics
    // (`.duckdb`) quarantine is rebuilt from SQLite and loses nothing. A
    // detection (`.db`) quarantine *is* the detection history: `birds.db`
    // failed its check with no usable backup, or a backup was restored over
    // it and it holds everything recorded after that backup was taken. This
    // used to describe both as "no detections were lost".
    let sqlite: Vec<&PathBuf> = found
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".db.corrupt."))
        })
        .collect();
    if let Some(newest_db) = sqlite.last() {
        return Check::warn(
            QUARANTINE_NAME,
            format!(
                "{} quarantined detection database(s) found; the most recent is {}",
                sqlite.len(),
                newest_db.display()
            ),
            "That file is detection history that is not in the live database: either it \
             failed its integrity check with no usable backup, or a backup was restored over \
             it and it holds everything recorded after the backup was taken. Recover its rows \
             (`sqlite3 <file> .recover | sqlite3 rescued.db`, then import) or restore a newer \
             backup, and do not delete it until you have. Repeated quarantines point at \
             failing storage.",
        );
    }
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

/// What the analytics engine will be allowed to use, and whether that is
/// enough (`G-33`).
///
/// Reported here because the decision is otherwise invisible: it is made once
/// at startup, written to the journal, and an operator diagnosing an
/// OOM-killed station months later is looking at `--doctor`, not at the boot
/// they no longer have.
fn memory_verdict(budget: &birdnet_behavioral::memory::Budget) -> Check {
    use birdnet_behavioral::memory::Budget;

    const NAME: &str = "Analytics memory";
    match budget {
        Budget::TooSmall { .. } => Check::warn(
            NAME,
            budget.explain(),
            "Analytics is off on this machine. Give the process more memory — raise \
             MemoryMax= in the systemd unit, or the container's memory limit — or accept \
             that this station records and classifies without the behavioural dashboards.",
        ),
        Budget::Undetected => Check::warn(
            NAME,
            budget.explain(),
            "Neither /proc/meminfo nor a cgroup limit could be read, so the buffer pool \
             could not be sized to this machine. Set BIRDNET_DUCKDB_MEMORY_LIMIT \
             explicitly if analytics queries are being killed.",
        ),
        Budget::Configured { .. } | Budget::Sized { .. } => Check::pass(NAME, budget.explain()),
    }
}

/// The analytics preflight checks.
pub(super) fn check_analytics(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let dir = analytics_dir(cli, config);
    let budget = birdnet_behavioral::memory::decide(
        std::env::var("BIRDNET_DUCKDB_MEMORY_LIMIT").ok().as_deref(),
        birdnet_behavioral::memory::detect_ceiling(),
    );
    vec![
        verdict(
            cfg!(feature = "analytics"),
            analytics_request(cli),
            dir.as_deref(),
        ),
        memory_verdict(&budget),
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

    /// `birds.db.corrupt.<ts>` is the whole detection history, set aside
    /// because it failed its check with no usable backup — or, since the
    /// restore keeps it, the file a backup replaced, holding everything
    /// recorded after that backup. The check used to describe every
    /// quarantine as "analytics … rebuilt automatically from SQLite, so no
    /// detections were lost", which for this file is the opposite of true.
    #[test]
    fn a_quarantined_detection_database_is_never_described_as_lossless() {
        let c = quarantine_verdict(&[PathBuf::from(
            "/var/lib/birdnet/birds.db.corrupt.1800000000",
        )]);
        assert_eq!(c.status, Status::Warn);
        let advice = c.remediation.clone().unwrap_or_default();
        assert!(
            !advice.contains("no detections were lost"),
            "a detection-database quarantine was called lossless: {advice}"
        );
        assert!(
            c.message.contains("detection database"),
            "the message must say which store this is: {}",
            c.message
        );
        assert!(
            advice.contains(".recover") || advice.contains("restore"),
            "the advice must say how to get the history back: {advice}"
        );

        // The counterpart: an analytics-only quarantine keeps the reassurance,
        // because for that store it is true.
        let c = quarantine_verdict(&[PathBuf::from(
            "/var/lib/birdnet/birds.duckdb.corrupt.1800000000",
        )]);
        assert!(
            c.remediation
                .clone()
                .unwrap_or_default()
                .contains("no detections were lost"),
            "{c:?}"
        );
    }

    /// Both stores' quarantines are found, and the sidecars are not double-counted.
    ///
    /// This test used to assert the opposite for SQLite — that
    /// `birds.db.corrupt.<ts>` was *not* counted, because it "belongs to the
    /// other check". There was no other check: nothing in the product looked
    /// for it, so the quarantine that means the whole detection history is gone
    /// was the one nothing reported.
    ///
    /// Observed failing before the scan was widened: `found.len()` was 1, not 2.
    #[test]
    fn the_scan_finds_a_quarantine_of_either_store() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("birds.duckdb"), b"live").unwrap();
        std::fs::write(d.join("birds.db"), b"live").unwrap();
        std::fs::write(d.join("birds.duckdb.corrupt.1700000000"), b"old").unwrap();
        // The analytics store's own sidecar must not become a second incident.
        std::fs::write(d.join("birds.duckdb.corrupt.1700000000.wal"), b"wal").unwrap();
        // The one that means the entire detection history is gone.
        std::fs::write(d.join("birds.db.corrupt.1700000000"), b"sqlite").unwrap();
        // SQLite's sidecars, moved alongside it by `quarantine_corrupt_database`.
        std::fs::write(d.join("birds.db.corrupt.1700000000-wal"), b"wal").unwrap();
        std::fs::write(d.join("birds.db.corrupt.1700000000-shm"), b"shm").unwrap();

        let found = quarantined_files(Some(d));
        assert_eq!(found.len(), 2, "found: {found:?}");
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"birds.duckdb.corrupt.1700000000".to_owned()),
            "{names:?}"
        );
        assert!(
            names.contains(&"birds.db.corrupt.1700000000".to_owned()),
            "the SQLite quarantine is the one that costs the whole history: {names:?}"
        );
    }

    /// The discrimination: a live database is not a quarantine.
    ///
    /// A scan that matched anything with `.db` in the name would satisfy the
    /// test above and report a healthy station as having lost its history.
    #[test]
    fn a_live_database_is_not_a_quarantine() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("birds.db"), b"live").unwrap();
        std::fs::write(d.join("birds.db-wal"), b"wal").unwrap();
        std::fs::write(d.join("birds.duckdb"), b"live").unwrap();
        std::fs::write(d.join("birds.db.backup.1700000000"), b"backup").unwrap();

        assert!(
            quarantined_files(Some(d)).is_empty(),
            "a healthy station must report nothing"
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
    fn check_analytics_returns_every_verdict() {
        // Whatever the build's feature set, the check yields its three Checks —
        // the capability verdict, the memory budget and the quarantine scan —
        // and never panics (it opens no DuckDB).
        let checks = check_analytics(&cli(), None);
        assert_eq!(checks.len(), 3, "{checks:?}");
        assert_eq!(checks[0].name, NAME);
        assert_eq!(checks[1].name, "Analytics memory");
        assert_eq!(checks[2].name, QUARANTINE_NAME);
    }

    /// The memory verdict says something an operator can act on, whichever
    /// case this machine lands in.
    ///
    /// The *values* depend on the machine the tests run on, so what is asserted
    /// is the shape: a refusal and an undetectable machine both warn and both
    /// carry a remediation, and a sized or configured pool passes and names a
    /// number.
    #[test]
    fn the_memory_verdict_is_actionable_in_every_case() {
        use birdnet_behavioral::memory::{Budget, CeilingSource};

        let refused = memory_verdict(&Budget::TooSmall {
            ceiling_mib: 200,
            source: CeilingSource::PhysicalRam,
        });
        assert_eq!(refused.status, Status::Warn);
        assert!(refused.remediation.is_some());
        assert!(refused.message.contains("analytics is off"), "{refused:?}");

        let undetected = memory_verdict(&Budget::Undetected);
        assert_eq!(undetected.status, Status::Warn);
        assert!(undetected.remediation.is_some());

        let sized = memory_verdict(&Budget::Sized {
            pool_mib: 256,
            ceiling_mib: 1024,
            source: CeilingSource::Cgroup,
        });
        assert_eq!(sized.status, Status::Pass);
        assert!(sized.message.contains("256"), "{sized:?}");

        let configured = memory_verdict(&Budget::Configured {
            limit: "2GB".to_owned(),
        });
        assert_eq!(configured.status, Status::Pass);
        assert!(configured.message.contains("2GB"), "{configured:?}");
    }
}
