//! Disk-space check (best-effort, via `df`).

use std::path::{Path, PathBuf};

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
        |bytes| vec![grade_free_space(bytes, &dir)],
    )
}

/// Grade free space into a check.
///
/// Split from [`check_disk_space`] so every branch is unit-testable: the caller
/// shells out to `df` against the real host, whose free space decides the
/// verdict, so the existing test could only assert structure — which is exactly
/// how a hard error sat on the low-space branch unnoticed.
fn grade_free_space(bytes: u64, dir: &Path) -> Check {
    let gib = bytes / (1024 * 1024 * 1024);
    if gib >= 5 {
        Check::pass(
            "Disk space",
            format!("{gib} GiB free on the volume containing {}", dir.display()),
        )
    } else if gib >= 1 {
        Check::warn(
            "Disk space",
            format!("only {gib} GiB free on {}", dir.display()),
            "recordings will accumulate quickly; \
             consider --max-files-per-species or external storage",
        )
    } else {
        // A WARNING, not an error — for the same reason as the database
        // integrity check next door. The unit gates startup on
        //   ExecStartPre=... --doctor ... || [ $? -le 1 ]
        // so failing here (exit 2) stops systemd from starting the daemon, and
        // the daemon is what runs `start_disk_manager`: the purge that reclaims
        // space at DISK_PURGE_THRESHOLD lives *inside* the process this would
        // refuse to start.
        //
        // That inverts the design. A full disk is the most predictable failure
        // mode of a 24/7 recorder — every station reaches it eventually — and
        // it is precisely the one the purge exists to absorb. Blocking startup
        // means the reclaim never runs, `Restart=always` spends
        // StartLimitBurst in under a minute, and the unit parks in `failed`: a
        // station killed permanently by the condition it was built to survive
        // unattended.
        //
        // It is also mistimed. The purge triggers on a *percentage* (95% by
        // default), so on a small card it fires well below 1 GiB free — this
        // check would refuse startup before the mechanism that fixes it had
        // even been reached.
        //
        // Exit 2 means "errors that will prevent operation". A nearly full disk
        // degrades operation and then self-corrects; it does not prevent it.
        Check::warn(
            "Disk space",
            format!(
                "less than 1 GiB free on {} — the disk manager will purge oldest recordings at the configured threshold",
                dir.display()
            ),
            "if this persists the purge is not keeping up: lower MAX_FILES_SPECIES, \
             lower DISK_PURGE_THRESHOLD, or move RECS_DIR to larger storage",
        )
    }
}

/// Best-effort free-bytes query.
///
/// Delegates to [`birdnet_core::audio::capture::disk_usage`] rather than
/// shelling out again. There were two `df` implementations in this workspace
/// and they had drifted: this one used POSIX `-Pk`, and the capture disk
/// manager's used `--output=size,used,avail -B1`, which are GNU coreutils
/// extensions that BSD and `BusyBox` `df` reject. The one that ran on a macOS
/// station was the one that mattered, and it was the broken one — so the
/// preflight reported disk space happily while the purge that keeps the card
/// from filling never ran.
fn disk_free_bytes(path: &Path) -> Option<u64> {
    birdnet_core::audio::capture::disk_usage(path)
        .ok()
        .map(|u| u.available_bytes)
}

#[cfg(test)]
mod tests {
    use super::{check_disk_space, disk_free_bytes, grade_free_space};
    use crate::doctor::Status;
    use std::path::Path;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// The regression: a nearly full disk must warn, never fail.
    ///
    /// The unit gates startup on `--doctor ... || [ $? -le 1 ]`, so an error
    /// here stops systemd from starting the daemon — and `start_disk_manager`,
    /// the purge that reclaims the space, runs *inside* that daemon. Reported
    /// as an error, the diagnostic blocks its own remedy and the unit then
    /// parks in `failed` via `StartLimitBurst`.
    #[test]
    fn a_nearly_full_disk_warns_so_the_purge_can_run() {
        let check = grade_free_space(GIB / 2, Path::new("/data"));
        assert_ne!(
            check.status,
            Status::Pass,
            "low space must still be reported: {}",
            check.message
        );
        assert_eq!(
            check.status,
            Status::Warn,
            "must be a warning (exit 1) so ExecStartPre lets the daemon start and purge, \
             not an error (exit 2) which blocks it: {}",
            check.message
        );
    }

    #[test]
    fn free_space_grades_across_the_thresholds() {
        assert_eq!(
            grade_free_space(9 * GIB, Path::new("/")).status,
            Status::Pass
        );
        assert_eq!(
            grade_free_space(5 * GIB, Path::new("/")).status,
            Status::Pass
        );
        // 1–5 GiB is the pre-existing "accumulating quickly" warning.
        assert_eq!(
            grade_free_space(3 * GIB, Path::new("/")).status,
            Status::Warn
        );
        assert_eq!(grade_free_space(GIB, Path::new("/")).status, Status::Warn);
        // Below 1 GiB, and at genuinely zero.
        assert_eq!(grade_free_space(0, Path::new("/")).status, Status::Warn);
    }

    #[test]
    fn the_low_space_warning_names_the_mechanism_that_recovers_it() {
        let check = grade_free_space(0, Path::new("/data"));
        assert!(
            check.message.contains("purge"),
            "an operator reading this should learn the station recovers on its own: {}",
            check.message
        );
        assert!(
            check
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("DISK_PURGE_THRESHOLD") || r.contains("MAX_FILES")),
            "remediation should name the knobs that fix a purge that cannot keep up"
        );
    }

    #[test]
    fn check_disk_space_returns_one_named_check() {
        use crate::cli::Cli;
        use clap::Parser as _;
        // Shells out to `df` on the default volume; the exact verdict depends on
        // the host's free space, so we assert structure rather than the branch.
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let checks = check_disk_space(&cli, None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "Disk space");
    }

    /// The preflight and the purge must be looking at the same filesystem.
    ///
    /// They used to shell out to `df` separately, with different flags, and
    /// only one of the two was POSIX. Output-shape parsing is gated in
    /// `birdnet_core::audio::capture::disk` across GNU, BSD and `BusyBox`
    /// fixtures; what this adds is that the doctor reads that same
    /// implementation rather than a second one of its own.
    ///
    /// Deliberately **not** an equality assertion. The first draft of this test
    /// compared two live `df` readings and failed in the full suite with a
    /// 4096-byte difference — one block, written by another test between the
    /// two calls. That is the filesystem moving, not the implementations
    /// disagreeing, and a gate that cannot tell those apart is a flake. Two
    /// separate parsers would differ by orders of magnitude (bytes vs KiB, or a
    /// column misread), so a tolerance well below that and well above ordinary
    /// churn discriminates exactly the thing this is for.
    #[test]
    fn the_doctor_and_the_purge_read_one_implementation() {
        let dir = std::env::temp_dir();
        let doctor = disk_free_bytes(&dir).expect("df answers on this host");
        let capture = birdnet_core::audio::capture::disk_usage(&dir)
            .expect("df answers on this host")
            .available_bytes;
        assert!(doctor > 0 && capture > 0, "both must read the filesystem");
        let delta = doctor.abs_diff(capture);
        let tolerance = capture / 100;
        assert!(
            delta <= tolerance.max(64 * 1024 * 1024),
            "the preflight and the disk manager disagree about free space by \
             {delta} bytes (doctor {doctor}, capture {capture}) — that is too \
             large to be concurrent writes and looks like a second parser"
        );
    }
}
