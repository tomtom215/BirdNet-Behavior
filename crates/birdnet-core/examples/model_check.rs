//! Instruments for the model and the station's output (ACC-03, ACC-12, ACC-20,
//! MMP-13, MMP-15, RES-31). No field measurement has been made with them; see
//! `docs/book/guides/evaluation.md` for the protocol.
//!
//! ```text
//! model_check record  --model M --labels L --input WAV --out golden.json [--window-secs S] [--threads N]
//! model_check compare A.json B.json [--threshold T] [--max-abs-diff X]
//! model_check bench   --model M --labels L [--input WAV] [--runs N] [--warmup N] [--threads N]
//! model_check predict --model M --labels L --recordings DIR [--window-secs S] [--floor F] [--threads N]
//! model_check score   --predictions P.tsv --selections PATH... [--species-column C]
//!                     [--thresholds 0.1,0.2,...] [--replicates N] [--seed N] [--min-overlap S]
//! ```
//!
//! `compare` exits 1 when the records differ (bit for bit, or beyond
//! `--max-abs-diff` when it is given), so it can gate a CI job.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use birdnet_core::evaluation::golden::{self, GoldenRecord};
use birdnet_core::evaluation::scoring::{self, MatchRule, Prediction, ScoringPlan};
use birdnet_core::evaluation::selection_table::{self, recording_stem};
use birdnet_core::evaluation::timing::{self, TimingSummary};
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::model::{BirdNetModel, ModelConfig};

struct Args(Vec<String>);

impl Args {
    fn value(&self, flag: &str) -> Option<&str> {
        self.0
            .iter()
            .position(|a| a == flag)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }

