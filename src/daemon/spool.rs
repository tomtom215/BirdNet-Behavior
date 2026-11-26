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
        }
    }

    /// Keep `kept` for the next start: write the detections and the audio
    /// their clips need into a new batch. Returns how many were kept.
    ///
    /// # Errors
    ///
    /// The filesystem's, when the batch could not be written; the batch is
    /// then removed, so a half-written one is never replayed.
    pub(super) fn keep(&self, kept: &[Kept]) -> std::io::Result<usize> {
        if kept.is_empty() {
            return Ok(0);
        }
        let batch = self.new_batch()?;
        let written = write_batch(&batch, kept, self.window);
        if written.is_err() {
            let _ = std::fs::remove_dir_all(&batch);
        }
        written?;
        sync_dir(&self.dir)?;
        Ok(kept.len())
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
                if batch.join(format!("{EVENTS}.part")).exists() {
                    tracing::warn!(
                        batch = %batch.display(),
                        "a batch a stop did not finish writing; left as it is"
                    );
                }
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
    let legacy_done = match std::fs::read_to_string(batch.join(LEGACY_DONE)) {
        Ok(s) => s.trim().parse::<usize>().unwrap_or_else(|_| {
            tracing::warn!(batch = %batch.display(), "a batch's progress is unreadable; replayed from its start");
            0
        }),
        Err(_) => 0,
    };
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
        .map(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
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

#[allow(
    clippy::cast_precision_loss,
    reason = "the extractor's own f32 arithmetic"
)]
fn duration_of(audio: &AudioData) -> f32 {
    audio.samples.len() as f32 / audio.sample_rate as f32
}

/// One recording a batch keeps part of.
struct KeptAudio {
    /// Its name in the batch.
    name: String,
    audio: AudioData,
    /// The sample ranges a clip needs.
    keep: Vec<std::ops::Range<usize>>,
}

