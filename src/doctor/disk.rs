//! Disk-space check (best-effort, via `df`).

use std::path::{Path, PathBuf};

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;

/// The name the data-volume check reports under.
const DATA_VOLUME: &str = "Data volume";

pub(super) fn check_disk_space(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    // The data partition first (PS-8): this used to read `--watch-dir`, which
    // the shipped unit always sets to the RAM-backed stream directory, so the
    // preflight graded a tmpfs while the card filled. The database's directory
    // is where the detections, the clips and the backups live.
    let data_dir = crate::helpers::db_path_from_config(config)
        .parent()
        .map_or_else(|| PathBuf::from("/"), std::path::Path::to_path_buf);
    let mut out = vec![disk_free_bytes(&data_dir).map_or_else(
        || Check::skip("Disk space", "could not query filesystem usage"),
        |bytes| grade_free_space(bytes, &data_dir),
    )];
    // The stream directory too, when it is a different filesystem — a full
    // tmpfs stalls capture just as surely — reported under its own name so the
    // two cannot be mistaken for each other.
    let scratch = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from));
    if let Some(scratch) = scratch
        && scratch.exists()
        && !same_filesystem(&scratch, &data_dir)
        && let Some(bytes) = disk_free_bytes(&scratch)
    {
        let mut check = grade_free_space(bytes, &scratch);
        "Disk space (stream directory)".clone_into(&mut check.name);
        out.push(check);
    }
    out
}

/// Whether the data directory is somewhere its contents outlive the container.
///
/// A data directory with nothing mounted at it leaves the database, the
/// recordings and the settings on the container's own writable layer. (This
/// project's image declares `VOLUME /data`, so a bare `docker run` gets an
/// anonymous volume there and passes; this finds a `DB_PATH` outside `/data`, or
/// an image without the declaration.) The
/// station runs, records and answers; the directory is writable, has space and
/// was never a mount, so nothing else here (the disk-space check above, the
/// runtime probes in `birdnet_web::data_volume`) has anything to say — and all
/// of it is deleted with the container the next time it is recreated, which an
/// image upgrade is enough to do. The Station Health banner and header badge
/// already say so; this is the same question asked of `--doctor`, by the same
/// function ([`birdnet_web::data_volume::on_container_layer`]), so the two
/// cannot disagree about it.
///
/// The directory is the one holding the database, as the station resolves it
/// (`DB_PATH`, else `$HOME/BirdNet-Behavior/birds.db`) and as the banner reads
/// it. A warning and never an error: the station works, and the unit's
/// `ExecStartPre` gate only stops on exit 2.
pub(super) fn check_data_volume(config: Option<&Config>) -> Check {
    let dir = data_directory(config);
    grade_data_volume(
        birdnet_web::data_volume::in_container(),
        birdnet_web::data_volume::on_container_layer(&dir),
        &dir,
    )
}

/// The directory holding the station's database: what the Station Health
/// banner reads ([`birdnet_web::data_volume::data_dir_of`] of the database path
/// the station resolves), so the two cannot name different directories.
fn data_directory(config: Option<&Config>) -> PathBuf {
    birdnet_web::data_volume::data_dir_of(&crate::helpers::db_path_from_config(config))
}

/// Grade the two facts [`check_data_volume`] reads from the host, so every
/// branch is testable with a mount-table fixture instead of the machine the
/// tests happen to run on.
fn grade_data_volume(in_container: bool, on_container_layer: bool, dir: &Path) -> Check {
    if on_container_layer {
        Check::warn(
            DATA_VOLUME,
            format!(
                "{} is on this container's own writable layer, not on a volume: the database, \
                 the recordings and the settings are deleted when the container is removed or \
                 recreated, and an image upgrade recreates it",
                dir.display()
            ),
            "mount a volume at the data directory: `-v birdnet-data:/data` on `docker run`, or \
             `birdnet-data:/data` under `volumes:` in compose (the shipped docker-compose.yml \
             has it). Copy what is there out first with `docker cp <container>:/data .`",
        )
    } else if in_container {
        // "Not on the layer" is what the mount table says; it is also what an
        // unreadable table says (`on_container_layer` answers false when it
        // cannot tell), so the message claims no more than the table showed.
        Check::pass(
            DATA_VOLUME,
            format!(
                "the mount table does not put {} on the container's own layer",
                dir.display()
            ),
        )
    } else {
        Check::skip(
            DATA_VOLUME,
            "not running in a container; nothing here is lost with one",
        )
    }
}

