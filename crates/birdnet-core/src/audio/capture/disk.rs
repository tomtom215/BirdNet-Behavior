//! Disk space monitoring and recording cleanup.
//!
//! Replaces `disk_check.sh` from BirdNET-Pi.  Uses the `df` command to
//! query filesystem statistics without `unsafe` code or `libc` bindings.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use super::types::CaptureError;
use super::process::is_audio_file;

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
    /// Percentage of disk used (0.0 – 100.0).
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

// ---------------------------------------------------------------------------
// Disk manager
// ---------------------------------------------------------------------------

/// What to do when the disk reaches the purge threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullDiskAction {
    /// Delete oldest files to free space.
    Purge,
    /// Stop recording (signal the caller) instead of deleting.
    Keep,
}

/// Configuration for automatic disk management.
#[derive(Debug, Clone)]
pub struct DiskManagerConfig {
    /// Directory to monitor (where extracted audio lives, e.g. `~/BirdSongs/Extracted`).
    pub monitored_dir: PathBuf,
    /// Disk-usage percentage at which to trigger purge (default 95).
    pub purge_threshold: u8,
    /// Action to take when the threshold is exceeded.
    pub full_disk_action: FullDiskAction,
    /// Maximum recordings per species directory (0 = unlimited).
    pub max_files_per_species: u32,
    /// Interval between checks in seconds (default 60).
    pub check_interval_secs: u64,
}

impl Default for DiskManagerConfig {
    fn default() -> Self {
        Self {
            monitored_dir: PathBuf::from("BirdSongs/Extracted"),
            purge_threshold: 95,
            full_disk_action: FullDiskAction::Purge,
            max_files_per_species: 0,
            check_interval_secs: 60,
        }
    }
}

/// Automatic disk manager that periodically checks usage and purges old files.
#[derive(Debug)]
pub struct DiskManager {
    config: DiskManagerConfig,
}

impl DiskManager {
    /// Create a new disk manager with the given configuration.
    pub const fn new(config: DiskManagerConfig) -> Self {
        Self { config }
    }

    /// Return a reference to the disk manager configuration.
    pub const fn config(&self) -> &DiskManagerConfig {
        &self.config
    }

    /// Check disk usage and purge oldest files if the threshold is exceeded.
    ///
    /// Returns the number of files removed.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if disk usage cannot be determined, or if the
    /// action is `Keep` and the threshold is exceeded (signals the caller to
    /// stop recording).
    pub fn check_and_purge(&self) -> Result<u32, CaptureError> {
        let usage = disk_usage(&self.config.monitored_dir)?;
        let percent = usage.used_percent();

        #[allow(clippy::cast_possible_truncation)]
        let threshold = f64::from(self.config.purge_threshold);

        if percent < threshold {
            tracing::debug!(
                used_pct = format!("{percent:.1}"),
                threshold = self.config.purge_threshold,
                "disk usage below threshold"
            );
            return Ok(0);
        }

        tracing::warn!(
            used_pct = format!("{percent:.1}"),
            threshold = self.config.purge_threshold,
            "disk usage exceeds threshold"
        );

        match self.config.full_disk_action {
            FullDiskAction::Keep => Err(CaptureError::Config(
                "disk full: stopping recording (full_disk_action=Keep)".into(),
            )),
            FullDiskAction::Purge => {
                let removed = purge_oldest_files(&self.config.monitored_dir);
                cleanup_empty_dirs(&self.config.monitored_dir);
                Ok(removed)
            }
        }
    }

