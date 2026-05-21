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
