//! The detections a stop interrupts, kept on the data disk until the next
//! start processes them.
//!
//! # What a stop did to them
//!
//! The detection loop hands each detection to the event processor over an
//! in-memory channel, and the processor cuts its clip and writes its row one
//! at a time. A stop with a backlog (about 300 detections from one busy
//! recording) waited for the processor to work through all of it — about 52 s
//! measured, against the unit's `TimeoutStopSec=30` — so systemd killed the
//! process part-way, and every detection still in the channel was gone: the
//! channel is memory, and the recordings they came from live in the service's
//! `PrivateTmp`, which the next start does not see.
//!
//! # What it does now
//!
//! Once the station is shutting down, the processor stops cutting clips and
//! hands every detection it has not processed to [`ClipQueue::keep`], which
//! writes a batch directory beside the database: one line per detection, and
//! the audio its clip will be cut from. The next start takes the batches back
//! ([`ClipQueue::take`]) and the processor runs them through the path a live
//! detection takes, before anything new.
//!
//! # What a batch holds, and what it promises (review F01–F05)
//!
//! * **The run and the bars.** Each line (format version [`VERSION`]) carries
//!   the analysis run that produced the detection and the two bars it was to be
//!   judged against ([`Bars`]). The next start stores it under that run and
//!   judges it at those bars, not under its own run and whatever the
//!   thresholds have become. A line written before lines carried a run (no
//!   `v`) is stored with the run left unknown, never attributed to the run that
//!   happens to replay it.
//! * **Only the audio the clip needs.** Each recording a clip reaches into is
//!   kept as a WAV of its own length with every sample outside the clip's
//!   window set to zero: the extractor cuts the identical clip from it, and no
//!   other audio reaches the data disk. On a station whose clips stay inside
//!   their own segment (the privacy filter is on) that is the window of its own
//!   recording and no neighbour; the filter judged that window and nothing
//!   else.
//! * **Nothing is gone until it is settled.** `events.jsonl` is never
//!   rewritten. Each detection the processor finishes with — stored, already
//!   stored, quarantined, or dropped by a rule — is appended to `acked`, synced;
//!   one the database could not take is appended to `failed` and offered again
//!   at the next start. After [`MAX_ATTEMPTS`] failed starts, or when its line
//!   cannot be read, a detection is **held**: left in its batch, reported at
//!   every start, and never deleted by the station. A batch goes when every
//!   one of its lines is settled.
//! * **Whole or absent.** Every file of a batch is written under a `.part` name,
//!   synced, renamed and the directory synced (`atomic_file`), and the queue's
//!   directory is synced after a batch is added or removed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use birdnet_core::audio::decode::AudioData;
use birdnet_core::audio::extraction::ExtractionConfig;
use birdnet_core::detection::daemon::DetectionEvent;
use birdnet_core::detection::types::{Detection, RecordingFile};

use super::disposition::Bars;

/// The queue's directory name, beside the database.
const DIR_NAME: &str = "clip-queue";
/// A batch's list of detections, one JSON object per line. Never rewritten.
const EVENTS: &str = "events.jsonl";
/// How many of a batch's first detections were processed: written by stations
/// before [`ACKED`], read so their batches are not replayed from the start.
const LEGACY_DONE: &str = "done";
/// The index of each detection of a batch that is settled, one per line.
const ACKED: &str = "acked";
/// The index of a detection each time a start failed to store it.
const FAILED: &str = "failed";
/// The batch line format this build writes.
pub(super) const VERSION: u64 = 2;
/// Starts that may fail to store a kept detection before it is held for an
/// operator instead of offered again.
pub(super) const MAX_ATTEMPTS: usize = 3;
/// Audio the queue keeps before it keeps detections without theirs.
pub(super) const MAX_QUEUE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Free space on the data disk below which the queue keeps no audio.
pub(super) const FREE_RESERVE_BYTES: u64 = 512 * 1024 * 1024;
/// Refused live detections held in memory before they are written to the
/// queue; they are written sooner when the channel is idle or closes.
const DEFER_MAX: usize = 32;
/// Settlements awaiting durability before the database is asked again.
const ACK_BATCH: usize = 64;

/// The run a kept detection was analysed under, as a batch line carries it:
/// the processor's `RunProvenance`, with the classifier's `BirdWeather` name
/// as text.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct KeptRun {
    /// The `analysis_runs` row.
    pub run_id: i64,
    /// `BirdWeather`'s name for the run's classifier.
    pub algorithm: Option<String>,
    /// Station latitude the run used.
    pub lat: Option<f64>,
    /// Station longitude the run used.
    pub lon: Option<f64>,
    /// Sensitivity the run used.
    pub sensitivity: f64,
    /// Overlap the run used.
    pub overlap: f64,
    /// Each classifier's own threshold in the run.
    pub model_thresholds: HashMap<String, f32>,
}

/// A detection a stop hands to the queue: the detection as the processor
/// would have judged it, and what it would have been judged by.
#[derive(Debug)]
pub(super) struct Kept {
    /// The detection.
    pub event: DetectionEvent,
    /// The run it was analysed under; `None` when that is not known.
    pub run: Option<KeptRun>,
    /// The bars it was to be judged at; `None` when they are not known, and
    /// the bars in force are used.
    pub bars: Option<Bars>,
}

/// Where a detection the processor is handed comes from.
#[derive(Debug)]
pub(super) enum Origin {
    /// The detection loop, now.
    Live,
    /// A batch an earlier stop kept.
    Replayed {
        /// The run that analysed it, when the batch recorded one.
        run: Option<KeptRun>,
        /// The bars it was kept under, when the batch recorded them.
        bars: Option<Bars>,
        /// What to hand back to [`Inbox::settle`] once it is finished with.
        ticket: Ticket,
    },
}

/// One replayed detection, to be settled.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    batch: PathBuf,
    index: usize,
}

/// What became of a replayed detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Finished with: stored, already stored, quarantined, or dropped by a
    /// rule. It is not offered again.
    Settled,
    /// The station could not record it; offer it again at the next start.
    Retry,
}

/// The window of a recording a clip is cut from, around its detection: what
/// [`ExtractionConfig`] makes of a detection's start and stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ClipWindow {
    /// Seconds before the detection's start.
    lead_in: f32,
    /// Seconds after the detection's stop.
    spacer: f32,
    /// Whether clips stay inside their own recording.
    own_segment_only: bool,
}

impl ClipWindow {
    /// The window the extractor configured by `config` cuts: the same
    /// arithmetic as `Extractor::extract_detection_clip`.
    #[must_use]
    pub(super) fn of(config: &ExtractionConfig) -> Self {
        let spacer = config.spacer();
        Self {
            lead_in: spacer + config.pre_capture_secs.max(0.0),
            spacer,
            own_segment_only: config.own_segment_only,
        }
    }
}

/// A batch line for `kept`, its recording named `source` in the batch.
fn kept_line(kept: &Kept, source: &str) -> serde_json::Result<String> {
    let event = &kept.event;
    let run = kept.run.as_ref().map(|r| {
        serde_json::json!({
            "run_id": r.run_id,
            "algorithm": r.algorithm,
            "lat": r.lat,
            "lon": r.lon,
            "sensitivity": r.sensitivity,
            "overlap": r.overlap,
            "model_thresholds": r.model_thresholds,
        })
    });
    let policy = kept
        .bars
        .map(|b| serde_json::json!({"species_bar": b.species_bar, "global_bar": b.global_bar}));
    serde_json::to_string(&serde_json::json!({
        "v": VERSION,
        "detection": event.detection,
        "source": source,
        "latency_ms": event.latency_ms,
        "correlation_id": event.correlation_id,
        "out_of_range": event.out_of_range,
        "run": run,
        "policy": policy,
    }))
}

