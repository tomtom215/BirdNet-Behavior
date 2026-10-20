//! What the data volume is doing right now: writable, still mounted, how
//! full (PS-9, AD-4, DD-19, DD-20).
//!
//! `/api/v2/health` answered `200 "healthy"` on a 100 % full card while
//! `/api/v2/system/disk` said `critical`, DuckDB had been quarantined on
//! `ENOSPC` and the admin bootstrap had failed; and after the data mount was
//! detached it kept writing to the directory underneath, `/system/disk`
//! reported the *parent* filesystem as fine, and the only signal was a
//! per-minute `df failed` in the journal. Nothing probed writability at
//! runtime at all: a read-only remount — what the kernel does after repeated
//! I/O errors — left `SELECT 1` succeeding while every detection was
//! classified and discarded.
//!
//! This module measures the three things once a minute and keeps the last
//! answer on [`crate::state::AppState`], where the health verdict and the
//! station-health conditions read it:
//!
//! * **writable** — a small file is created, synced and removed in the data
//!   directory. A read-only remount and a full volume both fail this, and
//!   nothing else in the process finds out any other way.
//! * **mount** — the data directory's device id against its parent's. A
//!   directory that was its own mount at start and shares its parent's device
//!   now is a volume that has gone away; the directory underneath it is where
//!   the writes are landing. A directory that never was a separate mount
//!   cannot vanish this way, and says so rather than guessing.
//! * **usage** — the same `df` reading the disk endpoint makes, with a `df`
//!   failure recorded as `unknown` rather than lost.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::state::AppState;

/// How often the volume is probed.
pub const POLL_EVERY: Duration = Duration::from_secs(60);

/// The probe file's name. Removed after every probe; a stale one from a
/// crash is overwritten by the next.
const PROBE_FILE: &str = ".bnb-write-probe";

/// How the data directory was mounted when the station started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountShape {
    /// The data directory is on a different device from its parent: its own
    /// mount, which can go away.
    SeparateMount {
        /// The device id at start.
        device: u64,
    },
    /// The data directory shares its parent's device: not a mount point, so
    /// "vanished" is not a state it can be in.
    NotAMount,
    /// The device ids could not be read (no parent, or not Unix).
    Unknown,
}

/// The mount's state now, against how it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MountState {
    /// A separate mount that is still there, on the same device.
    Intact,
    /// A separate mount at start; now the directory shares its parent's
    /// device. The volume is gone and writes are landing underneath it.
    Vanished,
    /// Never a separate mount; nothing to lose.
    NotAMount,
    /// Cannot be determined.
    Unknown,
}

/// The `df` verdict, the same thresholds `/api/v2/system/disk` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskVerdict {
    /// Below the "low" threshold.
    Ok,
    /// At or above 90 % used.
    Low,
    /// At or above 95 % used.
    Critical,
    /// `df` failed or its output could not be read.
    Unknown,
}

/// The last probe's answer.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DataVolumeStatus {
    /// Whether a file could be created, synced and removed in the data
    /// directory.
    pub writable: bool,
    /// Why not, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_error: Option<String>,
    /// The mount, against how it started.
    pub mount: MountState,
    /// The `df` verdict.
    pub disk: DiskVerdict,
    /// Used percentage, when `df` answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    /// Seconds since the Unix epoch when this was measured.
    pub checked_at: u64,
}

impl DataVolumeStatus {
    /// Whether the station is recording nothing to disk: unwritable, or
    /// writing into the directory under a vanished mount. Degraded on every
    /// reading of `/api/v2/health`, `?strict` or not, for the same reason a
    /// halted ingest is: the station runs, classifies, and keeps nothing.
    #[must_use]
    pub const fn loses_writes(&self) -> bool {
        !self.writable || matches!(self.mount, MountState::Vanished)
    }

    /// Whether the volume is a fault the strict probe reports: the above, or
    /// a critically full disk, or a `df` that cannot answer.
    #[must_use]
    pub const fn is_strict_fault(&self) -> bool {
        self.loses_writes() || matches!(self.disk, DiskVerdict::Critical | DiskVerdict::Unknown)
    }
}

/// Record how the data directory is mounted, at start.
#[must_use]
pub fn mount_shape(dir: &Path) -> MountShape {
    match (device_of(dir), dir.parent().and_then(device_of)) {
        (Some(d), Some(p)) if d != p => MountShape::SeparateMount { device: d },
        (Some(_), Some(_)) => MountShape::NotAMount,
        _ => MountShape::Unknown,
    }
}

