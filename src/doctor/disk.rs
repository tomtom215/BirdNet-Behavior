//! Disk-space check (best-effort, via `df`).

use std::path::{Path, PathBuf};
use std::process::Command;

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;

pub(super) fn check_disk_space(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    // Best-effort disk space check: use the recordings dir if known, else /.
    let dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));
    disk_free_bytes(&dir).map_or_else(
        || {
            vec![Check::skip(
                "Disk space",
                "could not query filesystem usage",
            )]
        },
        |bytes| {
            let gib = bytes / (1024 * 1024 * 1024);
            if gib >= 5 {
                vec![Check::pass(
                    "Disk space",
                    format!("{gib} GiB free on the volume containing {}", dir.display()),
                )]
            } else if gib >= 1 {
                vec![Check::warn(
                    "Disk space",
                    format!("only {gib} GiB free on {}", dir.display()),
                    "recordings will accumulate quickly; \
                     consider --max-files-per-species or external storage",
                )]
            } else {
                vec![Check::fail(
                    "Disk space",
                    format!("less than 1 GiB free on {}", dir.display()),
                    "free up space immediately — the disk manager may not be able to keep up",
                )]
            }
        },
    )
}

/// Best-effort free-bytes query that shells out to `df` so we don't have to
/// pull a libc crate or write unsafe FFI. Returns `None` if `df` is missing
/// or its output cannot be parsed.
fn disk_free_bytes(path: &Path) -> Option<u64> {
    let out = Command::new("df")
        .args(["-Pk", "--"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_df_available_kib(&text).map(|kib| kib * 1024)
}

fn parse_df_available_kib(df_output: &str) -> Option<u64> {
    // POSIX `df -Pk` prints exactly two lines: a header and one data row.
    // Columns: Filesystem  1024-blocks  Used  Available  Capacity  Mounted on
    // We want the 4th column of the data row. Handle wrapped lines defensively.
    let data = df_output.lines().nth(1)?;
    data.split_whitespace().nth(3)?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::parse_df_available_kib;

    #[test]
    fn parse_df_available_kib_reads_fourth_column() {
        let df = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                  /dev/sda1  104857600  41943040 62914560 40% /";
        assert_eq!(parse_df_available_kib(df), Some(62_914_560));
    }

    #[test]
    fn parse_df_available_kib_none_without_data_row() {
        assert_eq!(parse_df_available_kib("only a header line"), None);
    }
}
