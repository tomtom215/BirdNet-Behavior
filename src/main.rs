//! BirdNet-Behavior: Real-time acoustic bird classification with behavioral analytics.
//!
//! Single binary entry point that starts all subsystems:
//! - Detection pipeline (audio capture → ML inference → reporting)
//! - Web server (REST API, WebSocket, HTMX, admin panel)
//! - Database management (`SQLite` operational + `DuckDB` analytics)
//! - External integrations (`BirdWeather`, notifications)
//! - BirdNET-Pi migration tooling

mod app;
mod capture;
mod cli;
mod daemon;
mod doctor;
mod helpers;
mod integrations;
mod maintenance;
mod sd_notify;
mod weekly_report;

use clap::Parser;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, reload};

use cli::Cli;
use helpers::{run_backup, run_integrity_check};

/// Default log filter when `RUST_LOG` is not set.
const DEFAULT_LOG_FILTER: &str = "info,birdnet_behavior=debug";

/// The mutually-exclusive top-level action the binary takes, decided from
/// the parsed CLI before any subsystem is built.
///
/// Extracted from `main` so the precedence rules — maintenance commands and
/// the doctor preflight each short-circuit the server — are unit-testable
/// without standing up tokio, a TCP listener, or a database. `main` was at
/// 0 % coverage because every decision lived inside the async orchestrator;
/// pulling the decision out into a pure function lets the contract be pinned
/// by tests (carryover item A2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// `--check-db`: run a `SQLite` integrity check and exit.
    CheckDb,
    /// `--backup-db`: take a hot backup and exit.
    BackupDb,
    /// `--refresh-extension`: reinstall the behavioral `DuckDB` extension and exit.
    RefreshExtension,
    /// `--doctor` / `--doctor-json`: print diagnostics and exit with a
    /// status-derived code. Carries the chosen render format.
    Doctor(doctor::Format),
    /// No short-circuit flag set: start the detection daemon + web server.
    RunServer,
}