    fn values(&self, flag: &str) -> Vec<&str> {
        let Some(i) = self.0.iter().position(|a| a == flag) else {
            return Vec::new();
        };
        self.0[i + 1..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .map(String::as_str)
            .collect()
    }

    fn need(&self, flag: &str) -> Result<&str, String> {
        self.value(flag)
            .ok_or_else(|| format!("{flag} is required"))
    }

    fn parsed<T: std::str::FromStr>(&self, flag: &str, default: T) -> Result<T, String> {
        self.value(flag).map_or(Ok(default), |v| {
            v.parse()
                .map_err(|_| format!("{flag}: `{v}` does not parse"))
        })
    }

    fn positional(&self) -> Vec<&str> {
        let mut out = Vec::new();
        let mut skip = false;
        for a in &self.0[1..] {
            if skip {
                skip = false;
            } else if a.starts_with("--") {
                skip = true;
            } else {
                out.push(a.as_str());
            }
        }
        out
    }
}

fn load(args: &Args, threshold_all: bool) -> Result<(BirdNetModel, PathBuf, PathBuf), String> {
    let model_path = PathBuf::from(args.need("--model")?);
    let labels_path = PathBuf::from(args.need("--labels")?);
    let labels = LabelSet::load(&labels_path).map_err(|e| e.to_string())?;
    let n = labels.len();
    let mut config = ModelConfig {
        num_threads: args.parsed("--threads", ModelConfig::default().num_threads)?,
        ..ModelConfig::default()
    };
    if threshold_all {
        config.confidence_threshold = 0.0;
        config.top_n = n;
    }
    let model = BirdNetModel::load(&model_path, labels, config).map_err(|e| e.to_string())?;
    Ok((model, model_path, labels_path))
}

fn pipeline(
    args: &Args,
    model: &BirdNetModel,
) -> Result<birdnet_core::detection::pipeline::PipelineConfig, String> {
    let mut config = golden::pipeline_for(model);
    if let Some(w) = args.value("--window-secs") {
        config.chunk_duration_secs = w.parse().map_err(|_| format!("--window-secs `{w}`"))?;
    }
    Ok(config)
}

/// The pipeline reads the recording's date and time from a BirdNET-style
/// name, so each input is staged under one.
fn staged(dir: &Path, input: &Path) -> Result<PathBuf, String> {
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("wav");
    let to = dir.join(format!("2000-01-03-birdnet-00:00:00.{ext}"));
    std::fs::copy(input, &to).map_err(|e| format!("{}: {e}", input.display()))?;
    Ok(to)
}

fn record(args: &Args) -> Result<ExitCode, String> {
    let (mut model, model_path, labels_path) = load(args, false)?;
    let config = pipeline(args, &model)?;
    let input = PathBuf::from(args.need("--input")?);
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let wav = staged(tmp.path(), &input)?;
    let rec = golden::record(&wav, &config, &mut model, &model_path, &labels_path)
        .map_err(|e| e.to_string())?;
    let out = args.need("--out")?;
    let json = serde_json::to_string_pretty(&rec).map_err(|e| e.to_string())?;
    std::fs::write(out, json + "\n").map_err(|e| e.to_string())?;
    eprintln!(
        "{} windows recorded on {}/{}",
        rec.windows.len(),
        rec.header.arch,
        rec.header.os
    );
    Ok(ExitCode::SUCCESS)
}

fn compare(args: &Args) -> Result<ExitCode, String> {
    let files = args.positional();
    let [a, b] = files[..] else {
        return Err("compare takes two records".to_owned());
    };
    let read = |p: &str| -> Result<GoldenRecord, String> {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{p}: {e}"))
    };
    let c = golden::compare(&read(a)?, &read(b)?, args.parsed("--threshold", 0.5)?);
    println!(
        "{}",
        serde_json::to_string_pretty(&c).map_err(|e| e.to_string())?
    );
    let pass = match args.value("--max-abs-diff") {
        Some(x) => {
            let x: f64 = x.parse().map_err(|_| format!("--max-abs-diff `{x}`"))?;
            c.same_inputs
                && c.same_window_inputs
                && c.windows.0 == c.windows.1
                && c.finiteness_mismatches == 0
                && c.max_abs_diff <= x
        }
        None => c.identical && c.same_inputs,
    };
    Ok(if pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn bench(args: &Args) -> Result<ExitCode, String> {
    let (mut model, _, _) = load(args, false)?;
    let spec = model.input_spec();
    let config = pipeline(args, &model)?;
    let window: Vec<f32> = if let Some(input) = args.value("--input") {
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let wav = staged(tmp.path(), Path::new(input))?;
        let chunks = birdnet_core::detection::pipeline::process_file(&wav, &config)
            .map_err(|e| e.to_string())?;
        chunks
            .into_iter()
            .next()
            .ok_or("the input has no window")?
            .spectrogram
            .data
    } else {
        // Deterministic low-level noise, one window long.
        let mut x: u32 = 1;
        (0..spec.window_samples)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "a 16-bit noise sample scaled into a small amplitude"
                )]
                let v = f32::from(u16::try_from(x >> 16).unwrap_or(0)) / 65_536.0 - 0.5;
                v * 0.05
            })
            .collect()
    };
    let runs = args.parsed("--runs", 50_usize)?;
    let warmup = args.parsed("--warmup", 5_usize)?;
    let samples = timing::bench(&mut model, &window, warmup, runs).map_err(|e| e.to_string())?;
    let summary = TimingSummary::from_durations(&samples).ok_or("no runs")?;
    let out = serde_json::json!({
        "arch": std::env::consts::ARCH,
        "os": std::env::consts::OS,
        "threads": args.parsed("--threads", ModelConfig::default().num_threads)?,
        "sample_rate": spec.sample_rate,
        "window_samples": spec.window_samples,
        "warmup": warmup,
        "per_window": summary,
        "note": "one machine under its load at the time; not a Pi measurement unless run on one",
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
    );
    Ok(ExitCode::SUCCESS)
}

