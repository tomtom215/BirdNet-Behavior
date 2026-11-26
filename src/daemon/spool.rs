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
//! hands every detection it has not processed to [`ClipQueue::keep`]: the
//! detection as the daemon produced it, and a copy of each recording it needs
//! (the segment, and the segments on either side of it that a clip may reach
//! into), written to a batch directory beside the database. That takes the
//! time to copy a few recordings, not to cut hundreds of clips. The next start
//! takes the batches back ([`ClipQueue::take`]) and the processor runs them
//! through exactly the path a live detection takes, before anything new, then
//! deletes them.
//!
//! A batch records how many of its detections were processed, after each one,
//! so a station killed during the replay does not record them twice.

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use birdnet_core::detection::daemon::DetectionEvent;
use birdnet_core::detection::types::{Detection, RecordingFile};

/// The queue's directory name, beside the database.
const DIR_NAME: &str = "clip-queue";
/// A batch's list of detections, one JSON object per line.
const EVENTS: &str = "events.jsonl";
/// How many of a batch's detections have been processed.
const DONE: &str = "done";

/// A detection as it is kept: everything the processor reads from a
/// [`DetectionEvent`] but the in-flight claim, which belongs to the process,
/// with the recording named by its file name in the batch directory. One
/// JSON object per line.
fn kept_line(event: &DetectionEvent, source: &str) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "detection": event.detection,
        "source": source,
        "latency_ms": event.latency_ms,
        "correlation_id": event.correlation_id,
        "out_of_range": event.out_of_range,
    }))
}

/// [`kept_line`] read back, its recording in `batch`.
fn read_kept(line: &str, batch: &Path) -> serde_json::Result<DetectionEvent> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    let detection: Detection = serde_json::from_value(value["detection"].take())?;
    let source: String = serde_json::from_value(value["source"].take())?;
    Ok(DetectionEvent {
        detection,
        source_file: batch.join(source),
        latency_ms: value["latency_ms"].as_u64().unwrap_or(0),
        correlation_id: value["correlation_id"].as_str().unwrap_or("").to_owned(),
        lease: None,
        out_of_range: serde_json::from_value(value["out_of_range"].take())?,
    })
}

/// The detections a stop left unprocessed, on the data disk.
#[derive(Debug, Clone)]
pub struct ClipQueue {
    dir: PathBuf,
}

impl ClipQueue {
    /// The queue beside the database at `db_path`.
    #[must_use]
    pub fn beside(db_path: &Path) -> Self {
        Self {
            dir: db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(DIR_NAME),
        }
    }

    /// Keep `events` for the next start: copy the recordings each needs into
    /// a new batch and write the detections beside them. Returns how many were
    /// kept.
    ///
    /// # Errors
    ///
    /// The filesystem's, when the batch could not be written; the batch is
    /// then removed, so a half-written one is never replayed.
    pub fn keep(&self, events: &[DetectionEvent]) -> std::io::Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let batch = self.new_batch()?;
        let written = write_batch(&batch, events);
        if written.is_err() {
            let _ = std::fs::remove_dir_all(&batch);
        }
        written.map(|()| events.len())
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
    /// taken for a name in use and searched past for ever.
    fn new_batch_at(&self, nanos: u128) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
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

    /// Every kept detection not yet processed, oldest batch first.
    #[must_use]
    pub fn take(&self) -> Replay {
        // No directory is no batch: nothing was ever kept.
        let mut batches: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(EVENTS).is_file())
            .collect();
        batches.sort();
        let mut queue = VecDeque::new();
        for batch in batches {
            let done = std::fs::read_to_string(batch.join(DONE))
                .ok()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let text = match std::fs::read_to_string(batch.join(EVENTS)) {
                Ok(text) => text,
                Err(e) => {
                    tracing::warn!(batch = %batch.display(), error = %e, "could not read kept detections");
                    continue;
                }
            };
            let total = text.lines().filter(|l| !l.trim().is_empty()).count();
            for (index, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
                if index < done {
                    continue;
                }
                match read_kept(line, &batch) {
                    Ok(event) => queue.push_back(Pending {
                        batch: batch.clone(),
                        index,
                        last: index + 1 == total,
                        event,
                    }),
                    Err(e) => {
                        tracing::warn!(batch = %batch.display(), index, error = %e, "a kept detection could not be read; skipped");
                    }
                }
            }
            if done >= total {
                let _ = std::fs::remove_dir_all(&batch);
            }
        }
        if !queue.is_empty() {
            tracing::info!(
                detections = queue.len(),
                "processing the detections the last stop kept, before any new one"
            );
        }
        Replay {
            queue,
            current: None,
        }
    }
}