    /// Enforce per-species file count limits.
    ///
    /// Walks `By_Date/*/Species_Name/` directories and removes the oldest
    /// files when the count exceeds `max_files_per_species`.
    ///
    /// Returns the total number of files removed.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if directories cannot be read.
    pub fn enforce_species_limits(&self) -> Result<u32, CaptureError> {
        if self.config.max_files_per_species == 0 {
            return Ok(0);
        }

        let by_date_dir = self.config.monitored_dir.join("By_Date");
        if !by_date_dir.is_dir() {
            return Ok(0);
        }

        let mut total_removed = 0_u32;

        // Collect all species directories across all dates.
        let mut species_files: std::collections::HashMap<String, Vec<(PathBuf, std::time::SystemTime)>> =
            std::collections::HashMap::new();

        let date_entries =
            std::fs::read_dir(&by_date_dir).map_err(|e| CaptureError::Config(e.to_string()))?;

        for date_entry in date_entries.flatten() {
            if !date_entry.path().is_dir() {
                continue;
            }

            let Ok(species_entries) = std::fs::read_dir(date_entry.path()) else {
                continue;
            };

            for species_entry in species_entries.flatten() {
                let species_dir = species_entry.path();
                if !species_dir.is_dir() {
                    continue;
                }

                let species_name = species_entry
                    .file_name()
                    .to_string_lossy()
                    .into_owned();

                let Ok(file_entries) = std::fs::read_dir(&species_dir) else {
                    continue;
                };

                let files = species_files.entry(species_name).or_default();

                for file_entry in file_entries.flatten() {
                    let path = file_entry.path();
                    if path.is_file() && is_audio_file(&path) {
                        let modified = file_entry
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .unwrap_or(std::time::UNIX_EPOCH);
                        files.push((path, modified));
                    }
                }
            }
        }

        // For each species, remove oldest files exceeding the limit.
        let limit = self.config.max_files_per_species as usize;

        for (species, mut files) in species_files {
            if files.len() <= limit {
                continue;
            }

            // Sort by modification time, oldest first.
            files.sort_by_key(|(_, modified)| *modified);

            let to_remove = files.len() - limit;
            for (path, _) in files.iter().take(to_remove) {
                if std::fs::remove_file(path).is_ok() {
                    tracing::debug!(
                        path = %path.display(),
                        species = %species,
                        "removed file (species limit)"
                    );
                    total_removed += 1;
                }
            }
        }

        if total_removed > 0 {
            tracing::info!(
                count = total_removed,
                limit = self.config.max_files_per_species,
                "enforced species file limits"
            );
            cleanup_empty_dirs(&self.config.monitored_dir);
        }

        Ok(total_removed)
    }

    /// Run the disk manager loop (blocking).
    ///
    /// Periodically checks disk usage and enforces species limits until a
    /// stop signal is received on `stop_rx`.
    pub fn run(&self, stop_rx: &mpsc::Receiver<()>) {
        tracing::info!(
            dir = %self.config.monitored_dir.display(),
            interval_secs = self.config.check_interval_secs,
            threshold = self.config.purge_threshold,
            "disk manager started"
        );

        let interval = Duration::from_secs(self.config.check_interval_secs);

        loop {
            match stop_rx.recv_timeout(interval) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("disk manager stopping");
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Time to check.
                }
            }

            if let Err(e) = self.check_and_purge() {
                tracing::error!(error = %e, "disk manager check_and_purge failed");
            }

            if let Err(e) = self.enforce_species_limits() {
                tracing::error!(error = %e, "disk manager enforce_species_limits failed");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Purge the oldest audio files under `base_dir/By_Date/` to free space.
///
/// Collects all audio files, sorts by modification time, and deletes the
/// oldest 10% (minimum 1 file).
///
/// Returns the number of files removed.
fn purge_oldest_files(base_dir: &Path) -> u32 {
    let by_date_dir = base_dir.join("By_Date");
    if !by_date_dir.is_dir() {
        return 0;
    }

    let mut all_files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    collect_audio_files_recursive(&by_date_dir, &mut all_files);

    if all_files.is_empty() {
        return 0;
    }

    // Sort by modification time, oldest first.
    all_files.sort_by_key(|(_, modified)| *modified);

    // Delete oldest 10% (minimum 1).
    let to_remove = (all_files.len() / 10).max(1);
    let mut removed = 0_u32;

    for (path, _) in all_files.iter().take(to_remove) {
        if std::fs::remove_file(path).is_ok() {
            tracing::debug!(path = %path.display(), "purged old file");
            removed += 1;
        }
    }

    if removed > 0 {
        tracing::info!(count = removed, "purged oldest audio files");
    }

    removed
}

/// Recursively collect audio files and their modification times.
fn collect_audio_files_recursive(
    dir: &Path,
    out: &mut Vec<(PathBuf, std::time::SystemTime)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_audio_files_recursive(&path, out);
        } else if path.is_file() && is_audio_file(&path) {
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            out.push((path, modified));
        }
    }
}

