//! Disk manager for automatic disk usage monitoring and purging.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use super::is_kept_audio_file as is_audio_file;
use crate::audio::capture::types::CaptureError;

use super::disk_usage;
use super::purge::{
    cleanup_empty_dirs, is_protected, purge_flat_older_than, purge_flat_over_size,
    purge_oldest_files, purge_oldest_flat_files, purge_recordings,
};

/// How the disk-full purge chooses what goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgePolicy {
    /// Oldest files first, regardless of what they are. Right for the
    /// transient raw-capture segments, which carry no species and are worth
    /// nothing once read.
    OldestFirst,
    /// Keep every species (S-2): the most-recorded species gives up clips
    /// first, lowest confidence then oldest within it, and no species is ever
    /// taken below `floor` clips. Right for the extracted clips, where
    /// oldest-first deleted the single clip of the year's rarest bird before
    /// the thousandth clip of the commonest.
    KeepEverySpecies {
        /// Clips a species always keeps, however full the disk is.
        floor: u32,
    },
}

/// The per-species floor the recordings purge keeps when nothing sets one.
pub const DEFAULT_PURGE_SPECIES_FLOOR: u32 = 5;

/// Where kept raw audio lives, under the recordings directory (R-4).
pub const RAW_KEEP_SUBDIR: &str = "raw";

/// Keep a share of the raw segments the stream drain would delete (R-4).
///
/// The raw audio used to be worth nothing once analysed: only what already
/// triggered a detection survived, so the archive was selected by the very
/// model under test and nothing could be re-scored. Kept at a duty cycle —
/// one segment in `every` — it is what a season can be re-analysed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawKeep {
    /// Where the kept segments go; created on first use.
    pub dir: PathBuf,
    /// Keep one aged segment in this many, in capture order; `1` keeps all.
    pub every: u32,
    /// Segments the human-sound rule masked speech in, which are drained and
    /// never kept (RES-28).
    pub speech: SpeechMarks,
}

/// Stream segments the human-sound rule masked speech in (RES-28).
///
/// Shared between the daemon, which marks a segment when the rule masks any
/// window of it, and the stream drain, which keeps no raw copy of a marked
/// segment. The keep copied a segment verbatim, so a station that masks speech from
/// analysis still wrote the conversation to its data disk. A mark is spent
/// when the drain removes its segment, and marks for segments gone by any
/// other route are forgotten at the end of each drain, so the set holds at
/// most the segments still in the stream directory. Above [`Self::CAP`] the
/// set is emptied, which bounds it on a station whose drain never runs; the
/// cost is that a few segments marked just before may be kept.
///
/// Equal when shared: two handles are the same set or different ones.
#[derive(Debug, Clone, Default)]
pub struct SpeechMarks(Arc<std::sync::Mutex<std::collections::HashSet<std::ffi::OsString>>>);

impl PartialEq for SpeechMarks {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SpeechMarks {}

impl SpeechMarks {
    /// Marks held at most: about seventeen hours of 15 s segments, far past
    /// any stream retention.
    pub const CAP: usize = 4096;

    /// Mark the segment at `path` as holding speech.
    pub fn mark(&self, path: &Path) {
        let Some(name) = path.file_name() else {
            return;
        };
        if let Ok(mut set) = self.0.lock() {
            if set.len() >= Self::CAP {
                set.clear();
            }
            set.insert(name.to_owned());
        }
    }

    /// Whether the segment named `name` was marked, spending the mark.
    pub fn take(&self, name: &std::ffi::OsStr) -> bool {
        self.0.lock().is_ok_and(|mut set| set.remove(name))
    }

    /// Forget the marks of segments no longer in `dir`.
    pub fn forget_missing(&self, dir: &Path) {
        if let Ok(mut set) = self.0.lock() {
            set.retain(|name| dir.join(name).exists());
        }
    }

