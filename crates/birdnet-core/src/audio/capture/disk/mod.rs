//! Disk space monitoring and recording cleanup.
//!
//! Replaces `disk_check.sh` from BirdNET-Pi.  Uses the `df` command to
//! query filesystem statistics without `unsafe` code or `libc` bindings.

mod purge;

pub mod manager;

use std::path::Path;
use std::process::{Command, Stdio};

use super::process::is_audio_file;
use super::types::CaptureError;

// Re-export public API from sub-modules.
pub use manager::{DiskManager, DiskManagerConfig, FullDiskAction, LockedFilesProvider};

/// Disk space information for a filesystem.
#[derive(Debug, Clone, Copy)]
pub struct DiskUsage {
    /// Total space in bytes.
    pub total_bytes: u64,
    /// Used space in bytes.
    pub used_bytes: u64,
    /// Available space in bytes.
    pub available_bytes: u64,
}

impl DiskUsage {
    /// Percentage of disk used (0.0 -- 100.0).
    ///
    /// Computed as `used / (used + available)` — the same definition `df`'s
    /// `Use%` column reports — NOT `used / total`. The two diverge whenever
    /// part of the device is invisible to this user (ext4 root-reserved
    /// blocks, container quotas): there `used / total` understates fullness
    /// and contradicts the "running low" wording shown beside it.
    #[allow(clippy::cast_precision_loss)]
    pub fn used_percent(&self) -> f64 {
        let reachable = self.used_bytes.saturating_add(self.available_bytes);
        if reachable == 0 {
            return 0.0;
        }
        self.used_bytes as f64 / reachable as f64 * 100.0
    }

    /// Whether the disk is critically low (< 5 % available).
    pub const fn is_critical(&self) -> bool {
        self.available_bytes < self.total_bytes / 20
    }

    /// Whether the disk is getting low (< 10 % available).
    pub const fn is_low(&self) -> bool {
        self.available_bytes < self.total_bytes / 10
    }
}

/// Arguments passed to `df`, kept in one place so a gate can assert they stay
/// POSIX.
///
/// `-P` selects the portable output format and `-k` fixes the block size at
/// 1024 bytes; both are in POSIX. `--` stops option parsing so a path beginning
/// with `-` is still a path. Nothing here may become a GNU extension again:
/// `--output=` and `-B1` exist in neither BSD `df` nor `BusyBox`, and their
/// absence failed silently.
const DF_ARGS: [&str; 2] = ["-Pk", "--"];

/// Parse the data row of `df -Pk` output into a [`DiskUsage`].
///
/// POSIX fixes this format exactly: a header line, then one row per operand of
///
/// ```text
/// Filesystem  1024-blocks  Used  Available  Capacity  Mounted on
/// ```
///
/// Columns 2, 3 and 4 are the ones wanted, in 1024-byte blocks. Parsing by
/// *position* rather than by "the first three things that look like numbers"
/// matters: a device name such as `/dev/mmcblk0p2` contains no spaces, but a
/// mount point can (`/media/My Disk`), and a filesystem field long enough to
/// push the row onto a second line is why the header is skipped by index rather
/// than by matching.
///
/// Pure, so every shape below is testable without a filesystem that has them.
fn parse_df_pk(stdout: &str) -> Option<DiskUsage> {
    let row = stdout.lines().nth(1)?;
    let mut cols = row.split_whitespace().skip(1);
    let total_k: u64 = cols.next()?.parse().ok()?;
    let used_k: u64 = cols.next()?.parse().ok()?;
    let avail_k: u64 = cols.next()?.parse().ok()?;
    Some(DiskUsage {
        total_bytes: total_k.saturating_mul(1024),
        used_bytes: used_k.saturating_mul(1024),
        available_bytes: avail_k.saturating_mul(1024),
    })
}

/// Get disk usage information for the filesystem containing `path`.
///
/// Shells out to `df` rather than calling `statvfs`, because this workspace
/// sets `unsafe_code = "forbid"` and every safe wrapper for it is an FFI crate.
///
/// `-Pk` and nothing else: both flags are POSIX, and the previous
/// `--output=size,used,avail -B1` were GNU coreutils extensions that exist
/// neither in BSD `df` (so every macOS station — a documented target — failed
/// this check) nor in `BusyBox`. The failure was quiet, too: the disk manager
/// treats an error as "cannot tell", so a station whose card was filling up
/// simply never purged.
///
/// # Errors
///
/// Returns `CaptureError` if `df` is not available, `path` doesn't exist, or
/// the output is not the format POSIX specifies.
pub fn disk_usage(path: &Path) -> Result<DiskUsage, CaptureError> {
    let output = Command::new("df")
        .args(DF_ARGS)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(CaptureError::Spawn)?;

    if !output.status.success() {
        return Err(CaptureError::Config(format!(
            "df failed for {}",
            path.display()
        )));
    }

    parse_df_pk(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| CaptureError::Config("unexpected df output format".into()))
}

