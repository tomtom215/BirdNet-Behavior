//! Put a recording's pieces in the watch directory as station segments, a few
//! at a time, and mark each as imported history.
//!
//! The detection daemon is the only analyser this station has, and it reads
//! the watch directory, so that is where pieces go: named as a segment of the
//! import's source (`YYYY-MM-DD-birdnet-<source>-HH:MM:SS.wav`, from
//! `recording_filename_at`, the formatter the capture writers use), written
//! under a name no watcher takes for audio and renamed whole.
//!
//! **Backpressure.** No more than `max_pending` of this run's pieces are in
//! the watch directory at once; staging waits for the daemon to consume one
//! (it deletes what it has analysed). A watch directory is often a small
//! tmpfs, and a flood of hours there would fill it and starve the live
//! sources. If the daemon consumes nothing for `stall`, the run stops and says
//! so; the journal lets the same command resume.
//!
//! **Imported history.** Before a piece is given its name, a marker with the
//! same name is written to the marker directory, carrying the import's key.
//! [`imported_mark`] is the hook the event processor needs (see its doc) so
//! that a detection in imported audio is stored but does not notify, post to
//! BirdWeather or publish to MQTT as if it had just been heard.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use birdnet_core::audio::capture::{AudioFormat, recording_filename_at};
use birdnet_core::audio::split::{PieceEnd, PieceSink, PieceStart, SplitError, WavPieceFile};
use serde_json::{Value, json};

use super::journal::Journal;
use super::start::station_local;

/// Where the pieces go, and how many may wait there.
#[derive(Debug, Clone)]
pub struct StageConfig {
    /// The daemon's watch directory.
    pub watch_dir: PathBuf,
    /// Where each staged piece's marker is written.
    pub marker_dir: PathBuf,
    /// The source the pieces are named for.
    pub source: String,
    /// At most this many of the run's pieces wait in the watch directory.
    pub max_pending: usize,
    /// How often to look whether one has been consumed.
    pub poll: Duration,
    /// How long to wait for the daemon to consume one before stopping.
    pub stall: Duration,
}

/// The recording being staged.
#[derive(Debug, Clone)]
pub struct RecordingCtx {
    /// Its journal key.
    pub key: String,
    /// The instant its first frame was recorded, in ms since the epoch.
    pub start_unix_ms: i64,
}

/// The piece open in the sink.
enum Open {
    /// Already staged by an earlier run: its audio is read and dropped.
    Skip,
    /// Being written.
    Writing {
        file: WavPieceFile,
        name: String,
        detail: Value,
    },
}

/// What one recording's staging did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StageTally {
    /// Pieces written to the watch directory this run.
    pub staged: u64,
    /// Pieces an earlier run had staged.
    pub already_staged: u64,
    /// Pieces whose local name the daemon will read as another instant.
    pub ambiguous_local_time: u64,
    /// Pieces whose name was moved a second later than their start, because
    /// the piece before had the same second.
    pub renamed_later: u64,
    /// Pieces whose start is uncertain (audio of unknown length lost before).
    pub uncertain_start: u64,
    /// The most of this run's pieces seen waiting in the watch directory.
    pub peak_pending: usize,
}

/// A [`PieceSink`] that stages pieces for the daemon.
pub struct StagingSink<'a> {
    cfg: &'a StageConfig,
    rec: RecordingCtx,
    journal: &'a mut Journal,
    pending: &'a mut VecDeque<PathBuf>,
    open: Option<Open>,
    last_name_secs: Option<i64>,
    /// What it did.
    pub tally: StageTally,
}

impl<'a> StagingSink<'a> {
    /// Stage `rec` under `cfg`, recording in `journal`; `pending` is the run's
    /// pieces not yet seen consumed, carried from one recording to the next.
    pub fn new(
        cfg: &'a StageConfig,
        rec: RecordingCtx,
        journal: &'a mut Journal,
        pending: &'a mut VecDeque<PathBuf>,
    ) -> Self {
        Self {
            cfg,
            rec,
            journal,
            pending,
            open: None,
            last_name_secs: None,
            tally: StageTally::default(),
        }
    }

    /// Wait until fewer than `max_pending` of the run's pieces are waiting.
    fn wait_for_room(&mut self) -> Result<(), SplitError> {
        let mut last_progress = Instant::now();
        loop {
            let before = self.pending.len();
            self.pending.retain(|p| p.exists());
            if self.pending.len() < before {
                last_progress = Instant::now();
            }
            if self.pending.len() < self.cfg.max_pending.max(1) {
                return Ok(());
            }
            if last_progress.elapsed() >= self.cfg.stall {
                return Err(SplitError::Sink(format!(
                    "the station analysed none of the {} pieces waiting in {} for {} s: is the \
                     service running with that watch directory? Nothing is lost; run the same \
                     command again to carry on from here",
                    self.pending.len(),
                    self.cfg.watch_dir.display(),
                    self.cfg.stall.as_secs()
                )));
            }
            std::thread::sleep(self.cfg.poll);
        }
    }
}

fn sink_err(what: &str, e: &std::io::Error) -> SplitError {
    SplitError::Sink(format!("{what}: {e}"))
}

