//! `--ingest-recordings` end to end, against a stand-in for the daemon that
//! consumes the watch directory as the real one does: read a segment, delete
//! it.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const RATE: u32 = 8_000;

/// A 16-bit mono WAV of `secs` seconds.
fn recording(path: &Path, secs: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..secs * RATE {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "a ramp below 400 as a test sample"
        )]
        w.write_sample(((i % 400) as i16 - 200) * 50).unwrap();
    }
    w.finalize().unwrap();
}

struct Station {
    root: tempfile::TempDir,
}

impl Station {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for d in ["watch", "db/ingest/staged", "cards"] {
            std::fs::create_dir_all(root.path().join(d)).unwrap();
        }
        Self { root }
    }
    fn watch(&self) -> PathBuf {
        self.root.path().join("watch")
    }
    fn cards(&self) -> PathBuf {
        self.root.path().join("cards")
    }
    fn options(&self, input: &Path, source: &str, start: StartRule) -> IngestOptions {
        let db = self.root.path().join("db/birds.db");
        IngestOptions {
            input: input.to_path_buf(),
            start,
            piece_secs: 15,
            journal_dir: ingest_dir(&db),
            stage: StageConfig {
                watch_dir: self.watch(),
                marker_dir: marker_dir(&db),
                source: source.to_owned(),
                max_pending: 2,
                poll: Duration::from_millis(5),
                stall: Duration::from_secs(20),
            },
        }
    }
}

/// What the stand-in daemon saw.
#[derive(Default)]
struct Seen {
    names: Vec<String>,
    frames: Vec<usize>,
    most_waiting: usize,
}

/// Consume the watch directory until `stop`: the oldest finished segment is
/// decoded and deleted, and the most seen waiting at once is kept.
fn daemon(
    watch: PathBuf,
    stop: Arc<AtomicBool>,
) -> (std::thread::JoinHandle<()>, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let out = Arc::clone(&seen);
    let handle = std::thread::spawn(move || {
        loop {
            let mut waiting: Vec<PathBuf> = std::fs::read_dir(&watch)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "wav"))
                .collect();
            waiting.sort();
            {
                let mut s = out.lock().unwrap();
                s.most_waiting = s.most_waiting.max(waiting.len());
            }
            if let Some(next) = waiting.first() {
                let audio = birdnet_core::audio::decode::decode_file(next).unwrap();
                let mut s = out.lock().unwrap();
                s.names
                    .push(next.file_name().unwrap().to_string_lossy().into_owned());
                s.frames.push(audio.samples.len());
                drop(s);
                std::fs::remove_file(next).unwrap();
            } else if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    });
    (handle, seen)
}

fn run_with_daemon(st: &Station, opts: &IngestOptions) -> (Result<Summary, String>, Seen) {
    let stop = Arc::new(AtomicBool::new(false));
    let (h, seen) = daemon(st.watch(), Arc::clone(&stop));
    let result = ingest(opts);
    stop.store(true, Ordering::SeqCst);
    h.join().unwrap();
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    (result, seen)
}

const START: &str = "2026-05-19T04:30:00Z";

