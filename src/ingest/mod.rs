//! `--ingest-recordings`: analyse long recordings by splitting them into
//! station segments (review F11, RES-05).
//!
//! The live detection path decodes each file whole, which a station segment
//! (at most `SEGMENT_DURATION_MAX`) allows and a night's recording from an
//! SD card does not. This command reads such a recording one packet at a time
//! (`birdnet_core::audio::split`), writes each segment-length piece into the
//! watch directory under the name the station's own segments carry, and lets
//! the running station analyse them as it analyses any segment: same
//! classifiers, filters, run provenance and storage. It is a one-shot command;
//! the service must be running for the pieces to be consumed.
//!
//! What it takes, and refuses to guess:
//!
//! - `--ingest-source NAME`: the source the pieces are recorded under. Use a
//!   name no live source has, so imported history is never mistaken for, or
//!   named over, a live stream's segments.
//! - when the recording started: `--ingest-start` (an instant with its UTC
//!   offset; one recording only), or `--ingest-name-rule yyyymmdd_hhmmss`
//!   with `--ingest-utc-offset` (the offset the recorder wrote its names in).
//!
//! Each recording is identified by its SHA-256, the source and its start, and
//! the journal (`<database dir>/ingest/journal.jsonl`) makes a rerun stage
//! nothing twice. See [`stage`] for backpressure and the imported-history
//! marker, and [`start`] for how times are named.

pub mod journal;
pub mod stage;
pub mod start;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use birdnet_core::audio::split::{SplitReport, split_file};
use serde_json::json;

use crate::cli::Cli;
use journal::Journal;
use stage::{RecordingCtx, StageConfig, StageTally, StagingSink};
use start::NameRule;

/// How long a marker outlives its piece.
const MARKER_KEEP: Duration = Duration::from_secs(7 * 86_400);

/// The longest a source name may be.
const MAX_SOURCE_LEN: usize = 40;

/// The ingest directory for the station whose database is `db_path`.
#[must_use]
pub fn ingest_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ingest")
}

/// Where the imported-history markers are, for `db_path`'s station.
#[must_use]
pub fn marker_dir(db_path: &Path) -> PathBuf {
    ingest_dir(db_path).join("staged")
}

/// How a recording's start is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRule {
    /// Given, in seconds since the epoch.
    Given(i64),
    /// Read from its name, written at this offset east of UTC.
    Named(NameRule, i64),
}

/// Everything one run needs.
#[derive(Debug, Clone)]
pub struct IngestOptions {
    /// A recording, or a directory of them (not descended into).
    pub input: PathBuf,
    /// How each recording's start is known.
    pub start: StartRule,
    /// Piece length, in seconds.
    pub piece_secs: u32,
    /// Where the journal is.
    pub journal_dir: PathBuf,
    /// Where pieces go and how many may wait.
    pub stage: StageConfig,
}

/// What a run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Recordings staged whole this run.
    pub recordings: u64,
    /// Recordings the journal already had whole.
    pub already_imported: u64,
    /// Totals over this run's recordings.
    pub tally: StageTally,
    /// Frames decoded.
    pub decoded_frames: u64,
    /// Packets the decoder refused, and timestamp jumps, each a piece break.
    pub discontinuities: u64,
}

/// Check a source name: letters, digits and `_`, so it survives the segment
/// name's parser (`RecordingFile::parse`) intact.
///
/// # Errors
///
/// Anything else.
pub fn check_source(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > MAX_SOURCE_LEN
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(format!(
            "{name:?} is not a source name for imported recordings: use 1 to \
             {MAX_SOURCE_LEN} letters, digits or _ (for example card_2026_05)"
        ));
    }
    Ok(())
}

/// The recordings `input` names, in name order.
fn recordings(input: &Path) -> Result<Vec<PathBuf>, String> {
    let is_audio = |p: &Path| {
        p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ["wav", "flac", "mp3"]
                .iter()
                .any(|k| e.eq_ignore_ascii_case(k))
        })
    };
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let entries =
        std::fs::read_dir(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_audio(p))
        .collect();
    out.sort();
    Ok(out)
}