/// A batch line read back, its recording in `batch`. Refused, with the
/// reason, when it is not one this build can trust: unreadable, of a later
/// format, or naming a recording outside its batch.
fn read_kept(line: &str, batch: &Path) -> Result<Kept, String> {
    let mut value: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let version = match value.get("v") {
        None | Some(serde_json::Value::Null) => 1,
        Some(v) => v.as_u64().ok_or("the format version is not a number")?,
    };
    if version > VERSION {
        return Err(format!(
            "written in format {version}, which this build (format {VERSION}) does not read"
        ));
    }
    let detection: Detection =
        serde_json::from_value(value["detection"].take()).map_err(|e| e.to_string())?;
    let source: String =
        serde_json::from_value(value["source"].take()).map_err(|e| e.to_string())?;
    // A name inside the batch and nothing else: the line is data on a disk
    // other people may write to, and it names a file to read and later a
    // clip to cut.
    if source.is_empty()
        || Path::new(&source).file_name() != Some(std::ffi::OsStr::new(&source))
        || source == ".."
    {
        return Err(format!(
            "names {source:?}, which is not a file in its batch"
        ));
    }
    let out_of_range =
        serde_json::from_value(value["out_of_range"].take()).map_err(|e| e.to_string())?;
    let run = match value["run"].take() {
        serde_json::Value::Null => None,
        run => Some(read_run(&run).ok_or("its run is not readable")?),
    };
    let bars = match value["policy"].take() {
        serde_json::Value::Null => None,
        p => Some(Bars {
            species_bar: p["species_bar"].as_f64(),
            global_bar: p["global_bar"]
                .as_f64()
                .filter(|b| b.is_finite())
                .ok_or("its bars are not readable")?,
        }),
    };
    Ok(Kept {
        event: DetectionEvent {
            detection,
            source_file: batch.join(source),
            latency_ms: value["latency_ms"].as_u64().unwrap_or(0),
            correlation_id: value["correlation_id"].as_str().unwrap_or("").to_owned(),
            lease: None,
            out_of_range,
        },
        run,
        bars,
    })
}

fn read_run(run: &serde_json::Value) -> Option<KeptRun> {
    Some(KeptRun {
        run_id: run["run_id"].as_i64()?,
        algorithm: run["algorithm"].as_str().map(str::to_owned),
        lat: run["lat"].as_f64(),
        lon: run["lon"].as_f64(),
        sensitivity: run["sensitivity"].as_f64()?,
        overlap: run["overlap"].as_f64()?,
        model_thresholds: run["model_thresholds"]
            .as_object()?
            .iter()
            .filter_map(|(k, v)| {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "written from an f32 threshold, read back to one"
                )]
                let t = v.as_f64()? as f32;
                Some((k.clone(), t))
            })
            .collect(),
    })
}

/// The detections a stop left unprocessed, on the data disk.
#[derive(Debug, Clone)]
pub struct ClipQueue {
    dir: PathBuf,
    window: ClipWindow,
    /// Audio kept before detections are kept without theirs.
    max_bytes: u64,
}

impl ClipQueue {
    /// The queue beside the database at `db_path`, for clips cut in `window`.
    #[must_use]
    pub(super) fn beside(db_path: &Path, window: ClipWindow) -> Self {
        Self {
            dir: db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(DIR_NAME),
            window,
            max_bytes: MAX_QUEUE_BYTES,
        }
    }

    /// The queue with a byte budget of `max_bytes`, for tests.
    #[cfg(test)]
    const fn with_budget(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Keep `kept` for the next start: write the detections and the audio
    /// their clips need into a new batch. Returns how many were kept.
    ///
    /// The queue is bounded: past [`MAX_QUEUE_BYTES`] of kept audio, or with
    /// less than [`FREE_RESERVE_BYTES`] left on its disk, a batch keeps the
    /// detections without their audio, and a batch whose audio cannot be
    /// written (a full disk) is written again without it. A detection kept
    /// without audio is recorded without a clip; it is not lost.
    ///
    /// # Errors
    ///
    /// The filesystem's, when not even the detections could be written; the
    /// batch is then removed, so a half-written one is never replayed.
    pub(super) fn keep(&self, kept: &[Kept]) -> std::io::Result<usize> {
        if kept.is_empty() {
            return Ok(0);
        }
        let with_audio = self.room_for_audio();
        match self.keep_once(kept, with_audio) {
            Err(e) if with_audio => {
                tracing::warn!(
                    error = %e,
                    detections = kept.len(),
                    "the audio of the detections being kept could not be written; keeping \
                     them without it"
                );
                self.keep_once(kept, false)
            }
            other => other,
        }
    }

    fn keep_once(&self, kept: &[Kept], with_audio: bool) -> std::io::Result<usize> {
        let batch = self.new_batch()?;
        let written = write_batch(&batch, kept, self.window, with_audio);
        if written.is_err() {
            let _ = std::fs::remove_dir_all(&batch);
        }
        written?;
        sync_dir(&self.dir)?;
        Ok(kept.len())
    }

    /// Whether the queue may take more audio: under its byte budget, with the
    /// reserve free on its disk. A disk whose free space cannot be read is
    /// taken to have room; the write itself then says otherwise.
    fn room_for_audio(&self) -> bool {
        let used = dir_bytes(&self.dir);
        if used >= self.max_bytes {
            tracing::warn!(
                queue = %self.dir.display(),
                used,
                limit = self.max_bytes,
                "the stop queue holds its limit of audio; detections are kept without theirs"
            );
            return false;
        }
        let probe = if self.dir.exists() {
            self.dir.clone()
        } else {
            self.dir
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };
        match birdnet_core::audio::capture::disk_usage(&probe) {
            Ok(u) if u.available_bytes < FREE_RESERVE_BYTES => {
                tracing::warn!(
                    available = u.available_bytes,
                    reserve = FREE_RESERVE_BYTES,
                    "the data disk is nearly full; detections are kept without their audio"
                );
                false
            }
            _ => true,
        }
    }

    /// A new, empty batch directory, named so batches sort in the order they
    /// were written.
    fn new_batch(&self) -> std::io::Result<PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        self.new_batch_at(nanos)
    }

    /// [`Self::new_batch`] at `nanos` past the epoch: the first of
    /// `<nanos>-0`, `<nanos>-1`, … not already there.
    ///
    /// The queue's own directory is made first, and once: a file in its
    /// place reads as `AlreadyExists` too, which inside the loop below was
    /// taken for a name in use and searched past for ever. It is readable by
    /// the service alone: it holds recordings.
    fn new_batch_at(&self, nanos: u128) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut n = 0_u32;
        loop {
            let dir = self.dir.join(format!("{nanos:024}-{n}"));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    }

    /// Every kept detection not yet settled and not held, oldest batch first.
    ///
    /// Nothing here deletes a detection that is not settled. A queue that
    /// cannot be listed, a batch whose list cannot be read and a line that
    /// cannot be trusted are each said in the journal at error level and left
    /// where they are.
    #[must_use]
    pub fn take(&self) -> Replay {
        let mut replay = Replay {
            queue: VecDeque::new(),
            unsettled: HashMap::new(),
            held: 0,
            queue_dir: self.dir.clone(),
            pending_acks: Vec::new(),
        };
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            // No directory is no batch: nothing was ever kept.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return replay,
            Err(e) => {
                tracing::error!(
                    queue = %self.dir.display(),
                    error = %e,
                    "the detections earlier stops kept cannot be listed; none is replayed, \
                     and none is deleted"
                );
                return replay;
            }
        };
        let mut batches: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        batches.sort();
        let mut removed = false;
        for batch in batches {
            if !batch.is_dir() {
                continue;
            }
            if !batch.join(EVENTS).is_file() {
                // `events.jsonl` is the batch's commit: without it nothing in
                // the directory was ever kept (a stop interrupted while writing,
                // whose `keep` never returned), only audio for it. Removed, so
                // interrupted stops do not accumulate.
                tracing::warn!(
                    batch = %batch.display(),
                    "removing a batch a stop did not finish writing; it held no kept detection"
                );
                let _ = std::fs::remove_dir_all(&batch);
                removed = true;
                continue;
            }
            match load_batch(&batch, &mut replay) {
                Ok(0) => {
                    let _ = std::fs::remove_dir_all(&batch);
                    removed = true;
                }
                Ok(unsettled) => {
                    replay.unsettled.insert(batch, unsettled);
                }
                Err(e) => tracing::error!(
                    batch = %batch.display(),
                    error = %e,
                    "the detections a stop kept could not be read; left as they are"
                ),
            }
        }
        if removed {
            let _ = sync_dir(&self.dir);
        }
        if !replay.queue.is_empty() {
            tracing::info!(
                detections = replay.queue.len(),
                "processing the detections the last stop kept, before any new one"
            );
        }
        if replay.held > 0 {
            tracing::error!(
                held = replay.held,
                queue = %self.dir.display(),
                "kept detections the station could not record or read are held in the queue \
                 for an operator; the station does not delete them"
            );
        }
        replay
    }
}