    /// Whether no segment is marked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.lock().map_or(true, |set| set.is_empty())
    }
}

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
///
/// `None` means the set **could not be read** (the database was busy, failing,
/// or gone), and the cycle then deletes nothing. It used to be impossible to
/// say: the station's provider answered a failed query with an empty set, and
/// "nothing is locked" is exactly the answer that lets a disk-full purge
/// delete every clip its owner locked — irreversibly, and at the one moment
/// the disk is under stress.
pub type LockedFilesProvider = std::sync::Arc<dyn Fn() -> Option<Vec<String>> + Send + Sync>;

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
    /// Where to publish that the purge has stopped achieving anything (PR-7),
    /// for the health surfaces; `None` publishes nowhere.
    pub ineffective_flag: Option<Arc<AtomicBool>>,
    /// What the disk-full purge takes first (S-2).
    pub purge_policy: PurgePolicy,
    /// Keep a share of the raw segments the age drain removes (R-4); `None`
    /// keeps nothing. Only meaningful on the transient stream directory.
    pub raw_keep: Option<RawKeep>,
    /// Where to publish that recording is on hold because the disk is full and
    /// [`FullDiskAction::Keep`] was chosen; `None` publishes nowhere. The
    /// capture supervisor reads it and stops its sources while it is set.
    pub hold_flag: Option<Arc<AtomicBool>>,
    /// Where to publish that the disk is over the purge threshold and the
    /// last disk-full purge found nothing it is allowed to delete: every clip
    /// left is locked, reviewed, at its species floor or excluded, or there
    /// are none. `None` publishes nowhere.
    ///
    /// Not a latch, unlike [`Self::ineffective_flag`]: every pass looks
    /// again, so a clip that becomes deletable is still purged, and the flag
    /// clears on the first pass that removes one or the first reading below
    /// the threshold.
    pub nothing_purgeable_flag: Option<Arc<AtomicBool>>,
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
            ineffective_flag: None,
            purge_policy: PurgePolicy::KeepEverySpecies {
                floor: DEFAULT_PURGE_SPECIES_FLOOR,
            },
            raw_keep: None,
            hold_flag: None,
            nothing_purgeable_flag: None,
        }
    }
}

/// How far below the threshold usage must fall before a held recording
/// resumes, in percentage points.
///
/// Without a margin a disk sitting at the threshold would stop and restart
/// capture every cycle as one clip lands and the next is not written.
pub const HOLD_RESUME_MARGIN_PERCENT: f64 = 2.0;

/// Whether `percent` used is far enough below `threshold` for a held recording
/// to resume (see [`HOLD_RESUME_MARGIN_PERCENT`]).
fn clear_of_threshold(percent: f64, threshold: f64) -> bool {
    percent < release_below(threshold)
}

/// The usage, in per cent, a held recording must fall **below** before it
/// resumes: the threshold less [`HOLD_RESUME_MARGIN_PERCENT`].
///
/// The one place that number is computed, so the message that tells the
/// operator what to do and the rule that decides are the same expression. They
/// were not: both messages said "free space or raise the threshold and
/// recording resumes", which is true only for a disk freed by more than the
/// margin, or, for a raised threshold, after the restart that applying it needs
/// (which clears the hold, as the hold is not stored).
#[must_use]
pub fn release_below(threshold: f64) -> f64 {
    threshold - HOLD_RESUME_MARGIN_PERCENT
}

/// [`hold_advice`] without the absolute numbers, for a surface that does not
/// know which threshold is in force (a command-line flag can override the
/// setting the page shows).
#[must_use]
pub fn hold_advice_relative() -> String {
    format!(
        "Free space until usage is at least {HOLD_RESUME_MARGIN_PERCENT:.0} points below the \
         purge threshold (the margin stops a disk at the threshold from starting and stopping \
         recording) and recording resumes by itself. Or raise the threshold in Settings; that \
         takes effect after a restart"
    )
}

/// What an operator can do about a held recording, and when it resumes, for the
/// log.
///
/// `threshold` is the configured purge threshold in per cent. Says the margin
/// outright; a disk freed to 94 % under a 95 % threshold does not resume, and
/// "recording resumes by itself" without the number is how that is mistaken for
/// a fault.
#[must_use]
pub fn hold_advice(threshold: f64) -> String {
    format!(
        "Free space until usage is below {:.0} % (the {:.0} % threshold less a margin of {:.0}          points, so a disk at the threshold does not stop and start recording) and recording          resumes by itself. Or raise the threshold in Settings; that takes effect after a restart",
        release_below(threshold),
        threshold,
        HOLD_RESUME_MARGIN_PERCENT,
    )
}

/// What an operator can do when the disk is over the purge threshold and the
/// purge found nothing it is allowed to delete (N25-01), for the log and the
/// station-health condition.
///
/// It says only what the purge knows: that nothing it may delete was there.
/// Whether the space is in protected clips or in something else entirely, it
/// cannot tell, so the advice covers both.
pub const NOTHING_PURGEABLE_ADVICE: &str = "Every recording left is one the purge keeps: \
     locked, the audio of a detection someone confirmed, rejected or commented on, one of the \
     clips a species always keeps (PURGE_SPECIES_FLOOR), or under a path excluded with \
     BIRDNET_DISK_EXCLUDE; or one it tried and could not delete (the log says which); or \
     there are no recordings left and the space is in something else. To make room: unlock clips you no longer need, lower PURGE_SPECIES_FLOOR (applies \
     after a restart), start the station with BIRDNET_RETAIN_REVIEWED_AUDIO=false to let the \
     purge take reviewed audio, or free or move other data (`du -sh <data dir>/*` shows what \
     is filling the disk). The purge looks again every minute and deletes what it may as \
     soon as there is something";