impl PieceSink for StagingSink<'_> {
    fn begin(&mut self, start: &PieceStart) -> Result<(), SplitError> {
        let offset_ms = i64::try_from(start.offset_ms()).unwrap_or(i64::MAX);
        let instant_ms = self.rec.start_unix_ms.saturating_add(offset_ms);
        let mut secs = instant_ms.div_euclid(1000);
        // A name has whole seconds, and a piece after a short one can start
        // in the same second; two pieces may not share a name.
        let renamed = self.last_name_secs.is_some_and(|last| secs <= last);
        if let Some(last) = self.last_name_secs.filter(|_| renamed) {
            secs = last + 1;
        }
        self.last_name_secs = Some(secs);

        let stamp = station_local(secs)
            .ok_or_else(|| SplitError::Sink(format!("cannot name the instant {secs}")))?;
        let at = birdnet_core::civil::parse_civil(&stamp.date, &stamp.time)
            .ok_or_else(|| SplitError::Sink(format!("cannot name {stamp:?}")))?;
        let name = recording_filename_at(Some(&self.cfg.source), AudioFormat::Wav, at);
        let dest = self.cfg.watch_dir.join(&name);
        let marker = self.cfg.marker_dir.join(&name);

        if self.journal.is_staged(&self.rec.key, start.index) {
            self.tally.already_staged += 1;
            self.open = Some(Open::Skip);
            return Ok(());
        }
        // Staged and committed by a run that stopped before its journal line:
        // the marker is there and no half-written file is.
        let mut writing = dest.as_os_str().to_owned();
        writing.push(birdnet_core::audio::split::WRITING_SUFFIX);
        let writing = PathBuf::from(writing);
        if marker.exists()
            && !writing.exists()
            && read_mark(&marker).is_some_and(|m| {
                m.get("key").and_then(Value::as_str) == Some(self.rec.key.as_str())
                    && m.get("index").and_then(Value::as_u64) == Some(start.index)
            })
        {
            self.journal
                .record_piece(
                    &self.rec.key,
                    start.index,
                    json!({ "name": name, "recovered": true }),
                )
                .map_err(|e| sink_err("cannot write the journal", &e))?;
            self.tally.already_staged += 1;
            self.open = Some(Open::Skip);
            return Ok(());
        }

        self.wait_for_room()?;
        let detail = json!({
            "name": name,
            "source": self.cfg.source,
            "key": self.rec.key,
            "index": start.index,
            "offset_frames": start.offset_frames,
            "sample_rate": start.sample_rate,
            "start_unix_ms": instant_ms,
            "named_unix_secs": secs,
            "renamed_later": renamed,
            "ambiguous_local_time": stamp.ambiguous,
            "timing": format!("{:?}", start.timing),
            "cause": format!("{:?}", start.cause),
        });
        self.tally.renamed_later += u64::from(renamed);
        self.tally.ambiguous_local_time += u64::from(stamp.ambiguous);
        self.tally.uncertain_start +=
            u64::from(start.timing == birdnet_core::audio::split::Timing::Uncertain);
        let file = WavPieceFile::create(&dest, start.sample_rate)
            .map_err(|e| sink_err(&format!("cannot write {}", dest.display()), &e))?;
        self.open = Some(Open::Writing { file, name, detail });
        Ok(())
    }

    fn write(&mut self, mono: &[f32]) -> Result<(), SplitError> {
        match self.open.as_mut() {
            Some(Open::Writing { file, .. }) => file
                .write(mono)
                .map_err(|e| sink_err("cannot write a piece", &e)),
            Some(Open::Skip) => Ok(()),
            None => Err(SplitError::Sink("samples with no piece open".into())),
        }
    }

    fn end(&mut self, piece: &PieceEnd) -> Result<(), SplitError> {
        let Some(Open::Writing {
            file,
            name,
            mut detail,
        }) = self.open.take()
        else {
            return Ok(());
        };
        detail["frames"] = json!(piece.frames);
        // The marker first: the daemon must never find the piece without it.
        let marker = self.cfg.marker_dir.join(&name);
        write_mark(&marker, &detail).map_err(|e| sink_err("cannot write the marker", &e))?;
        let dest = match file.commit() {
            Ok(d) => d,
            Err(e) => {
                let _ = std::fs::remove_file(&marker);
                return Err(sink_err("cannot place a piece", &e));
            }
        };
        self.journal
            .record_piece(&self.rec.key, piece.start.index, detail)
            .map_err(|e| sink_err("cannot write the journal", &e))?;
        self.pending.push_back(dest);
        self.tally.staged += 1;
        self.tally.peak_pending = self.tally.peak_pending.max(self.pending.len());
        Ok(())
    }
}

fn write_mark(path: &Path, detail: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = birdnet_core::atomic_file::part_path(path);
    std::fs::write(&part, detail.to_string())?;
    birdnet_core::atomic_file::commit(&part, path)
}

fn read_mark(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The import marker for `source_file`, when it is a staged piece of imported
/// history; `None` for live audio.
///
/// The event processor's hook: asked once per stored detection, before any
/// notification, and a `Some` is stored and announced to no one
/// (`daemon::processor::event_processor`). `marker_dir` is
/// [`super::marker_dir`] of the station's database path. One `stat` and, for
/// an imported piece, one small read.
#[must_use]
pub fn imported_mark(marker_dir: &Path, source_file: &Path) -> Option<Value> {
    read_mark(&marker_dir.join(source_file.file_name()?))
}

/// Remove markers older than `max_age` whose piece is no longer waiting.
///
/// Kept a week, not until the piece is consumed: a detection the daemon kept
/// over a stop is replayed after the piece is gone, and must still be known
/// for imported.
pub fn prune_markers(marker_dir: &Path, watch_dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(marker_dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if old && !watch_dir.join(e.file_name()).exists() && std::fs::remove_file(e.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}