/// Read one batch into `replay`: its unsettled detections queued, those held
/// counted. Returns how many are unsettled, held included.
fn load_batch(batch: &Path, replay: &mut Replay) -> std::io::Result<usize> {
    let text = std::fs::read_to_string(batch.join(EVENTS))?;
    let legacy_done = std::fs::read_to_string(batch.join(LEGACY_DONE)).map_or(0, |s| {
        s.trim().parse::<usize>().unwrap_or_else(|_| {
            tracing::warn!(batch = %batch.display(), "a batch's progress is unreadable; replayed from its start");
            0
        })
    });
    let acked: HashSet<usize> = indices(&batch.join(ACKED)).into_iter().collect();
    let mut failures: HashMap<usize, usize> = HashMap::new();
    for i in indices(&batch.join(FAILED)) {
        *failures.entry(i).or_default() += 1;
    }
    let mut unsettled = 0;
    for (index, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        if index < legacy_done || acked.contains(&index) {
            continue;
        }
        unsettled += 1;
        let failed = failures.get(&index).copied().unwrap_or(0);
        if failed >= MAX_ATTEMPTS {
            replay.held += 1;
            tracing::error!(
                batch = %batch.display(),
                index,
                attempts = failed,
                "a kept detection the station failed to record at {failed} starts is held"
            );
            continue;
        }
        match read_kept(line, batch) {
            Ok(kept) => replay.queue.push_back(Pending {
                batch: batch.to_path_buf(),
                index,
                kept,
            }),
            Err(e) => {
                replay.held += 1;
                tracing::error!(
                    batch = %batch.display(),
                    index,
                    error = %e,
                    "a kept detection cannot be read; it is held, not deleted"
                );
            }
        }
    }
    Ok(unsettled)
}

/// The indices listed in `path`, one per line; none when it is absent. A
/// line that is not a number (a write a power cut tore) is skipped.
fn indices(path: &Path) -> Vec<usize> {
    std::fs::read_to_string(path)
        .map_or_default(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
}

/// Append `index` to `path` and sync it.
fn append_index(path: &Path, index: usize) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{index}")?;
    f.sync_data()
}

/// `fsync` a directory, so an entry added or removed inside it is on disk.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Sample `secs` into audio at `rate` with `len` samples: the index the
/// extractor's own slicing takes (`span::slice_secs`), so a window kept here
/// is the window cut there, sample for sample.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the extractor's own f32 arithmetic, reproduced exactly"
)]
fn sample_at(secs: f32, rate: u32, len: usize) -> usize {
    ((secs.max(0.0) * rate as f32) as usize).min(len)
}

/// Samples and rate of `path`, decoded and dropped: what the extractor would
/// see of its length. `None` for a recording gone or unreadable (said once in
/// the journal when it is there and unreadable).
fn length_of(path: &Path) -> Option<(usize, u32)> {
    match birdnet_core::audio::decode::decode_file(path) {
        Ok(a) if !a.samples.is_empty() => Some((a.samples.len(), a.sample_rate)),
        Ok(_) => None,
        Err(e) => {
            // A recording already gone is not an error: its detection is still
            // kept, and recorded without a clip.
            if path.exists() {
                tracing::warn!(
                    recording = %path.display(),
                    error = %e,
                    "a recording a kept detection needs cannot be read; kept without its audio"
                );
            }
            None
        }
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "the extractor's own f32 arithmetic"
)]
fn secs_of(len: usize, rate: u32) -> f32 {
    len as f32 / rate as f32
}

/// Write `kept`, and with `with_audio` the audio their clips need, into
/// `batch`.
///
/// Two passes, so no more than one recording's samples are held at once,
/// whatever the backlog: the first decodes each recording a clip reaches for
/// its length alone and works out the sample ranges; the second decodes each
/// recording that has a range again, writes its masked copy and drops it.
fn write_batch(
    batch: &Path,
    kept: &[Kept],
    window: ClipWindow,
    with_audio: bool,
) -> std::io::Result<()> {
    let mut lines = String::new();
    let mut lengths: HashMap<PathBuf, Option<(usize, u32)>> = HashMap::new();
    let mut ranges: std::collections::BTreeMap<PathBuf, Vec<std::ops::Range<usize>>> =
        std::collections::BTreeMap::new();
    let mut length = |path: &Path| {
        *lengths
            .entry(path.to_path_buf())
            .or_insert_with(|| length_of(path))
    };
    for k in kept {
        let det = &k.event.detection;
        let source = &k.event.source_file;
        if with_audio && let Some((len, rate)) = length(source) {
            let d = secs_of(len, rate);
            let mut start = det.start - window.lead_in;
            let mut stop = (det.stop + window.spacer).max(start);
            if window.own_segment_only {
                start = start.clamp(0.0, d);
                stop = stop.clamp(start, d);
            }
            let mut need = |path: &Path, from: f32, to: f32, len: usize, rate: u32| {
                let range = sample_at(from, rate, len)..sample_at(to, rate, len);
                if !range.is_empty() {
                    ranges.entry(path.to_path_buf()).or_default().push(range);
                }
            };
            need(source, start.max(0.0), stop.min(d), len, rate);
            if !window.own_segment_only {
                // What the clip reaches into either neighbour, as a range that
                // is empty when it reaches nothing. Unguarded on purpose: a
                // `start < 0.0` test in front only saved a header read, and
                // was a condition no test could tell from `<=` (PR #313).
                let (before, after) = neighbours(source);
                if let Some(prev) = before
                    && let Some((plen, prate)) = length(&prev)
                {
                    let pd = secs_of(plen, prate);
                    let take = (-start).clamp(0.0, pd);
                    need(&prev, pd - take, pd, plen, prate);
                }
                if let Some(next) = after
                    && let Some((nlen, nrate)) = length(&next)
                {
                    need(&next, 0.0, (stop - d).max(0.0), nlen, nrate);
                }
            }
        }
        lines.push_str(&kept_line(k, &kept_name(source)).map_err(std::io::Error::other)?);
        lines.push('\n');
    }
    let mut names: HashMap<String, PathBuf> = HashMap::new();
    for (path, keep) in ranges {
        let name = kept_name(&path);
        if let Some(other) = names.get(&name)
            && *other != path
        {
            tracing::warn!(
                recording = %path.display(),
                "two recordings a stop kept share a name; the second is kept without its audio"
            );
            continue;
        }
        let Ok(audio) = birdnet_core::audio::decode::decode_file(&path) else {
            continue;
        };
        let _held = held_audio::Held::new();
        write_masked(&batch.join(&name), &audio, &keep)?;
        names.insert(name, path);
    }
    let part = batch.join(format!("{EVENTS}.part"));
    let mut file = std::fs::File::create(&part)?;
    file.write_all(lines.as_bytes())?;
    drop(file);
    birdnet_core::atomic_file::commit(&part, &batch.join(EVENTS))
}