/// A two-minute recording becomes eight 15-second segments of the import's
/// source, named a segment apart from the given start, each with its
/// imported-history marker; and the watch directory never holds more than
/// `max_pending` of them.
#[test]
fn a_recording_is_staged_as_segments_a_few_at_a_time() {
    let st = Station::new();
    let card = st.cards().join("night.wav");
    recording(&card, 120);
    let opts = st.options(
        &card,
        "card_1",
        StartRule::Given(start::parse_start(START).unwrap()),
    );
    let (result, seen) = run_with_daemon(&st, &opts);
    let summary = result.expect("imports");
    assert_eq!((summary.recordings, summary.tally.staged), (1, 8));
    assert_eq!(summary.decoded_frames, 120 * u64::from(RATE));
    assert_eq!(seen.frames, vec![15 * RATE as usize; 8]);
    assert!(
        seen.most_waiting <= 2 && summary.tally.peak_pending <= 2,
        "at most two waiting: the daemon saw {}, the run counted {}",
        seen.most_waiting,
        summary.tally.peak_pending
    );
    let start = start::parse_start(START).unwrap();
    for (i, name) in seen.names.iter().enumerate() {
        let rec = birdnet_core::detection::types::RecordingFile::parse(name)
            .unwrap_or_else(|| panic!("{name} is a station segment name"));
        assert_eq!(rec.rtsp_id.as_deref(), Some("card_1"));
        let want = start::station_local(start + 15 * i64::try_from(i).unwrap()).unwrap();
        assert_eq!((rec.date, rec.time), (want.date, want.time), "{name}");
        let mark = stage::imported_mark(&opts.stage.marker_dir, Path::new(name))
            .unwrap_or_else(|| panic!("{name} is marked imported"));
        assert_eq!(mark["index"], i);
        assert_eq!(mark["source"], "card_1");
    }
    // The counterpart: a live segment has no mark.
    assert!(
        stage::imported_mark(
            &opts.stage.marker_dir,
            Path::new("2026-05-19-birdnet-04:30:00.wav")
        )
        .is_none()
    );
}

/// Run twice, the second stages nothing. Imported as another source, the
/// same file is another import and is staged again.
#[test]
fn a_rerun_stages_nothing_twice() {
    let st = Station::new();
    let card = st.cards().join("night.wav");
    recording(&card, 40);
    let start = StartRule::Given(start::parse_start(START).unwrap());
    let opts = st.options(&card, "card_1", start);
    let (first, seen) = run_with_daemon(&st, &opts);
    assert_eq!(first.unwrap().tally.staged, 3);
    assert_eq!(seen.names.len(), 3);
    // A copy elsewhere is the same recording: identified by content.
    let copy = st.cards().join("copy.wav");
    std::fs::copy(&card, &copy).unwrap();
    let (again, seen) = run_with_daemon(&st, &st.options(&copy, "card_1", start));
    let again = again.unwrap();
    assert_eq!((again.already_imported, again.tally.staged), (1, 0));
    assert!(seen.names.is_empty(), "{:?}", seen.names);

    let (other, seen) = run_with_daemon(&st, &st.options(&card, "card_2", start));
    assert_eq!(other.unwrap().tally.staged, 3);
    assert!(seen.names.iter().all(|n| n.contains("-birdnet-card_2-")));
}

