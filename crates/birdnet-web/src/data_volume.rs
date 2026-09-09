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