/// How many decoded recordings the queue holds at once, for the test that
/// keeps a large backlog bounded. Per thread, so tests running side by side
/// do not add to each other's count.
mod held_audio {
    #[cfg(test)]
    thread_local! {
        static LIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub static PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// One decoded recording held, for as long as it lives.
    pub struct Held;

    impl Held {
        #[cfg(test)]
        pub fn new() -> Self {
            LIVE.with(|live| {
                live.set(live.get() + 1);
                PEAK.with(|peak| peak.set(peak.get().max(live.get())));
            });
            Self
        }

        #[cfg(not(test))]
        pub const fn new() -> Self {
            Self
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            #[cfg(test)]
            LIVE.with(|live| live.set(live.get().saturating_sub(1)));
        }
    }
}

/// Bytes of the files directly in `dir` and one level below: a batch is a
/// directory of files.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                std::fs::read_dir(&p).map_or(0, |inner| {
                    inner
                        .flatten()
                        .filter_map(|f| f.metadata().ok())
                        .map(|m| m.len())
                        .sum()
                })
            } else {
                e.metadata().map_or(0, |m| m.len())
            }
        })
        .sum()
}

/// A recording's name in its batch: its own name with a `.wav` extension,
/// which is what [`write_masked`] writes whatever the recording was.
fn kept_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map_or_else(|| "recording".into(), |s| s.to_string_lossy().into_owned());
    format!("{stem}.wav")
}

