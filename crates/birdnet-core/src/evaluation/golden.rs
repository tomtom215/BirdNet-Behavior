//! A classifier's raw output on a fixed input, recorded bit for bit (audit
//! ACC-12, RES-31).
//!
//! [`record`] runs one recording through the station's own pipeline (decode,
//! resample, chunk) and the classifier, and keeps every window's raw output as
//! the bit pattern of each `f32`, with the digests of the model, the labels and
//! the input and the platform it ran on. [`compare`] reads two records.
//!
//! - **Reproducibility (RES-31):** the same input, the same files and the same
//!   platform twice must be *bit-identical*; [`Comparison::identical`] says
//!   whether they were.
//! - **Another platform (ACC-12):** aarch64 against x86-64 need not be
//!   bit-identical (different SIMD kernels sum in a different order), so the
//!   comparison also reports the largest absolute difference, how many windows
//!   changed their top class, and how many scores crossed a threshold. Whether
//!   a difference is acceptable is the reader's judgement, made with those
//!   numbers in front of them; nothing here declares a tolerance correct.
//!
//! Neither is a statement about accuracy. A record of the real model has not
//! been produced on aarch64 by this project; `docs/book/guides/evaluation.md`
//! describes the job that would.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::detection::pipeline::{self, PipelineConfig};
use crate::inference::classifier::Classifier;

/// The format version of a record. A change to what is recorded changes it.
pub const GOLDEN_FORMAT: u32 = 1;

/// Where and from what a record was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenHeader {
    /// [`GOLDEN_FORMAT`].
    pub format: u32,
    /// `std::env::consts::ARCH` of the machine that ran it.
    pub arch: String,
    /// `std::env::consts::OS`.
    pub os: String,
    /// The crate version that produced it.
    pub crate_version: String,
    /// Lowercase hex SHA-256 of the model file.
    pub model_sha256: String,
    /// Lowercase hex SHA-256 of the labels file.
    pub labels_sha256: String,
    /// Lowercase hex SHA-256 of the input recording.
    pub input_sha256: String,
    /// The rate the pipeline resampled to.
    pub sample_rate: u32,
    /// Window length, seconds.
    pub window_secs: f32,
    /// Window step, seconds.
    pub step_secs: f32,
}

/// One window's raw output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenWindow {
    /// Where the window starts, as the bit pattern of its `f32` seconds.
    pub start_bits: u32,
    /// SHA-256 of the window's input tensor (its `f32`s, little-endian), so a
    /// difference in output can be told from a difference in input.
    pub input_sha256: String,
    /// Each output value as the bit pattern of its `f32`, in label order.
    pub output_bits: Vec<u32>,
}

/// A record: the header and every window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenRecord {
    /// Provenance.
    pub header: GoldenHeader,
    /// Every window, in order.
    pub windows: Vec<GoldenWindow>,
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn f32s_digest(values: &[f32]) -> String {
    let mut h = Sha256::new();
    for v in values {
        h.update(v.to_le_bytes());
    }
    h.finalize()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Why a record could not be made.
#[derive(Debug)]
pub enum GoldenError {
    /// A file could not be read.
    Io(std::io::Error),
    /// The pipeline refused the recording.
    Pipeline(String),
    /// The classifier failed a window.
    Inference(String),
}

impl std::fmt::Display for GoldenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Pipeline(e) => write!(f, "the pipeline refused the recording: {e}"),
            Self::Inference(e) => write!(f, "the classifier failed a window: {e}"),
        }
    }
}

impl std::error::Error for GoldenError {}

/// The pipeline a classifier is fed by when it runs alone.
///
/// Its own rate, its own input format, its own window, no overlap. A caller that wants the
/// daemon's chunk length for a model of dynamic shape (V3.0's 4.5 s) sets
/// `chunk_duration_secs` on the result.
#[must_use]
pub fn pipeline_for(classifier: &dyn Classifier) -> PipelineConfig {
    let spec = classifier.input_spec();
    PipelineConfig {
        target_sample_rate: spec.sample_rate,
        raw_audio_input: spec.is_waveform(),
        chunk_duration_secs: spec.window_secs(),
        chunk_overlap_secs: 0.0,
        ..PipelineConfig::default()
    }
}

