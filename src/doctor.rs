//! End-user diagnostic subcommand.
//!
//! Runs a suite of preflight checks that answer the question
//! "is my BirdNet-Behavior install in a state where it can actually
//! detect birds?" and prints a one-screen report that a non-technical
//! operator can act on without having to read a stack trace.
//!
//! Each check is independent: a failure in one does not skip the others,
//! so the operator sees every issue in a single pass.
//!
//! The exit code summarises the worst severity observed:
//!   * `0` — all checks passed (some may be skipped/informational)
//!   * `1` — at least one warning, no errors
//!   * `2` — at least one error
//!
//! This makes the command useful both interactively and from monitoring
//! scripts (`birdnet-behavior --doctor; echo $?`).
//!
//! ## Module layout
//!
//! This module is the facade: it owns the shared [`Status`] / [`Check`] /
//! [`Format`] types, the [`run_with_format`] orchestration, and the two
//! filesystem helpers ([`writable`], [`tool_exists`]) several checks share.
//! Each family of checks lives in its own submodule (`config`, `database`,
//! `paths`, `audio`, `model`, `environment`, `disk`, `watchdog`), and all
//! report rendering (`text` / `json` / exit-code) lives in `render`.

use std::fmt;
use std::path::Path;

use birdnet_core::config::Config;

use crate::cli::Cli;

mod analytics;
mod audio;
mod clock;
mod config;
mod database;
mod disk;
mod environment;
mod fix;
mod model;
mod paths;
mod render;
mod watchdog;

/// Verdict of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Everything looks healthy.
    Pass,
    /// Check did not apply in this configuration (informational only).
    Skip,
    /// Functionality is degraded but the system will still start.
    Warn,
    /// The system will not work correctly until this is fixed.
    Fail,
}

impl Status {
    const fn tag(self) -> &'static str {
        match self {
            Self::Pass => "[ PASS ]",
            Self::Skip => "[ SKIP ]",
            Self::Warn => "[ WARN ]",
            Self::Fail => "[ FAIL ]",
        }
    }
}

/// Outcome of a single diagnostic check.
#[derive(Debug, Clone)]
pub struct Check {
    /// Short, human-readable name of the check.
    pub name: String,
    /// Verdict.
    pub status: Status,
    /// Short message shown next to the status tag.
    pub message: String,
    /// Optional remediation hint (printed on the next line if present).
    pub remediation: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Pass,
            message: message.into(),
            remediation: None,
        }
    }
    fn skip(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Skip,
            message: message.into(),
            remediation: None,
        }
    }
    fn warn(name: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            message: message.into(),
            remediation: Some(fix.into()),
        }
    }
    fn fail(name: impl Into<String>, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Fail,
            message: message.into(),
            remediation: Some(fix.into()),
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {} — {}", self.status.tag(), self.name, self.message)?;
        if let Some(fix) = &self.remediation {
            writeln!(f, "         → {fix}")?;
        }
        Ok(())
    }
}

/// Output format for the diagnostic report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Human-readable text with one check per line and a trailing summary.
    Text,
    /// Machine-readable single-line JSON object suitable for monitoring
    /// scripts. Schema:
    /// `{"summary":{"passed":N,"warnings":N,"errors":N,"skipped":N,"exit_code":N},`
    /// ` "checks":[{"status":"pass|warn|fail|skip","name":"...","message":"...","remediation":"..."|null}, ...]}`
    Json,
}

