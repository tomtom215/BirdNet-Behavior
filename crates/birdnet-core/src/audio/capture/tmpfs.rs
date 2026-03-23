//! Tmpfs transient audio mount helper.
//!
//! Provides helpers for using tmpfs for transient audio files to reduce
//! SD card wear on Raspberry Pi deployments.
//!
//! | Function | Purpose |
//! |----------|---------|
//! | `is_tmpfs_mounted` | Check if tmpfs is already mounted at a path |
//! | `mount_tmpfs`      | Mount a tmpfs at the configured mount point |
//! | `unmount_tmpfs`    | Unmount a tmpfs |
//! | `generate_systemd_mount_unit` | Generate a systemd `.mount` unit file |

use std::path::{Path, PathBuf};
use std::process::Command;

/// Configuration for a tmpfs mount.
#[derive(Debug, Clone)]
pub struct TmpfsConfig {
    /// Directory where the tmpfs will be mounted.
    pub mount_point: PathBuf,
    /// Size of the tmpfs in megabytes (default: 64).
    pub size_mb: u32,
}

impl Default for TmpfsConfig {
    fn default() -> Self {
        Self {
            mount_point: PathBuf::from("/tmp/birdnet-audio"),
            size_mb: 64,
        }
    }
}

/// Errors that can occur during tmpfs operations.
#[derive(Debug)]
pub enum TmpfsError {
    /// A tmpfs is already mounted at the given path.
    AlreadyMounted,
    /// The mount or umount command failed.
    MountFailed(String),
    /// An I/O error occurred.
    Io(std::io::Error),
}

impl std::fmt::Display for TmpfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyMounted => write!(f, "tmpfs is already mounted at that path"),
            Self::MountFailed(msg) => write!(f, "mount failed: {msg}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl From<std::io::Error> for TmpfsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Check if a tmpfs is already mounted at the given path.
///
/// Reads `/proc/mounts` and looks for a line matching the path with
/// filesystem type `tmpfs`.
pub fn is_tmpfs_mounted(path: &Path) -> bool {
    let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else {
        return false;
    };

    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let target = canonical.to_string_lossy();

    mounts.lines().any(|line| {
        let mut fields = line.split_whitespace();
        let mount_point = fields.nth(1).unwrap_or("");
        let fs_type = fields.next().unwrap_or("");
        mount_point == target.as_ref() && fs_type == "tmpfs"
    })
}

/// Mount a tmpfs at the configured mount point.
///
/// Creates the directory if it does not already exist. Returns
/// [`TmpfsError::AlreadyMounted`] if a tmpfs is already present at the
/// mount point. Requires root or sudo privileges.
///
/// # Errors
///
/// Returns `Err(TmpfsError::AlreadyMounted)` if a tmpfs is already mounted
/// at the configured mount point. Returns `Err(TmpfsError::Io)` if the
/// directory cannot be created. Returns `Err(TmpfsError::MountFailed)` if
/// the `mount` command fails.
pub fn mount_tmpfs(config: &TmpfsConfig) -> Result<(), TmpfsError> {
    if is_tmpfs_mounted(&config.mount_point) {
        return Err(TmpfsError::AlreadyMounted);
    }

    // Create the directory if it doesn't exist.
    if !config.mount_point.exists() {
        std::fs::create_dir_all(&config.mount_point)?;
    }

    let size_arg = format!("size={}m", config.size_mb);
    let mount_path = config.mount_point.to_string_lossy();

    let output = Command::new("mount")
        .args(["-t", "tmpfs", "-o", &size_arg, "tmpfs", mount_path.as_ref()])
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(TmpfsError::MountFailed(stderr))
    }
}

/// Unmount the tmpfs at the given path.
///
/// Returns an error if the umount command fails. Does **not** check
/// whether the path is actually a tmpfs mount first.
///
/// # Errors
///
/// Returns `Err(TmpfsError::Io)` if the `umount` command cannot be spawned.
/// Returns `Err(TmpfsError::MountFailed)` if the `umount` command exits
/// with a non-zero status.
pub fn unmount_tmpfs(path: &Path) -> Result<(), TmpfsError> {
    let mount_path = path.to_string_lossy();

    let output = Command::new("umount").arg(mount_path.as_ref()).output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(TmpfsError::MountFailed(stderr))
    }
}

/// Generate a systemd `.mount` unit file for persistent tmpfs.
///
/// The unit file can be installed to `/etc/systemd/system/` so the tmpfs
/// is created automatically on every boot.
///
/// # Example
///
/// ```
/// use std::path::PathBuf;
/// use birdnet_core::audio::capture::tmpfs::{TmpfsConfig, generate_systemd_mount_unit};
///
/// let config = TmpfsConfig {
///     mount_point: PathBuf::from("/tmp/birdnet-audio"),
///     size_mb: 64,
/// };
/// let unit = generate_systemd_mount_unit(&config);
/// assert!(unit.contains("What=tmpfs"));
/// assert!(unit.contains("Where=/tmp/birdnet-audio"));
/// ```
pub fn generate_systemd_mount_unit(config: &TmpfsConfig) -> String {
    let mount_path = config.mount_point.to_string_lossy();

    // systemd mount unit names are derived from the mount point path:
    // /tmp/birdnet-audio → tmp-birdnet\\x2daudio.mount
    // We include a comment with the expected filename.
    let unit_name = mount_path
        .trim_start_matches('/')
        .replace('/', "-")
        .replace('.', "\\x2e");

    format!(
        r"# Systemd mount unit for BirdNet-Behavior transient audio tmpfs.
# Install as: /etc/systemd/system/{unit_name}.mount
# Then run:   systemctl enable --now {unit_name}.mount

[Unit]
Description=BirdNet-Behavior transient audio tmpfs ({size}M)
DefaultDependencies=no
Conflicts=umount.target
Before=local-fs.target umount.target

[Mount]
What=tmpfs
Where={path}
Type=tmpfs
Options=size={size}m,mode=0755,nodev,nosuid,noexec

[Install]
WantedBy=local-fs.target
",
        unit_name = unit_name,
        path = mount_path,
        size = config.size_mb,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = TmpfsConfig::default();
        assert_eq!(config.size_mb, 64);
        assert_eq!(config.mount_point, PathBuf::from("/tmp/birdnet-audio"));
    }

    #[test]
    fn systemd_unit_contains_expected_fields() {
        let config = TmpfsConfig {
            mount_point: PathBuf::from("/tmp/birdnet-audio"),
            size_mb: 128,
        };
        let unit = generate_systemd_mount_unit(&config);
        assert!(unit.contains("Type=tmpfs"));
        assert!(unit.contains("Where=/tmp/birdnet-audio"));
        assert!(unit.contains("size=128m"));
        assert!(unit.contains("[Mount]"));
        assert!(unit.contains("[Install]"));
    }

    #[test]
    fn non_existent_path_not_mounted() {
        assert!(!is_tmpfs_mounted(Path::new(
            "/nonexistent/birdnet/tmpfs/test"
        )));
    }

    #[test]
    fn error_display() {
        let e = TmpfsError::AlreadyMounted;
        assert!(e.to_string().contains("already mounted"));

        let e = TmpfsError::MountFailed("permission denied".to_string());
        assert!(e.to_string().contains("permission denied"));

        let e = TmpfsError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(e.to_string().contains("not found"));
    }
}
