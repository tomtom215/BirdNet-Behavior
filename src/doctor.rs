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
//! The checks live in thirteen submodules — `analytics`, `audio`, `clock`,
//! `config`, `database`, `disk`, `environment`, `model`, `offsite`, `paths`,
//! `tls`, `watchdog`, plus `fix`, whose `repair` runs *before* the checks when
//! `--fix` is given — and all report rendering (`text` / `json` / exit-code)
//! lives in `render`.
//!
//! **Every check family the doctor runs is a row of [`CHECK_FAMILIES`].**
//! `collect` iterates that table and nothing else, and two gates read it back:
//! `every_check_entry_point_is_in_the_table` scans the submodules for
//! `pub(super) fn check_*` and fails on one the table does not name (or a row
//! the source no longer has), and `the_module_doc_names_every_submodule` holds
//! the list above to the `mod` declarations. Before the table, `collect` was
//! twenty-two scattered `push`/`extend` calls, and a check dropped in a
//! refactor produced no failure, no warning and no output — which is exactly
//! what a healthy station produces (RC-7, RC-8; the same hole
//! `station_health`'s `CHECKS` table closed).

use std::fmt;
use std::path::Path;

use birdnet_core::config::Config;

use crate::cli::Cli;

pub mod analytics;
mod audio;
mod clock;
mod config;
mod database;
mod disk;
mod environment;
mod fix;
mod model;
mod offsite;
mod paths;
mod render;
mod tls;
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
    let checks = collect(cli, config);
    let exit_code = render::summarise(&checks);
    match format {
        Format::Text => print!("{}", render::render_text(&checks)),
        Format::Json => println!("{}", render::render_json(&checks, exit_code)),
    }
    exit_code
}

/// The diagnostic as a JSON document, for callers that want it in a file
/// rather than on stdout — `--support-bundle` in particular.
#[must_use]
pub fn collect_json(cli: &Cli, config: Option<&Config>) -> String {
    let checks = collect(cli, config);
    let exit_code = render::summarise(&checks);
    render::render_json(&checks, exit_code)
}

/// The diagnostic as the same text report `--doctor` prints.
#[must_use]
pub fn collect_text(cli: &Cli, config: Option<&Config>) -> String {
    render::render_text(&collect(cli, config))
}

/// One check family: the `module::function` it is, and the call.
type Family = fn(&Cli, Option<&Config>) -> Vec<Check>;

/// Every check family the doctor runs, in the order the report prints them.
///
/// The name is the entry point's path in the source, and it is what the gate
/// compares against a scan of `src/doctor/*.rs` — so a `pub(super) fn
/// check_*` added without a row here, or a row whose function was renamed
/// away, turns a test red instead of silently dropping out of the report.
/// The adapters exist only to give the families one signature; none of them
/// decides anything.
const CHECK_FAMILIES: [(&str, Family); 23] = [
    ("environment::check_runtime_environment", |_, _| {
        environment::check_runtime_environment()
    }),
    ("environment::check_environment_variables", |_, _| {
        environment::check_environment_variables()
    }),
    ("config::check_config_file", |cli, cfg| {
        vec![config::check_config_file(cli, cfg)]
    }),
    ("config::check_config_values", |cli, cfg| {
        cfg.map(|c| config::check_config_values(cli, c))
            .unwrap_or_default()
    }),
    ("config::check_station_location", |cli, cfg| {
        vec![config::check_station_location(cli, cfg)]
    }),
    ("config::check_occurrence_filter", |cli, cfg| {
        vec![config::check_occurrence_filter(cli, cfg)]
    }),
    ("config::check_confirmation_filter", |cli, cfg| {
        vec![config::check_confirmation_filter(cli, cfg)]
    }),
    ("config::check_listen_address", |cli, _| {
        vec![config::check_listen_address(cli)]
    }),
    ("config::check_admin_exposure", |cli, cfg| {
        vec![config::check_admin_exposure(cli, cfg)]
    }),
    ("config::check_api_surface", |_, cfg| {
        vec![config::check_api_surface(cfg)]
    }),
    ("tls::check_tls", tls::check_tls),
    ("clock::check_clock", clock::check_clock),
    ("database::check_database", database::check_database),
    (
        "database::check_maintenance_verdicts",
        database::check_maintenance_verdicts,
    ),
    ("offsite::check_offsite", offsite::check_offsite),
    ("paths::check_paths", paths::check_paths),
    ("audio::check_audio_source", audio::check_audio_source),
    ("model::check_model", model::check_model),
    ("analytics::check_analytics", analytics::check_analytics),
    ("environment::check_egress", |cli, _| {
        environment::check_egress(cli)
    }),
    (
        "environment::check_optional_tools",
        environment::check_optional_tools,
    ),
    ("disk::check_disk_space", disk::check_disk_space),
    ("watchdog::check_systemd_watchdog", |_, _| {
        vec![watchdog::check_systemd_watchdog()]
    }),
];