/// Run every preflight check and print a report in the given format.
///
/// When `cli.fix` is set, safe idempotent repairs run first (creating missing
/// configured directories) and their outcomes are reported alongside the
/// checks, which then reflect the repaired state.
///
/// Returns the process exit code that should be used (`0`/`1`/`2`).
pub fn run_with_format(cli: &Cli, config: Option<&Config>, format: Format) -> i32 {
    let mut checks: Vec<Check> = Vec::new();

    // Repairs run before the checks so the subsequent diagnostics observe the
    // healed state (e.g. a recreated recordings directory now reads as Pass).
    if cli.fix {
        checks.extend(fix::repair(cli, config));
    }

    checks.extend(environment::check_runtime_environment());
    checks.push(config::check_config_file(cli, config));
    if let Some(cfg) = config {
        checks.extend(config::check_config_values(cfg));
    }
    checks.push(config::check_station_location(cli, config));
    checks.push(config::check_listen_address(cli));
    checks.push(config::check_admin_exposure(cli, config));
    checks.extend(clock::check_clock(cli, config));
    checks.extend(database::check_database(cli, config));
    checks.extend(paths::check_paths(cli, config));
    checks.extend(audio::check_audio_source(cli, config));
    checks.extend(model::check_model(cli, config));
    checks.extend(analytics::check_analytics(cli, config));
    checks.extend(environment::check_egress(cli));
    checks.extend(environment::check_optional_tools(cli, config));
    checks.extend(disk::check_disk_space(cli, config));
    checks.push(watchdog::check_systemd_watchdog());

    let exit_code = render::summarise(&checks);
    match format {
        Format::Text => print!("{}", render::render_text(&checks)),
        Format::Json => println!("{}", render::render_json(&checks, exit_code)),
    }
    exit_code
}

// ── Shared helpers ───────────────────────────────────────────────────────────
//
// Used by more than one check submodule, so they live here in the facade
// rather than in any single check module.

/// Probe whether `path` is writable by trying to create (and delete) a file.
fn writable(path: &Path) -> bool {
    let probe = path.join(".birdnet-doctor-write-probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Whether an executable named `name` is on `PATH`.
///
/// Delegates to `birdnet_core::audio::capture::is_tool_available` rather than
/// keeping its own `PATH` walk. This used to be a second implementation, and a
/// second implementation of "can we run X?" is a second answer: the doctor
/// checked `is_file()` on `PATH` while capture forked `which`, so the doctor
/// could report `arecord` present on a host where `CaptureManager::start`
/// refused with `arecord not found in PATH`. One question, one answer.
fn tool_exists(name: &str) -> bool {
    birdnet_core::audio::capture::is_tool_available(name)
}

#[cfg(test)]
mod tests {
    use super::{Check, Status, tool_exists, writable};

    #[test]
    fn status_ordering() {
        assert!(Status::Pass < Status::Skip);
        assert!(Status::Skip < Status::Warn);
        assert!(Status::Warn < Status::Fail);
    }

    #[test]
    fn check_format_includes_status_tag() {
        let c = Check::warn("X", "m", "fix me");
        let s = format!("{c}");
        assert!(s.contains("[ WARN ]"));
        assert!(s.contains('X'));
        assert!(s.contains('m'));
        assert!(s.contains("fix me"));
    }

    #[test]
    fn tool_exists_finds_basic_unix_binaries() {
        if cfg!(unix) {
            assert!(tool_exists("ls"), "ls should exist on a POSIX system");
        }
        assert!(!tool_exists("definitely-not-a-real-binary-name-93kfh"));
    }

    #[test]
    fn writable_detects_writable_tempdir() {
        let tmp = std::env::temp_dir();
        assert!(writable(&tmp));
    }

    #[test]
    fn writable_false_for_nonexistent_dir() {
        let missing = std::path::Path::new("/nonexistent-bnb-doctor-dir-x9k/sub");
        assert!(!writable(missing));
    }

    #[test]
    fn run_with_format_runs_every_check_and_returns_valid_exit_code() {
        use crate::cli::Cli;
        use clap::Parser as _;
        // A bare CLI with no config: every check still runs without panicking
        // and the command returns a summary exit code in {0,1,2}. Nothing is
        // configured, so the verdict is at least a warning.
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let json_code = super::run_with_format(&cli, None, super::Format::Json);
        let text_code = super::run_with_format(&cli, None, super::Format::Text);
        assert!((1..=2).contains(&json_code), "got {json_code}");
        assert_eq!(
            json_code, text_code,
            "output format must not change the verdict"
        );
    }
}