/// Run `input` through the pipeline `config` describes and `classifier`, and
/// record every window. `model_file` and `labels_file` are hashed for the
/// header; the classifier must be the one loaded from them.
///
/// # Errors
///
/// [`GoldenError`] when a file cannot be read, the pipeline refuses the
/// recording, or the classifier fails a window: a record with a missing window
/// would compare as a difference that is not one.
pub fn record(
    input: &Path,
    config: &PipelineConfig,
    classifier: &mut dyn Classifier,
    model_file: &Path,
    labels_file: &Path,
) -> Result<GoldenRecord, GoldenError> {
    let digest = |p: &Path| {
        std::fs::read(p)
            .map(|b| sha256_hex(&b))
            .map_err(GoldenError::Io)
    };
    let chunks =
        pipeline::process_file(input, config).map_err(|e| GoldenError::Pipeline(e.to_string()))?;
    let mut windows = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let tensor = &chunk.spectrogram.data;
        let out = classifier
            .infer(tensor)
            .map_err(|e| GoldenError::Inference(e.to_string()))?;
        windows.push(GoldenWindow {
            start_bits: chunk.start_secs.to_bits(),
            input_sha256: f32s_digest(tensor),
            output_bits: out.iter().map(|v| v.to_bits()).collect(),
        });
    }
    #[allow(clippy::cast_precision_loss)]
    let step_secs = pipeline::chunk_step_samples(config) as f32 / config.target_sample_rate as f32;
    Ok(GoldenRecord {
        header: GoldenHeader {
            format: GOLDEN_FORMAT,
            arch: std::env::consts::ARCH.to_owned(),
            os: std::env::consts::OS.to_owned(),
            crate_version: env!("CARGO_PKG_VERSION").to_owned(),
            model_sha256: digest(model_file)?,
            labels_sha256: digest(labels_file)?,
            input_sha256: digest(input)?,
            sample_rate: config.target_sample_rate,
            window_secs: config.chunk_duration_secs,
            step_secs,
        },
        windows,
    })
}

/// What [`compare`] found between two records.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    /// The two records describe the same model, labels and input (their
    /// digests agree). When `false` the rest compares different things.
    pub same_inputs: bool,
    /// Every window's input tensor digest agreed.
    pub same_window_inputs: bool,
    /// Windows in each record.
    pub windows: (usize, usize),
    /// Every window and every output bit agreed.
    pub identical: bool,
    /// Output values whose bits differed.
    pub values_differing: u64,
    /// The largest absolute difference between two finite outputs.
    pub max_abs_diff: f64,
    /// Outputs finite in one record and not in the other.
    pub finiteness_mismatches: u64,
    /// Windows whose highest-scoring class differs.
    pub top1_changed: u64,
    /// Outputs on opposite sides of `threshold` in the two records (read on
    /// the raw output's own scale).
    pub threshold_crossings: u64,
    /// The threshold `threshold_crossings` was counted at.
    pub threshold: f32,
}

fn top1(v: &[f32]) -> Option<usize> {
    v.iter()
        .enumerate()
        .filter(|(_, x)| x.is_finite())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
}