/// Run an import.
///
/// # Errors
///
/// The first thing that stops it, as a sentence for the operator. Whatever
/// was staged before is in the journal, and the same command carries on.
pub fn ingest(opts: &IngestOptions) -> Result<Summary, String> {
    let files = recordings(&opts.input)?;
    if files.is_empty() {
        return Err(format!(
            "no .wav, .flac or .mp3 recordings in {}",
            opts.input.display()
        ));
    }
    if matches!(opts.start, StartRule::Given(_)) && files.len() > 1 {
        return Err(format!(
            "--ingest-start gives one start, and {} holds {} recordings: give one recording, \
             or read each start from its name with --ingest-name-rule and --ingest-utc-offset",
            opts.input.display(),
            files.len()
        ));
    }
    std::fs::create_dir_all(&opts.stage.marker_dir)
        .map_err(|e| format!("cannot create {}: {e}", opts.stage.marker_dir.display()))?;
    let mut journal = Journal::open(&opts.journal_dir).map_err(|e| {
        format!(
            "cannot open the journal in {}: {e}",
            opts.journal_dir.display()
        )
    })?;
    if journal.unreadable_lines > 0 {
        tracing::warn!(
            lines = journal.unreadable_lines,
            "the import journal has lines that do not parse (a stop mid-write leaves one); \
             skipped"
        );
    }
    stage::prune_markers(&opts.stage.marker_dir, &opts.stage.watch_dir, MARKER_KEEP);

    let mut summary = Summary::default();
    let mut pending = VecDeque::new();
    for path in &files {
        let start_secs = match opts.start {
            StartRule::Given(s) => s,
            StartRule::Named(rule, offset) => {
                let stem = path.file_stem().map_or_default(|s| s.to_string_lossy());
                rule.start(&stem, offset).ok_or_else(|| {
                    format!(
                        "{} does not name its start as YYYYMMDD_HHMMSS; give it with \
                         --ingest-start instead",
                        path.display()
                    )
                })?
            }
        };
        if u64::try_from(start_secs)
            .map_or(true, |s| !birdnet_core::civil::clock_looks_plausible(s))
        {
            return Err(format!(
                "{} starts at {}, before 2024 or past 2063: the station quarantines detections \
                 dated outside those years as a clock fault, so it is not imported",
                path.display(),
                birdnet_core::civil::rfc3339_utc(start_secs)
            ));
        }
        let digest = birdnet_core::inference::identity::file_digest(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let start_ms = start_secs * 1000;
        let key = journal::key(&digest.sha256, &opts.stage.source, start_ms);
        if journal.is_done(&key) {
            summary.already_imported += 1;
            tracing::info!(file = %path.display(), "already imported; skipped");
            continue;
        }
        let rec = RecordingCtx {
            key: key.clone(),
            start_unix_ms: start_ms,
        };
        let (report, tally) = {
            let mut sink = StagingSink::new(&opts.stage, rec, &mut journal, &mut pending);
            let report = split_file(path, opts.piece_secs, &mut sink)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            (report, sink.tally)
        };
        journal
            .record_done(
                &key,
                done_detail(
                    path,
                    &digest.sha256,
                    &opts.stage.source,
                    start_secs,
                    &report,
                    &tally,
                ),
            )
            .map_err(|e| format!("cannot write the journal: {e}"))?;
        tracing::info!(
            file = %path.display(),
            pieces = report.pieces,
            staged = tally.staged,
            already_staged = tally.already_staged,
            lost_packets = report.lost_packets,
            gaps = report.gaps,
            "recording staged"
        );
        summary.recordings += 1;
        summary.decoded_frames += report.decoded_frames;
        summary.discontinuities += report.lost_packets + report.gaps;
        add(&mut summary.tally, &tally);
    }
    Ok(summary)
}

fn add(total: &mut StageTally, t: &StageTally) {
    total.staged += t.staged;
    total.already_staged += t.already_staged;
    total.ambiguous_local_time += t.ambiguous_local_time;
    total.renamed_later += t.renamed_later;
    total.uncertain_start += t.uncertain_start;
    total.peak_pending = total.peak_pending.max(t.peak_pending);
}

fn done_detail(
    path: &Path,
    sha256: &str,
    source: &str,
    start_secs: i64,
    r: &SplitReport,
    t: &StageTally,
) -> serde_json::Value {
    json!({
        "input": path.to_string_lossy(),
        "sha256": sha256,
        "source": source,
        "start_utc": birdnet_core::civil::rfc3339_utc(start_secs),
        "sample_rate": r.sample_rate,
        "pieces": r.pieces,
        "decoded_frames": r.decoded_frames,
        "zero_filled_frames": r.zero_filled_frames,
        "lost_packets": r.lost_packets,
        "lost_frames": r.lost_frames,
        "lost_packets_of_unknown_length": r.lost_packets_of_unknown_length,
        "gaps": r.gaps,
        "ambiguous_local_time": t.ambiguous_local_time,
        "renamed_later": t.renamed_later,
        "uncertain_start": t.uncertain_start,
    })
}

/// Build the options from the command line and the configuration.
///
/// # Errors
///
/// A missing or malformed argument, as a sentence.
pub fn options_from_cli(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Result<IngestOptions, String> {
    let input = cli
        .ingest_recordings
        .clone()
        .ok_or("--ingest-recordings names no recording")?;
    let source = cli.ingest_source.clone().ok_or(
        "--ingest-source is required: the source name the imported audio is recorded under",
    )?;
    check_source(&source)?;
    let start = match (
        &cli.ingest_start,
        &cli.ingest_name_rule,
        &cli.ingest_utc_offset,
    ) {
        (Some(s), None, None) => StartRule::Given(start::parse_start(s)?),
        (None, Some(rule), Some(offset)) => {
            StartRule::Named(NameRule::parse(rule)?, start::parse_utc_offset(offset)?)
        }
        (None, Some(_), None) => {
            return Err(
                "--ingest-name-rule needs --ingest-utc-offset: a recorder's name does \
                        not say which zone it was written in"
                    .into(),
            );
        }
        (None, None, _) => {
            return Err(
                "when did the recording start? Give --ingest-start (an instant with \
                        its UTC offset), or --ingest-name-rule with --ingest-utc-offset; \
                        nothing is guessed"
                    .into(),
            );
        }
        _ => {
            return Err(
                "give --ingest-start, or --ingest-name-rule with --ingest-utc-offset, not both"
                    .into(),
            );
        }
    };
    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config.and_then(|c| c.get("RECS_DIR").map(PathBuf::from)))
        .ok_or(
            "no watch directory: give --watch-dir, or set RECS_DIR, to the directory the \
                running station analyses",
        )?;
    let segment = crate::helpers::resolve::setting::<u32>(
        cli,
        "segment_duration",
        cli.segment_duration,
        config,
        "SEGMENT_DURATION",
    );
    let piece_secs = cli.ingest_segment_secs.unwrap_or(segment);
    let db = crate::helpers::db_path_from_config(config);
    Ok(IngestOptions {
        input,
        start,
        piece_secs,
        journal_dir: ingest_dir(&db),
        stage: StageConfig {
            watch_dir,
            marker_dir: marker_dir(&db),
            source,
            max_pending: usize::try_from(cli.ingest_max_pending.max(1)).unwrap_or(1),
            poll: Duration::from_millis(500),
            stall: Duration::from_secs(u64::from(cli.ingest_stall_secs.max(1))),
        },
    })
}

/// `--ingest-recordings`: run and print what happened. Returns the exit code.
pub fn run(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> i32 {
    let opts = match options_from_cli(cli, config) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match ingest(&opts) {
        Ok(s) => {
            println!(
                "{} recording(s) staged ({} pieces written, {} staged by an earlier run), {} \
                 already imported. The station analyses them as it reaches them; pieces are \
                 named for source {:?} in {}.",
                s.recordings,
                s.tally.staged,
                s.tally.already_staged,
                s.already_imported,
                opts.stage.source,
                opts.stage.watch_dir.display()
            );
            if s.discontinuities > 0 {
                println!(
                    "{} damaged packet(s) or timestamp jump(s): each began a new piece where the \
                     audio resumed; nothing was filled in.",
                    s.discontinuities
                );
            }
            if s.tally.ambiguous_local_time > 0 {
                println!(
                    "{} piece(s) fall in the repeated hour of a clock change: their detections \
                     may be stamped an hour out (the journal lists them).",
                    s.tally.ambiguous_local_time
                );
            }
            if s.tally.uncertain_start > 0 {
                println!(
                    "{} piece(s) follow audio of unknown length that could not be read: their \
                     times are late by at least that length.",
                    s.tally.uncertain_start
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests;
