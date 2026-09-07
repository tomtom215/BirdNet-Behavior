//! The binary's diagnostics, handed to the web layer (`OP-1`).
//!
//! `--doctor` and `--support-bundle` were written, tested, and reachable only
//! by someone who could already SSH in. Both are pure functions of the CLI and
//! the loaded configuration, so exposing them to a browser is two closures
//! over clones of those — nothing about the checks changes.
//!
//! One rule is enforced here rather than trusted: **a `GET` never repairs.**
//! `--fix` runs `doctor::fix::repair` before the checks, which creates
//! directories and rewrites files; a process started with `--fix` must not
//! re-run that on every page view. [`read_only`] strips it, and the test below
//! holds that the hook built from a `--fix` command line still runs no repair.

use std::path::Path;

use birdnet_core::config::Config;
use birdnet_web::diagnostics::Diagnostics;

use crate::cli::Cli;

/// The command line as the web diagnostics may see it: the same station,
/// with every repairing flag cleared.
fn read_only(cli: &Cli) -> Cli {
    let mut cli = cli.clone();
    cli.fix = false;
    cli
}

/// Build the hooks the web layer calls for `/admin/doctor.json` and
/// `/admin/support-bundle`.
#[must_use]
pub fn hooks(cli: &Cli, config: Option<&Config>) -> Diagnostics {
    let cli = read_only(cli);
    let config = config.cloned();
    let (cli_for_bundle, config_for_bundle) = (cli.clone(), config.clone());
    Diagnostics::new(
        move || crate::doctor::collect_json(&cli, config.as_ref()),
        move |dest: &Path| {
            let bundle = crate::support::build(&cli_for_bundle, config_for_bundle.as_ref(), dest)?;
            for w in bundle.warnings {
                tracing::warn!(member = %w, "support bundle member could not be staged");
            }
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    /// A `GET` on a station started with `--fix` must not repair anything.
    ///
    /// Observed failing with `read_only` replaced by `cli.clone()`: the JSON
    /// then opens with the repair family's checks (`"name":"Repair: …"`).
    #[test]
    fn the_web_doctor_never_repairs_even_on_a_fix_command_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist-yet");
        let cli = Cli::parse_from([
            "birdnet-behavior",
            "--fix",
            "--watch-dir",
            missing.to_str().unwrap(),
        ]);
        assert!(cli.fix, "the fixture must ask for repairs");

        let json = hooks(&cli, None).doctor_json();
        assert!(
            !json.contains("\"name\":\"Repair"),
            "a GET ran the repair family: {json}"
        );
        assert!(
            !missing.exists(),
            "a GET created a directory `--fix` would have created"
        );

        // The counterpart: the same command line at the terminal *does* repair,
        // so the difference is the web seam and not a broken `--fix`.
        let json = crate::doctor::collect_json(&cli, None);
        assert!(
            json.contains("\"name\":\"Repair"),
            "the CLI with --fix must still run repairs: {json}"
        );
    }

    #[test]
    fn the_bundle_hook_writes_the_same_archive_the_cli_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let dest = dir.path().join("bundle.tar.gz");
        hooks(&cli, None)
            .write_support_bundle(&dest)
            .expect("bundle");
        let listing = std::process::Command::new("tar")
            .args(["tzf", dest.to_str().unwrap()])
            .output()
            .expect("tar");
        let names = String::from_utf8_lossy(&listing.stdout);
        for member in [
            "doctor.json",
            "doctor.txt",
            "config.redacted",
            "version.txt",
        ] {
            assert!(
                names.contains(&format!("birdnet-support/{member}")),
                "{member} missing from {names}"
            );
        }
    }
}