/// Compare two records window by window (see the module documentation).
#[must_use]
pub fn compare(a: &GoldenRecord, b: &GoldenRecord, threshold: f32) -> Comparison {
    let same_inputs = a.header.model_sha256 == b.header.model_sha256
        && a.header.labels_sha256 == b.header.labels_sha256
        && a.header.input_sha256 == b.header.input_sha256;
    let mut c = Comparison {
        same_inputs,
        same_window_inputs: a.windows.len() == b.windows.len(),
        windows: (a.windows.len(), b.windows.len()),
        identical: a.windows.len() == b.windows.len(),
        values_differing: 0,
        max_abs_diff: 0.0,
        finiteness_mismatches: 0,
        top1_changed: 0,
        threshold_crossings: 0,
        threshold,
    };
    for (wa, wb) in a.windows.iter().zip(&b.windows) {
        if wa.input_sha256 != wb.input_sha256 || wa.start_bits != wb.start_bits {
            c.same_window_inputs = false;
            c.identical = false;
        }
        if wa.output_bits.len() != wb.output_bits.len() {
            c.identical = false;
        }
        let va: Vec<f32> = wa.output_bits.iter().map(|b| f32::from_bits(*b)).collect();
        let vb: Vec<f32> = wb.output_bits.iter().map(|b| f32::from_bits(*b)).collect();
        if top1(&va) != top1(&vb) {
            c.top1_changed += 1;
        }
        for (x, y) in va.iter().zip(&vb) {
            if x.to_bits() != y.to_bits() {
                c.values_differing += 1;
                c.identical = false;
            }
            match (x.is_finite(), y.is_finite()) {
                (true, true) => {
                    c.max_abs_diff = c.max_abs_diff.max((f64::from(*x) - f64::from(*y)).abs());
                    if (*x >= threshold) != (*y >= threshold) {
                        c.threshold_crossings += 1;
                    }
                }
                (false, false) => {}
                _ => c.finiteness_mismatches += 1,
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(outputs: &[&[f32]]) -> GoldenRecord {
        GoldenRecord {
            header: GoldenHeader {
                format: GOLDEN_FORMAT,
                arch: "x86_64".into(),
                os: "linux".into(),
                crate_version: "0".into(),
                model_sha256: "m".into(),
                labels_sha256: "l".into(),
                input_sha256: "i".into(),
                sample_rate: 48_000,
                window_secs: 3.0,
                step_secs: 3.0,
            },
            windows: outputs
                .iter()
                .enumerate()
                .map(|(i, o)| GoldenWindow {
                    #[allow(clippy::cast_precision_loss)]
                    start_bits: (i as f32 * 3.0).to_bits(),
                    input_sha256: format!("w{i}"),
                    output_bits: o.iter().map(|v| v.to_bits()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_record_is_identical_to_itself_and_survives_json() {
        let a = rec(&[&[0.1, 0.9], &[0.5, -0.0]]);
        let json = serde_json::to_string(&a).unwrap();
        let b: GoldenRecord = serde_json::from_str(&json).unwrap();
        let c = compare(&a, &b, 0.5);
        assert!(
            c.identical && c.same_inputs && c.same_window_inputs,
            "{c:?}"
        );
        assert_eq!(c.values_differing, 0);
    }

    #[test]
    fn one_ulp_is_a_difference_and_is_measured() {
        let a = rec(&[&[0.1, 0.9]]);
        let b = rec(&[&[0.1, f32::from_bits(0.9_f32.to_bits() + 1)]]);
        let c = compare(&a, &b, 0.5);
        assert!(!c.identical);
        assert_eq!(c.values_differing, 1);
        assert!(c.max_abs_diff > 0.0 && c.max_abs_diff < 1e-6);
        assert_eq!((c.top1_changed, c.threshold_crossings), (0, 0));
        // Negative zero is not zero, bit for bit.
        let z = compare(&rec(&[&[0.0]]), &rec(&[&[-0.0]]), 0.5);
        assert!(!z.identical);
        assert!(z.max_abs_diff.abs() < f64::EPSILON);
    }

    #[test]
    fn a_change_of_top_class_and_a_threshold_crossing_are_counted() {
        let c = compare(&rec(&[&[0.49, 0.6]]), &rec(&[&[0.61, 0.6]]), 0.5);
        assert_eq!(c.top1_changed, 1);
        assert_eq!(c.threshold_crossings, 1);
        let n = compare(&rec(&[&[f32::NAN, 0.1]]), &rec(&[&[0.2, 0.1]]), 0.5);
        assert_eq!(n.finiteness_mismatches, 1);
    }

    #[test]
    fn a_missing_window_or_other_files_are_said() {
        let c = compare(&rec(&[&[0.1], &[0.2]]), &rec(&[&[0.1]]), 0.5);
        assert!(!c.identical);
        assert_eq!(c.windows, (2, 1));
        let mut other = rec(&[&[0.1]]);
        other.header.model_sha256 = "other".into();
        assert!(!compare(&rec(&[&[0.1]]), &other, 0.5).same_inputs);
    }
}