/// Decide which top-level [`Action`] the parsed CLI selects.
///
/// Precedence matches the original inline `if` chain in `main`:
/// `--check-db` > `--backup-db` > doctor preflight > run the server. The
/// first matching flag wins, so e.g. `--check-db --doctor` runs the
/// integrity check and never reaches the doctor. `--fix` implies the doctor
/// (it runs safe repairs, then the diagnostic).
const fn dispatch_subcommand(cli: &Cli) -> Action {
    if cli.check_db {
        Action::CheckDb
    } else if cli.backup_db {
        Action::BackupDb
    } else if cli.refresh_extension {
        Action::RefreshExtension
    } else if cli.doctor || cli.doctor_json || cli.fix {
        // `--doctor-json` wins the format choice when both are passed so a
        // monitoring script that sets both still gets machine-readable output.
        // `--fix` alone implies the human-readable doctor.
        let format = if cli.doctor_json {
            doctor::Format::Json
        } else {
            doctor::Format::Text
        };
        Action::Doctor(format)
    } else {
        Action::RunServer
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use a reloadable filter so SIGHUP can change the log level at runtime.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);
    // Send logs to stderr (Unix convention) so stdout stays clean for
    // structured output like `--doctor-json`.
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    // Spawn SIGHUP handler for runtime log level changes.
    // Usage: set RUST_LOG env var then `kill -HUP <pid>`.
    #[cfg(unix)]
    {
        let handle = reload_handle;
        tokio::spawn(async move {
            let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                .expect("failed to install SIGHUP handler");
            loop {
                sighup.recv().await;
                let new_filter = EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
                match handle.reload(new_filter) {
                    Ok(()) => tracing::info!("log filter reloaded via SIGHUP"),
                    Err(e) => tracing::error!(error = %e, "failed to reload log filter"),
                }
            }
        });
    }

    let cli = Cli::parse();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %cli.config.display(),
        "starting BirdNet-Behavior"
    );

    // Load configuration (optional — may not exist on fresh installs).
    let config = match birdnet_core::config::Config::load_from(&cli.config) {
        Ok(c) => {
            tracing::info!(model = c.get_or("MODEL", "unknown"), "configuration loaded");
            Some(c)
        }
        Err(e) => {
            tracing::warn!(error = %e, "config not loaded, using defaults");
            None
        }
    };

    // Maintenance commands and the doctor preflight each run and exit before
    // any subsystem is constructed. The decision is a pure function so its
    // precedence is unit-tested; `main` only carries out the chosen action,
    // delegating the server orchestration to `app::run`.
    match dispatch_subcommand(&cli) {
        Action::CheckDb => return run_integrity_check(config.as_ref()),
        Action::BackupDb => return run_backup(config.as_ref()),
        Action::RefreshExtension => return helpers::run_refresh_extension(&cli, config.as_ref()),
        Action::Doctor(format) => {
            let code = doctor::run_with_format(&cli, config.as_ref(), format);
            std::process::exit(code);
        }
        Action::RunServer => app::run(cli, config).await,
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure top-level dispatch.
    //!
    //! `main` itself binds a TCP listener and runs the axum server, so it
    //! can't be exercised in a unit test. Pulling the subcommand precedence
    //! into [`dispatch_subcommand`] lets the contract — which flag wins, and
    //! how `--doctor` / `--doctor-json` choose a render format — be pinned
    //! here instead of being implicitly trusted inside the orchestrator.

    use super::{Action, dispatch_subcommand};
    use crate::cli::Cli;
    use crate::doctor;
    use clap::Parser;

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["birdnet-behavior"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    #[test]
    fn no_flags_runs_the_server() {
        assert_eq!(dispatch_subcommand(&cli(&[])), Action::RunServer);
    }

    #[test]
    fn check_db_flag_selects_check_db() {
        assert_eq!(dispatch_subcommand(&cli(&["--check-db"])), Action::CheckDb);
    }

    #[test]
    fn backup_db_flag_selects_backup_db() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--backup-db"])),
            Action::BackupDb
        );
    }

    #[test]
    fn refresh_extension_flag_selects_refresh_extension() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--refresh-extension"])),
            Action::RefreshExtension
        );
    }

    #[test]
    fn backup_db_takes_precedence_over_refresh_extension() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--backup-db", "--refresh-extension"])),
            Action::BackupDb
        );
    }

    #[test]
    fn refresh_extension_takes_precedence_over_doctor() {
        // `--refresh-extension` is a run-and-exit maintenance command, so it
        // short-circuits before the doctor preflight just like the DB commands.
        assert_eq!(
            dispatch_subcommand(&cli(&["--refresh-extension", "--doctor"])),
            Action::RefreshExtension
        );
    }

    #[test]
    fn doctor_flag_selects_text_format() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--doctor"])),
            Action::Doctor(doctor::Format::Text)
        );
    }

    #[test]
    fn fix_flag_implies_doctor_text() {
        // `--fix` runs repairs then the diagnostic, so on its own it selects the
        // human-readable doctor rather than falling through to the server.
        assert_eq!(
            dispatch_subcommand(&cli(&["--fix"])),
            Action::Doctor(doctor::Format::Text)
        );
    }

    #[test]
    fn fix_with_doctor_json_keeps_json_format() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--fix", "--doctor-json"])),
            Action::Doctor(doctor::Format::Json)
        );
    }

    #[test]
    fn preflight_alias_selects_doctor_text() {
        // `--preflight` is a visible alias for `--doctor`; it must take the
        // same path.
        assert_eq!(
            dispatch_subcommand(&cli(&["--preflight"])),
            Action::Doctor(doctor::Format::Text)
        );
    }

    #[test]
    fn doctor_json_flag_selects_json_format() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--doctor-json"])),
            Action::Doctor(doctor::Format::Json)
        );
    }

    #[test]
    fn doctor_json_wins_format_when_both_doctor_flags_set() {
        // Both `--doctor` and `--doctor-json`: JSON wins so a script that
        // passes both still gets machine-readable output.
        assert_eq!(
            dispatch_subcommand(&cli(&["--doctor", "--doctor-json"])),
            Action::Doctor(doctor::Format::Json)
        );
    }

    #[test]
    fn check_db_takes_precedence_over_backup_db() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--check-db", "--backup-db"])),
            Action::CheckDb
        );
    }

    #[test]
    fn check_db_takes_precedence_over_doctor() {
        // `--check-db` short-circuits before the doctor preflight is even
        // considered: the integrity check runs and the process exits.
        assert_eq!(
            dispatch_subcommand(&cli(&["--check-db", "--doctor"])),
            Action::CheckDb
        );
    }

    #[test]
    fn backup_db_takes_precedence_over_doctor() {
        assert_eq!(
            dispatch_subcommand(&cli(&["--backup-db", "--doctor-json"])),
            Action::BackupDb
        );
    }

    #[test]
    fn web_only_alone_still_runs_the_server() {
        // `--web-only` is handled later inside the server path, not by the
        // subcommand dispatch — it must not divert away from RunServer.
        assert_eq!(
            dispatch_subcommand(&cli(&["--web-only"])),
            Action::RunServer
        );
    }
}