/// Remove empty directories under `base_dir` (depth-first).
fn cleanup_empty_dirs(base_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cleanup_empty_dirs(&path);
            // Try to remove; will fail if non-empty, which is fine.
            if std::fs::remove_dir(&path).is_ok() {
                tracing::debug!(path = %path.display(), "removed empty directory");
            }
        }
    }
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

    // -----------------------------------------------------------------------
    // DiskManager tests
    // -----------------------------------------------------------------------

    #[test]
    fn default_disk_manager_config() {
        let config = DiskManagerConfig::default();
        assert_eq!(config.purge_threshold, 95);
        assert_eq!(config.full_disk_action, FullDiskAction::Purge);
        assert_eq!(config.max_files_per_species, 0);
        assert_eq!(config.check_interval_secs, 60);
    }

    #[test]
    fn full_disk_action_equality() {
        assert_eq!(FullDiskAction::Purge, FullDiskAction::Purge);
        assert_ne!(FullDiskAction::Purge, FullDiskAction::Keep);
    }

    #[test]
    fn check_and_purge_below_threshold() {
        // Use /tmp which should be well below 95%.
        let config = DiskManagerConfig {
            monitored_dir: PathBuf::from("/tmp"),
            purge_threshold: 99, // very high threshold
            full_disk_action: FullDiskAction::Purge,
            max_files_per_species: 0,
            check_interval_secs: 60,
        };
        let manager = DiskManager::new(config);
        let result = manager.check_and_purge();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn enforce_species_limits_unlimited() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            max_files_per_species: 0, // unlimited
            ..DiskManagerConfig::default()
        };
        let manager = DiskManager::new(config);
        let result = manager.enforce_species_limits();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn enforce_species_limits_removes_excess() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create By_Date/2026-03-14/Test_Bird/ with 5 wav files.
        let species_dir = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&species_dir).expect("create dirs");

        for i in 0..5 {
            let wav_path = species_dir.join(format!("clip_{i}.wav"));
            // Write a minimal valid WAV header (44 bytes).
            let header = create_minimal_wav_header();
            std::fs::write(&wav_path, &header).expect("write wav");
            // Stagger modification times so we have a deterministic oldest.
            let mtime = filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0);
            filetime::set_file_mtime(&wav_path, mtime).expect("set mtime");
        }

        let config = DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            max_files_per_species: 3,
            ..DiskManagerConfig::default()
        };
        let manager = DiskManager::new(config);
        let removed = manager.enforce_species_limits().expect("enforce limits");
        assert_eq!(removed, 2); // 5 - 3 = 2

        // Verify 3 files remain.
        let remaining: Vec<_> = std::fs::read_dir(&species_dir)
            .expect("read dir")
            .flatten()
            .filter(|e| e.path().is_file())
            .collect();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn enforce_species_limits_no_by_date_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            max_files_per_species: 5,
            ..DiskManagerConfig::default()
        };
        let manager = DiskManager::new(config);
        let result = manager.enforce_species_limits();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn purge_oldest_files_removes_oldest() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create By_Date directory with 20 files.
        let species_dir = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&species_dir).expect("create dirs");

        for i in 0..20 {
            let wav_path = species_dir.join(format!("clip_{i:02}.wav"));
            let header = create_minimal_wav_header();
            std::fs::write(&wav_path, &header).expect("write wav");
            let mtime = filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0);
            filetime::set_file_mtime(&wav_path, mtime).expect("set mtime");
        }

        let removed = purge_oldest_files(dir.path());
        // 10% of 20 = 2
        assert_eq!(removed, 2);
    }

    #[test]
    fn cleanup_empty_dirs_removes_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("create dirs");

        cleanup_empty_dirs(dir.path());

        // All empty nested dirs should be gone.
        assert!(!dir.path().join("a").exists());
    }

    #[test]
    fn disk_manager_run_stops_on_signal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            check_interval_secs: 1,
            purge_threshold: 99,
            ..DiskManagerConfig::default()
        };
        let manager = DiskManager::new(config);

        let (tx, rx) = mpsc::channel();

        // Run in a thread and immediately send stop.
        let handle = std::thread::spawn(move || {
            manager.run(&rx);
        });

        tx.send(()).expect("send stop");
        handle.join().expect("join");
    }

    /// Create a minimal valid WAV file (44-byte header, no data).
    fn create_minimal_wav_header() -> Vec<u8> {
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&36_u32.to_le_bytes()); // file size - 8
        header.extend_from_slice(b"WAVE");
        header.extend_from_slice(b"fmt ");
        header.extend_from_slice(&16_u32.to_le_bytes()); // fmt chunk size
        header.extend_from_slice(&1_u16.to_le_bytes());  // PCM
        header.extend_from_slice(&1_u16.to_le_bytes());  // mono
        header.extend_from_slice(&48000_u32.to_le_bytes()); // sample rate
        header.extend_from_slice(&96000_u32.to_le_bytes()); // byte rate
        header.extend_from_slice(&2_u16.to_le_bytes());  // block align
        header.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0_u32.to_le_bytes()); // data size
        header
    }
}