/// Write `kept` and the audio their clips need into `batch`.
fn write_batch(batch: &Path, kept: &[Kept], window: ClipWindow) -> std::io::Result<()> {
    let mut lines = String::new();
    // Each recording decoded once; `None` for one that is gone or unreadable.
    let mut audio: HashMap<PathBuf, Option<KeptAudio>> = HashMap::new();
    let need = |path: &Path,
                from: f32,
                to: f32,
                audio: &mut HashMap<PathBuf, Option<KeptAudio>>| {
        let entry = audio.entry(path.to_path_buf()).or_insert_with(|| {
            let decoded = match birdnet_core::audio::decode::decode_file(path) {
                Ok(a) if !a.samples.is_empty() => a,
                Ok(_) => return None,
                Err(e) => {
                    // A recording already gone is not an error: its detection
                    // is still kept, and recorded without a clip.
                    if path.exists() {
                        tracing::warn!(
                            recording = %path.display(),
                            error = %e,
                            "a recording a kept detection needs cannot be read; kept without its audio"
                        );
                    }
                    return None;
                }
            };
            Some(KeptAudio {
                name: kept_name(path),
                audio: decoded,
                keep: Vec::new(),
            })
        });
        if let Some(k) = entry {
            let (rate, len) = (k.audio.sample_rate, k.audio.samples.len());
            let range = sample_at(from, rate, len)..sample_at(to, rate, len);
            if !range.is_empty() {
                k.keep.push(range);
            }
        }
        entry.as_ref().map(|k| k.name.clone())
    };
    for k in kept {
        let det = &k.event.detection;
        let source = &k.event.source_file;
        let mut start = det.start - window.lead_in;
        let mut stop = (det.stop + window.spacer).max(start);
        // Decoded first, for its length, as the extractor does.
        let name = need(source, 0.0, 0.0, &mut audio);
        let duration = audio
            .get(source)
            .and_then(Option::as_ref)
            .map(|k| duration_of(&k.audio));
        if let Some(d) = duration {
            if window.own_segment_only {
                start = start.clamp(0.0, d);
                stop = stop.clamp(start, d);
            }
            need(source, start.max(0.0), stop.min(d), &mut audio);
            if !window.own_segment_only {
                let (before, after) = neighbours(source);
                if start < 0.0
                    && let Some(prev) = before
                {
                    need(&prev, 0.0, 0.0, &mut audio);
                    if let Some(pd) = audio
                        .get(&prev)
                        .and_then(Option::as_ref)
                        .map(|k| duration_of(&k.audio))
                    {
                        let take = (-start).min(pd);
                        need(&prev, pd - take, pd, &mut audio);
                    }
                }
                if stop > d
                    && let Some(next) = after
                {
                    need(&next, 0.0, (stop - d).max(0.0), &mut audio);
                }
            }
        }
        let shown = name.unwrap_or_else(|| kept_name(source));
        lines.push_str(&kept_line(k, &shown).map_err(std::io::Error::other)?);
        lines.push('\n');
    }
    for k in audio.into_values().flatten() {
        if k.keep.is_empty() {
            continue;
        }
        write_masked(&batch.join(&k.name), &k.audio, &k.keep)?;
    }
    let part = batch.join(format!("{EVENTS}.part"));
    let mut file = std::fs::File::create(&part)?;
    file.write_all(lines.as_bytes())?;
    drop(file);
    birdnet_core::atomic_file::commit(&part, &batch.join(EVENTS))
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
    pub(super) fn settle(&mut self, ticket: Ticket, outcome: Outcome) {
        let Ticket { batch, index } = ticket;
        match outcome {
            Outcome::Settled => {
                if let Err(e) = append_index(&batch.join(ACKED), index) {
                    // Offered again at the next start, where the processor
                    // finds its row and settles it without writing a second.
                    tracing::warn!(batch = %batch.display(), error = %e, "could not record a kept detection as settled");
                    return;
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
            Outcome::Retry => {
                if let Err(e) = append_index(&batch.join(FAILED), index) {
                    tracing::warn!(batch = %batch.display(), error = %e, "could not record a kept detection's failed attempt");
                }
            }
        }
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
#[derive(Debug)]
pub(super) struct Inbox {
    queue: ClipQueue,
    replay: Replay,
    stopping: tokio::sync::watch::Receiver<bool>,
}

impl Inbox {
    /// The inbox of the station whose database is `db_path`, whose clips are
    /// cut in `window`, stopping when `stopping` turns true
    /// (`AppState::subscribe_shutdown`).
    #[must_use]
    pub(super) fn new(
        db_path: &Path,
        window: ClipWindow,
        stopping: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        let queue = ClipQueue::beside(db_path, window);
        let replay = queue.take();
        Self {
            queue,
            replay,
            stopping,
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
            match events.recv_timeout(poll) {
                Ok(event) => return Next::Event(Box::new((event, Origin::Live))),
                Err(mpsc::RecvTimeoutError::Timeout) => idle(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Next::Closed,
            }
        }
    }

    /// Record what became of a replayed detection.
    pub(super) fn settle(&mut self, ticket: Ticket, outcome: Outcome) {
        self.replay.settle(ticket, outcome);
    }

    /// Keep everything the detection loop sends until it closes its channel
    /// (or [`KEEP_WAIT`] passes), each made into a [`Kept`] by `freeze`.
    /// Returns how many were kept: written to the disk, not merely offered.
    ///
    /// What is left of the replay stays in its own batches, which are already
    /// on the disk with the run and bars they were kept under.
    pub(super) fn keep_the_rest(
        &mut self,
        events: &mpsc::Receiver<DetectionEvent>,
        freeze: &mut dyn FnMut(DetectionEvent) -> Kept,
    ) -> usize {
        let deadline = keep_deadline(Instant::now());
        let mut rest = Vec::new();
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
        for i in 0..(SR as i32 * 15) {
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
        assert_eq!(queue.keep(&[kept(&src, "Pica pica", 0.0)]).unwrap(), 1);

        let (_stop, stopping) = tokio::sync::watch::channel(true);
        let mut inbox = Inbox::new(&db, window(), stopping);
        let (tx, rx) = mpsc::channel();
        let event = |sci: &str| kept(&src, sci, 3.0).event;
        tx.send(event("Corvus corax")).unwrap();
        tx.send(event("Parus major")).unwrap();
        drop(tx);
        let mut idled = false;
        let got = inbox.next(&rx, Duration::from_millis(1), &mut || idled = true);
        assert!(matches!(got, Next::Stopping), "{got:?}");
        assert!(!idled, "a stopping inbox does not wait for detections");
        let n = inbox.keep_the_rest(&rx, &mut |event| Kept {
            event,
            run: None,
            bars: None,
        });
        assert_eq!(n, 2);

        let mut replay = queue.take();
        let mut names = drain(&mut replay);
        names.sort();
        assert_eq!(names, ["Corvus corax", "Parus major", "Pica pica"]);
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

    /// Nothing kept is nothing written, and nothing to take.
    #[test]
    fn an_empty_stop_keeps_nothing() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"), window());
        assert_eq!(queue.keep(&[]).unwrap(), 0);
        assert!(!data.path().join(DIR_NAME).exists());
        assert!(queue.take().next().is_none());
    }
}