/// `audio` as a 32-bit float WAV at `path`, every sample outside `keep`
/// silent. Float so the samples the extractor reads back are the ones it
/// would have read from the recording.
fn write_masked(
    path: &Path,
    audio: &AudioData,
    keep: &[std::ops::Range<usize>],
) -> std::io::Result<()> {
    let mut mask = vec![false; audio.samples.len()];
    for r in keep {
        mask[r.clone()].fill(true);
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let part = birdnet_core::atomic_file::part_path(path);
    let written = (|| {
        let mut w = hound::WavWriter::create(&part, spec).map_err(std::io::Error::other)?;
        for (s, &kept) in audio.samples.iter().zip(&mask) {
            w.write_sample(if kept { *s } else { 0.0 })
                .map_err(std::io::Error::other)?;
        }
        w.finalize().map_err(std::io::Error::other)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    birdnet_core::atomic_file::commit(&part, path)
}

/// The recordings from the same source just before and just after `source`
/// in its directory: where a clip that reaches past either end of its own
/// segment finds the rest of its audio.
fn neighbours(source: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let (Some(dir), Some(here)) = (
        source.parent(),
        source.to_str().and_then(RecordingFile::parse),
    ) else {
        return (None, None);
    };
    let key = |r: &RecordingFile| format!("{} {}", r.date, r.time);
    let here_at = key(&here);
    // Every recording of the same source, by when it starts; two that start
    // together are told apart by name, so the choice does not depend on the
    // order the directory lists them in.
    let same_source: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter_map(|path| {
            let other = path.to_str().and_then(RecordingFile::parse)?;
            (other.rtsp_id == here.rtsp_id).then(|| (key(&other), path))
        })
        .collect();
    let before = same_source.iter().filter(|(at, _)| *at < here_at).max();
    let after = same_source.iter().filter(|(at, _)| *at > here_at).min();
    (
        before.map(|(_, p)| p.clone()),
        after.map(|(_, p)| p.clone()),
    )
}

/// One kept detection on its way back through the processor.
#[derive(Debug)]
struct Pending {
    batch: PathBuf,
    index: usize,
    kept: Kept,
}

/// The kept detections of earlier stops, handed back one at a time.
#[derive(Debug)]
pub struct Replay {
    queue: VecDeque<Pending>,
    /// Each batch's detections not yet settled, held ones included.
    unsettled: HashMap<PathBuf, usize>,
    /// Detections left for an operator.
    held: usize,
    queue_dir: PathBuf,
    /// Settled, and not yet known to be durable in the database.
    pending_acks: Vec<Ticket>,
}

impl Replay {
    /// The next kept detection, with the ticket that settles it.
    pub(super) fn next(&mut self) -> Option<(Ticket, Kept)> {
        let p = self.queue.pop_front()?;
        Some((
            Ticket {
                batch: p.batch,
                index: p.index,
            },
            p.kept,
        ))
    }

    /// Record what became of the detection `ticket` was handed out with.
    ///
    /// A settled detection is acknowledged only by [`Self::flush`], once the
    /// database says what was settled is on the disk (review F04/F17, second
    /// batch): with `synchronous=NORMAL` a commit can be rolled back by a
    /// power cut until the next checkpoint, and an acknowledgement written
    /// before it would lose the detection for good. Until then a crash offers
    /// it again, where the processor finds its row and writes nothing twice.
    pub(super) fn settle(&mut self, ticket: Ticket, outcome: Outcome) {
        match outcome {
            Outcome::Settled => self.pending_acks.push(ticket),
            Outcome::Retry => {
                let Ticket { batch, index } = ticket;
                if let Err(e) = append_index(&batch.join(FAILED), index) {
                    tracing::warn!(batch = %batch.display(), error = %e, "could not record a kept detection's failed attempt");
                }
            }
        }
    }

    /// Acknowledge the settled detections when `durable` (the database
    /// confirmed its commits are on the disk); keep them pending otherwise.
    pub(super) fn flush(&mut self, durable: bool) {
        if !durable {
            return;
        }
        for Ticket { batch, index } in std::mem::take(&mut self.pending_acks) {
            if let Err(e) = append_index(&batch.join(ACKED), index) {
                tracing::warn!(batch = %batch.display(), error = %e, "could not record a kept detection as settled");
                continue;
            }
            let left = self.unsettled.get_mut(&batch).map(|n| {
                *n = n.saturating_sub(1);
                *n
            });
            if left == Some(0) {
                self.unsettled.remove(&batch);
                let _ = std::fs::remove_dir_all(&batch);
                let _ = sync_dir(&self.queue_dir);
            }
        }
    }

    /// How many settlements await the database's confirmation.
    pub(super) const fn awaiting_durability(&self) -> usize {
        self.pending_acks.len()
    }

    /// How many detections are held for an operator.
    #[cfg(test)]
    pub(super) const fn held(&self) -> usize {
        self.held
    }

    /// How many are still to be handed out.
    pub(super) fn len(&self) -> usize {
        self.queue.len()
    }
}

/// How long a stopping processor waits for the detection loop to finish the
/// file it is reading and close its channel, so that file's detections are
/// kept too: the application's own wait for that loop (`DAEMON_STOP_GRACE`).
const KEEP_WAIT: Duration = Duration::from_secs(15);

/// When a stopping processor, starting to keep at `now`, gives up waiting for
/// the detection loop's channel to close: [`KEEP_WAIT`] later.
fn keep_deadline(now: Instant) -> Instant {
    now + KEEP_WAIT
}

/// What the processor is to do next.
#[derive(Debug)]
pub(super) enum Next {
    /// Process this detection.
    Event(Box<(DetectionEvent, Origin)>),
    /// The detection loop has closed its channel and nothing is left.
    Closed,
    /// The station is stopping: hand what is left to [`Inbox::keep_the_rest`].
    Stopping,
}

/// Where the processor's detections come from: what the last stop kept,
/// then the detection loop's channel, until the station stops.
pub(super) struct Inbox {
    queue: ClipQueue,
    replay: Replay,
    stopping: tokio::sync::watch::Receiver<bool>,
    /// Live detections the database refused, to keep for the next start.
    deferred: Vec<Kept>,
    /// Whether the database's commits so far are on the disk; asked before
    /// settlements are acknowledged.
    durable: Box<dyn FnMut() -> bool>,
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inbox")
            .field("queue", &self.queue)
            .field("replay", &self.replay)
            .field("deferred", &self.deferred.len())
            .finish_non_exhaustive()
    }
}

impl Inbox {
    /// The inbox of the station whose database is `db_path`, whose clips are
    /// cut in `window`, stopping when `stopping` turns true
    /// (`AppState::subscribe_shutdown`), and asking `durable` whether the
    /// database's commits are on the disk before acknowledging what it settled.
    #[must_use]
    pub(super) fn new(
        db_path: &Path,
        window: ClipWindow,
        stopping: tokio::sync::watch::Receiver<bool>,
        durable: Box<dyn FnMut() -> bool>,
    ) -> Self {
        let queue = ClipQueue::beside(db_path, window);
        let replay = queue.take();
        Self {
            queue,
            replay,
            stopping,
            deferred: Vec::new(),
            durable,
        }
    }

    /// The next detection, calling `idle` at each `poll` that passes with
    /// none.
    pub(super) fn next(
        &mut self,
        events: &mpsc::Receiver<DetectionEvent>,
        poll: Duration,
        idle: &mut dyn FnMut(),
    ) -> Next {
        loop {
            if *self.stopping.borrow() {
                return Next::Stopping;
            }
            if let Some((ticket, kept)) = self.replay.next() {
                return Next::Event(Box::new((
                    kept.event,
                    Origin::Replayed {
                        run: kept.run,
                        bars: kept.bars,
                        ticket,
                    },
                )));
            }
            self.flush_acks();
            match events.recv_timeout(poll) {
                Ok(event) => return Next::Event(Box::new((event, Origin::Live))),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.flush_deferred();
                    idle();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.flush_deferred();
                    self.flush_acks();
                    return Next::Closed;
                }
            }
        }
    }

    /// Record what became of a replayed detection.
    pub(super) fn settle(&mut self, ticket: Ticket, outcome: Outcome) {
        self.replay.settle(ticket, outcome);
        if self.replay.awaiting_durability() >= ACK_BATCH {
            self.flush_acks();
        }
    }

    /// Keep a live detection the database refused, for the next start
    /// (review F02, second batch). Written to the queue within [`DEFER_MAX`]
    /// of them, or when the channel is idle, closes or the station stops.
    pub(super) fn defer(&mut self, kept: Kept) {
        self.deferred.push(kept);
        if self.deferred.len() >= DEFER_MAX {
            self.flush_deferred();
        }
    }

    fn flush_deferred(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let deferred = std::mem::take(&mut self.deferred);
        match self.queue.keep(&deferred) {
            Ok(n) => tracing::warn!(
                detections = n,
                "detections the database refused are kept beside it and recorded at the next start"
            ),
            Err(e) => tracing::error!(
                detections = deferred.len(),
                error = %e,
                "detections the database refused could not be kept either, and are lost"
            ),
        }
    }

    fn flush_acks(&mut self) {
        if self.replay.awaiting_durability() == 0 {
            return;
        }
        let durable = (self.durable)();
        if !durable {
            tracing::debug!(
                pending = self.replay.awaiting_durability(),
                "the database could not confirm its commits are on the disk; acknowledging later"
            );
        }
        self.replay.flush(durable);
    }

    /// Keep everything the detection loop sends until it closes its channel
    /// (or [`KEEP_WAIT`] passes), each made into a [`Kept`] by `freeze`, with
    /// the refused live detections not yet kept. Returns how many were kept:
    /// written to the disk, not merely offered.
    ///
    /// What is left of the replay stays in its own batches, which are already
    /// on the disk with the run and bars they were kept under.
    pub(super) fn keep_the_rest(
        &mut self,
        events: &mpsc::Receiver<DetectionEvent>,
        freeze: &mut dyn FnMut(DetectionEvent) -> Kept,
    ) -> usize {
        self.flush_acks();
        let deadline = keep_deadline(Instant::now());
        let mut rest = std::mem::take(&mut self.deferred);
        while let Ok(event) =
            events.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            rest.push(freeze(event));
        }
        if self.replay.len() > 0 {
            tracing::info!(
                detections = self.replay.len(),
                "stopping: kept detections not yet replayed stay in the queue"
            );
        }
        let n = rest.len();
        match self.queue.keep(&rest) {
            Ok(0) => 0,
            Ok(kept) => {
                tracing::info!(
                    detections = kept,
                    "stopping: the detections not yet processed are kept beside the database \
                     and are recorded at the next start"
                );
                kept
            }
            Err(e) => {
                tracing::error!(
                    detections = n,
                    error = %e,
                    "stopping: the detections not yet processed could not be kept, and are lost"
                );
                0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn window() -> ClipWindow {
        ClipWindow::of(&ExtractionConfig::default())
    }

    fn detection(sci: &str, start: f32) -> Detection {
        Detection {
            date: "2026-05-19".into(),
            time: "09:01:00".into(),
            scientific_name: sci.into(),
            common_name: sci.into(),
            confidence: 0.9,
            start,
            stop: start + 3.0,
            week: 20,
            file_name_extr: None,
            model_id: Some("birdnet".into()),
            agreeing_models: None,
            model_scores: Vec::new(),
        }
    }

    fn kept(source: &Path, sci: &str, start: f32) -> Kept {
        Kept {
            event: DetectionEvent {
                detection: detection(sci, start),
                source_file: source.to_path_buf(),
                latency_ms: 5,
                correlation_id: "c".into(),
                lease: None,
                out_of_range: None,
            },
            run: None,
            bars: None,
        }
    }

    /// A 15 s 16-bit WAV whose samples are all different, so a test can see
    /// which ones came back.
    fn ramp(path: &Path, seed: i32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SR,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(i32::try_from(SR).unwrap() * 15) {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a test signal, wrapped on purpose"
            )]
            w.write_sample((((i * 7 + seed) % 30_000) + 1) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
    }

    fn drain(replay: &mut Replay) -> Vec<String> {
        let mut names = Vec::new();
        while let Some((t, k)) = replay.next() {
            names.push(k.event.detection.scientific_name);
            replay.settle(t, Outcome::Settled);
        }
        replay.flush(true);
        names
    }

    /// What a stop keeps comes back at the next start, even after the
    /// recordings' own directory is gone; each comes back until it is
    /// settled, and once settled never again.
    #[test]
    fn kept_detections_come_back_until_they_are_settled() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let src = stream.path().join("2026-05-19-birdnet-09:01:00.wav");
        ramp(&src, 0);
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let n = queue
            .keep(&[
                kept(&src, "Pica pica", 3.0),
                kept(&src, "Corvus corax", 6.0),
                kept(&src, "Parus major", 9.0),
            ])
            .unwrap();
        assert_eq!(n, 3);
        drop(stream);

        let mut replay = queue.take();
        let (t, first) = replay.next().unwrap();
        assert_eq!(first.event.detection.scientific_name, "Pica pica");
        let batch = first.event.source_file.parent().unwrap().to_path_buf();
        assert!(first.event.source_file.is_file(), "its audio came back");
        replay.settle(t, Outcome::Settled);
        replay.flush(true);
        let (_t, second) = replay.next().unwrap();
        assert_eq!(second.event.detection.scientific_name, "Corvus corax");
        // Killed here: the second was handed out and never settled.
        drop(replay);

        let mut again = queue.take();
        assert_eq!(drain(&mut again), ["Corvus corax", "Parus major"]);
        assert!(
            !batch.exists(),
            "a batch goes once every detection is settled"
        );
        assert!(queue.take().next().is_none());
    }

    /// A detection the station could not record is offered again at the next
    /// start, and after [`MAX_ATTEMPTS`] starts it is held: not offered, and
    /// not deleted.
    #[test]
    fn a_detection_that_keeps_failing_is_held_and_not_deleted() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let src = data.path().join("2026-05-19-birdnet-09:01:00.wav");
        queue.keep(&[kept(&src, "Pica pica", 0.0)]).unwrap();
        for attempt in 1..=MAX_ATTEMPTS {
            let mut replay = queue.take();
            let (t, k) = replay
                .next()
                .unwrap_or_else(|| panic!("offered again at start {attempt}"));
            assert_eq!(k.event.detection.scientific_name, "Pica pica");
            replay.settle(t, Outcome::Retry);
        }
        let mut replay = queue.take();
        assert!(
            replay.next().is_none(),
            "offered after {MAX_ATTEMPTS} failures"
        );
        assert_eq!(replay.held(), 1);
        // Said at every start while it is held, and only then (mutation
        // survivors in `take`'s `held > 0`, PR #313).
        let held = "held in the queue for an operator";
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::ERROR, || queue.take());
        assert!(said.contains(held) && said.contains("held=1"), "{said}");
        let clean = tempfile::tempdir().unwrap();
        let other = ClipQueue::beside(&clean.path().join("birds.db"), window());
        other.keep(&[kept(&src, "Pica pica", 0.0)]).unwrap();
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::ERROR, || other.take());
        assert!(!said.contains(held), "{said}");
        let batches = std::fs::read_dir(data.path().join(DIR_NAME))
            .unwrap()
            .count();
        assert_eq!(batches, 1, "the held detection's batch was deleted");
    }

    /// A batch written by a station before this format resumes after what it
    /// recorded as processed, and its lines read with no run and no bars.
    #[test]
    fn a_batch_from_before_acknowledgements_resumes_where_it_was() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let batch = data
            .path()
            .join(DIR_NAME)
            .join("000000000000000000000001-0");
        std::fs::create_dir_all(&batch).unwrap();
        let line = |sci: &str| {
            serde_json::json!({
                "detection": detection(sci, 0.0),
                "source": "2026-05-19-birdnet-09:01:00.wav",
                "latency_ms": 5,
                "correlation_id": "c",
                "out_of_range": null,
            })
            .to_string()
        };
        std::fs::write(
            batch.join(EVENTS),
            format!("{}\n{}\n{}\n", line("A a"), line("B b"), line("C c")),
        )
        .unwrap();
        std::fs::write(batch.join(LEGACY_DONE), "1").unwrap();
        let mut replay = queue.take();
        let (t, k) = replay.next().unwrap();
        assert_eq!(k.event.detection.scientific_name, "B b");
        assert!(k.run.is_none() && k.bars.is_none());
        replay.settle(t, Outcome::Settled);
        assert_eq!(drain(&mut replay), ["C c"]);
        assert!(!batch.exists());
    }

    /// A line this build cannot trust is held, not read and not deleted: one
    /// of a later format, and one naming a file outside its batch.
    #[test]
    fn a_line_of_a_later_format_or_naming_a_path_outside_its_batch_is_held() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let batch = data
            .path()
            .join(DIR_NAME)
            .join("000000000000000000000001-0");
        std::fs::create_dir_all(&batch).unwrap();
        let line = |v: u64, source: &str| {
            serde_json::json!({
                "v": v,
                "detection": detection("Pica pica", 0.0),
                "source": source,
                "latency_ms": 5,
                "correlation_id": "c",
                "out_of_range": null,
            })
            .to_string()
        };
        std::fs::write(
            batch.join(EVENTS),
            format!(
                "{}\n{}\n{}\n{}\n",
                line(VERSION + 1, "a.wav"),
                line(VERSION, "../../outside.wav"),
                line(VERSION, "/etc/passwd"),
                line(VERSION, "a.wav"),
            ),
        )
        .unwrap();
        let mut replay = queue.take();
        assert_eq!(replay.held(), 3);
        assert_eq!(
            drain(&mut replay),
            ["Pica pica"],
            "the good line is still replayed"
        );
        assert!(
            batch.join(EVENTS).is_file(),
            "the batch with held lines was deleted"
        );
    }

    /// The clip cut from what a stop kept is, sample for sample, the clip cut
    /// from the recordings themselves: for a window inside its recording, one
    /// reaching into the one before and one reaching into the one after, with
    /// clips allowed to span recordings and with them kept inside their own.
    #[test]
    fn the_clip_cut_from_what_a_stop_kept_is_the_clip_cut_from_the_recordings() {
        use birdnet_core::audio::extraction::Extractor;
        for own_segment_only in [false, true] {
            for start in [0.5_f32, 6.0, 11.5] {
                let data = tempfile::tempdir().unwrap();
                let stream = tempfile::tempdir().unwrap();
                for (i, t) in ["09:00:45", "09:01:00", "09:01:15"].iter().enumerate() {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_possible_truncation,
                        reason = "0..3"
                    )]
                    ramp(
                        &stream.path().join(format!("2026-05-19-birdnet-{t}.wav")),
                        i as i32 * 1000,
                    );
                }
                let src = stream.path().join("2026-05-19-birdnet-09:01:00.wav");
                let config = |dir: &Path| ExtractionConfig {
                    output_dir: dir.to_path_buf(),
                    own_segment_only,
                    ..ExtractionConfig::default()
                };
                let direct_dir = data.path().join("direct");
                let direct = Extractor::new(config(&direct_dir))
                    .extract_detection_clip(&src, &detection("Pica pica", start))
                    .unwrap();
                let queue = ClipQueue::beside(
                    &data.path().join("birds.db"),
                    ClipWindow::of(&config(&direct_dir)),
                );
                queue.keep(&[kept(&src, "Pica pica", start)]).unwrap();
                let mut replay = queue.take();
                let (_, k) = replay.next().unwrap();
                let replayed_dir = data.path().join("replayed");
                let replayed = Extractor::new(config(&replayed_dir))
                    .extract_detection_clip(&k.event.source_file, &k.event.detection)
                    .unwrap();
                let a = birdnet_core::audio::decode::decode_file(&direct.path).unwrap();
                let b = birdnet_core::audio::decode::decode_file(&replayed.path).unwrap();
                assert!(!a.samples.is_empty());
                assert_eq!(
                    a.samples, b.samples,
                    "own_segment_only={own_segment_only} start={start}: the replayed clip differs"
                );
                assert!(
                    (direct.pre_detection_secs - replayed.pre_detection_secs).abs() < 1e-6,
                    "own_segment_only={own_segment_only} start={start}: lead-in differs"
                );
                let kept_files = std::fs::read_dir(k.event.source_file.parent().unwrap())
                    .unwrap()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".wav"))
                    .count();
                if own_segment_only {
                    assert_eq!(kept_files, 1, "start={start}: a neighbour was kept");
                }
            }
        }
    }

    /// Review F03: from the next recording the queue keeps only the samples a
    /// clip reaches into it, and zeros the rest; a recording a clip does not
    /// reach into is not kept at all. (Mutation survivors in `write_batch`'s
    /// neighbour arithmetic, PR #313: the clip-equality test passes when more
    /// is kept than the clip reads.)
    #[test]
    fn from_a_neighbour_only_what_the_clip_reaches_is_kept() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let name = |t: &str| stream.path().join(format!("2026-05-19-birdnet-{t}.wav"));
        for (i, t) in [(0, "09:00:45"), (1, "09:01:00"), (2, "09:01:15")] {
            ramp(&name(t), i * 1000);
        }
        let w = window();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), w);
        let start = 11.5_f32;
        queue
            .keep(&[kept(&name("09:01:00"), "Pica pica", start)])
            .unwrap();
        let mut replay = queue.take();
        let (_, k) = replay.next().unwrap();
        let batch = k.event.source_file.parent().unwrap().to_path_buf();
        let next =
            birdnet_core::audio::decode::decode_file(&batch.join(kept_name(&name("09:01:15"))))
                .expect("the next recording is kept: the clip reaches into it");
        let reach = start + 3.0 + w.spacer - 15.0;
        assert!(
            reach > 0.1,
            "precondition: the clip reaches into the next recording"
        );
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a few seconds of samples at 48 kHz, exact in f32"
        )]
        let edge = (reach * SR as f32) as usize;
        assert!(
            next.samples[..edge - 2].iter().all(|&x| x != 0.0),
            "what the clip reads is kept"
        );
        assert!(
            next.samples[edge + 2..].iter().all(|&x| x == 0.0),
            "past {reach} s into the next recording nothing is kept"
        );
        assert!(
            !batch.join(kept_name(&name("09:00:45"))).exists(),
            "the clip does not reach back into the recording before"
        );
    }

    /// A recording that decodes to no samples keeps nothing, and nothing of
    /// its neighbours either (mutation survivor in the empty-recording guard,
    /// PR #313).
    #[test]
    fn an_empty_recording_keeps_no_audio_of_its_own_or_its_neighbours() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let name = |t: &str| stream.path().join(format!("2026-05-19-birdnet-{t}.wav"));
        ramp(&name("09:00:45"), 0);
        ramp(&name("09:01:15"), 2000);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SR,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        hound::WavWriter::create(name("09:01:00"), spec)
            .unwrap()
            .finalize()
            .unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        queue
            .keep(&[kept(&name("09:01:00"), "Pica pica", 0.0)])
            .unwrap();
        let mut replay = queue.take();
        let (_, k) = replay.next().expect("the detection is kept without audio");
        let audio = std::fs::read_dir(k.event.source_file.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".wav"))
            .count();
        assert_eq!(audio, 0, "no audio is kept for an empty recording");
    }

    /// A clip's lead-in is its spacer and its pre-capture together (mutation
    /// survivor in `ClipWindow::of`, PR #313).
    #[test]
    fn a_clip_s_lead_in_is_its_spacer_and_its_pre_capture() {
        let config = ExtractionConfig {
            pre_capture_secs: 1.5,
            ..ExtractionConfig::default()
        };
        let w = ClipWindow::of(&config);
        assert!(
            (w.lead_in - (config.spacer() + 1.5)).abs() < 1e-6,
            "{}",
            w.lead_in
        );
        assert!((w.spacer - config.spacer()).abs() < f32::EPSILON);
    }

    /// Syncing a directory that is not there is an error, so a commit into a
    /// queue whose directory vanished reports failure instead of durability
    /// (mutation survivor `sync_dir -> Ok(())`, PR #313).
    #[test]
    fn syncing_a_missing_directory_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(sync_dir(tmp.path()).is_ok());
        assert!(sync_dir(&tmp.path().join("gone")).is_err());
    }

    /// A start says when a queue it cannot list holds detections it will not
    /// replay, and says nothing when there is simply no queue (mutation
    /// survivors in `take`'s not-found guard, PR #313).
    #[test]
    fn a_queue_that_cannot_be_listed_is_reported_and_a_missing_one_is_not() {
        const SAID: &str = "cannot be listed";
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("birds.db");
        let queue = ClipQueue::beside(&db, window());
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::ERROR, || queue.take());
        assert!(!said.contains(SAID), "no queue is not an error: {said}");
        std::fs::write(data.path().join(DIR_NAME), b"not a directory").unwrap();
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::ERROR, || queue.take());
        assert!(said.contains(SAID), "{said}");
    }

    /// Two batches started in the same nanosecond are two batches: the
    /// second takes the next free name instead of failing or writing into the
    /// first. The call runs on its own thread so a loop that never ends
    /// reads as a failure here, not a hung test.
    #[test]
    fn a_batch_name_already_taken_takes_the_next() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let first = queue.new_batch_at(7).unwrap();
        std::fs::write(first.join(EVENTS), "taken").unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(queue.new_batch_at(7)));
        let second = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the second batch was named")
            .expect("and created");
        assert_ne!(first, second);
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            format!("{:024}-1", 7)
        );
        assert!(second.is_dir());
        assert_eq!(std::fs::read_dir(&second).unwrap().count(), 0);
        assert_eq!(
            std::fs::read_to_string(first.join(EVENTS)).unwrap(),
            "taken"
        );
    }

    /// A queue that cannot be created (its place on the disk is a file) is
    /// an error the caller reports, at once, and not a search for a free name
    /// that never ends.
    #[test]
    fn a_queue_that_cannot_be_created_is_an_error() {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(data.path().join(DIR_NAME), "not a directory").unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(queue.new_batch_at(7)));
        let got = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the attempt ended");
        assert!(got.is_err(), "{got:?}");
    }

    /// A batch that cannot be made for any reason but its name being taken
    /// is an error, at once, not a search for a free name: here its path is
    /// longer than the system allows, though the queue's own directory is not.
    #[test]
    fn a_batch_that_cannot_be_made_is_an_error() {
        let data = tempfile::tempdir().unwrap();
        // A queue directory a little short of PATH_MAX (4096 on Linux), so
        // the queue is made and a batch inside it is not.
        let mut parent = data.path().to_path_buf();
        while parent.as_os_str().len() + 2 * 200 + DIR_NAME.len() < 4085 {
            parent.push("a".repeat(200));
        }
        let pad = 4085 - parent.as_os_str().len() - 2 - DIR_NAME.len();
        parent.push("b".repeat(pad));
        let queue = ClipQueue::beside(&parent.join("birds.db"), window());
        if std::fs::create_dir_all(&queue.dir).is_err() {
            // A system whose paths are shorter cannot set this up; the
            // queue's own directory failing is covered above.
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(queue.new_batch_at(7)));
        let got = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the attempt ended");
        assert!(got.is_err(), "{got:?}");
    }

    /// A stop keeps every detection the loop sent before closing its channel,
    /// says how many, and leaves what was left of the last stop's replay in
    /// its own batch: the next start gets each exactly once.
    #[test]
    fn a_stop_keeps_the_channel_and_leaves_the_replay_where_it_is() {
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("birds.db");
        let src = data.path().join("2026-05-19-birdnet-09:01:00.wav");
        ramp(&src, 0);
        let queue = ClipQueue::beside(&db, window());
        assert_eq!(
            queue
                .keep(&[
                    kept(&src, "Pica pica", 0.0),
                    kept(&src, "Turdus merula", 6.0)
                ])
                .unwrap(),
            2
        );

        let (_stop, stopping) = tokio::sync::watch::channel(true);
        let mut inbox = Inbox::new(&db, window(), stopping, Box::new(|| true));
        let (tx, rx) = mpsc::channel();
        let event = |sci: &str| kept(&src, sci, 3.0).event;
        tx.send(event("Corvus corax")).unwrap();
        tx.send(event("Parus major")).unwrap();
        drop(tx);
        let mut idled = false;
        let got = inbox.next(&rx, Duration::from_millis(1), &mut || idled = true);
        assert!(matches!(got, Next::Stopping), "{got:?}");
        assert!(!idled, "a stopping inbox does not wait for detections");
        let (n, said) = crate::daemon::test_support::logged(tracing::Level::INFO, || {
            inbox.keep_the_rest(&rx, &mut |event| Kept {
                event,
                run: None,
                bars: None,
            })
        });
        assert_eq!(n, 2);
        // The stop says how many of the last stop's detections it leaves in
        // the queue (mutation survivors in `keep_the_rest` and `Replay::len`,
        // PR #313).
        let line = said
            .lines()
            .find(|l| l.contains("not yet replayed stay in the queue"))
            .unwrap_or_else(|| panic!("the stop says what it leaves: {said}"));
        assert!(line.contains("detections=2"), "{line}");

        let mut replay = queue.take();
        let mut names = drain(&mut replay);
        names.sort();
        assert_eq!(
            names,
            ["Corvus corax", "Parus major", "Pica pica", "Turdus merula"]
        );
    }

    /// A stopping processor waits for the detection loop's channel no longer
    /// than [`KEEP_WAIT`] from when it starts keeping, and no shorter: a
    /// deadline already passed would keep only what had arrived, and lose the
    /// detections of the file the loop was finishing.
    #[test]
    fn a_stop_waits_keep_wait_for_the_loop_to_finish() {
        let now = Instant::now();
        assert_eq!(keep_deadline(now).duration_since(now), KEEP_WAIT);
    }

    /// A recording already gone, or one that is not audio, does not fail the
    /// keep: the detection is kept without audio and comes back.
    #[test]
    fn a_missing_or_unreadable_recording_is_kept_without_its_audio() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let gone = stream.path().join("2026-05-19-birdnet-09:01:00.wav");
        let not_audio = stream.path().join("2026-05-19-birdnet-09:02:00.wav");
        std::fs::create_dir(&not_audio).unwrap();
        assert_eq!(
            queue
                .keep(&[
                    kept(&gone, "Pica pica", 0.0),
                    kept(&not_audio, "Corvus corax", 0.0)
                ])
                .unwrap(),
            2
        );
        let mut replay = queue.take();
        let (t, k) = replay.next().unwrap();
        assert!(!k.event.source_file.exists());
        replay.settle(t, Outcome::Settled);
        assert_eq!(drain(&mut replay), ["Corvus corax"]);
    }

    /// The start says in the journal that it is replaying what the last stop
    /// kept, and says nothing when there is nothing to replay.
    #[test]
    fn a_start_says_when_it_replays_kept_detections() {
        const SAID: &str = "processing the detections the last stop kept";
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::INFO, || queue.take());
        assert!(!said.contains(SAID), "{said}");
        let src = data.path().join("2026-05-19-birdnet-09:01:00.wav");
        queue.keep(&[kept(&src, "Pica pica", 0.0)]).unwrap();
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::INFO, || queue.take());
        assert!(said.contains(SAID), "{said}");
    }

    /// Review F04/F17, second batch: a settlement is acknowledged only once the
    /// database says what it settled is durable. Settled and not yet confirmed
    /// durable, a detection is offered again at the next start (where the
    /// processor finds its row and writes nothing twice); confirmed, it is not.
    #[test]
    fn a_settlement_is_acknowledged_only_once_durable() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let src = data.path().join("2026-05-19-birdnet-09:01:00.wav");
        queue.keep(&[kept(&src, "Pica pica", 0.0)]).unwrap();
        let mut replay = queue.take();
        let (t, _) = replay.next().unwrap();
        replay.settle(t, Outcome::Settled);
        // Killed before the database's commit was known to be on the disk.
        drop(replay);
        let mut again = queue.take();
        let (t, k) = again
            .next()
            .expect("a settlement nobody confirmed durable was acknowledged");
        assert_eq!(k.event.detection.scientific_name, "Pica pica");
        again.settle(t, Outcome::Settled);
        again.flush(true);
        assert!(
            queue.take().next().is_none(),
            "confirmed durable, it is not offered again"
        );
    }

    /// Resource bounds (second batch): a backlog spread over several sources'
    /// recordings is kept with no more than one recording decoded at a time,
    /// and each source's clips reach only that source's neighbours.
    #[test]
    fn a_large_backlog_over_several_sources_holds_one_recording_at_a_time() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let mut kept_events = Vec::new();
        for cam in ["cam1", "cam2", "cam3"] {
            for t in ["09:00:45", "09:01:00", "09:01:15", "09:01:30"] {
                let p = stream
                    .path()
                    .join(format!("2026-05-19-birdnet-{cam}-{t}.wav"));
                ramp(&p, 0);
                for start in [0.5_f32, 6.0, 11.5] {
                    for sci in [
                        "Pica pica",
                        "Corvus corax",
                        "Parus major",
                        "Turdus merula",
                        "Sitta europaea",
                    ] {
                        kept_events.push(kept(&p, sci, start));
                    }
                }
            }
        }
        assert_eq!(kept_events.len(), 180);
        held_audio::PEAK.with(|p| p.set(0));
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        assert_eq!(queue.keep(&kept_events).unwrap(), 180);
        assert_eq!(
            held_audio::PEAK.with(std::cell::Cell::get),
            1,
            "more than one recording was held decoded at once"
        );
        let batch = std::fs::read_dir(data.path().join(DIR_NAME))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        let wavs: Vec<String> = std::fs::read_dir(&batch)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
            })
            .collect();
        // Every recording of every source is reached by some clip, and none of
        // another source's names appears under the wrong one.
        assert_eq!(wavs.len(), 12, "{wavs:?}");
        let mut replay = queue.take();
        assert_eq!(drain(&mut replay).len(), 180);
    }

    /// Over its byte budget the queue keeps detections without their audio:
    /// they come back and are recorded without a clip, not lost.
    #[test]
    fn over_its_budget_the_queue_keeps_detections_without_audio() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let src = stream.path().join("2026-05-19-birdnet-09:01:00.wav");
        ramp(&src, 0);
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window()).with_budget(1);
        queue.keep(&[kept(&src, "Pica pica", 3.0)]).unwrap();
        // The first batch had room (nothing was kept yet) and holds audio.
        queue.keep(&[kept(&src, "Corvus corax", 3.0)]).unwrap();
        let mut replay = queue.take();
        let mut with_audio = 0;
        while let Some((t, k)) = replay.next() {
            with_audio += usize::from(k.event.source_file.is_file());
            replay.settle(t, Outcome::Settled);
        }
        replay.flush(true);
        assert_eq!(with_audio, 1, "the batch over budget kept its audio");
    }

    /// A batch whose audio cannot be written is written again without it, and
    /// its detections come back. Here the audio's file name is longer than
    /// the path limit allows beside the batch, as a full disk fails a write.
    #[test]
    fn a_batch_whose_audio_cannot_be_written_keeps_its_detections() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let src = stream.path().join("2026-05-19-birdnet-09:01:00.wav");
        ramp(&src, 0);
        // A queue whose batch paths leave room for `events.jsonl` and not for
        // the 31-character recording name: batch dir is `<24 digits>-0`.
        let mut parent = data.path().to_path_buf();
        let target = 4095 - 31 + 2 - (1 + DIR_NAME.len() + 1 + 26);
        while parent.as_os_str().len() + 201 < target {
            parent.push("a".repeat(200));
        }
        let pad = target - parent.as_os_str().len() - 1;
        parent.push("b".repeat(pad));
        if std::fs::create_dir_all(&parent).is_err() {
            return;
        }
        let queue = ClipQueue::beside(&parent.join("birds.db"), window());
        assert_eq!(queue.keep(&[kept(&src, "Pica pica", 3.0)]).unwrap(), 1);
        let mut replay = queue.take();
        let (t, k) = replay
            .next()
            .expect("the detection was kept without its audio");
        assert!(
            !k.event.source_file.exists(),
            "precondition: its audio could not be written"
        );
        replay.settle(t, Outcome::Settled);
        replay.flush(true);
    }

    /// A batch a stop did not finish writing (no `events.jsonl`, the batch's
    /// commit) holds no kept detection and is removed at the next start, so
    /// interrupted stops do not accumulate audio.
    #[test]
    fn an_unfinished_batch_is_removed_at_the_next_start() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        let batch = queue.new_batch_at(7).unwrap();
        ramp(&batch.join("2026-05-19-birdnet-09:01:00.wav"), 0);
        std::fs::write(batch.join(format!("{EVENTS}.part")), "{").unwrap();
        assert!(queue.take().next().is_none());
        assert!(!batch.exists(), "an unfinished batch was left on the disk");
    }

    /// Nothing kept is nothing written, and nothing to take.
    #[test]
    fn an_empty_stop_keeps_nothing() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        assert_eq!(queue.keep(&[]).unwrap(), 0);
        assert!(!data.path().join(DIR_NAME).exists());
        assert!(queue.take().next().is_none());
    }

    /// A stop with nothing left of the last one's detections says nothing
    /// about them (the counterpart of the stop's count, PR #313).
    #[test]
    fn a_stop_with_no_replay_left_says_nothing_of_one() {
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("birds.db");
        let (_stop, stopping) = tokio::sync::watch::channel(true);
        let mut inbox = Inbox::new(&db, window(), stopping, Box::new(|| true));
        let (tx, rx) = mpsc::channel::<DetectionEvent>();
        drop(tx);
        let (n, said) = crate::daemon::test_support::logged(tracing::Level::INFO, || {
            inbox.keep_the_rest(&rx, &mut |event| Kept {
                event,
                run: None,
                bars: None,
            })
        });
        assert_eq!(n, 0);
        assert!(!said.contains("not yet replayed"), "{said}");
    }
}
