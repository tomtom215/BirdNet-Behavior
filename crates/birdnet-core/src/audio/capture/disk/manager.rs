//! Disk manager for automatic disk usage monitoring and purging.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::audio::capture::process::is_audio_file;
use crate::audio::capture::types::CaptureError;

use super::disk_usage;
use super::purge::{
    cleanup_empty_dirs, is_protected, purge_flat_older_than, purge_flat_over_size,
    purge_oldest_files, purge_oldest_flat_files,
};

/// What to do when the disk reaches the purge threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullDiskAction {
    /// Delete oldest files to free space.
    Purge,
    /// Stop recording (signal the caller) instead of deleting.
    Keep,
}

/// Supplies the set of file names locked against purge, re-read once per
/// purge cycle.
///
/// The locked set is operator-driven and changes at runtime: `/admin/recordings`
/// → "lock" writes `is_locked` the moment somebody hears a clip worth keeping.
/// A `Vec` captured at process start therefore protects almost nothing — it
/// holds whatever was locked at the last reboot, so every clip locked since
/// then is fair game for the purge. Because `birdnet-core` must not depend on
/// `birdnet-db`, the database query is injected as this callback rather than
/// performed here.
pub type LockedFilesProvider = std::sync::Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Configuration for automatic disk management.
#[derive(Clone)]
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
    ///
    /// Used as the starting set and as the fallback whenever
    /// [`Self::locked_provider`] is `None`. Long-running callers should set a
    /// provider instead, so runtime lock changes are honoured.
    pub locked_file_names: Vec<String>,
    /// Optional callback re-read once per purge cycle to refresh
    /// [`Self::locked_file_names`]. See [`LockedFilesProvider`].
    pub locked_provider: Option<LockedFilesProvider>,
    /// Retention for *flat* raw-capture segments in `monitored_dir`, in seconds
    /// (0 = disabled). When set, segments older than this are drained every
    /// cycle so the RAM-backed stream dir self-empties. Only ever set by the
    /// caller for the *transient* watch/stream dir — never a persistent
    /// recordings dir, whose files must not be deleted by age.
    pub stream_retention_secs: u64,
    /// Hard ceiling on the total bytes of *flat* raw-capture segments in
    /// `monitored_dir` (0 = disabled). Oldest segments are dropped first when
    /// exceeded. Like [`Self::stream_retention_secs`], only set for the transient
    /// stream dir.
    pub stream_max_bytes: u64,
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
            locked_provider: None,
            stream_retention_secs: 0,
            stream_max_bytes: 0,
        }
    }
}

// Hand-rolled because `locked_provider` holds a closure, which cannot derive
// `Debug`. Every other field is reproduced verbatim so the config still prints
// usefully in traces; the provider renders as its presence, which is the part
// an operator reading a log needs to know.
impl std::fmt::Debug for DiskManagerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskManagerConfig")
            .field("monitored_dir", &self.monitored_dir)
            .field("purge_threshold", &self.purge_threshold)
            .field("full_disk_action", &self.full_disk_action)
            .field("max_files_per_species", &self.max_files_per_species)
            .field("check_interval_secs", &self.check_interval_secs)
            .field("exclude_paths", &self.exclude_paths)
            .field("locked_file_names", &self.locked_file_names)
            .field("locked_provider", &self.locked_provider.is_some())
            .field("stream_retention_secs", &self.stream_retention_secs)
            .field("stream_max_bytes", &self.stream_max_bytes)
            .finish()
    }
}