fn predict(args: &Args) -> Result<ExitCode, String> {
    let (mut model, _, _) = load(args, true)?;
    let config = pipeline(args, &model)?;
    let floor: f32 = args.parsed("--floor", 0.01)?;
    let dir = PathBuf::from(args.need("--recordings")?);
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["wav", "flac", "mp3", "ogg"].contains(&e.to_ascii_lowercase().as_str())
            })
        })
        .collect();
    inputs.sort();
    let mut predictions: Vec<Prediction> = Vec::new();
    for input in &inputs {
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let wav = staged(tmp.path(), input)?;
        let chunks = birdnet_core::detection::pipeline::process_file(&wav, &config)
            .map_err(|e| format!("{}: {e}", input.display()))?;
        let recording = recording_stem(&input.to_string_lossy());
        for chunk in &chunks {
            let p = model
                .predict_chunk(
                    &chunk.spectrogram.data,
                    "2000-01-03",
                    "00:00:00",
                    chunk.start_secs,
                    chunk.end_secs,
                    1,
                )
                .map_err(|e| format!("{}: {e}", input.display()))?;
            for d in p.detections.into_iter().filter(|d| d.confidence >= floor) {
                predictions.push(Prediction {
                    recording: recording.clone(),
                    begin_secs: f64::from(chunk.start_secs),
                    end_secs: f64::from(chunk.end_secs),
                    species: d.scientific_name,
                    score: d.confidence,
                });
            }
        }
    }
    print!("{}", scoring::to_predictions_tsv(&predictions));
    eprintln!(
        "{} recordings, {} predictions at or above {floor}",
        inputs.len(),
        predictions.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// The recording a table describes when it has no file column: Raven names
/// tables `<recording>.Table.1.selections.txt`.
fn table_recording(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name.split_once(".Table")
        .map_or_else(|| recording_stem(name), |(r, _)| r.to_owned())
}

fn score(args: &Args) -> Result<ExitCode, String> {
    let text = std::fs::read_to_string(args.need("--predictions")?).map_err(|e| e.to_string())?;
    let predictions = scoring::parse_predictions_tsv(&text)?;
    let column = args.value("--species-column").unwrap_or("Species");
    let mut tables: Vec<PathBuf> = Vec::new();
    for p in args.values("--selections") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(&p)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|f| f.extension().is_some_and(|e| e == "txt"))
                .collect();
            found.sort();
            tables.extend(found);
        } else {
            tables.push(p);
        }
    }
    if tables.is_empty() {
        return Err("--selections names no table".to_owned());
    }
    let mut annotated = BTreeSet::new();
    let mut annotations = Vec::new();
    for t in &tables {
        let content = std::fs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
        let recording = table_recording(t);
        let rows = selection_table::parse(&content, column, Some(&recording))
            .map_err(|e| format!("{}: {e}", t.display()))?;
        let has_file_column = content
            .lines()
            .next()
            .is_some_and(|h| h.contains("Begin File") || h.contains("Begin Path"));
        if !has_file_column {
            annotated.insert(recording);
        }
        annotated.extend(rows.iter().map(|a| a.recording.clone()));
        annotations.extend(rows);
    }
    let thresholds: Vec<f32> = args
        .value("--thresholds")
        .unwrap_or("0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9")
        .split(',')
        .map(|t| t.trim().parse().map_err(|_| format!("--thresholds `{t}`")))
        .collect::<Result<_, _>>()?;
    let plan = ScoringPlan {
        thresholds,
        rule: MatchRule {
            min_overlap_secs: args.parsed("--min-overlap", 0.0)?,
        },
        replicates: args.parsed("--replicates", 1000)?,
        seed: args.parsed("--seed", 1)?,
    };
    let rows = scoring::score(&annotated, &annotations, &predictions, &plan);
    print!("{}", scoring::to_tsv(&rows));
    eprintln!(
        "{} annotated recordings, {} annotations, {} predictions; Wilson 95% and recording-cluster \
         bootstrap ({} replicates, seed {}) intervals",
        annotated.len(),
        annotations.len(),
        predictions.len(),
        plan.replicates,
        plan.seed
    );
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args = Args(std::env::args().skip(1).collect());
    let result = match args.0.first().map(String::as_str) {
        Some("record") => record(&args),
        Some("compare") => compare(&args),
        Some("bench") => bench(&args),
        Some("predict") => predict(&args),
        Some("score") => score(&args),
        _ => Err(
            "usage: model_check record|compare|bench|predict|score ... (see the source's header)"
                .to_owned(),
        ),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("model_check: {e}");
            ExitCode::from(2)
        }
    }
}