/// The mount's state now, from the device ids and how it started.
#[must_use]
pub const fn mount_state(
    shape: MountShape,
    dir_dev: Option<u64>,
    parent_dev: Option<u64>,
) -> MountState {
    match shape {
        MountShape::SeparateMount { device } => match (dir_dev, parent_dev) {
            (Some(d), Some(p)) if d == p => MountState::Vanished,
            (Some(d), _) if d == device => MountState::Intact,
            (Some(_), Some(_)) => MountState::Intact,
            _ => MountState::Unknown,
        },
        MountShape::NotAMount => MountState::NotAMount,
        MountShape::Unknown => MountState::Unknown,
    }
}

#[cfg(unix)]
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

/// Create, sync and remove a small file in `dir`.
///
/// # Errors
///
/// Whatever the filesystem refuses: `EROFS` on a read-only remount, `ENOSPC`
/// on a full volume, `EACCES`, a missing directory.
pub fn write_probe(dir: &Path) -> std::io::Result<()> {
    use std::io::Write as _;
    let path = dir.join(PROBE_FILE);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    file.write_all(b"birdnet-behavior write probe\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::remove_file(&path)
}

/// Measure the volume once.
#[must_use]
pub fn probe(dir: &Path, shape: MountShape) -> DataVolumeStatus {
    let (writable, write_error) = match write_probe(dir) {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    let mount = mount_state(shape, device_of(dir), dir.parent().and_then(device_of));
    let (disk, used_percent) = birdnet_core::audio::capture::disk_usage(dir).map_or(
        (DiskVerdict::Unknown, None),
        |usage| {
            let verdict = if usage.is_critical() {
                DiskVerdict::Critical
            } else if usage.is_low() {
                DiskVerdict::Low
            } else {
                DiskVerdict::Ok
            };
            (verdict, Some(usage.used_percent()))
        },
    );
    DataVolumeStatus {
        writable,
        write_error,
        mount,
        disk,
        used_percent,
        checked_at: unix_now(),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The data directory a database path implies: its parent, or `.`.
#[must_use]
pub fn data_dir_of(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

// ---------------------------------------------------------------------------
// The container's own layer
// ---------------------------------------------------------------------------
//
// A data directory that is not a mount leaves everything on the container's
// writable layer: the station runs, records and answers, and every detection,
// recording and setting is deleted with the container the next time it is
// recreated — an image upgrade is enough. (This project's image declares
// `VOLUME /data`, so a bare `docker run` gets an anonymous volume there, which
// is a mount and is not what this finds; it finds a `DB_PATH` outside `/data`,
// or an image without the declaration.) Nothing about that looks wrong from
// inside: the directory is writable, has space, and was never a mount, so none
// of the probes above has anything to say. The question has to be asked of the
// mount table: is the directory on the container's root filesystem, or on
// something mounted over it?

/// Filesystem types a container runtime builds its root from out of image
/// layers plus a throwaway writable layer.
///
/// Deliberately the layered ones only. A container whose root is `btrfs`, `zfs`
/// or `devicemapper` is just as ephemeral, but on those the root is
/// indistinguishable from an ordinary host filesystem by type alone, and this
/// check would rather say nothing than be wrong on a host.
const LAYERED_ROOT_FS: &[&str] = &["overlay", "overlayfs", "aufs", "fuse.fuse-overlayfs"];

/// One line of `/proc/self/mountinfo`, reduced to what the question needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// Where the mount is attached.
    pub mount_point: String,
    /// The filesystem type (`overlay`, `ext4`, `tmpfs` …).
    pub fs_type: String,
    /// The path *within* the mounted filesystem that appears here: `/` for a
    /// whole filesystem, a subdirectory for a bind mount or a volume.
    pub root: String,
}

/// Undo `mountinfo`'s octal escapes (`\040` for a space, `\011`, `\012`,
/// `\134`): the kernel escapes the bytes that would break the field split.
fn unescape_mountinfo(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let value = (u32::from(bytes[i + 1] - b'0') << 6)
                | (u32::from(bytes[i + 2] - b'0') << 3)
                | u32::from(bytes[i + 3] - b'0');
            if let Ok(byte) = u8::try_from(value) {
                out.push(byte);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse the text of `/proc/self/mountinfo`.
///
/// Format (proc(5)): `ID PARENT MAJ:MIN ROOT MOUNT-POINT OPTIONS [TAGS…] - FSTYPE
/// SOURCE SUPER-OPTIONS`. The optional tags are variable in number and end at a
/// lone `-`, so the filesystem type is found by that separator and not by
/// position. A line that does not parse is skipped: a mount table read at all
/// is far likelier to be right with one line missing than to be worth failing
/// on.
#[must_use]
pub fn parse_mountinfo(text: &str) -> Vec<MountEntry> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let dash = fields.iter().position(|f| *f == "-")?;
            if dash < 6 {
                return None;
            }
            Some(MountEntry {
                root: unescape_mountinfo(fields[3]),
                mount_point: unescape_mountinfo(fields[4]),
                fs_type: fields.get(dash + 1)?.to_string(),
            })
        })
        .collect()
}

/// The mount `path` lives on.
///
/// The entry with the longest mount point that is a whole-component prefix of
/// the path (so `/data2` is not on a mount at `/data`). When mounts are stacked
/// on one point, the one listed last is on top.
#[must_use]
pub fn mount_holding<'a>(entries: &'a [MountEntry], path: &Path) -> Option<&'a MountEntry> {
    entries
        .iter()
        .filter(|e| path.starts_with(&e.mount_point))
        .max_by_key(|e| Path::new(&e.mount_point).components().count())
    // `max_by_key` returns the *last* of equal maxima, which is the topmost
    // of a stack — the one a write actually lands on.
}

/// Whether `entry` is the root filesystem of a container: attached at `/` and
/// built from image layers.
#[must_use]
pub fn is_container_root(entry: &MountEntry) -> bool {
    entry.mount_point == "/" && LAYERED_ROOT_FS.contains(&entry.fs_type.as_str())
}

/// Whether `dir` is on a container's own writable layer, given the mount table
/// and whether this process is in a container at all.
///
/// Both halves are needed. A layered root filesystem alone is also what a
/// Raspberry Pi set up with a read-only overlay root looks like, and flagging
/// that as a container would be a false alarm on bare metal; the container
/// marker alone says nothing about where *this* directory is. A directory that
/// is a mount of its own — a named volume, a bind mount, a tmpfs — is on that
/// mount, not on the root, and is not flagged: this reports where data is
/// *lost*, not where it is *safe*.
#[must_use]
pub fn is_on_container_layer(entries: &[MountEntry], dir: &Path, in_container: bool) -> bool {
    in_container && mount_holding(entries, dir).is_some_and(is_container_root)
}

/// Whether this process is running in a container: the marker file Docker
/// (`/.dockerenv`) or Podman (`/run/.containerenv`) creates.
#[must_use]
pub fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

/// Whether the data directory `dir` is on the container's own writable layer,
/// read from this process's mount table.
///
/// `false` when it is not, and also `false` when it cannot be told — no marker
/// file, an unreadable `/proc`, a path that will not canonicalise. Reporting a
/// loss that may not be happening would teach the reader to ignore the banner it
/// is on.
///
/// The one call both the Station Health page and `--doctor` can make.
#[must_use]
pub fn on_container_layer(dir: &Path) -> bool {
    if !in_container() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let Ok(dir) = std::fs::canonicalize(dir) else {
        return false;
    };
    is_on_container_layer(&parse_mountinfo(&text), &dir, true)
}

/// Probe now, then every [`POLL_EVERY`] on a thread of its own.
///
/// Each answer is published to `state`. The first probe runs before this
/// returns, so the health endpoint never answers a request without a reading.
///
/// # Panics
///
/// If the OS refuses to create the thread, which at start-up is a station
/// that cannot run at all.
pub fn spawn_watch(state: AppState) -> std::thread::JoinHandle<()> {
    let dir = data_dir_of(state.db_path());
    let shape = mount_shape(&dir);
    let first = probe(&dir, shape);
    log_transition(None, &first, &dir);
    state.set_data_volume(first.clone());
    let previous = Arc::new(std::sync::Mutex::new(first));
    std::thread::Builder::new()
        .name("data-volume-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(POLL_EVERY);
                let now = probe(&dir, shape);
                if let Ok(mut prev) = previous.lock() {
                    log_transition(Some(&prev), &now, &dir);
                    *prev = now.clone();
                }
                state.set_data_volume(now);
            }
        })
        .expect("spawn data-volume watch thread")
}

