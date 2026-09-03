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

    /// Used percentage at or above which the disk is "critical".
    ///
    /// The same number as `DiskManagerConfig::purge_threshold`'s default, and
    /// deliberately so: "critical" means the purger is now deleting
    /// recordings to stay under it, which is a thing that has happened rather
    /// than a thing that might.
    pub const CRITICAL_PERCENT: f64 = 95.0;

    /// Used percentage at or above which the disk is "getting low".
    ///
    /// Shared with the station-health disk condition, so the badge on the page
    /// and the alert in the operator's inbox change colour at the same
    /// reading. They used to agree only by coincidence.
    pub const LOW_PERCENT: f64 = 90.0;

    /// Whether the disk is critically full.
    ///
    /// # Why this reads `used_percent` rather than `available / total`
    ///
    /// It used to be `available_bytes < total_bytes / 20`, nine lines below a
    /// doc comment forbidding exactly that denominator. The two disagree
    /// whenever part of the device is invisible to this user — an ext4 root
    /// reserve (5 % by default on every Pi image), a container quota, an
    /// overlay — because there `total > used + available`.
    ///
    /// Measured on the filesystem this was written on: `df` reported 77 %
    /// used, `used_percent()` agreed at 76.6 %, and `is_critical()` returned
    /// **true**, because 8.5 GiB of available space is less than a twentieth
    /// of a 252 GiB device that only has 37 GiB reachable. `/api/v2/system/disk`
    /// therefore served HTTP 503 "critical" with a body saying 76.6 %. A
    /// monitor pointed at that endpoint pages the operator on a healthy
    /// station, which is how a channel gets muted before the real alert
    /// arrives.
    #[must_use]
    pub fn is_critical(&self) -> bool {
        self.used_percent() >= Self::CRITICAL_PERCENT
    }

    /// Whether the disk is getting low. See [`Self::is_critical`] for why the
    /// denominator is not `total_bytes`.
    #[must_use]
    pub fn is_low(&self) -> bool {
        self.used_percent() >= Self::LOW_PERCENT
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
        // This test's second assertion was `assert!(u.is_critical(), "7/252
        // available is critical")` — the same fixture, the opposite verdict.
        // It encoded the defect rather than a requirement, and it is what made
        // the defect look deliberate: a reader finding `available < total / 20`
        // would see a passing test beside it and conclude the denominator was
        // a choice.
        //
        // It is not a defensible one. The first assertion here says `df` would
        // report this filesystem as 80 % used, so the station has a fifth of
        // its reachable space free; calling that critical made
        // `/api/v2/system/disk` answer HTTP 503 with a body saying 80 %, and a
        // monitor polling it pages the operator on a healthy station. The
        // fixture is kept and the assertion inverted, so the history shows
        // which way this flipped.
        let u = DiskUsage {
            total_bytes: 252_000_000,
            used_bytes: 28_000_000,
            available_bytes: 7_000_000, // quota: most of "total" is unreachable
        };
        assert!((u.used_percent() - 80.0).abs() < 0.01);
        assert!(
            !u.is_critical(),
            "80 % used is not critical, whatever fraction of an unreachable \
             `total` the available bytes happen to be"
        );
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

#[cfg(test)]
mod one_denominator {
    //! Every disk verdict is measured against the space this user can reach.
    //!
    //! The three surfaces that grade a disk — the JSON endpoint's HTTP status,
    //! the Station Health badge, and the operator alert — used to disagree,
    //! because `used_percent()` divides by `used + available` while
    //! `is_critical`/`is_low` divided by `total`. Those are the same number
    //! only on a filesystem with nothing held back, which no ext4 default and
    //! no container quota is.

    use super::DiskUsage;

    /// The filesystem this was written on: `df -Pk /` reported
    /// 264 212 084 total, 29 896 308 used, 8 952 216 available (1 K blocks),
    /// i.e. 77 % used with 85 % of the device unreachable.
    fn quota_filesystem() -> DiskUsage {
        DiskUsage {
            total_bytes: 264_212_084 * 1024,
            used_bytes: 29_896_308 * 1024,
            available_bytes: 8_952_216 * 1024,
        }
    }

    #[test]
    fn a_quota_filesystem_is_not_graded_against_space_it_cannot_reach() {
        // The reproduction. Against `available < total / 20` this returned
        // critical, so `/api/v2/system/disk` served HTTP 503 with a body
        // saying 76.6 % used — and a monitor pointed at it paged the operator
        // about a healthy station.
        let d = quota_filesystem();
        let pct = d.used_percent();
        assert!(
            (76.0..77.0).contains(&pct),
            "df said 77 %; used_percent says {pct:.1}"
        );
        assert!(
            !d.is_critical(),
            "a disk 76.6 % full is not critical (was: {} available < {} = total/20)",
            d.available_bytes,
            d.total_bytes / 20
        );
        assert!(!d.is_low(), "nor low");
    }

    #[test]
    fn a_genuinely_full_disk_is_still_critical() {
        // The counterpart that stops the fix being "return false always".
        let d = DiskUsage {
            total_bytes: 100,
            used_bytes: 96,
            available_bytes: 4,
        };
        assert!(d.is_critical(), "{:.1} %", d.used_percent());
        assert!(d.is_low());
    }

    #[test]
    fn a_full_disk_behind_a_root_reserve_is_still_critical() {
        // The shape that matters on a Pi: ext4 holds back 5 %, so `total`
        // exceeds `used + available` on a card that really is out of room.
        // Both denominators call this critical, which is why it cannot stand
        // in for the reproduction above — it is here to prove the fix did not
        // simply stop reporting.
        let d = DiskUsage {
            total_bytes: 100,
            used_bytes: 94,
            available_bytes: 1,
        };
        assert!(d.total_bytes > d.used_bytes + d.available_bytes, "reserved");
        assert!(d.is_critical(), "{:.1} %", d.used_percent());
    }

    #[test]
    fn every_verdict_agrees_with_the_percentage_it_is_shown_beside() {
        // The property, swept rather than sampled: whatever the shape of the
        // filesystem — reserve, quota, neither — a verdict and the number
        // rendered next to it must never contradict each other. This is the
        // gate the three surfaces failed.
        for total in [100_u64, 1_000, 264_212_084] {
            for used in (0..=total).step_by((total / 37).max(1) as usize) {
                for hidden_pct in [0_u64, 5, 50, 85] {
                    let hidden = total * hidden_pct / 100;
                    let Some(available) = total.checked_sub(used + hidden) else {
                        continue;
                    };
                    let d = DiskUsage {
                        total_bytes: total,
                        used_bytes: used,
                        available_bytes: available,
                    };
                    let pct = d.used_percent();
                    assert_eq!(
                        d.is_critical(),
                        pct >= DiskUsage::CRITICAL_PERCENT,
                        "critical disagrees with {pct:.2} % for {d:?}"
                    );
                    assert_eq!(
                        d.is_low(),
                        pct >= DiskUsage::LOW_PERCENT,
                        "low disagrees with {pct:.2} % for {d:?}"
                    );
                }
            }
        }
    }

    /// The warning has to arrive before the deleting starts.
    const _: () = assert!(DiskUsage::LOW_PERCENT < DiskUsage::CRITICAL_PERCENT);

    #[test]
    fn critical_means_the_purger_is_already_running() {
        // "Critical" is not a spare adjective: it is the reading at which
        // `DiskManagerConfig`'s default threshold starts deleting recordings.
        // If these drift apart, the badge says critical while nothing is being
        // purged, or recordings vanish while the page is still amber.
        //
        // Exact equality is right here: both sides are literal constants, not
        // the result of any arithmetic.
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(
                DiskUsage::CRITICAL_PERCENT,
                f64::from(super::DiskManagerConfig::default().purge_threshold),
            );
        }
    }

    #[test]
    fn an_empty_reading_is_not_a_full_disk() {
        // `df` on a pseudo-filesystem can report zeroes. Dividing by
        // `used + available` there must not produce a verdict at all.
        let d = DiskUsage {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
        };
        assert!(d.used_percent().abs() < f64::EPSILON);
        assert!(!d.is_critical());
        assert!(!d.is_low());
    }
}