/// Whether a purge pass is worth running, from what the last one achieved
/// (PR-7).
///
/// When the card fills for a reason that is not recordings — the database,
/// the analytics store, the backup ring, a runaway log — the purge deleted 10 %
/// of the operator's clips every minute until every one was gone and the disk
/// was still 96.9 % full. A pass that removed files and moved usage by nothing
/// is proof the space is elsewhere, and deleting more clips will not find it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PurgeGovernor {
    /// Set by a pass that removed files without lowering usage; cleared when
    /// usage falls below the threshold again by any means.
    ineffective: bool,
}

impl PurgeGovernor {
    /// Below one fifth of a per cent is the filesystem breathing, not a purge
    /// working: a 3-second clip on a 32 GB card is 0.001 %.
    const MIN_EFFECT_PERCENT: f64 = 0.2;

    /// Whether to purge now, at `percent` used against `threshold`.
    pub fn should_purge(&mut self, percent: f64, threshold: f64) -> bool {
        if percent < threshold {
            self.ineffective = false;
            return false;
        }
        !self.ineffective
    }

    /// What the pass achieved: `removed` files, usage from `before` to
    /// `after` per cent. Returns whether the pass was ineffective.
    pub fn observe(&mut self, removed: u32, before: f64, after: f64) -> bool {
        if removed > 0 && before - after < Self::MIN_EFFECT_PERCENT {
            self.ineffective = true;
        }
        self.ineffective
    }

    /// Whether the last pass found the space to be somewhere the purge
    /// cannot reach.
    #[must_use]
    pub const fn is_ineffective(&self) -> bool {
        self.ineffective
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
            .field("ineffective_flag", &self.ineffective_flag.is_some())
            .field("purge_policy", &self.purge_policy)
            .field("raw_keep", &self.raw_keep)
            .field("hold_flag", &self.hold_flag.is_some())
            .field(
                "nothing_purgeable_flag",
                &self.nothing_purgeable_flag.is_some(),
            )
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
    /// What the last purge achieved; shared across the per-cycle copies
    /// [`Self::with_fresh_locks`] makes, so the verdict survives a cycle.
    governor: Arc<Mutex<PurgeGovernor>>,
    /// Aged segments the drain has seen, for the keep cadence (R-4).
    raw_seen: Arc<Mutex<u64>>,
}

impl DiskManager {
    /// Create a new disk manager with the given configuration.
    #[must_use]
    pub fn new(config: DiskManagerConfig) -> Self {
        Self {
            config,
            governor: Arc::new(Mutex::new(PurgeGovernor::default())),
            raw_seen: Arc::new(Mutex::new(0)),
        }
    }

    /// Whether the last purge removed recordings without lowering usage.
    #[must_use]
    pub fn purge_ineffective(&self) -> bool {
        self.governor.lock().is_ok_and(|g| g.is_ineffective())
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
    /// Returns [`CaptureError`] if disk usage cannot be determined.
    ///
    /// With [`FullDiskAction::Keep`] over the threshold this deletes nothing
    /// and returns `Ok(0)`; the hold is signalled through
    /// [`DiskManagerConfig::hold_flag`], which the capture supervisor reads.
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
            // Usage fell, by whatever means: the purge is allowed again.
            if let Ok(mut g) = self.governor.lock() {
                g.should_purge(percent, threshold);
            }
            self.publish_ineffective(false);
            self.publish_nothing_purgeable(false, percent);
            // A held recording resumes only once usage is clear of the
            // threshold by a margin (see `HOLD_RESUME_MARGIN_PERCENT`).
            if clear_of_threshold(percent, threshold) {
                self.release_hold(percent);
            }
            return Ok(0);
        }

        tracing::warn!(
            used_pct = format!("{percent:.1}"),
            threshold = self.config.purge_threshold,
            "disk usage exceeds threshold"
        );