/// With nothing consuming, the run stops when it has waited `stall`, saying
/// how to carry on; the rerun with the station running stages only what the
/// first did not, so every segment reaches the station exactly once.
///
/// Two records say a piece was staged, the journal line and the marker, and
/// either alone resumes the run: `lose` removes one before the rerun.
fn stalled_then_rerun(lose: Option<&str>) {
    let st = Station::new();
    let card = st.cards().join("night.wav");
    recording(&card, 90);
    let mut opts = st.options(
        &card,
        "card_1",
        StartRule::Given(start::parse_start(START).unwrap()),
    );
    opts.stage.stall = Duration::from_millis(200);
    let err = ingest(&opts).expect_err("nothing consumes, so it stops");
    assert!(err.contains("run the same command again"), "{err}");
    let mut first: Vec<String> = std::fs::read_dir(st.watch())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    first.sort();
    assert_eq!(first.len(), 2, "two waited, as allowed: {first:?}");

    match lose {
        Some("journal") => {
            std::fs::remove_file(opts.journal_dir.join(journal::JOURNAL_FILE)).unwrap();
        }
        Some("markers") => {
            for e in std::fs::read_dir(&opts.stage.marker_dir).unwrap().flatten() {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        _ => {}
    }
    opts.stage.stall = Duration::from_secs(20);
    let (rerun, seen) = run_with_daemon(&st, &opts);
    let rerun = rerun.unwrap();
    assert_eq!((rerun.tally.staged, rerun.tally.already_staged), (4, 2));
    let mut all = seen.names;
    all.sort();
    let mut want: Vec<String> = all.clone();
    want.dedup();
    assert_eq!(all, want, "no segment staged twice");
    assert_eq!(all.len(), 6);
    assert!(first.iter().all(|n| all.contains(n)));
}

#[test]
fn a_stalled_run_stops_and_the_rerun_carries_on() {
    stalled_then_rerun(None);
}

/// A stop after a piece was placed and before its journal line was synced:
/// the marker alone says it was staged.
#[test]
fn a_rerun_without_the_journal_lines_resumes_from_the_markers() {
    stalled_then_rerun(Some("journal"));
}

/// The counterpart: with the markers gone (pruned), the journal alone says it.
#[test]
fn a_rerun_without_the_markers_resumes_from_the_journal() {
    stalled_then_rerun(Some("markers"));
}

/// `AudioMoth` names give each recording's start; a directory needs them.
#[test]
fn a_folder_of_recorder_files_is_dated_by_their_names() {
    let st = Station::new();
    recording(&st.cards().join("20260519_043000.WAV"), 20);
    recording(&st.cards().join("20260519_050000.WAV"), 20);
    std::fs::write(st.cards().join("CONFIG.TXT"), "not audio").unwrap();
    let opts = st.options(
        &st.cards(),
        "moth_a",
        StartRule::Named(NameRule::YyyymmddHhmmss, 0),
    );
    let (result, seen) = run_with_daemon(&st, &opts);
    assert_eq!(result.unwrap().recordings, 2);
    let second = start::station_local(start::parse_start("2026-05-19T05:00:00Z").unwrap()).unwrap();
    assert!(
        seen.names
            .iter()
            .any(|n| n.starts_with(&second.date) && n.ends_with(&format!("{}.wav", second.time))),
        "{:?}",
        seen.names
    );
    assert_eq!(seen.names.len(), 4);

    // One start for a folder of recordings would date them all alike.
    let given = st.options(&st.cards(), "moth_a", StartRule::Given(0));
    assert!(ingest(&given).unwrap_err().contains("gives one start"));
}

/// The station quarantines a detection dated before 2024 as a clock fault, so
/// such a recording is refused here rather than imported into quarantine.
#[test]
fn a_recording_dated_before_2024_is_refused() {
    let st = Station::new();
    let card = st.cards().join("old.wav");
    recording(&card, 5);
    let opts = st.options(
        &card,
        "card_1",
        StartRule::Given(start::parse_start("2019-05-19T04:30:00Z").unwrap()),
    );
    let err = ingest(&opts).unwrap_err();
    assert!(err.contains("before 2024"), "{err}");
    assert_eq!(std::fs::read_dir(st.watch()).unwrap().count(), 0);
}

#[test]
fn the_command_line_names_what_is_missing() {
    use crate::helpers::test_support::cli_from;
    let e = options_from_cli(
        &cli_from(&[
            "birdnet-behavior",
            "--ingest-recordings",
            "/x",
            "--ingest-source",
            "c1",
            "--watch-dir",
            "/w",
        ]),
        None,
    )
    .unwrap_err();
    assert!(e.contains("nothing is guessed"), "{e}");
    let e = options_from_cli(
        &cli_from(&[
            "birdnet-behavior",
            "--ingest-recordings",
            "/x",
            "--ingest-source",
            "c-1",
            "--ingest-start",
            START,
        ]),
        None,
    )
    .unwrap_err();
    assert!(e.contains("not a source name"), "{e}");
    let e = options_from_cli(
        &cli_from(&[
            "birdnet-behavior",
            "--ingest-recordings",
            "/x",
            "--ingest-source",
            "c1",
            "--ingest-name-rule",
            "audiomoth",
        ]),
        None,
    )
    .unwrap_err();
    assert!(e.contains("--ingest-utc-offset"), "{e}");
    let ok = options_from_cli(
        &cli_from(&[
            "birdnet-behavior",
            "--ingest-recordings",
            "/x",
            "--ingest-source",
            "c1",
            "--ingest-start",
            START,
            "--watch-dir",
            "/w",
        ]),
        None,
    )
    .unwrap();
    assert_eq!(ok.piece_secs, 15, "the station's segment length");
    assert_eq!(ok.stage.max_pending, 4);
}