/// Write `events` and the recordings they need into `batch`.
fn write_batch(batch: &Path, events: &[DetectionEvent]) -> std::io::Result<()> {
    let mut lines = String::new();
    let mut copied: Vec<PathBuf> = Vec::new();
    for event in events {
        let Some(name) = event.source_file.file_name() else {
            continue;
        };
        for path in std::iter::once(event.source_file.clone()).chain(neighbours(&event.source_file))
        {
            if copied.contains(&path) {
                continue;
            }
            if let Some(file) = path.file_name()
                && !batch.join(file).exists()
            {
                // A recording already gone is not an error: its detection is
                // still kept, and the clip is cut from what there is.
                match std::fs::copy(&path, batch.join(file)) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            copied.push(path);
        }
        lines.push_str(&kept_line(event, &name.to_string_lossy()).map_err(std::io::Error::other)?);
        lines.push('\n');
    }
    let part = batch.join(format!("{EVENTS}.part"));
    let mut file = std::fs::File::create(&part)?;
    file.write_all(lines.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&part, batch.join(EVENTS))
}

/// The recordings from the same source just before and just after `source`
/// in its directory: where a clip that reaches past either end of its own
/// segment finds the rest of its audio.
fn neighbours(source: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(here)) = (
        source.parent(),
        source.to_str().and_then(RecordingFile::parse),
    ) else {
        return Vec::new();
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
    before
        .into_iter()
        .chain(after)
        .map(|(_, p)| p.clone())
        .collect()
}

/// One kept detection on its way back through the processor.
#[derive(Debug)]
struct Pending {
    batch: PathBuf,
    index: usize,
    /// The batch's last detection: once it is processed, the batch goes.
    last: bool,
    event: DetectionEvent,
}

/// The kept detections of earlier stops, handed back one at a time.
#[derive(Debug)]
pub struct Replay {
    queue: VecDeque<Pending>,
    /// The one handed out and not yet known to be processed.
    current: Option<(PathBuf, usize, bool)>,
}

impl Replay {
    /// The next kept detection, after recording that the previous one was
    /// processed.
    pub fn next(&mut self) -> Option<DetectionEvent> {
        self.processed();
        let pending = self.queue.pop_front()?;
        self.current = Some((pending.batch, pending.index, pending.last));
        Some(pending.event)
    }

    /// Record that the detection handed out last was processed: its batch's
    /// count, or the batch itself gone after its last.
    fn processed(&mut self) {
        let Some((batch, index, last)) = self.current.take() else {
            return;
        };
        if last {
            let _ = std::fs::remove_dir_all(&batch);
        } else if let Err(e) = std::fs::write(batch.join(DONE), (index + 1).to_string()) {
            tracing::warn!(batch = %batch.display(), error = %e, "could not record a kept detection as processed");
        }
    }

    /// Whatever is left, for a stop during the replay to keep again, and the
    /// batches they came from, to delete once they are kept elsewhere.
    pub fn rest(&mut self) -> (Vec<DetectionEvent>, Vec<PathBuf>) {
        self.processed();
        let mut batches: Vec<PathBuf> = Vec::new();
        let events = self
            .queue
            .drain(..)
            .map(|p| {
                if !batches.contains(&p.batch) {
                    batches.push(p.batch);
                }
                p.event
            })
            .collect();
        (events, batches)
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
pub enum Next {
    /// Process this detection.
    Event(Box<DetectionEvent>),
    /// The detection loop has closed its channel and nothing is left.
    Closed,
    /// The station is stopping: everything not processed was handed to the
    /// queue, this many of them.
    Kept(usize),
}

/// Where the processor's detections come from: what the last stop kept,
/// then the detection loop's channel, until the station stops.
#[derive(Debug)]
pub struct Inbox {
    queue: ClipQueue,
    replay: Replay,
    stopping: tokio::sync::watch::Receiver<bool>,
}

impl Inbox {
    /// The inbox of the station whose database is `db_path`, stopping when
    /// `stopping` turns true (`AppState::subscribe_shutdown`).
    #[must_use]
    pub fn new(db_path: &Path, stopping: tokio::sync::watch::Receiver<bool>) -> Self {
        let queue = ClipQueue::beside(db_path);
        let replay = queue.take();
        Self {
            queue,
            replay,
            stopping,
        }
    }

    /// The next detection, calling `idle` at each `poll` that passes with
    /// none; once the station is stopping, everything not processed is kept
    /// instead.
    pub fn next(
        &mut self,
        events: &mpsc::Receiver<DetectionEvent>,
        poll: Duration,
        idle: &mut dyn FnMut(),
    ) -> Next {
        loop {
            if *self.stopping.borrow() {
                return Next::Kept(self.keep_the_rest(events));
            }
            if let Some(event) = self.replay.next() {
                return Next::Event(Box::new(event));
            }
            match events.recv_timeout(poll) {
                Ok(event) => return Next::Event(Box::new(event)),
                Err(mpsc::RecvTimeoutError::Timeout) => idle(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Next::Closed,
            }
        }
    }

    /// Keep what is left of the replay and everything the detection loop
    /// sends until it closes its channel (or [`KEEP_WAIT`] passes).
    fn keep_the_rest(&mut self, events: &mpsc::Receiver<DetectionEvent>) -> usize {
        let (mut rest, replayed_from) = self.replay.rest();
        let deadline = keep_deadline(Instant::now());
        while let Ok(event) =
            events.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            rest.push(event);
        }
        let n = rest.len();
        match self.queue.keep(&rest) {
            Ok(0) => {}
            Ok(kept) => {
                for batch in replayed_from {
                    let _ = std::fs::remove_dir_all(batch);
                }
                tracing::info!(
                    detections = kept,
                    "stopping: the detections not yet processed are kept beside the database \
                     and are recorded at the next start"
                );
            }
            Err(e) => tracing::error!(
                detections = n,
                error = %e,
                "stopping: the detections not yet processed could not be kept, and are lost"
            ),
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(source: &Path, sci: &str) -> DetectionEvent {
        DetectionEvent {
            detection: Detection {
                date: "2026-05-19".into(),
                time: "09:01:00".into(),
                scientific_name: sci.into(),
                common_name: sci.into(),
                confidence: 0.9,
                start: 0.0,
                stop: 3.0,
                week: 20,
                file_name_extr: None,
                model_id: Some("birdnet".into()),
                agreeing_models: None,
                model_scores: Vec::new(),
            },
            source_file: source.to_path_buf(),
            latency_ms: 5,
            correlation_id: "c".into(),
            lease: None,
            out_of_range: None,
        }
    }

    /// What a stop keeps comes back at the next start, with the recordings
    /// it needs (its own and its neighbours, and no other), even after the
    /// recordings' own directory is gone; each comes back once, and a replay
    /// interrupted part-way resumes after the last one processed.
    #[test]
    fn kept_detections_come_back_once_with_their_recordings() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let name = |t: &str| stream.path().join(format!("2026-05-19-birdnet-{t}.wav"));
        for t in ["09:00:45", "09:01:00", "09:01:15", "09:01:30"] {
            std::fs::write(name(t), t).unwrap();
        }
        let queue = ClipQueue::beside(&data.path().join("birds.db"));
        let src = name("09:01:00");
        let kept = queue
            .keep(&[
                event(&src, "Pica pica"),
                event(&src, "Corvus corax"),
                event(&src, "Parus major"),
            ])
            .unwrap();
        assert_eq!(kept, 3);
        drop(stream);

        let mut replay = queue.take();
        let first = replay.next().unwrap();
        assert_eq!(first.detection.scientific_name, "Pica pica");
        let batch = first.source_file.parent().unwrap().to_path_buf();
        let mut copies: Vec<_> = std::fs::read_dir(&batch)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
            })
            .collect();
        copies.sort();
        assert_eq!(
            copies,
            [
                "2026-05-19-birdnet-09:00:45.wav",
                "2026-05-19-birdnet-09:01:00.wav",
                "2026-05-19-birdnet-09:01:15.wav"
            ]
        );
        assert_eq!(
            std::fs::read_to_string(&first.source_file).unwrap(),
            "09:01:00"
        );
        assert_eq!(
            replay.next().unwrap().detection.scientific_name,
            "Corvus corax"
        );
        // Killed here: the second was handed out and not finished.
        drop(replay);

        let mut again = queue.take();
        assert_eq!(
            again.next().unwrap().detection.scientific_name,
            "Corvus corax",
            "the one in progress is processed again, the one before it is not"
        );
        assert_eq!(
            again.next().unwrap().detection.scientific_name,
            "Parus major"
        );
        assert!(again.next().is_none());
        assert!(
            !batch.exists(),
            "a batch goes once its last detection is processed"
        );
        assert!(queue.take().next().is_none());
    }

    /// Two batches started in the same nanosecond are two batches: the
    /// second takes the next free name instead of failing or writing into the
    /// first. The call runs on its own thread so a loop that never ends
    /// reads as a failure here, not a hung test.
    #[test]
    fn a_batch_name_already_taken_takes_the_next() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"));
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
        let queue = ClipQueue::beside(&data.path().join("birds.db"));
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
        let queue = ClipQueue::beside(&parent.join("birds.db"));
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

    /// A stop keeps everything the processor had not processed: what was
    /// left of the last stop's replay and every detection the loop sent before
    /// closing its channel. The processor is told how many, and the next start
    /// gets each of them back exactly once (the batch the replay came from is
    /// gone, so nothing is processed twice).
    #[test]
    fn a_stop_keeps_the_replay_and_the_channel_and_says_how_many() {
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("birds.db");
        let src = data.path().join("2026-05-19-birdnet-09:01:00.wav");
        std::fs::write(&src, "audio").unwrap();
        let queue = ClipQueue::beside(&db);
        assert_eq!(queue.keep(&[event(&src, "Pica pica")]).unwrap(), 1);

        let (_stop, stopping) = tokio::sync::watch::channel(true);
        let mut inbox = Inbox::new(&db, stopping);
        let (tx, rx) = mpsc::channel();
        tx.send(event(&src, "Corvus corax")).unwrap();
        tx.send(event(&src, "Parus major")).unwrap();
        drop(tx);
        let mut idled = false;
        let got = inbox.next(&rx, Duration::from_millis(1), &mut || idled = true);
        assert!(matches!(got, Next::Kept(3)), "{got:?}");
        assert!(!idled, "a stopping inbox does not wait for detections");

        let mut replay = queue.take();
        let mut names = Vec::new();
        while let Some(e) = replay.next() {
            names.push(e.detection.scientific_name);
        }
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

    /// A recording already gone when the stop keeps its detection is not an
    /// error: the detection is kept, and comes back. A recording that cannot
    /// be copied for another reason (here it is a directory) fails the keep,
    /// and leaves no half-written batch to replay.
    #[test]
    fn a_missing_recording_is_kept_without_it_and_an_unreadable_one_fails_the_keep() {
        let data = tempfile::tempdir().unwrap();
        let stream = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"));
        let gone = stream.path().join("2026-05-19-birdnet-09:01:00.wav");
        assert_eq!(queue.keep(&[event(&gone, "Pica pica")]).unwrap(), 1);
        let mut replay = queue.take();
        assert_eq!(
            replay.next().unwrap().detection.scientific_name,
            "Pica pica"
        );
        assert!(replay.next().is_none());

        let not_a_file = stream.path().join("2026-05-19-birdnet-09:02:00.wav");
        std::fs::create_dir(&not_a_file).unwrap();
        let kept = queue.keep(&[event(&not_a_file, "Corvus corax")]);
        assert!(kept.is_err(), "{kept:?}");
        assert!(queue.take().next().is_none());
        assert_eq!(
            std::fs::read_dir(data.path().join(DIR_NAME))
                .unwrap()
                .count(),
            0,
            "the failed batch was removed"
        );
    }

    /// The start says in the journal that it is replaying what the last stop
    /// kept, and says nothing when there is nothing to replay.
    #[test]
    fn a_start_says_when_it_replays_kept_detections() {
        const SAID: &str = "processing the detections the last stop kept";
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"));
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::INFO, || queue.take());
        assert!(!said.contains(SAID), "{said}");
        let src = data.path().join("2026-05-19-birdnet-09:01:00.wav");
        queue.keep(&[event(&src, "Pica pica")]).unwrap();
        let (_, said) = crate::daemon::test_support::logged(tracing::Level::INFO, || queue.take());
        assert!(said.contains(SAID), "{said}");
    }

    /// Nothing kept is nothing written, and nothing to take.
    #[test]
    fn an_empty_stop_keeps_nothing() {
        let data = tempfile::tempdir().unwrap();
        let queue = ClipQueue::beside(&data.path().join("birds.db"));
        assert_eq!(queue.keep(&[]).unwrap(), 0);
        assert!(!data.path().join(DIR_NAME).exists());
        assert!(queue.take().next().is_none());
    }
}