/// Whether two directories are on the same filesystem, by `df` totals — the
/// same test the station page uses to decide whether to show a scratch tile.
fn same_filesystem(a: &Path, b: &Path) -> bool {
    match (
        birdnet_core::audio::capture::disk_usage(a),
        birdnet_core::audio::capture::disk_usage(b),
    ) {
        (Ok(x), Ok(y)) => x.total_bytes == y.total_bytes,
        _ => true,
    }
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
    use super::{
        check_data_volume, check_disk_space, data_directory, disk_free_bytes, grade_data_volume,
        grade_free_space,
    };
    use crate::doctor::Status;
    use birdnet_web::data_volume::{in_container, is_on_container_layer, parse_mountinfo};
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

    // ── the container's own layer ───────────────────────────────────────
    //
    // The mount tables are the ones `birdnet_web::data_volume`'s own tests use
    // (modelled on Docker's documented overlay2 output; this sandbox is not a
    // container and could not supply a captured one). The doctor's job is to
    // turn that module's two answers into a verdict, so each case goes through
    // the real `is_on_container_layer` and then the grader.

    /// A container started with `docker run` and no volume.
    const DOCKER_NO_VOLUME: &str = "\
1122 1020 0:63 / / rw,relatime master:416 - overlay overlay rw,lowerdir=/var/lib/docker/overlay2/l/ABCDEFG:/var/lib/docker/overlay2/l/HIJKLMN,upperdir=/var/lib/docker/overlay2/3f9c/diff,workdir=/var/lib/docker/overlay2/3f9c/work
1123 1122 0:66 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw
1124 1122 0:67 / /dev rw,nosuid - tmpfs tmpfs rw,size=65536k,mode=755
1128 1122 254:1 /var/lib/docker/containers/9f3c/resolv.conf /etc/resolv.conf rw,relatime - ext4 /dev/vda1 rw
";

    /// A Raspberry Pi running from its SD card.
    const BARE_METAL_PI: &str = "\
22 1 179:2 / / rw,noatime - ext4 /dev/root rw,discard
23 22 0:21 / /proc rw,relatime - proc proc rw
31 22 0:27 / /tmp rw,nosuid,nodev - tmpfs tmpfs rw,size=1966080k
40 22 179:1 / /boot/firmware rw,relatime - vfat /dev/mmcblk0p1 rw,fmask=0022,dmask=0022
";

    /// What `check_data_volume` would conclude on a host with this mount table.
    fn verdict(table: &str, dir: &str, in_container: bool) -> crate::doctor::Check {
        let on_layer = is_on_container_layer(&parse_mountinfo(table), Path::new(dir), in_container);
        grade_data_volume(in_container, on_layer, Path::new(dir))
    }

    /// **The case this exists for.** The Dockerfile declares `VOLUME ["/data"]`
    /// and the station keeps its database in `/data/BirdNet-Behavior`; with the
    /// volume absent from the mount table that directory is on the overlay
    /// root, and goes when the container does.
    #[test]
    fn a_data_directory_on_the_containers_own_layer_warns_and_names_the_remedy() {
        let check = verdict(DOCKER_NO_VOLUME, "/data/BirdNet-Behavior", true);
        assert_eq!(check.status, Status::Warn, "{check:?}");
        assert_eq!(check.name, "Data volume");
        assert!(
            check.message.contains("/data/BirdNet-Behavior")
                && check.message.contains("deleted when the container"),
            "the message names the directory and what is lost: {}",
            check.message
        );
        let fix = check.remediation.as_deref().unwrap_or_default();
        assert!(
            fix.contains("-v birdnet-data:/data") && fix.contains("docker cp"),
            "the remedy is a volume at the data directory, and not losing what is there: {fix}"
        );
    }

    /// A finding that can be fixed by restarting must never stop the station
    /// starting: the unit's gate is `--doctor ... || [ $? -le 1 ]`, so exit 2
    /// would park a recording station in `failed` for a storage choice.
    #[test]
    fn the_container_layer_finding_is_a_warning_and_never_an_error() {
        let check = verdict(DOCKER_NO_VOLUME, "/data", true);
        assert_eq!(check.status, Status::Warn);
        assert_ne!(check.status, Status::Fail);
    }

    /// The counterparts: every station whose data survives must stay quiet, or
    /// the doctor's warning becomes the noise the banner was careful not to be.
    #[test]
    fn a_volume_a_bind_mount_a_tmpfs_and_a_host_are_not_warned_about() {
        let with = |root: &str| {
            format!(
                "{DOCKER_NO_VOLUME}1131 1122 254:1 {root} /data rw,relatime master:1 - ext4 /dev/vda1 rw\n"
            )
        };
        // A named volume, an anonymous one (what a bare `docker run` of this
        // image gets from the Dockerfile's VOLUME line) and a bind mount are all
        // a mount of the host's filesystem at /data.
        for (what, root) in [
            ("named volume", "/var/lib/docker/volumes/birdnet-data/_data"),
            (
                "anonymous volume",
                "/var/lib/docker/volumes/9d1cbb0cc1d5e7a6c5a5f1c6e7d2c8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5/_data",
            ),
            ("bind mount", "/home/pi/birdnet"),
        ] {
            let check = verdict(&with(root), "/data/BirdNet-Behavior", true);
            assert_eq!(check.status, Status::Pass, "{what}: {check:?}");
        }
        // ...while the same container's other directories are still on the layer.
        assert_eq!(
            verdict(&with("/home/pi/birdnet"), "/srv/recordings", true).status,
            Status::Warn
        );
        // A tmpfs at the data directory is a mount of its own: lost on restart,
        // but not the layer, and not what this reports.
        let tmpfs = format!("{DOCKER_NO_VOLUME}1140 1122 0:90 / /data rw - tmpfs tmpfs rw\n");
        assert_eq!(
            verdict(&tmpfs, "/data/BirdNet-Behavior", true).status,
            Status::Pass
        );
        // Bare metal: nothing to say, and said as a skip, not a pass.
        let host = verdict(BARE_METAL_PI, "/home/pi/BirdNet-Behavior", false);
        assert_eq!(host.status, Status::Skip, "{host:?}");
        // An overlay root with no container marker is a Pi with a read-only
        // overlay root, not a container.
        let ro_root = verdict(DOCKER_NO_VOLUME, "/data", false);
        assert_eq!(ro_root.status, Status::Skip, "{ro_root:?}");
    }

    /// The whole check against this machine's real `/proc` and marker files:
    /// a development VM or CI runner is not a container without a volume, and
    /// the doctor must not say it is. (Asserted only where the marker files
    /// are absent, as in `birdnet_web::data_volume`'s own copy of this test.)
    #[test]
    fn this_machine_is_not_warned_about_when_it_is_not_a_container() {
        if in_container() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let cfg = birdnet_core::config::Config::parse(&format!(
            "DB_PATH={}/BirdNet-Behavior/birds.db",
            dir.path().display()
        ))
        .unwrap();
        let check = check_data_volume(Some(&cfg));
        assert_eq!(check.status, Status::Skip, "{check:?}");
    }

    /// The check reads the directory the station keeps its database in, as the
    /// station resolves it, and says which one it read.
    #[test]
    fn the_directory_checked_is_the_one_holding_the_database() {
        let cfg = birdnet_core::config::Config::parse("DB_PATH=/srv/birds/birds.db").unwrap();
        assert_eq!(data_directory(Some(&cfg)), Path::new("/srv/birds"));
        // No DB_PATH: the station's own default, under $HOME.
        let default = data_directory(None);
        assert!(
            default.ends_with("BirdNet-Behavior"),
            "the default database directory is $HOME/BirdNet-Behavior: {}",
            default.display()
        );
        let check = grade_data_volume(true, true, &data_directory(Some(&cfg)));
        assert!(check.message.contains("/srv/birds"), "{}", check.message);
    }

    #[test]
    fn check_disk_space_grades_the_data_partition_first() {
        use crate::cli::Cli;
        use clap::Parser as _;
        // Shells out to `df`; the exact verdict depends on the host's free
        // space, so this asserts structure rather than the branch.
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let checks = check_disk_space(&cli, None);
        assert!(!checks.is_empty());
        assert_eq!(checks[0].name, "Disk space");
    }

    /// The gate for PS-8: with `--watch-dir` on the tmpfs, as the unit sets
    /// it, the first verdict is about the data partition, and the message
    /// names the data directory, not the stream directory.
    #[test]
    fn the_data_partition_is_graded_not_the_stream_directory() {
        use crate::cli::Cli;
        use clap::Parser as _;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let cfg =
            birdnet_core::config::Config::parse(&format!("DB_PATH={}/birds.db", data.display()))
                .unwrap();
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.watch_dir = Some(std::env::temp_dir());
        let checks = check_disk_space(&cli, Some(&cfg));
        let first = &checks[0];
        assert_eq!(first.name, "Disk space");
        assert!(
            first.message.contains(&data.display().to_string()),
            "the verdict must be about the data directory: {first:?}"
        );
        assert!(
            !first
                .message
                .contains(&std::env::temp_dir().display().to_string())
                || data.starts_with(std::env::temp_dir()),
            "the stream directory was graded as the data partition: {first:?}"
        );
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
