//! Path checks: recordings directory and image-cache directory.

use std::path::PathBuf;

use birdnet_core::config::Config;

use super::{Check, writable};
use crate::cli::Cli;

pub(super) fn check_paths(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let mut out = Vec::new();

    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from));
    if let Some(dir) = watch_dir {
        if !dir.exists() {
            out.push(Check::warn(
                "Recordings directory",
                format!("{} does not exist", dir.display()),
                "create it with `mkdir -p` or let the daemon create it on first capture",
            ));
        } else if !writable(&dir) {
            out.push(Check::fail(
                "Recordings directory",
                format!("{} is not writable", dir.display()),
                "fix ownership/permissions on this directory",
            ));
        } else {
            out.push(Check::pass(
                "Recordings directory",
                format!("{} is writable", dir.display()),
            ));
        }
    } else {
        out.push(Check::skip(
            "Recordings directory",
            "no --watch-dir or RECS_DIR configured (file-watcher mode disabled)",
        ));
    }

    if let Some(image_dir) = cli
        .image_cache_dir
        .clone()
        .or_else(|| config?.get("IMAGE_CACHE_DIR").map(PathBuf::from))
    {
        if image_dir.exists() && !writable(&image_dir) {
            out.push(Check::warn(
                "Image cache directory",
                format!(
                    "{} is not writable — species images will not be cached",
                    image_dir.display()
                ),
                "fix ownership/permissions on this directory",
            ));
        } else {
            out.push(Check::pass(
                "Image cache directory",
                format!("{} is OK", image_dir.display()),
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn skip_when_no_watch_dir_configured() {
        let checks = check_paths(&cli(), None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Skip);
        assert!(checks[0].name.contains("Recordings"));
    }

    #[test]
    fn pass_for_writable_watch_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.watch_dir = Some(dir.path().to_path_buf());
        let checks = check_paths(&cli, None);
        assert_eq!(checks[0].status, Status::Pass);
        assert!(checks[0].message.contains("writable"));
    }

    #[test]
    fn warn_for_missing_watch_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.watch_dir = Some(dir.path().join("does-not-exist"));
        let checks = check_paths(&cli, None);
        assert_eq!(checks[0].status, Status::Warn);
        assert!(checks[0].message.contains("does not exist"));
    }

    #[test]
    fn includes_image_cache_when_set() {
        let watch = tempfile::tempdir().unwrap();
        let image = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.watch_dir = Some(watch.path().to_path_buf());
        cli.image_cache_dir = Some(image.path().to_path_buf());
        let checks = check_paths(&cli, None);
        assert_eq!(checks.len(), 2);
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Image cache") && c.status == Status::Pass)
        );
    }

    #[test]
    fn reads_recs_dir_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::parse(&format!("RECS_DIR={}", dir.path().display())).unwrap();
        let checks = check_paths(&cli(), Some(&cfg));
        assert_eq!(checks[0].status, Status::Pass);
    }
}
