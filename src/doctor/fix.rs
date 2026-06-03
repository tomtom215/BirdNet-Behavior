//! Safe automatic repairs for `--fix`.
//!
//! These are idempotent and never destructive: they create missing configured
//! directories so the daemon can record and cache. A missing recordings/watch
//! directory is the single most common cause of "the service runs but nothing
//! is detected" — for example a watch directory that lived on tmpfs and vanished
//! on reboot (the v0.5.x field-hardening releases fixed this reactively in the
//! systemd unit; `--fix` lets an operator recover any install on demand).
//!
//! Anything that needs root — directory *ownership*, system packages — is left
//! to the installer and only *reported* by the regular checks, never changed
//! here, so `--fix` is safe to run as the unprivileged service user.

use std::path::{Path, PathBuf};

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;

/// Run every safe repair, returning one [`Check`] per action (created / already
/// present / failed). Resolution mirrors `paths::check_paths` so a repair lines
/// up exactly with the directory the diagnostic checks afterwards.
pub(super) fn repair(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let mut out = Vec::new();

    // Recordings / watch directory — without it the file-watcher silently
    // processes nothing.
    if let Some(dir) = cli
        .watch_dir
        .clone()
        .or_else(|| config.and_then(|c| c.get("RECS_DIR")).map(PathBuf::from))
    {
        out.push(ensure_dir("recordings directory", &dir));
    }

    // Image-cache directory — without it species photos cannot be cached.
    if let Some(dir) = cli.image_cache_dir.clone().or_else(|| {
        config
            .and_then(|c| c.get("IMAGE_CACHE_DIR"))
            .map(PathBuf::from)
    }) {
        out.push(ensure_dir("image cache directory", &dir));
    }

    if out.is_empty() {
        out.push(Check::skip(
            "Repair",
            "no recordings or image-cache directory is configured, so there is nothing to repair",
        ));
    }
    out
}

/// Create `dir` (and any missing parents) if absent; report the outcome.
fn ensure_dir(label: &str, dir: &Path) -> Check {
    let name = format!("Repair: {label}");
    if dir.is_dir() {
        return Check::pass(name, format!("{} already exists", dir.display()));
    }
    match std::fs::create_dir_all(dir) {
        Ok(()) => Check::pass(name, format!("created {}", dir.display())),
        Err(e) => Check::fail(
            name,
            format!("could not create {}: {e}", dir.display()),
            "create it manually, or fix the permissions on its parent directory",
        ),
    }
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
    fn ensure_dir_creates_missing_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested/recordings");
        assert!(!target.exists());

        let check = ensure_dir("recordings directory", &target);
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("created"));
        assert!(target.is_dir(), "the directory must exist after the repair");
    }

    #[test]
    fn ensure_dir_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        // Second call on an already-present directory is a clean no-op pass.
        let check = ensure_dir("recordings directory", tmp.path());
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("already exists"));
    }

    #[test]
    fn repair_creates_the_configured_watch_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let watch = tmp.path().join("watch");
        let mut cli = cli();
        cli.watch_dir = Some(watch.clone());

        let checks = repair(&cli, None);
        assert!(
            checks
                .iter()
                .any(|c| c.status == Status::Pass && c.name.contains("recordings")),
            "watch directory should be repaired: {checks:?}"
        );
        assert!(watch.is_dir());
    }

    #[test]
    fn repair_skips_when_nothing_configured() {
        let checks = repair(&cli(), None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Skip);
    }
}
