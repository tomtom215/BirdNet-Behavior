//! Disk manager for automatic disk usage monitoring and purging.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::audio::capture::process::is_audio_file;
use crate::audio::capture::types::CaptureError;

use super::disk_usage;
use super::purge::{cleanup_empty_dirs, is_protected, purge_oldest_files};

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
    /// Directory to monitor (e.g. `~/BirdSongs/Extracted`).
    pub monitored_dir: PathBuf,
    /// Disk-usage percentage at which to trigger purge (default 95).
    pub purge_threshold: u8,
    /// Action to take when the threshold is exceeded.
    pub full_disk_action: FullDiskAction,
    /// Maximum recordings per species directory (0 = unlimited).
    pub max_files_per_species: u32,
    /// Interval between checks in seconds (default 60).
    pub check_interval_secs: u64,
    /// Paths to exclude from purge (never deleted).
    pub exclude_paths: Vec<PathBuf>,
    /// File names to protect from purge (locked recordings from DB).
    pub locked_file_names: Vec<String>,
}

impl Default for DiskManagerConfig {
    fn default() -> Self {
        Self {
            monitored_dir: PathBuf::from("BirdSongs/Extracted"),
            purge_threshold: 95,
            full_disk_action: FullDiskAction::Purge,
            max_files_per_species: 0,
            check_interval_secs: 60,
            exclude_paths: Vec::new(),
            locked_file_names: Vec::new(),
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
                let removed = purge_oldest_files(
                    &self.config.monitored_dir,
                    &self.config.exclude_paths,
                    &self.config.locked_file_names,
                );
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
        let mut species_files: std::collections::HashMap<
            String,
            Vec<(PathBuf, std::time::SystemTime)>,
        > = std::collections::HashMap::new();

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

                let species_name = species_entry.file_name().to_string_lossy().into_owned();

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
            let mut removed_this_species = 0;
            for (path, _) in &files {
                if removed_this_species >= to_remove {
                    break;
                }
                if is_protected(
                    path,
                    &self.config.exclude_paths,
                    &self.config.locked_file_names,
                ) {
                    continue;
                }
                if std::fs::remove_file(path).is_ok() {
                    tracing::debug!(
                        path = %path.display(),
                        species = %species,
                        "removed file (species limit)"
                    );
                    total_removed += 1;
                    removed_this_species += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

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
            ..DiskManagerConfig::default()
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
        let species_dir = dir.path().join("By_Date/2026-03-14/Test_Bird");
        std::fs::create_dir_all(&species_dir).expect("create dirs");

        for i in 0..5 {
            let wav_path = species_dir.join(format!("clip_{i}.wav"));
            let header = create_minimal_wav_header();
            std::fs::write(&wav_path, &header).expect("write wav");
            // Stagger modification times so we have a deterministic oldest.
            filetime::set_file_mtime(
                &wav_path,
                filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0),
            )
            .expect("set mtime");
        }
        let config = DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            max_files_per_species: 3,
            ..DiskManagerConfig::default()
        };
        let manager = DiskManager::new(config);
        let removed = manager.enforce_species_limits().expect("enforce limits");
        assert_eq!(removed, 2); // 5 - 3 = 2

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
    fn disk_manager_run_stops_on_signal() {
        let config = DiskManagerConfig {
            monitored_dir: PathBuf::from("/tmp"),
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
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&36_u32.to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16_u32.to_le_bytes());
        h.extend_from_slice(&1_u16.to_le_bytes());
        h.extend_from_slice(&1_u16.to_le_bytes());
        h.extend_from_slice(&48000_u32.to_le_bytes());
        h.extend_from_slice(&96000_u32.to_le_bytes());
        h.extend_from_slice(&2_u16.to_le_bytes());
        h.extend_from_slice(&16_u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&0_u32.to_le_bytes());
        h
    }
}
