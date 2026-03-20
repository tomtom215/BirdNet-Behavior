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
pub use manager::{DiskManager, DiskManagerConfig, FullDiskAction};

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
    #[allow(clippy::cast_precision_loss)]
    pub fn used_percent(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.used_bytes as f64 / self.total_bytes as f64 * 100.0
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

/// Get disk usage information for the filesystem containing `path`.
///
/// Uses the `df` command to query filesystem statistics, avoiding
/// `unsafe` code and platform-specific `libc` bindings.
///
/// # Errors
///
/// Returns `CaptureError` if `df` is not available or `path` doesn't exist.
pub fn disk_usage(path: &Path) -> Result<DiskUsage, CaptureError> {
    let output = Command::new("df")
        .arg("--output=size,used,avail")
        .arg("-B1") // bytes
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data_line = stdout
        .lines()
        .nth(1)
        .ok_or_else(|| CaptureError::Config("unexpected df output".into()))?;

    let values: Vec<u64> = data_line
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();

    if values.len() < 3 {
        return Err(CaptureError::Config("unexpected df output format".into()));
    }

    Ok(DiskUsage {
        total_bytes: values[0],
        used_bytes: values[1],
        available_bytes: values[2],
    })
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