/// One line when something changes, none while it stays the same: a
/// per-minute repeat is the journal noise this replaces.
fn log_transition(prev: Option<&DataVolumeStatus>, now: &DataVolumeStatus, dir: &Path) {
    let changed = prev
        .is_none_or(|p| p.writable != now.writable || p.mount != now.mount || p.disk != now.disk);
    if !changed {
        return;
    }
    if now.loses_writes() {
        tracing::error!(
            dir = %dir.display(),
            writable = now.writable,
            error = now.write_error.as_deref().unwrap_or(""),
            mount = ?now.mount,
            disk = ?now.disk,
            "the data volume is not taking writes: detections are being lost"
        );
    } else if matches!(now.disk, DiskVerdict::Critical | DiskVerdict::Unknown) {
        tracing::warn!(
            dir = %dir.display(),
            disk = ?now.disk,
            used_percent = now.used_percent.unwrap_or(f64::NAN),
            "data volume"
        );
    } else {
        tracing::info!(
            dir = %dir.display(),
            mount = ?now.mount,
            disk = ?now.disk,
            used_percent = now.used_percent.unwrap_or(f64::NAN),
            "data volume ok"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writable_directory_probes_writable_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let status = probe(dir.path(), MountShape::NotAMount);
        assert!(status.writable, "{status:?}");
        assert!(status.write_error.is_none());
        assert_eq!(status.mount, MountState::NotAMount);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!status.loses_writes());
    }

    #[test]
    fn a_missing_directory_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let status = probe(&gone, MountShape::NotAMount);
        assert!(!status.writable, "{status:?}");
        assert!(status.write_error.is_some());
        assert_eq!(
            status.disk,
            DiskVerdict::Unknown,
            "df cannot answer for a missing path"
        );
        assert!(status.loses_writes());
        assert!(status.is_strict_fault());
    }

    /// The device-id rule, with the numbers under the test's control: a mount
    /// that now shares its parent's device has gone away.
    #[test]
    fn a_separate_mount_that_shares_its_parents_device_has_vanished() {
        let shape = MountShape::SeparateMount { device: 42 };
        assert_eq!(mount_state(shape, Some(42), Some(7)), MountState::Intact);
        assert_eq!(mount_state(shape, Some(7), Some(7)), MountState::Vanished);
        assert_eq!(mount_state(shape, None, Some(7)), MountState::Unknown);
        assert_eq!(
            mount_state(MountShape::NotAMount, Some(7), Some(7)),
            MountState::NotAMount
        );
        assert_eq!(
            mount_state(MountShape::Unknown, Some(7), Some(7)),
            MountState::Unknown
        );
    }

    #[test]
    fn a_temp_directory_is_not_its_own_mount() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data");
        std::fs::create_dir(&nested).unwrap();
        assert_eq!(mount_shape(&nested), MountShape::NotAMount);
    }

    // ── the container's own layer ───────────────────────────────────────

    /// A container started with `docker run` and no volume. The shape is what
    /// Docker's overlay2 driver writes: the root is an `overlay` mount with
    /// `lowerdir`/`upperdir`/`workdir` options, and the runtime bind-mounts a
    /// handful of files from the host's `/var/lib/docker/containers/<id>/` over
    /// it. (Modelled on Docker's documented output; this sandbox is not a
    /// container and could not supply a captured one.)
    const DOCKER_NO_VOLUME: &str = "\
1122 1020 0:63 / / rw,relatime master:416 - overlay overlay rw,lowerdir=/var/lib/docker/overlay2/l/ABCDEFG:/var/lib/docker/overlay2/l/HIJKLMN,upperdir=/var/lib/docker/overlay2/3f9c/diff,workdir=/var/lib/docker/overlay2/3f9c/work
1123 1122 0:66 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw
1124 1122 0:67 / /dev rw,nosuid - tmpfs tmpfs rw,size=65536k,mode=755
1125 1124 0:68 / /dev/pts rw,nosuid,noexec,relatime - devpts devpts rw,gid=5,mode=620,ptmxmode=666
1126 1122 0:69 / /sys ro,nosuid,nodev,noexec,relatime - sysfs sysfs ro
1127 1124 0:70 / /dev/shm rw,nosuid,nodev,noexec,relatime - tmpfs shm rw,size=65536k
1128 1122 254:1 /var/lib/docker/containers/9f3c/resolv.conf /etc/resolv.conf rw,relatime - ext4 /dev/vda1 rw
1129 1122 254:1 /var/lib/docker/containers/9f3c/hostname /etc/hostname rw,relatime - ext4 /dev/vda1 rw
1130 1122 254:1 /var/lib/docker/containers/9f3c/hosts /etc/hosts rw,relatime - ext4 /dev/vda1 rw
";

    /// The same container with `-v birdnet-data:/data` (a named volume) or
    /// `-v /home/pi/birdnet:/data` (a bind mount): `/data` is a mount of the
    /// host's filesystem, and data written there is not on the layer.
    fn docker_with(source_root: &str) -> String {
        format!(
            "{DOCKER_NO_VOLUME}1131 1122 254:1 {source_root} /data rw,relatime master:1 - ext4 /dev/vda1 rw\n"
        )
    }

    /// A Raspberry Pi running from its SD card, trimmed to the lines that matter.
    const BARE_METAL_PI: &str = "\
22 1 179:2 / / rw,noatime - ext4 /dev/root rw,discard
23 22 0:21 / /proc rw,relatime - proc proc rw
24 22 0:22 / /sys rw,nosuid,nodev,noexec,relatime - sysfs sysfs rw
25 22 0:6 / /dev rw,relatime - devtmpfs devtmpfs rw,size=1850000k,nr_inodes=462500,mode=755
31 22 0:27 / /tmp rw,nosuid,nodev - tmpfs tmpfs rw,size=1966080k
40 22 179:1 / /boot/firmware rw,relatime - vfat /dev/mmcblk0p1 rw,fmask=0022,dmask=0022
";

    /// This development machine's own table (a VM with an ext4 root), kept as
    /// the real-world "not a container" fixture.
    const REAL_VM: &str = "\
28 1 254:0 / / rw,relatime - ext4 /dev/vda rw,discard,no_prefetch_block_bitmaps,resv_strict,resuid=65534,resgid=65534
29 28 254:16 / /opt/rclone ro,relatime - squashfs /dev/vdb ro,errors=continue
30 28 254:32 / /opt/claude-code ro,nosuid,nodev,relatime - ext4 /dev/vdc ro,norecovery
48 28 0:40 / /run/sandbox-telemetry rw,nosuid,nodev,noexec,relatime - tmpfs tmpfs rw,size=1024k,mode=755,uid=64321,gid=64321
";

    fn on_layer(table: &str, dir: &str, in_container: bool) -> bool {
        is_on_container_layer(&parse_mountinfo(table), Path::new(dir), in_container)
    }

    /// **The case this exists for.** No volume: `/data` is just a directory on
    /// the overlay root, and goes when the container does.
    #[test]
    fn a_data_directory_on_the_overlay_root_of_a_container_is_flagged() {
        assert!(on_layer(DOCKER_NO_VOLUME, "/data", true));
        assert!(on_layer(DOCKER_NO_VOLUME, "/data/BirdNet-Behavior", true));
        // Anywhere else on the root is the same layer.
        assert!(on_layer(
            DOCKER_NO_VOLUME,
            "/home/birdnet/BirdNet-Behavior",
            true
        ));
    }

    /// The counterparts. Each of these is a station whose data survives, and
    /// each must stay quiet or the banner becomes noise.
    #[test]
    fn a_volume_a_bind_mount_and_bare_metal_are_not_flagged() {
        // A named volume: the mount's root is the volume's directory.
        let named = docker_with("/var/lib/docker/volumes/birdnet-data/_data");
        assert!(!on_layer(&named, "/data", true));
        assert!(!on_layer(&named, "/data/BirdNet-Behavior", true));
        // A bind mount from the host.
        let bind = docker_with("/home/pi/birdnet");
        assert!(!on_layer(&bind, "/data", true));
        // ...but the *same* container's other directories are still on the layer.
        assert!(on_layer(&bind, "/srv/recordings", true));
        // Bare metal, in either of its shapes.
        assert!(!on_layer(BARE_METAL_PI, "/home/pi/BirdNet-Behavior", true));
        assert!(!on_layer(BARE_METAL_PI, "/data", false));
        assert!(!on_layer(REAL_VM, "/home/user/data", false));
        // A tmpfs mounted at the data directory is a mount of its own: lost on
        // restart, but not a *layer*, and not what this reports.
        let tmpfs = format!("{DOCKER_NO_VOLUME}1140 1122 0:90 / /data rw - tmpfs tmpfs rw\n");
        assert!(!on_layer(&tmpfs, "/data", true));
    }

    /// An overlay root without a container marker is a Pi with a read-only
    /// overlay root, a live image, an installer — not a container. Flagging it
    /// would be a false alarm on bare metal.
    #[test]
    fn an_overlay_root_with_no_container_marker_is_not_a_container() {
        assert!(!on_layer(DOCKER_NO_VOLUME, "/data", false));
    }

    /// Only layered root filesystems count: a plain ext4 root inside a container
    /// might be anything (a `--privileged` host-root mount, an LXC with a
    /// directory rootfs), and saying nothing is the safer error.
    #[test]
    fn only_a_layered_root_counts() {
        assert!(!on_layer(REAL_VM, "/data", true));
        for fs in ["overlay", "overlayfs", "aufs", "fuse.fuse-overlayfs"] {
            let table = format!("1 0 0:1 / / rw - {fs} {fs} rw\n");
            assert!(on_layer(&table, "/data", true), "{fs}");
        }
        let zfs = "1 0 0:1 / / rw - zfs rpool/ROOT rw\n";
        assert!(!on_layer(zfs, "/data", true));
    }

    /// The path rule: a whole-component prefix, the longest mount wins, and a
    /// mount stacked on another is the one a write lands on.
    #[test]
    fn the_mount_holding_a_directory_is_decided_by_whole_components() {
        let table = parse_mountinfo(&docker_with("/home/pi/birdnet"));
        let holder = |dir: &str| {
            mount_holding(&table, Path::new(dir))
                .map(|e| e.mount_point.clone())
                .unwrap_or_default()
        };
        assert_eq!(holder("/data"), "/data");
        assert_eq!(holder("/data/model"), "/data");
        // `/data2` and `/database` are not under `/data`.
        assert_eq!(holder("/data2"), "/");
        assert_eq!(holder("/database/x"), "/");
        assert_eq!(holder("/dev/pts/0"), "/dev/pts");

        // Stacked: a volume mounted over an earlier `/data` mount.
        let stacked = parse_mountinfo(
            "1 0 0:1 / / rw - overlay overlay rw\n\
             2 1 8:1 / /data rw - tmpfs tmpfs rw\n\
             3 1 8:2 / /data rw - ext4 /dev/sda2 rw\n",
        );
        assert_eq!(
            mount_holding(&stacked, Path::new("/data/x")).map(|e| e.fs_type.as_str()),
            Some("ext4")
        );
    }

    /// The parser: optional tags before the `-` vary in number, and the kernel
    /// escapes spaces and backslashes in paths as octal.
    #[test]
    fn mountinfo_lines_parse_whatever_their_optional_fields() {
        let table = parse_mountinfo(
            "36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue\n\
             37 35 98:1 / /a\\040b\\134c rw shared:2 master:3 propagate_from:4 - ext4 /dev/x rw\n\
             38 35 98:2 / /bare rw - xfs /dev/y rw\n\
             not a mountinfo line\n\
             \n",
        );
        assert_eq!(table.len(), 3, "{table:?}");
        assert_eq!(table[0].root, "/mnt1");
        assert_eq!(table[0].mount_point, "/mnt2");
        assert_eq!(table[0].fs_type, "ext3");
        assert_eq!(table[1].mount_point, "/a b\\c");
        assert_eq!(table[1].fs_type, "ext4");
        assert_eq!(table[2].fs_type, "xfs");
    }

    /// The whole thing against this machine's real `/proc`: a development VM or
    /// CI runner is not a container with no volume, and the check must say so.
    /// (If this ever runs inside one, the temp dir really may be on the layer;
    /// the assertion is only made where the marker files are absent.)
    #[test]
    fn this_machine_is_not_reported_when_it_is_not_a_container() {
        if in_container() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(!on_container_layer(dir.path()));
    }

    #[test]
    fn the_verdicts_separate_lost_writes_from_a_full_disk() {
        let base = DataVolumeStatus {
            writable: true,
            write_error: None,
            mount: MountState::Intact,
            disk: DiskVerdict::Ok,
            used_percent: Some(40.0),
            checked_at: 0,
        };
        assert!(!base.is_strict_fault());
        let full = DataVolumeStatus {
            disk: DiskVerdict::Critical,
            ..base.clone()
        };
        assert!(
            !full.loses_writes(),
            "a full disk that still takes a probe write is strict-only"
        );
        assert!(full.is_strict_fault());
        let vanished = DataVolumeStatus {
            mount: MountState::Vanished,
            ..base.clone()
        };
        assert!(vanished.loses_writes());
        let read_only = DataVolumeStatus {
            writable: false,
            write_error: Some("Read-only file system".into()),
            ..base
        };
        assert!(read_only.loses_writes());
    }
}