        match self.config.full_disk_action {
            FullDiskAction::Keep => {
                // BirdNET-Pi's `FULL_DISK=keep`: delete nothing and stop
                // recording (`disk_check.sh` ran `stop_core_services.sh`).
                // The deletion arms below are not reached, which is the whole
                // promise of the setting: whatever else happens to the disk,
                // no clip is removed because it is full.
                self.hold_recording(percent);
                Ok(0)
            }
            FullDiskAction::Purge => {
                // PR-7: a pass that removed recordings and moved usage by
                // nothing proved the space is elsewhere; deleting more finds
                // nothing and costs the operator their clips.
                let allowed = self
                    .governor
                    .lock()
                    .map_or(true, |mut g| g.should_purge(percent, threshold));
                if !allowed {
                    self.publish_ineffective(true);
                    tracing::error!(
                        used_pct = format!("{percent:.1}"),
                        dir = %self.config.monitored_dir.display(),
                        "disk is over the purge threshold and the last purge freed nothing: the \
                         space is not in recordings; not deleting more"
                    );
                    return Ok(0);
                }
                let removed = match self.config.purge_policy {
                    PurgePolicy::KeepEverySpecies { floor } => purge_recordings(
                        &self.config.monitored_dir,
                        floor,
                        &self.config.exclude_paths,
                        &self.config.locked_file_names,
                    ),
                    PurgePolicy::OldestFirst => {
                        let by_date = purge_oldest_files(
                            &self.config.monitored_dir,
                            &self.config.exclude_paths,
                            &self.config.locked_file_names,
                        );
                        // The raw capture segments sit FLAT in the watch/stream
                        // dir (no `By_Date/` subtree), so `purge_oldest_files`
                        // above reclaims none of them. Purge the oldest flat
                        // segments too — without this the disk-full safety net
                        // frees nothing on the RAM-backed stream dir and the
                        // tmpfs runs to 100 % (breaking capture and even `apt`
                        // on a Pi).
                        by_date
                            + purge_oldest_flat_files(
                                &self.config.monitored_dir,
                                &self.config.exclude_paths,
                                &self.config.locked_file_names,
                            )
                    }
                };
                cleanup_empty_dirs(&self.config.monitored_dir);
                let after =
                    disk_usage(&self.config.monitored_dir).map_or(percent, |u| u.used_percent());
                let ineffective = self
                    .governor
                    .lock()
                    .is_ok_and(|mut g| g.observe(removed, percent, after));
                self.publish_ineffective(ineffective);
                // N25-01: over the threshold and nothing removed is not the
                // latch above (that needs a removal) and must not become one:
                // the next pass looks again, so a clip unlocked meanwhile is
                // still purged.
                self.publish_nothing_purgeable(removed == 0, percent);
                if ineffective {
                    tracing::error!(
                        removed,
                        before_pct = format!("{percent:.1}"),
                        after_pct = format!("{after:.1}"),
                        dir = %self.config.monitored_dir.display(),
                        "purge removed recordings and usage did not fall: the space is not in \
                         recordings (database, analytics store, backups, logs); stopping"
                    );
                }
                Ok(removed)
            }
        }
    }

    /// Stop recording until the disk has room again. Says so once per hold,
    /// loudly, because a station that has stopped listening must not be able
    /// to do it quietly.
    fn hold_recording(&self, percent: f64) {
        let Some(flag) = &self.config.hold_flag else {
            tracing::error!(
                used_pct = format!("{percent:.1}"),
                dir = %self.config.monitored_dir.display(),
                "disk is over the threshold and the station is set to keep everything, but \
                 nothing here can stop the recording; no clip was deleted"
            );
            return;
        };
        if !flag.swap(true, Ordering::Relaxed) {
            tracing::error!(
                used_pct = format!("{percent:.1}"),
                threshold = self.config.purge_threshold,
                dir = %self.config.monitored_dir.display(),
                advice = %hold_advice(f64::from(self.config.purge_threshold)),
                "disk is full and the station is set to keep everything: recording is on \
                 hold and no clip will be deleted"
            );
        }
    }

    /// Let a held recording resume.
    fn release_hold(&self, percent: f64) {
        if let Some(flag) = &self.config.hold_flag
            && flag.swap(false, Ordering::Relaxed)
        {
            tracing::warn!(
                used_pct = format!("{percent:.1}"),
                dir = %self.config.monitored_dir.display(),
                "the disk has room again: recording resumes"
            );
        }
    }

    /// Publish the governor's verdict where the health surfaces read it.
    fn publish_ineffective(&self, ineffective: bool) {
        if let Some(flag) = &self.config.ineffective_flag {
            flag.store(ineffective, Ordering::Relaxed);
        }
    }