/// Automatic disk manager that periodically checks usage and purges old files.
///
/// `Clone` so each purge cycle can be handed a manager carrying that cycle's
/// freshly-read locked-file set (see the private `with_fresh_locks`; not an
/// intra-doc link because this type is public and that helper is not).
#[derive(Debug, Clone)]
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
                let mut removed = purge_oldest_files(
                    &self.config.monitored_dir,
                    &self.config.exclude_paths,
                    &self.config.locked_file_names,
                );
                // The raw capture segments sit FLAT in the watch/stream dir (no
                // `By_Date/` subtree), so `purge_oldest_files` above reclaims
                // none of them. Purge the oldest flat segments too — without
                // this the disk-full safety net frees nothing on the RAM-backed
                // stream dir and the tmpfs runs to 100 % (breaking capture and
                // even `apt` on a Pi).
                removed += purge_oldest_flat_files(
                    &self.config.monitored_dir,
                    &self.config.exclude_paths,
                    &self.config.locked_file_names,
                );
                cleanup_empty_dirs(&self.config.monitored_dir);
                Ok(removed)
            }
        }
    }

    /// Drain the transient raw-capture segments from the stream dir.
    ///
    /// The watch/stream dir accumulates continuous raw audio segments that the
    /// detection pipeline reads but never deletes; left unbounded they fill the
    /// RAM-backed tmpfs to 100 %. This keeps the dir bounded two ways, each a
    /// no-op unless configured — so it only ever acts on the *transient* stream
    /// dir (the caller leaves both zero for a persistent recordings dir):
    ///
    ///   - **age**: drop segments older than `stream_retention_secs` (steady-state
    ///     drain; the window is far longer than the pipeline's processing latency,
    ///     so an unprocessed segment is never removed);
    ///   - **size**: drop the oldest segments until the dir is under
    ///     `stream_max_bytes` (a hard ceiling for high-ingest / backed-up runs).
    ///
    /// Returns the number of files removed.
    pub fn cleanup_stream_segments(&self) -> u32 {
        let mut removed = 0_u32;
        if self.config.stream_retention_secs > 0 {
            removed += purge_flat_older_than(
                &self.config.monitored_dir,
                Duration::from_secs(self.config.stream_retention_secs),
                &self.config.exclude_paths,
                &self.config.locked_file_names,
            );
        }
        if self.config.stream_max_bytes > 0 {
            removed += purge_flat_over_size(
                &self.config.monitored_dir,
                self.config.stream_max_bytes,
                &self.config.exclude_paths,
                &self.config.locked_file_names,
            );
        }
        removed
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

    /// Return a manager for this cycle whose locked-file set is freshly read
    /// from [`DiskManagerConfig::locked_provider`].
    ///
    /// Returns `self` unchanged when no provider is configured, so the static
    /// `locked_file_names` path (and every existing test) behaves exactly as
    /// before. Cloning the config once per cycle — a handful of paths and a
    /// string vector, at most once a minute — costs nothing next to the
    /// directory walk that follows, and keeps every purge entry point reading
    /// one consistent snapshot without interior mutability.
    fn with_fresh_locks(&self) -> std::borrow::Cow<'_, Self> {
        let Some(provider) = self.config.locked_provider.as_ref() else {
            return std::borrow::Cow::Borrowed(self);
        };
        let mut config = self.config.clone();
        config.locked_file_names = provider();
        std::borrow::Cow::Owned(Self::new(config))
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

            // Re-read the operator's locked clips before anything is deleted:
            // a clip locked since the last cycle must be protected by this one.
            let cycle = self.with_fresh_locks();

            // Proactively drain the transient stream segments first, so
            // steady-state usage stays low regardless of the disk-full backstop.
            cycle.cleanup_stream_segments();

            if let Err(e) = cycle.check_and_purge() {
                tracing::error!(error = %e, "disk manager check_and_purge failed");
            }

            if let Err(e) = cycle.enforce_species_limits() {
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
        // Stream cleanup is opt-in (0 = disabled) so it never touches a
        // persistent recordings dir unless the caller enables it for the
        // transient stream dir.
        assert_eq!(config.stream_retention_secs, 0);
        assert_eq!(config.stream_max_bytes, 0);
    }

    #[test]
    fn fresh_locks_are_read_from_the_provider_each_cycle() {
        // The regression: the locked set used to be a Vec captured at process
        // start, so a clip locked from /admin/recordings after boot was not
        // protected. Drive the provider twice and prove the second cycle sees
        // the newly-locked name.
        use std::sync::{Arc, Mutex};

        let locked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let provider_view = Arc::clone(&locked);
        let manager = DiskManager::new(DiskManagerConfig {
            locked_provider: Some(std::sync::Arc::new(move || {
                provider_view.lock().unwrap().clone()
            })),
            ..DiskManagerConfig::default()
        });

        assert!(
            manager
                .with_fresh_locks()
                .config()
                .locked_file_names
                .is_empty(),
            "nothing is locked yet"
        );

        // Operator locks a clip while the station is running.
        locked.lock().unwrap().push("keep-me.wav".to_string());

        assert_eq!(
            manager.with_fresh_locks().config().locked_file_names,
            vec!["keep-me.wav".to_string()],
            "a clip locked after startup must be visible to the very next cycle"
        );
    }

    #[test]
    fn without_a_provider_the_static_locked_list_is_kept() {
        // Counter-test: callers that pass a plain Vec (and every existing
        // test) must behave exactly as before — no provider, no surprise
        // clearing of the list.
        let manager = DiskManager::new(DiskManagerConfig {
            locked_file_names: vec!["static.wav".to_string()],
            ..DiskManagerConfig::default()
        });
        assert_eq!(
            manager.with_fresh_locks().config().locked_file_names,
            vec!["static.wav".to_string()]
        );
    }

    #[test]
    fn a_locked_segment_survives_the_stream_drain() {
        // End-to-end proof that the refreshed set actually protects a file:
        // an ancient segment that would otherwise be drained by age is kept
        // because the provider reports it as locked.
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["locked.wav", "unlocked.wav"] {
            let p = dir.path().join(name);
            std::fs::write(&p, vec![0_u8; 128]).expect("write");
            filetime::set_file_mtime(&p, filetime::FileTime::from_unix_time(1_000_000, 0))
                .expect("mtime");
        }
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            stream_retention_secs: 60,
            locked_provider: Some(std::sync::Arc::new(|| vec!["locked.wav".to_string()])),
            ..DiskManagerConfig::default()
        });

        let removed = manager.with_fresh_locks().cleanup_stream_segments();

        assert_eq!(removed, 1, "only the unlocked segment is drained");
        assert!(
            dir.path().join("locked.wav").exists(),
            "a locked clip must survive the drain"
        );
        assert!(!dir.path().join("unlocked.wav").exists());
    }

    #[test]
    fn cleanup_stream_segments_is_noop_when_disabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("2026-06-10-birdnet-00:00:00.wav"), b"x").expect("write");
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            ..DiskManagerConfig::default() // both stream limits 0
        });
        assert_eq!(manager.cleanup_stream_segments(), 0);
        assert!(
            dir.path().join("2026-06-10-birdnet-00:00:00.wav").exists(),
            "disabled cleanup must not delete anything"
        );
    }

    #[test]
    fn cleanup_stream_segments_drains_aged_flat_segments() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Two ancient segments (1970) and one fresh (now).
        for name in ["old_a.wav", "old_b.wav"] {
            let p = dir.path().join(name);
            std::fs::write(&p, vec![0_u8; 128]).expect("write");
            filetime::set_file_mtime(&p, filetime::FileTime::from_unix_time(1_000_000, 0))
                .expect("mtime");
        }
        std::fs::write(dir.path().join("fresh.wav"), vec![0_u8; 128]).expect("write");

        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            stream_retention_secs: 3600,
            ..DiskManagerConfig::default()
        });
        assert_eq!(manager.cleanup_stream_segments(), 2);
        assert!(dir.path().join("fresh.wav").exists(), "recent segment kept");
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