/// Run every check and return the results.
///
/// Split out of [`run_with_format`] so the support bundle renders the same
/// checks the operator sees rather than a second, drifting list.
fn collect(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();

    // Repairs run before the checks so the subsequent diagnostics observe the
    // healed state (e.g. a recreated recordings directory now reads as Pass).
    // Not a row of the table: it is a repair, not a check, and it is gated on
    // `--fix`.
    if cli.fix {
        checks.extend(fix::repair(cli, config));
    }

    for (_, family) in CHECK_FAMILIES {
        checks.extend(family(cli, config));
    }
    checks
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
pub fn tool_exists(name: &str) -> bool {
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

    /// The set of check families is written down once, in `CHECK_FAMILIES`,
    /// and read back here from the source: every `pub(super) fn check_*` in
    /// `src/doctor/*.rs` must be a row, and every row must still name a
    /// function that exists. Deleting the `clock::check_clock` row was applied
    /// as a mutation and passed every other test in this crate — a check that
    /// silently stops running is what a healthy station looks like (RC-7).
    /// Observed failing against that deletion:
    /// `clock::check_clock is a check the doctor no longer runs`.
    #[test]
    fn every_check_entry_point_is_in_the_table() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/doctor");
        let mut in_source = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&dir).expect("src/doctor") {
            let path = entry.expect("entry").path();
            let Some(module) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read");
            // Column-zero signatures only: an indented `fn check_*` is a
            // private helper inside a family, and a `//` line is prose.
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("pub(super) fn check_")
                    && let Some(end) = rest.find('(')
                {
                    in_source.insert(format!("{module}::check_{}", &rest[..end]));
                }
            }
        }
        let in_table: std::collections::BTreeSet<String> = super::CHECK_FAMILIES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        for name in &in_source {
            assert!(
                in_table.contains(name),
                "{name} is a check the doctor no longer runs: add it to CHECK_FAMILIES \
                 (or make it a private helper if it is one)"
            );
        }
        for name in &in_table {
            assert!(
                in_source.contains(name),
                "CHECK_FAMILIES names {name}, which no submodule defines any more"
            );
        }
        assert_eq!(
            in_table.len(),
            super::CHECK_FAMILIES.len(),
            "two rows name the same family"
        );
        assert!(in_source.len() >= 20, "the scan found only {in_source:?}");
    }

    /// The module doc lists the submodules; the `mod` declarations are the
    /// truth. The doc knew eight of fourteen for long enough that the two it
    /// mattered most for — `clock` and `tls` — were the ones missing (RC-8).
    /// Observed failing with `clock` deleted from the list:
    /// "the module doc does not mention the `clock` submodule".
    #[test]
    fn the_module_doc_names_every_submodule() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/doctor.rs"),
        )
        .expect("read");
        let doc: String = text
            .lines()
            .take_while(|l| l.starts_with("//!") || l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let mods: Vec<&str> = text
            .lines()
            .filter_map(|l| {
                l.strip_prefix("pub mod ")
                    .or_else(|| l.strip_prefix("mod "))
                    .and_then(|r| r.strip_suffix(';'))
            })
            .collect();
        assert!(mods.len() >= 14, "found only {mods:?}");
        for m in mods {
            assert!(
                doc.contains(&format!("`{m}`")),
                "the module doc does not mention the `{m}` submodule"
            );
        }
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