    /// Publish whether the last pass over the threshold found nothing it was
    /// allowed to delete (N25-01), and say so once per episode, loudly: the
    /// disk goes on filling and the purge can do nothing about it.
    fn publish_nothing_purgeable(&self, nothing: bool, percent: f64) {
        let Some(flag) = &self.config.nothing_purgeable_flag else {
            return;
        };
        if flag.swap(nothing, Ordering::Relaxed) == nothing {
            return;
        }
        if nothing {
            tracing::error!(
                used_pct = format!("{percent:.1}"),
                threshold = self.config.purge_threshold,
                dir = %self.config.monitored_dir.display(),
                advice = NOTHING_PURGEABLE_ADVICE,
                "disk is over the purge threshold and the purge found nothing it is allowed \
                 to delete; the disk will go on filling"
            );
        } else {
            tracing::info!(
                used_pct = format!("{percent:.1}"),
                dir = %self.config.monitored_dir.display(),
                "the disk purge can reclaim space again, or the disk is below its threshold"
            );
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
                self.config
                    .raw_keep
                    .as_ref()
                    .map(|policy| (policy, &*self.raw_seen)),
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
    ///
    /// `None` when the provider could not read the set: the caller must then
    /// delete nothing this cycle.
    fn with_fresh_locks(&self) -> Option<std::borrow::Cow<'_, Self>> {
        let Some(provider) = self.config.locked_provider.as_ref() else {
            return Some(std::borrow::Cow::Borrowed(self));
        };
        let mut config = self.config.clone();
        config.locked_file_names = provider()?;
        Some(std::borrow::Cow::Owned(Self {
            config,
            governor: Arc::clone(&self.governor),
            raw_seen: Arc::clone(&self.raw_seen),
        }))
    }

    /// One pass of the loop: drain, purge, enforce the species cap — or, when
    /// the locked set cannot be read, nothing at all. Returns whether it ran.
    fn run_cycle(&self) -> bool {
        // Re-read the operator's locked clips before anything is deleted:
        // a clip locked since the last cycle must be protected by this one.
        let Some(cycle) = self.with_fresh_locks() else {
            tracing::error!(
                dir = %self.config.monitored_dir.display(),
                "could not read which clips are locked; deleting nothing this cycle \
                 rather than risk a locked clip (retrying next cycle)"
            );
            return false;
        };

        // Proactively drain the transient stream segments first, so
        // steady-state usage stays low regardless of the disk-full backstop.
        cycle.cleanup_stream_segments();

        if let Err(e) = cycle.check_and_purge() {
            tracing::error!(error = %e, "disk manager check_and_purge failed");
        }

        if let Err(e) = cycle.enforce_species_limits() {
            tracing::error!(error = %e, "disk manager enforce_species_limits failed");
        }
        true
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

            self.run_cycle();
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
                Some(provider_view.lock().unwrap().clone())
            })),
            ..DiskManagerConfig::default()
        });

        assert!(
            manager
                .with_fresh_locks()
                .expect("locks read")
                .config()
                .locked_file_names
                .is_empty(),
            "nothing is locked yet"
        );

        // Operator locks a clip while the station is running.
        locked.lock().unwrap().push("keep-me.wav".to_string());

        assert_eq!(
            manager
                .with_fresh_locks()
                .expect("locks read")
                .config()
                .locked_file_names,
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
            manager
                .with_fresh_locks()
                .expect("locks read")
                .config()
                .locked_file_names,
            vec!["static.wav".to_string()]
        );
    }

    /// PR-1 / S-3: the lease the daemon holds while reading a segment is what
    /// the stream drain consults; the segment survives exactly while the
    /// guard is alive.
    #[test]
    fn a_segment_under_analysis_survives_the_stream_drain() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["reading.wav", "done.wav"] {
            let p = dir.path().join(name);
            std::fs::write(&p, vec![0_u8; 128]).expect("write");
            filetime::set_file_mtime(&p, filetime::FileTime::from_unix_time(1_000_000, 0))
                .expect("mtime");
        }
        let table = crate::detection::daemon::InFlight::new();
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            stream_retention_secs: 60,
            locked_provider: Some({
                let table = table.clone();
                std::sync::Arc::new(move || Some(table.names()))
            }),
            ..DiskManagerConfig::default()
        });
        let lease = table.claim(&dir.path().join("reading.wav"));
        assert_eq!(
            manager
                .with_fresh_locks()
                .expect("locks read")
                .cleanup_stream_segments(),
            1
        );
        assert!(
            dir.path().join("reading.wav").exists(),
            "under analysis: kept"
        );
        assert!(!dir.path().join("done.wav").exists());
        drop(lease);
        assert_eq!(
            manager
                .with_fresh_locks()
                .expect("locks read")
                .cleanup_stream_segments(),
            1,
            "released: drained"
        );
        assert!(!dir.path().join("reading.wav").exists());
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
            locked_provider: Some(std::sync::Arc::new(|| Some(vec!["locked.wav".to_string()]))),
            ..DiskManagerConfig::default()
        });

        let removed = manager
            .with_fresh_locks()
            .expect("locks read")
            .cleanup_stream_segments();

        assert_eq!(removed, 1, "only the unlocked segment is drained");
        assert!(
            dir.path().join("locked.wav").exists(),
            "a locked clip must survive the drain"
        );
        assert!(!dir.path().join("unlocked.wav").exists());
    }

    #[test]
    fn a_cycle_that_cannot_read_the_locks_deletes_nothing() {
        // The segments are ancient, so the drain would take both. The provider
        // cannot say which are locked — and "unknown" must not be read as
        // "none": the cycle deletes nothing and says it skipped.
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
            locked_provider: Some(std::sync::Arc::new(|| None)),
            ..DiskManagerConfig::default()
        });

        assert!(!manager.run_cycle(), "the cycle must report it skipped");
        assert!(dir.path().join("locked.wav").exists());
        assert!(dir.path().join("unlocked.wav").exists());

        // Counterpart: once the set is readable, the same cycle runs.
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            stream_retention_secs: 60,
            locked_provider: Some(std::sync::Arc::new(|| Some(vec!["locked.wav".to_string()]))),
            ..DiskManagerConfig::default()
        });
        assert!(manager.run_cycle());
        assert!(dir.path().join("locked.wav").exists());
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

    /// A directory of flat clips named as the extractor names them.
    fn clips(dir: &std::path::Path, n: u32) -> Vec<PathBuf> {
        (0..n)
            .map(|i| {
                let p = dir.join(format!(
                    "Eurasian_Robin-85-2026-03-01-birdnet-06:00:{i:02}.wav"
                ));
                std::fs::write(&p, create_minimal_wav_header()).expect("write clip");
                filetime::set_file_mtime(
                    &p,
                    filetime::FileTime::from_unix_time(1_000_000 + i64::from(i), 0),
                )
                .expect("mtime");
                p
            })
            .collect()
    }

    /// A disk that is always over its threshold: 0 % is below any real usage.
    fn always_full(
        dir: &std::path::Path,
        action: FullDiskAction,
        hold_flag: Option<Arc<AtomicBool>>,
    ) -> DiskManager {
        DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.to_path_buf(),
            purge_threshold: 0,
            full_disk_action: action,
            purge_policy: PurgePolicy::OldestFirst,
            hold_flag,
            ..DiskManagerConfig::default()
        })
    }

    /// `FULL_DISK=keep`: the disk is full, and not one clip is removed for it.
    #[test]
    fn keep_deletes_no_clip_however_full_the_disk_is() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = clips(dir.path(), 12);
        let hold = Arc::new(AtomicBool::new(false));
        let manager = always_full(dir.path(), FullDiskAction::Keep, Some(Arc::clone(&hold)));

        for _ in 0..3 {
            assert!(manager.run_cycle());
        }
        assert!(
            files.iter().all(|f| f.exists()),
            "a station set to keep everything lost a clip to a full disk"
        );
        assert!(
            hold.load(Ordering::Relaxed),
            "recording must be held when the disk is full and nothing is deleted"
        );
    }

    /// The counterpart that makes the one above mean something: the same
    /// files, the same full disk, and `Purge` removes clips. Without it
    /// "nothing was deleted" could be the fixture finding nothing to delete.
    #[test]
    fn purge_on_the_same_full_disk_does_delete_clips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = clips(dir.path(), 12);
        let hold = Arc::new(AtomicBool::new(false));
        let manager = always_full(dir.path(), FullDiskAction::Purge, Some(Arc::clone(&hold)));

        assert!(manager.run_cycle());
        let left = files.iter().filter(|f| f.exists()).count();
        assert!(left < 12, "the purge removed nothing from a full disk");
        assert!(
            !hold.load(Ordering::Relaxed),
            "a purging station never holds its recording"
        );
    }

    /// Keep with no flag to raise (a caller that cannot stop recording) still
    /// deletes nothing: the promise does not depend on the signal.
    #[test]
    fn keep_without_a_hold_flag_still_deletes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = clips(dir.path(), 5);
        let manager = always_full(dir.path(), FullDiskAction::Keep, None);
        assert!(manager.run_cycle());
        assert!(files.iter().all(|f| f.exists()));
    }

    /// M2: the advice and the rule are one expression. A message that says
    /// "below 93 %" must be true at 92.9 and false at 93.0, whatever the
    /// threshold; the old text promised a resume at "the threshold".
    #[test]
    fn the_hold_advice_states_the_number_the_release_rule_uses() {
        for t in [50.0, 80.0, 95.0, 99.0] {
            let edge = release_below(t);
            assert!(clear_of_threshold(edge - 0.01, t), "{t}");
            assert!(!clear_of_threshold(edge, t), "{t}");
            let advice = hold_advice(t);
            assert!(advice.contains(&format!("below {edge:.0} %")), "{advice}");
            assert!(advice.contains(&format!("{t:.0} % threshold")), "{advice}");
            assert!(advice.contains("restart"), "{advice}");
        }
        assert!(hold_advice_relative().contains("2 points below"));
    }

    /// A hold is lifted once the disk is clear of the threshold, not at the
    /// first reading under it.
    #[test]
    fn a_hold_lifts_only_clear_of_the_threshold() {
        assert!(!clear_of_threshold(95.0, 95.0));
        assert!(!clear_of_threshold(94.9, 95.0));
        assert!(
            !clear_of_threshold(93.0, 95.0),
            "exactly the margin is not clear"
        );
        assert!(clear_of_threshold(92.9, 95.0));

        // End to end on a real disk: a threshold of 100 is far above any
        // reading, so a hold left over from before is lifted.
        let dir = tempfile::tempdir().expect("tempdir");
        let hold = Arc::new(AtomicBool::new(true));
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            purge_threshold: 100,
            full_disk_action: FullDiskAction::Keep,
            hold_flag: Some(Arc::clone(&hold)),
            ..DiskManagerConfig::default()
        });
        manager.check_and_purge().expect("check");
        assert!(!hold.load(Ordering::Relaxed), "recording should resume");
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

    /// The gate for PR-7. The governor's decisions, one pass at a time:
    /// a pass that removed files and moved usage by nothing stops the next;
    /// a pass that lowered usage does not; usage below the threshold clears
    /// the stop.
    #[test]
    fn a_pass_that_frees_nothing_stops_the_purge_until_usage_falls() {
        let mut g = PurgeGovernor::default();
        assert!(g.should_purge(96.9, 95.0), "over the threshold: purge");
        // Deleted ten clips; usage did not move — the row's exact case.
        assert!(g.observe(10, 96.9, 96.9), "reported as ineffective");
        assert!(
            !g.should_purge(96.9, 95.0),
            "the next pass must be refused: the space is not in recordings"
        );
        assert!(g.is_ineffective());
        assert!(
            !g.should_purge(94.0, 95.0) && !g.is_ineffective(),
            "below: cleared"
        );
        assert!(
            g.should_purge(96.0, 95.0),
            "over again after clearing: purge"
        );

        // Counterpart: a pass that worked keeps the purge running.
        let mut working = PurgeGovernor::default();
        assert!(working.should_purge(96.0, 95.0));
        assert!(!working.observe(10, 96.0, 95.1), "usage fell: effective");
        assert!(working.should_purge(95.5, 95.0), "still allowed");
        // A pass that removed nothing says nothing about where the space is.
        assert!(!working.observe(0, 95.5, 95.5));
    }

    #[test]
    fn the_ineffective_verdict_reaches_the_shared_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: PathBuf::from("/tmp"),
            purge_threshold: 99,
            ineffective_flag: Some(Arc::clone(&flag)),
            ..DiskManagerConfig::default()
        });
        manager.publish_ineffective(true);
        assert!(flag.load(Ordering::Relaxed));
        assert!(
            !manager.purge_ineffective(),
            "the governor itself has seen no pass"
        );
        manager.publish_ineffective(false);
        assert!(!flag.load(Ordering::Relaxed));
    }

    /// A full disk whose purge is pointed at `dir` with `policy`, publishing
    /// "nothing purgeable" to `flag` and the PR-7 latch to `ineffective`, and
    /// protecting whatever `locked` names when the pass runs.
    fn full_with_protection(
        dir: &std::path::Path,
        policy: PurgePolicy,
        locked: &Arc<Mutex<Vec<String>>>,
        flag: &Arc<AtomicBool>,
        ineffective: &Arc<AtomicBool>,
    ) -> DiskManager {
        let view = Arc::clone(locked);
        DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.to_path_buf(),
            purge_threshold: 0,
            full_disk_action: FullDiskAction::Purge,
            purge_policy: policy,
            locked_provider: Some(Arc::new(move || Some(view.lock().unwrap().clone()))),
            nothing_purgeable_flag: Some(Arc::clone(flag)),
            ineffective_flag: Some(Arc::clone(ineffective)),
            ..DiskManagerConfig::default()
        })
    }

    fn names(files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// N25-01. Over the threshold with every clip protected, the purge removes
    /// nothing. That used to be said only by a warning in the log every
    /// minute: no flag, no metric, no condition, while the disk filled.
    #[test]
    fn a_full_disk_with_nothing_the_purge_may_delete_is_said() {
        for policy in [
            PurgePolicy::OldestFirst,
            PurgePolicy::KeepEverySpecies { floor: 0 },
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let files = clips(dir.path(), 6);
            let locked = Arc::new(Mutex::new(names(&files)));
            let flag = Arc::new(AtomicBool::new(false));
            let ineffective = Arc::new(AtomicBool::new(false));
            let manager = full_with_protection(dir.path(), policy, &locked, &flag, &ineffective);

            assert!(manager.run_cycle());
            assert!(
                files.iter().all(|f| f.exists()),
                "{policy:?}: a locked clip was purged"
            );
            assert!(
                flag.load(Ordering::Relaxed),
                "{policy:?}: over the threshold with nothing deletable, and nothing said"
            );
            // Not the PR-7 latch: that one stops the purge, and this must not.
            assert!(!ineffective.load(Ordering::Relaxed), "{policy:?}");
            assert!(!manager.purge_ineffective(), "{policy:?}");

            // The owner unlocks one clip. The very next pass must still look,
            // find it, delete it and lower the flag.
            locked.lock().unwrap().remove(0);
            assert!(manager.run_cycle());
            assert!(
                !files[0].exists(),
                "{policy:?}: the unlocked clip must be purgeable on the next pass"
            );
            assert!(
                files[1..].iter().all(|f| f.exists()),
                "{policy:?}: a locked clip was purged"
            );
            assert!(
                !flag.load(Ordering::Relaxed),
                "{policy:?}: a pass that removed a clip found something to delete"
            );
        }
    }

    /// The species floor protects as a lock does, and is said the same way.
    #[test]
    fn a_full_disk_held_entirely_by_the_species_floor_is_said() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = clips(dir.path(), 4);
        let locked = Arc::new(Mutex::new(Vec::new()));
        let flag = Arc::new(AtomicBool::new(false));
        let ineffective = Arc::new(AtomicBool::new(false));
        let manager = full_with_protection(
            dir.path(),
            PurgePolicy::KeepEverySpecies { floor: 4 },
            &locked,
            &flag,
            &ineffective,
        );
        assert!(manager.run_cycle());
        assert!(files.iter().all(|f| f.exists()));
        assert!(flag.load(Ordering::Relaxed));
    }

    /// The counterparts. A pass that removes a clip on a full disk does not
    /// raise the flag, and a reading below the threshold clears one left
    /// from before.
    #[test]
    fn a_purge_that_deletes_or_a_disk_below_the_threshold_does_not_say_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = clips(dir.path(), 6);
        let locked = Arc::new(Mutex::new(names(&files[1..])));
        let flag = Arc::new(AtomicBool::new(false));
        let ineffective = Arc::new(AtomicBool::new(false));
        let manager = full_with_protection(
            dir.path(),
            PurgePolicy::OldestFirst,
            &locked,
            &flag,
            &ineffective,
        );
        assert!(manager.run_cycle());
        assert!(!files[0].exists(), "the fixture must give the purge a clip");
        assert!(!flag.load(Ordering::Relaxed));

        let below = Arc::new(AtomicBool::new(true));
        let manager = DiskManager::new(DiskManagerConfig {
            monitored_dir: dir.path().to_path_buf(),
            purge_threshold: 100,
            nothing_purgeable_flag: Some(Arc::clone(&below)),
            ..DiskManagerConfig::default()
        });
        manager.check_and_purge().expect("check");
        assert!(
            !below.load(Ordering::Relaxed),
            "below the threshold the disk is not full"
        );
    }

    /// The PR-7 latch is unchanged by the new signal: a pass that removes a
    /// clip and frees nothing measurable still stops the next one, and the
    /// refused pass leaves "nothing purgeable" down, because it never looked.
    #[test]
    fn the_ineffective_latch_still_stops_the_purge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = clips(dir.path(), 20);
        let locked = Arc::new(Mutex::new(Vec::new()));
        let flag = Arc::new(AtomicBool::new(false));
        let ineffective = Arc::new(AtomicBool::new(false));
        let manager = full_with_protection(
            dir.path(),
            PurgePolicy::OldestFirst,
            &locked,
            &flag,
            &ineffective,
        );
        assert!(manager.run_cycle());
        let left = files.iter().filter(|f| f.exists()).count();
        assert!(left < 20, "the first pass removes clips");
        // 44-byte clips move usage by nothing: the latch is set.
        assert!(manager.purge_ineffective());
        assert!(ineffective.load(Ordering::Relaxed));
        assert!(manager.run_cycle());
        assert_eq!(
            files.iter().filter(|f| f.exists()).count(),
            left,
            "the latched purge must not delete more"
        );
        assert!(!flag.load(Ordering::Relaxed));
    }
}