/// Count audio files in a directory and their total size.
///
/// Returns `(file_count, total_size_bytes)`.
///
/// # Errors
///
/// Returns `CaptureError` if the directory cannot be read.
pub fn recording_stats(dir: &Path) -> Result<(u32, u64), CaptureError> {
    let entries = std::fs::read_dir(dir).map_err(|e| CaptureError::Config(e.to_string()))?;

    let mut count = 0_u32;
    let mut total_size = 0_u64;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_audio_file(&path) {
            count += 1;
            total_size += entry.metadata().map_or(0, |m| m.len());
        }
    }

    Ok((count, total_size))
}

/// Remove audio files older than `max_age_days` days from `dir`.
///
/// Returns the number of files removed.
///
/// # Errors
///
/// Returns `CaptureError` if the directory cannot be read.
pub fn cleanup_old_recordings(dir: &Path, max_age_days: u32) -> Result<u32, CaptureError> {
    let now = std::time::SystemTime::now();
    let max_age = std::time::Duration::from_secs(u64::from(max_age_days) * 86_400);
    let mut removed = 0_u32;

    let entries = std::fs::read_dir(dir).map_err(|e| CaptureError::Config(e.to_string()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_audio_file(&path) {
            continue;
        }

        let dominated = entry.metadata().ok().and_then(|m| {
            let modified = m.modified().ok()?;
            let age = now.duration_since(modified).ok()?;
            Some(age > max_age)
        });

        if dominated == Some(true) && std::fs::remove_file(&path).is_ok() {
            tracing::debug!(path = %path.display(), "removed old recording");
            removed += 1;
        }
    }

    if removed > 0 {
        tracing::info!(count = removed, "cleaned up old recordings");
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_usage_percent() {
        let u = DiskUsage {
            total_bytes: 1_000_000,
            used_bytes: 750_000,
            available_bytes: 250_000,
        };
        assert!((u.used_percent() - 75.0).abs() < 0.01);
        assert!(!u.is_critical());
        assert!(!u.is_low());
    }

    /// When reserved blocks / quotas hide space (`used + avail < total`),
    /// the percentage must follow `df`'s definition — used over what this
    /// user can reach — or the UI shows "11% used" beside "critically low".
    #[test]
    fn disk_usage_percent_with_reserved_space() {
        let u = DiskUsage {
            total_bytes: 252_000_000,
            used_bytes: 28_000_000,
            available_bytes: 7_000_000, // quota: most of "total" is unreachable
        };
        assert!((u.used_percent() - 80.0).abs() < 0.01);
        assert!(u.is_critical(), "7/252 available is critical");
    }

    /// Degenerate zero-sized readings must not divide by zero.
    #[test]
    fn disk_usage_percent_zero_is_zero() {
        let u = DiskUsage {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
        };
        assert!((u.used_percent() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn disk_usage_critical() {
        let u = DiskUsage {
            total_bytes: 1_000_000,
            used_bytes: 960_000,
            available_bytes: 40_000,
        };
        assert!(u.is_critical());
        assert!(u.is_low());
    }

    #[test]
    fn disk_usage_low() {
        let u = DiskUsage {
            total_bytes: 1_000_000,
            used_bytes: 920_000,
            available_bytes: 80_000,
        };
        assert!(!u.is_critical());
        assert!(u.is_low());
    }

    #[test]
    fn disk_usage_zero_total() {
        let u = DiskUsage {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
        };
        assert!((u.used_percent()).abs() < 0.01);
    }

    /// The flags must stay POSIX.
    ///
    /// This is the half the parser tests cannot cover: `--output=size,used,avail`
    /// and `-B1` parse nothing wrongly, they simply make `df` exit non-zero on
    /// BSD and `BusyBox`, and `disk_usage` then returns an error that the disk
    /// manager reads as "cannot tell" and skips the purge on. A station whose
    /// card was filling up never reclaimed anything, and nothing said so.
    #[test]
    fn df_is_invoked_with_posix_flags_only() {
        assert_eq!(DF_ARGS, ["-Pk", "--"]);
        for arg in DF_ARGS {
            assert!(
                !arg.starts_with("--output") && !arg.starts_with("-B"),
                "{arg} is a GNU coreutils extension; BSD and BusyBox `df` reject it"
            );
        }
    }

    /// The parser must read real `df -Pk` output from every platform this
    /// project claims to run on.
    ///
    /// The previous implementation passed `--output=size,used,avail -B1`, which
    /// are GNU coreutils extensions. BSD `df` (macOS — a documented target) and
    /// `BusyBox` `df` (Alpine) reject them, so `disk_usage` errored, and the disk
    /// manager reads an error as "cannot tell" and skips the purge: a station
    /// whose card was filling up simply never reclaimed anything, silently.
    ///
    /// Fixtures are the literal output of `df -Pk` on each platform.
    #[test]
    fn parses_df_pk_from_gnu_bsd_and_busybox() {
        // GNU coreutils (Debian / Raspberry Pi OS).
        let gnu = "Filesystem     1024-blocks     Used Available Capacity Mounted on\n                   /dev/mmcblk0p2    61312256 12058624  46123008      21% /\n";
        let u = parse_df_pk(gnu).expect("gnu");
        assert_eq!(u.total_bytes, 61_312_256 * 1024);
        assert_eq!(u.used_bytes, 12_058_624 * 1024);
        assert_eq!(u.available_bytes, 46_123_008 * 1024);

        // BSD / macOS: same columns, different spacing and device naming.
        let bsd = "Filesystem 1024-blocks      Used Available Capacity  Mounted on\n                   /dev/disk3s1s1  971350180  22371592 546820264    4%    /\n";
        let u = parse_df_pk(bsd).expect("bsd");
        assert_eq!(u.total_bytes, 971_350_180 * 1024);
        assert_eq!(u.available_bytes, 546_820_264 * 1024);

        // `BusyBox` (Alpine).
        let busybox = "Filesystem           1024-blocks      Used Available Capacity Mounted on\n                       overlay                 61312256  12058624  46123008  21% /\n";
        let u = parse_df_pk(busybox).expect("busybox");
        assert_eq!(u.used_bytes, 12_058_624 * 1024);
    }

    /// Columns are read by position, so a mount point containing spaces — which
    /// is the last field — cannot shift the numbers.
    #[test]
    fn a_mount_point_with_spaces_does_not_shift_the_columns() {
        let df = "Filesystem     1024-blocks     Used Available Capacity Mounted on\n                  /dev/sdb1          1000000   400000    600000      40% /media/My Field Disk\n";
        let u = parse_df_pk(df).expect("spaces");
        assert_eq!(u.total_bytes, 1_000_000 * 1024);
        assert_eq!(u.used_bytes, 400_000 * 1024);
        assert_eq!(u.available_bytes, 600_000 * 1024);
        assert!((u.used_percent() - 40.0).abs() < 0.001);
    }

    /// Anything that is not the POSIX shape is an error, not a guess. Reading
    /// three arbitrary numbers out of a malformed row would produce a
    /// confident, wrong fullness figure and either purge nothing or purge
    /// everything.
    #[test]
    fn malformed_df_output_is_rejected() {
        assert!(parse_df_pk("").is_none());
        assert!(parse_df_pk("only a header line\n").is_none());
        assert!(parse_df_pk("Header\n/dev/sda1 100 200\n").is_none());
        assert!(parse_df_pk("Header\n/dev/sda1 not a number here\n").is_none());
    }

    #[test]
    fn disk_usage_from_df() {
        let result = disk_usage(Path::new("/tmp"));
        assert!(result.is_ok());
        let u = result.unwrap();
        assert!(u.total_bytes > 0);
        assert!(u.available_bytes <= u.total_bytes);
    }

    #[test]
    fn recording_stats_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (count, size) = recording_stats(dir.path()).unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);
    }

    #[test]
    fn cleanup_nonexistent_dir_returns_error() {
        assert!(cleanup_old_recordings(Path::new("/nonexistent/dir"), 30).is_err());
    }

    #[test]
    fn cleanup_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let removed = cleanup_old_recordings(dir.path(), 30).unwrap();
        assert_eq!(removed, 0);
    }
}
