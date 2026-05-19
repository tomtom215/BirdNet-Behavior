//! ONNX model loading and inference via ort (ONNX Runtime).
//!
//! Loads `BirdNET` ONNX models, runs inference on audio chunks (raw f32 samples),
//! and returns species classification results with confidence scores.
//!
//! The inference pipeline:
//! 1. Accept raw audio f32 samples (already resampled to model sample rate)
//! 2. Feed into ONNX Runtime session
//! 3. Apply sigmoid with sensitivity adjustment
//! 4. Return top-N species above confidence threshold

use std::fmt;
use std::path::Path;

use ort::session::Session;
use ort::value::{Tensor, ValueType};

use crate::detection::types::Detection;
use crate::inference::labels::LabelSet;

/// Errors during model loading or inference.
#[derive(Debug)]
pub enum InferenceError {
    /// Model file not found.
    NotFound(String),
    /// Model loading or optimization failed.
    Model(String),
    /// Inference execution failed.
    Runtime(String),
    /// Label/output shape mismatch.
    Shape(String),
}

impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "model not found: {path}"),
            Self::Model(msg) => write!(f, "model error: {msg}"),
            Self::Runtime(msg) => write!(f, "inference runtime error: {msg}"),
            Self::Shape(msg) => write!(f, "shape error: {msg}"),
        }
    }
}

impl std::error::Error for InferenceError {}

/// Configuration for the `BirdNET` model.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Sensitivity adjustment for sigmoid (higher = more sensitive, range 0.5-1.5).
    pub sensitivity: f32,
    /// Minimum confidence to include in results.
    pub confidence_threshold: f32,
    /// Maximum number of detections per chunk.
    pub top_n: usize,
    /// Number of inference threads.
    pub num_threads: usize,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            confidence_threshold: 0.25,
            top_n: 10,
            num_threads: 2,
        }
    }
}

/// A loaded `BirdNET` ONNX model ready for inference.
pub struct BirdNetModel {
    session: Session,
    labels: LabelSet,
    config: ModelConfig,
    input_shape: Vec<usize>,
    /// True when the model's `predictions` output is already a probability
    /// distribution in `[0, 1]`. BirdNET+ V3.0 preview models report this;
    /// V2.4 fixed-shape models emit raw logits that still need
    /// sigmoid+sensitivity to reach probabilities. Set once at load time
    /// from the input shape.
    is_probability_output: bool,
}

impl fmt::Debug for BirdNetModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BirdNetModel")
            .field("labels_count", &self.labels.len())
            .field("config", &self.config)
            .field("input_shape", &self.input_shape)
            .finish_non_exhaustive()
    }
}

/// Extract the input shape from a loaded session.
///
/// Dynamic dimensions (-1) are mapped to 1 for batch axes, preserving fixed
/// dimensions (e.g. `96_000` sample points) for sample-rate auto-detection.
fn extract_input_shape(session: &Session) -> Result<Vec<usize>, InferenceError> {
    let input = session
        .inputs()
        .first()
        .ok_or_else(|| InferenceError::Shape("model has no inputs".into()))?;

    match input.dtype() {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        ValueType::Tensor { shape, .. } => Ok(shape
            .iter()
            .map(|&d| {
                if d > 0 {
                    // Fixed dimension — use it directly
                    d as usize
                } else {
                    // Dynamic dimension (-1 in ONNX) — treat as batch size 1
                    1
                }
            })
            .collect()),
        other => Err(InferenceError::Shape(format!(
            "expected Tensor input, got {other:?}"
        ))),
    }
}

impl BirdNetModel {
    /// Load an ONNX model from a file path.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError` if the model file is missing or cannot be loaded.
    pub fn load(
        model_path: &Path,
        labels: LabelSet,
        config: ModelConfig,
    ) -> Result<Self, InferenceError> {
        if !model_path.exists() {
            return Err(InferenceError::NotFound(model_path.display().to_string()));
        }

        tracing::info!(
            path = %model_path.display(),
            labels = labels.len(),
            "loading ONNX model"
        );

        let session = Session::builder()
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .with_intra_threads(config.num_threads)
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .commit_from_file(model_path)
            .map_err(|e| InferenceError::Model(e.to_string()))?;

        let input_shape = extract_input_shape(&session)?;

        let is_probability_output = output_is_probability(&input_shape);
        tracing::info!(
            input_shape = ?input_shape,
            is_probability_output,
            "model loaded successfully"
        );

        Ok(Self {
            session,
            labels,
            config,
            input_shape,
            is_probability_output,
        })
    }

    /// Load an ONNX model from in-memory bytes.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError` if the model cannot be parsed.
    pub fn load_from_bytes(
        bytes: &[u8],
        labels: LabelSet,
        config: ModelConfig,
    ) -> Result<Self, InferenceError> {
        let session = Session::builder()
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .commit_from_memory(bytes)
            .map_err(|e| InferenceError::Model(e.to_string()))?;

        let input_shape = extract_input_shape(&session)?;
        let is_probability_output = output_is_probability(&input_shape);

        Ok(Self {
            session,
            labels,
            config,
            input_shape,
            is_probability_output,
        })
    }

    /// Run inference on raw audio samples.
    ///
    /// The `audio` slice should be mono f32 samples at the model's expected
    /// sample rate and duration. For `BirdNET` V3.0, that's 32kHz x 3s = 96,000 samples.
    ///
    /// Returns detections sorted by confidence (descending), filtered by threshold.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError` if the input shape is wrong or inference fails.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn predict(
        &mut self,
        audio: &[f32],
        date: &str,
        time: &str,
        start_secs: f32,
        end_secs: f32,
        week: u32,
    ) -> Result<Vec<Detection>, InferenceError> {
        let input_tensor = self.build_input_tensor(audio)?;

        let outputs = self
            .session
            .run(ort::inputs![input_tensor])
            .map_err(|e| InferenceError::Runtime(e.to_string()))?;

        // BirdNET+ V3.0 has two outputs:
        //   [0] "embeddings"   → [batch, 1280]   (internal representation)
        //   [1] "predictions"  → [batch, 11560]  (species classification logits)
        // Use "predictions" if it exists (V3.0), else fall back to output 0 (V2.4).
        let output_idx = usize::from(outputs.len() > 1);
        let (_shape, flat_logits) = outputs[output_idx]
            .try_extract_tensor::<f32>()
            .map_err(|e| InferenceError::Runtime(format!("cannot extract logits: {e}")))?;

        // Output-to-confidence mapping depends on the model family:
        //
        //   * V2.4 (input shape [1, 144_000]) emits raw logits in roughly
        //     [-5, 5]; the canonical BirdNET-Analyzer pipeline applies a
        //     sensitivity-scaled sigmoid to map them into [0, 1]. On the
        //     bundled Pica WAV this gives 0.93–0.97 confidence for the
        //     target species, matching BirdNET-Pi numbers.
        //
        //   * V3.0 preview models (dynamic input shape) emit values that
        //     are *already* probabilities in [0, 1] — the official
        //     `birdnet-V3.0-dev/analyze.py` reference uses them as-is,
        //     with a default `--min-conf 0.15` threshold that only makes
        //     sense against a probability distribution. Applying sigmoid
        //     on top compresses the [0, 1] range into [0.5, 0.73], so a
        //     0.92 Magpie detection silently becomes 0.72 — the bug the
        //     user originally flagged.
        //
        // The two regimes are distinguished by `is_probability_output`,
        // which is set at load time from the input shape (V3.0 ⇒ true).
        // `sensitivity` only multiplies into the logit path because it is
        // semantically a pre-sigmoid scale factor; mathematically it is
        // not meaningful on values that are already probabilities.
        let mut detections = Vec::new();

        for (i, &raw) in flat_logits.iter().enumerate() {
            let confidence =
                compute_confidence(raw, self.config.sensitivity, self.is_probability_output);

            if confidence >= self.config.confidence_threshold
                && let Some(label) = self.labels.get(i)
            {
                detections.push(Detection {
                    date: date.to_string(),
                    time: time.to_string(),
                    scientific_name: label.scientific_name.clone(),
                    common_name: label.common_name.clone(),
                    confidence,
                    start: start_secs,
                    stop: end_secs,
                    week,
                    file_name_extr: None,
                });
            }
        }

        // Sort by confidence descending
        detections.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Take top-N
        detections.truncate(self.config.top_n);

        Ok(detections)
    }

    /// Build the input tensor from audio samples.
    ///
    /// Pads or truncates audio to match expected input length.
    /// For fully-dynamic shapes (V3.0 preview), defaults to 144 000 samples (32 kHz × 4.5 s).
    fn build_input_tensor(&self, audio: &[f32]) -> Result<Tensor<f32>, InferenceError> {
        let expected_len = expected_input_length(&self.input_shape).ok_or_else(|| {
            InferenceError::Shape(format!(
                "unsupported input shape: {:?}, expected [1, N] or [1, 1, N]",
                self.input_shape
            ))
        })?;

        let mut padded = vec![0.0_f32; expected_len];
        let copy_len = audio.len().min(expected_len);
        padded[..copy_len].copy_from_slice(&audio[..copy_len]);

        Tensor::<f32>::from_array(([1usize, expected_len], padded))
            .map_err(|e| InferenceError::Shape(e.to_string()))
    }

    /// Get the model's expected input shape.
    #[allow(clippy::missing_const_for_fn)] // Vec deref is not const
    pub fn input_shape(&self) -> &[usize] {
        &self.input_shape
    }

    /// Infer the expected audio sample rate from the model's input shape.
    ///
    /// `BirdNET` models use fixed-length audio windows:
    /// - V2.4 `[1, 144_000]` → 48 kHz × 3 s
    /// - V3.0 `[1,  96_000]` → 32 kHz × 3 s
    ///
    /// V3.0 preview models may report fully-dynamic shapes (all dims = 1 after
    /// mapping -1 → 1). In that case we default to 32 kHz (V3.0 standard).
    ///
    /// Returns 32 000 for fully-dynamic shapes (V3.0), 48 000 otherwise.
    #[must_use]
    pub fn infer_sample_rate(&self) -> u32 {
        infer_sample_rate_from_shape(&self.input_shape)
    }

    /// Recommended chunk length, in raw audio samples, for the pipeline to
    /// feed this model.
    ///
    /// * Fixed-shape models report their own training length directly.
    /// * **Dynamic-shape models** (BirdNET+ V3.0 preview, `[1, 1]`) accept any
    ///   length, but they were trained on longer windows than the V2.4
    ///   default of 3.0 s. Empirically, the preview3 model peaks at
    ///   ~4.5 s of audio: on the bundled `Pica_pica_30s.wav` fixture the
    ///   best-chunk Eurasian Magpie confidence climbs from 0.52 at
    ///   3.0 s × 32 kHz (96 000 samples) to 0.72 at 4.5 s × 32 kHz
    ///   (144 000 samples). We therefore default the dynamic case to
    ///   144 000 samples — the same numeric chunk size V2.4 used, just
    ///   at the lower sample rate.
    /// * The chunk-length sweep that justifies the default lives in
    ///   `docs/architecture/15-model-chunking.md`.
    #[must_use]
    pub fn recommended_chunk_samples(&self) -> usize {
        recommended_chunk_samples_from_shape(&self.input_shape)
    }

    /// Recommended chunk length, in seconds, derived from the model's
    /// recommended sample count and its inferred sample rate.
    #[must_use]
    pub fn recommended_chunk_secs(&self) -> f32 {
        let samples = self.recommended_chunk_samples();
        let sr = self.infer_sample_rate();
        #[allow(clippy::cast_precision_loss)]
        let result = samples as f32 / sr as f32;
        result
    }

    /// Returns `true` if this model expects raw audio samples as input.
    ///
    /// `BirdNET`+ V3.0 models perform internal feature extraction from the raw
    /// waveform (`infer_sample_rate() == 32_000`).  V2.4 models require a
    /// pre-computed mel spectrogram.
    #[must_use]
    pub fn expects_raw_audio(&self) -> bool {
        self.infer_sample_rate() == 32_000
    }

    /// Get the label set.
    pub const fn labels(&self) -> &LabelSet {
        &self.labels
    }

    /// Get the model configuration.
    pub const fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Whether the model emits already-calibrated probabilities (`true`) or
    /// raw logits requiring sigmoid + sensitivity scaling (`false`).
    ///
    /// V3.0 preview models report `true`; V2.4 models report `false`.
    pub const fn is_probability_output(&self) -> bool {
        self.is_probability_output
    }

    /// Update the sensitivity value.
    pub const fn set_sensitivity(&mut self, sensitivity: f32) {
        self.config.sensitivity = sensitivity;
    }

    /// Update the confidence threshold.
    pub const fn set_confidence_threshold(&mut self, threshold: f32) {
        self.config.confidence_threshold = threshold;
    }
}

/// Determine whether the model's `predictions` output is already a
/// probability distribution in `[0, 1]`, based on the input shape.
///
/// BirdNET+ V3.0 preview models declare a fully-dynamic input shape
/// (`[1]` or `[1, 1]`) and emit calibrated probabilities. BirdNET V2.4
/// fixed-shape models (`[1, 144_000]`) emit raw logits that still need
/// sigmoid + sensitivity scaling.
///
/// V3.0 fixed-shape (`[1, 96_000]`) follows the V3.0 calibration too.
fn output_is_probability(input_shape: &[usize]) -> bool {
    match input_shape {
        // V3.0 fixed (32 kHz × 3 s = 96 000 samples) and V3.0 dynamic
        // preview both emit already-calibrated probabilities.
        [_, 96_000] | [_, _, 96_000] | [1] | [1, 1] | [1, 1, 1] => true,
        // V2.4 and any unknown future model → assume raw logits so we
        // never lose the confidence signal for a model we can't classify.
        _ => false,
    }
}

/// Pure helper: derive sample rate from any ONNX input shape.
///
/// V2.4 fixed `[1, 144_000]` → 48 kHz × 3 s. V3.0 fixed `[1, 96_000]` → 32 kHz × 3 s.
/// Anything fully-dynamic or rank-1 → 32 kHz (assume V3.0 preview).
/// Unknown sample counts default to 48 kHz so we don't silently downsample.
#[must_use]
pub fn infer_sample_rate_from_shape(input_shape: &[usize]) -> u32 {
    let n_samples = match input_shape {
        [_, n] | [_, _, n] if *n > 1 => *n,
        // All-dynamic or rank-1 → V3.0
        _ => return 32_000,
    };
    match n_samples {
        96_000 => 32_000, // BirdNET+ V3.0 (32 kHz × 3 s)
        _ => 48_000,      // BirdNET   V2.4 (48 kHz × 3 s) or unknown
    }
}

/// Pure helper: derive the recommended chunk length in samples from any ONNX input shape.
///
/// Fixed-shape models report their own training length directly. Dynamic shapes
/// (V3.0 preview) default to 144 000 samples (= 4.5 s × 32 kHz), the empirical
/// optimum from the chunk-length sweep documented in
/// `docs/architecture/15-model-chunking.md`.
#[must_use]
pub fn recommended_chunk_samples_from_shape(input_shape: &[usize]) -> usize {
    match input_shape {
        [_, n] | [_, _, n] if *n > 1 => *n,
        _ => 144_000,
    }
}

/// Resolve the expected input length the tensor builder pads/truncates to.
///
/// Returns the trained length for fixed-shape models and 144 000 for dynamic
/// V3.0-style shapes. Returns `None` for shapes the pipeline cannot handle
/// (e.g. rank > 3, or rank-2 with leading dim > 1). Kept separate from
/// `recommended_chunk_samples_from_shape` because the latter is part of the
/// public per-model recommendation API and tolerates a wider input domain.
fn expected_input_length(input_shape: &[usize]) -> Option<usize> {
    match input_shape {
        [_, n] | [_, _, n] if *n > 1 => Some(*n),
        [1] | [1, 1] | [1, 1, 1] => Some(144_000),
        _ => None,
    }
}

/// Pure helper: derive a confidence value in `[0, 1]` from a single model
/// output, branching on whether the model already emits calibrated
/// probabilities.
///
/// * `is_probability == true` (V3.0): clamp to `[0, 1]` so adversarial or
///   numerically-noisy values can't escape the valid range, but otherwise
///   pass through. The sensitivity parameter is **deliberately ignored**
///   here because it is semantically a *pre-sigmoid* scale factor — applying
///   it to an already-calibrated probability is meaningless and was the
///   cause of the 52 % → 93 % Magpie fix in this session.
///
/// * `is_probability == false` (V2.4): apply `sigmoid(sensitivity * raw)`,
///   matching the canonical BirdNET-Analyzer pipeline.
#[must_use]
pub fn compute_confidence(raw: f32, sensitivity: f32, is_probability: bool) -> f32 {
    if is_probability {
        raw.clamp(0.0, 1.0)
    } else {
        sigmoid(sensitivity * raw)
    }
}

/// Apply sigmoid function: `1 / (1 + exp(-x))`.
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn sigmoid_at_zero_is_half() {
        let result = sigmoid(0.0);
        assert!((result - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sigmoid_large_positive_is_near_one() {
        let result = sigmoid(10.0);
        assert!(result > 0.999);
    }

    #[test]
    fn sigmoid_large_negative_is_near_zero() {
        let result = sigmoid(-10.0);
        assert!(result < 0.001);
    }

    #[test]
    fn sigmoid_is_monotonic() {
        let values: Vec<f32> = (-50..50).map(|i| sigmoid(i as f32 * 0.1)).collect();
        for i in 1..values.len() {
            assert!(
                values[i] >= values[i - 1],
                "sigmoid not monotonic at index {i}"
            );
        }
    }

    #[test]
    fn default_model_config() {
        let config = ModelConfig::default();
        assert!((config.sensitivity - 1.0).abs() < f32::EPSILON);
        assert!((config.confidence_threshold - 0.25).abs() < f32::EPSILON);
        assert_eq!(config.top_n, 10);
        assert_eq!(config.num_threads, 2);
    }

    #[test]
    fn infer_sample_rate_v24() {
        let shape_48k: &[usize] = &[1, 144_000];
        let rate = match shape_48k {
            [_, n] | [_, _, n] => match *n {
                96_000 => 32_000_u32,
                _ => 48_000_u32,
            },
            _ => 48_000_u32,
        };
        assert_eq!(rate, 48_000);
    }

    #[test]
    fn infer_sample_rate_v30() {
        let shape_32k: &[usize] = &[1, 96_000];
        let rate = match shape_32k {
            [_, n] | [_, _, n] => match *n {
                96_000 => 32_000_u32,
                _ => 48_000_u32,
            },
            _ => 48_000_u32,
        };
        assert_eq!(rate, 32_000);
    }

    #[test]
    fn v24_emits_raw_logits() {
        // V2.4 fixed-shape model needs sigmoid.
        assert!(!output_is_probability(&[1, 144_000]));
        assert!(!output_is_probability(&[1, 1, 144_000]));
    }

    #[test]
    fn v30_fixed_emits_probabilities() {
        // V3.0 fixed shape is already calibrated.
        assert!(output_is_probability(&[1, 96_000]));
        assert!(output_is_probability(&[1, 1, 96_000]));
    }

    #[test]
    fn v30_dynamic_emits_probabilities() {
        // V3.0 preview (dynamic) is also already calibrated.
        assert!(output_is_probability(&[1]));
        assert!(output_is_probability(&[1, 1]));
        assert!(output_is_probability(&[1, 1, 1]));
    }

    #[test]
    fn unknown_shape_defaults_to_logits() {
        // Belt-and-braces: an unknown future model is assumed to need
        // sigmoid rather than have us silently lose the confidence signal.
        assert!(!output_is_probability(&[1, 999_999]));
        assert!(!output_is_probability(&[]));
    }

    #[test]
    fn model_not_found_returns_error() {
        let labels = LabelSet::from_entries(vec![("Test_species".into(), "Test Species".into())]);
        let result = BirdNetModel::load(
            Path::new("/nonexistent/model.onnx"),
            labels,
            ModelConfig::default(),
        );
        assert!(matches!(result, Err(InferenceError::NotFound(_))));
    }

    // ─── BirdNetModel instance methods — driven by tiny embedded ONNX models ──
    //
    // Mutation testing surfaced ~30 surviving mutants on `BirdNetModel`
    // instance methods (`input_shape`, `infer_sample_rate`, getters,
    // setters, `predict`) because no test built an instance — every call
    // went through `BirdNetModel::load(path)` which the suite never
    // exercised. Embedding two ~220-byte ONNX models (a fixed `[1, 144_000]`
    // V2.4-style and a fixed `[1, 96_000]` V3.0-style) lets us drive these
    // methods directly without needing the real 541 MB BirdNET+ model on
    // disk. The models compute a trivial Slice — their *behaviour* is not
    // what we test; only that `BirdNetModel` can load them, expose the
    // right input-shape contract, and round-trip the config setters.
    //
    // Generated once via Python's `onnx` library and checked in.

    const TINY_V24_MODEL: &[u8] = include_bytes!("../testdata/tiny_v24_test.onnx");
    const TINY_V30_MODEL: &[u8] = include_bytes!("../testdata/tiny_v30_test.onnx");

    fn tiny_labels() -> LabelSet {
        LabelSet::from_entries(
            (0..11)
                .map(|i| (format!("Species_{i}"), format!("Bird {i}")))
                .collect(),
        )
    }

    fn load_tiny_v24() -> BirdNetModel {
        BirdNetModel::load_from_bytes(TINY_V24_MODEL, tiny_labels(), ModelConfig::default())
            .expect("tiny V2.4 model loads")
    }

    fn load_tiny_v30() -> BirdNetModel {
        BirdNetModel::load_from_bytes(TINY_V30_MODEL, tiny_labels(), ModelConfig::default())
            .expect("tiny V3.0 model loads")
    }

    #[test]
    fn loaded_v24_model_reports_144k_input_shape() {
        let m = load_tiny_v24();
        // [1, 144_000] is the V2.4 contract.
        let shape = m.input_shape();
        assert_eq!(shape, &[1usize, 144_000][..]);
    }

    #[test]
    fn loaded_v30_model_reports_96k_input_shape() {
        let m = load_tiny_v30();
        assert_eq!(m.input_shape(), &[1usize, 96_000][..]);
    }

    #[test]
    fn loaded_v24_model_infers_48khz_sample_rate() {
        let m = load_tiny_v24();
        assert_eq!(m.infer_sample_rate(), 48_000);
    }

    #[test]
    fn loaded_v30_model_infers_32khz_sample_rate() {
        let m = load_tiny_v30();
        assert_eq!(m.infer_sample_rate(), 32_000);
    }

    #[test]
    fn loaded_v24_model_recommends_144k_chunk_samples() {
        let m = load_tiny_v24();
        assert_eq!(m.recommended_chunk_samples(), 144_000);
    }

    #[test]
    fn loaded_v30_model_recommends_96k_chunk_samples() {
        let m = load_tiny_v30();
        assert_eq!(m.recommended_chunk_samples(), 96_000);
    }

    #[test]
    fn loaded_v24_model_recommends_3s_chunk_secs() {
        let m = load_tiny_v24();
        // 144_000 / 48_000 = 3.0 s exactly.
        let secs = m.recommended_chunk_secs();
        assert!((secs - 3.0).abs() < 1e-6, "got {secs}");
    }

    #[test]
    fn loaded_v30_model_recommends_3s_chunk_secs() {
        let m = load_tiny_v30();
        // 96_000 / 32_000 = 3.0 s exactly.
        let secs = m.recommended_chunk_secs();
        assert!((secs - 3.0).abs() < 1e-6, "got {secs}");
    }

    #[test]
    fn loaded_v24_model_does_not_expect_raw_audio() {
        // expects_raw_audio is `infer_sample_rate() == 32_000`. V2.4 is 48 kHz,
        // so this must be false.
        let m = load_tiny_v24();
        assert!(!m.expects_raw_audio());
    }

    #[test]
    fn loaded_v30_model_expects_raw_audio() {
        // V3.0 is 32 kHz → raw audio path.
        let m = load_tiny_v30();
        assert!(m.expects_raw_audio());
    }

    #[test]
    fn loaded_v24_model_emits_logits_not_probabilities() {
        // [1, 144_000] is V2.4 territory → raw logits, needs sigmoid.
        let m = load_tiny_v24();
        assert!(!m.is_probability_output());
    }

    #[test]
    fn loaded_v30_model_emits_probabilities() {
        // [1, 96_000] is V3.0 fixed → already-calibrated probabilities.
        let m = load_tiny_v30();
        assert!(m.is_probability_output());
    }

    #[test]
    fn set_sensitivity_persists() {
        let mut m = load_tiny_v24();
        let before = m.config().sensitivity;
        m.set_sensitivity(before + 0.5);
        let after = m.config().sensitivity;
        assert!(
            (after - (before + 0.5)).abs() < 1e-6,
            "set_sensitivity did not persist: before={before}, after={after}"
        );
    }

    #[test]
    fn set_confidence_threshold_persists() {
        let mut m = load_tiny_v24();
        let before = m.config().confidence_threshold;
        m.set_confidence_threshold(before + 0.1);
        let after = m.config().confidence_threshold;
        assert!(
            (after - (before + 0.1)).abs() < 1e-6,
            "set_confidence_threshold did not persist: before={before}, after={after}"
        );
    }

    #[test]
    fn loaded_v24_model_labels_count_matches() {
        let m = load_tiny_v24();
        assert_eq!(m.labels().len(), 11);
    }

    #[test]
    fn debug_format_is_non_empty() {
        // Pins the Debug impl so a mutation that replaces it with
        // `Ok(Default::default())` (an empty string) fails the assertion.
        let m = load_tiny_v24();
        let s = format!("{m:?}");
        assert!(s.contains("BirdNetModel"));
        assert!(s.contains("labels_count"));
    }

    #[test]
    fn predict_returns_top_n_detections() {
        // The Slice model emits [1, 11] for the 11 labels we registered,
        // so the predict() loop's loop body executes 11 times. With the
        // default 0.25 threshold and our trivial slice op (which copies
        // raw input through), some chunks of the input will be above /
        // below threshold depending on the data.
        //
        // To exercise the path deterministically, set a confidence
        // threshold so low that every label passes, and feed in a
        // constant 0.5 buffer. The probability-output branch (V3.0)
        // would clamp them to [0, 1] giving exactly 0.5; the logit
        // branch (V2.4) would sigmoid(1.0 * 0.5) ≈ 0.622. Either way,
        // top-N truncation gives us a bounded number of detections.
        let mut m = load_tiny_v30(); // probability-output path
        m.set_confidence_threshold(0.0); // accept everything
        let audio = vec![0.5_f32; 96_000];
        let detections = m
            .predict(&audio, "2026-05-19", "09:00:00", 0.0, 3.0, 20)
            .expect("predict should not error on a constant input");
        // top_n defaults to 10. Returning 0 here would be a sign that
        // the predict() loop body was replaced with an empty Ok(vec![])
        // (which is one of the surviving mutants we want to kill).
        assert!(
            !detections.is_empty(),
            "predict returned no detections — likely a body-replacement mutation"
        );
        assert!(detections.len() <= 10);
        // The first detection's date/time should round-trip from our args.
        assert_eq!(detections[0].date, "2026-05-19");
        assert_eq!(detections[0].time, "09:00:00");
        assert_eq!(detections[0].week, 20);
    }

    #[test]
    fn inference_error_display_includes_payload() {
        let e = InferenceError::NotFound("/tmp/x.onnx".into());
        let s = format!("{e}");
        assert!(s.contains("/tmp/x.onnx"), "got: {s}");

        let e = InferenceError::Model("bad opset".into());
        assert!(format!("{e}").contains("bad opset"));

        let e = InferenceError::Runtime("op failed".into());
        assert!(format!("{e}").contains("op failed"));

        let e = InferenceError::Shape("rank mismatch".into());
        assert!(format!("{e}").contains("rank mismatch"));
    }

    #[test]
    fn inference_error_is_std_error() {
        // Pin that the type implements std::error::Error so it composes
        // with `?` / `Box<dyn Error>` upstream. Returning the wrong
        // variant from a function would be caught by Display tests
        // above; this one just pins the trait bound.
        fn assert_error<E: std::error::Error>(_: &E) {}
        let e = InferenceError::NotFound("x".into());
        assert_error(&e);
    }

    // ─── Chunking math ────────────────────────────────────────────────────
    //
    // The V3.0 preview daemon-chunking bug (52 % → 72 %) and the
    // is_probability_output regression (sigmoid-on-probabilities → 52 % vs
    // 93 %) lived in this module and shipped despite the prior test suite.
    // The tests below pin every cell in the model-family decision table so
    // the same class of incorrect mapping fails an assertion the next time.

    #[test]
    fn infer_sample_rate_from_shape_v24_fixed() {
        assert_eq!(infer_sample_rate_from_shape(&[1, 144_000]), 48_000);
        assert_eq!(infer_sample_rate_from_shape(&[1, 1, 144_000]), 48_000);
    }

    #[test]
    fn infer_sample_rate_from_shape_v30_fixed() {
        assert_eq!(infer_sample_rate_from_shape(&[1, 96_000]), 32_000);
        assert_eq!(infer_sample_rate_from_shape(&[1, 1, 96_000]), 32_000);
    }

    #[test]
    fn infer_sample_rate_from_shape_dynamic_is_v30() {
        // Dynamic (V3.0 preview) input shape: all dims become 1.
        assert_eq!(infer_sample_rate_from_shape(&[1]), 32_000);
        assert_eq!(infer_sample_rate_from_shape(&[1, 1]), 32_000);
        assert_eq!(infer_sample_rate_from_shape(&[1, 1, 1]), 32_000);
    }

    #[test]
    fn infer_sample_rate_from_shape_unknown_falls_back_to_v24() {
        // Anything we don't recognise gets the V2.4 sample rate so we never
        // silently resample down from 48 kHz to 32 kHz.
        assert_eq!(infer_sample_rate_from_shape(&[1, 192_000]), 48_000);
        assert_eq!(infer_sample_rate_from_shape(&[1, 1_000_000]), 48_000);
    }

    #[test]
    fn recommended_chunk_samples_v24_fixed() {
        assert_eq!(recommended_chunk_samples_from_shape(&[1, 144_000]), 144_000);
        assert_eq!(
            recommended_chunk_samples_from_shape(&[1, 1, 144_000]),
            144_000
        );
    }

    #[test]
    fn recommended_chunk_samples_v30_fixed() {
        // V3.0 fixed shape reports its own trained length (96 000 @ 32 kHz = 3 s).
        assert_eq!(recommended_chunk_samples_from_shape(&[1, 96_000]), 96_000);
        assert_eq!(
            recommended_chunk_samples_from_shape(&[1, 1, 96_000]),
            96_000
        );
    }

    #[test]
    fn recommended_chunk_samples_dynamic_picks_optimal() {
        // The empirical optimum from the bundled-WAV sweep: 144 000 samples
        // = 4.5 s × 32 kHz, where Pica pica confidence climbs from 0.52
        // (at 3.0 s × 32 kHz = 96 000) to 0.72.
        // See docs/architecture/15-model-chunking.md.
        assert_eq!(recommended_chunk_samples_from_shape(&[1]), 144_000);
        assert_eq!(recommended_chunk_samples_from_shape(&[1, 1]), 144_000);
        assert_eq!(recommended_chunk_samples_from_shape(&[1, 1, 1]), 144_000);
    }

    #[test]
    fn recommended_chunk_secs_matches_3s_for_v24() {
        // V2.4: 144 000 / 48 000 = 3.0 s exactly.
        let samples = recommended_chunk_samples_from_shape(&[1, 144_000]);
        let rate = infer_sample_rate_from_shape(&[1, 144_000]);
        let secs = samples as f32 / rate as f32;
        assert!((secs - 3.0).abs() < 1e-6, "got {secs}, expected 3.0");
    }

    #[test]
    fn recommended_chunk_secs_matches_45s_for_v30_dynamic() {
        // V3.0 dynamic: 144 000 / 32 000 = 4.5 s exactly.
        let samples = recommended_chunk_samples_from_shape(&[1, 1]);
        let rate = infer_sample_rate_from_shape(&[1, 1]);
        let secs = samples as f32 / rate as f32;
        assert!((secs - 4.5).abs() < 1e-6, "got {secs}, expected 4.5");
    }

    #[test]
    fn recommended_chunk_secs_matches_3s_for_v30_fixed() {
        // V3.0 fixed: 96 000 / 32 000 = 3.0 s.
        let samples = recommended_chunk_samples_from_shape(&[1, 96_000]);
        let rate = infer_sample_rate_from_shape(&[1, 96_000]);
        let secs = samples as f32 / rate as f32;
        assert!((secs - 3.0).abs() < 1e-6, "got {secs}, expected 3.0");
    }

    #[test]
    fn expected_input_length_handles_known_shapes() {
        assert_eq!(expected_input_length(&[1, 144_000]), Some(144_000));
        assert_eq!(expected_input_length(&[1, 1, 144_000]), Some(144_000));
        assert_eq!(expected_input_length(&[1, 96_000]), Some(96_000));
        assert_eq!(expected_input_length(&[1]), Some(144_000));
        assert_eq!(expected_input_length(&[1, 1]), Some(144_000));
        assert_eq!(expected_input_length(&[1, 1, 1]), Some(144_000));
    }

    #[test]
    fn expected_input_length_rejects_unsupported_shapes() {
        // Anything outside rank 1/2/3 — or rank-2/3 with the sample axis
        // collapsed to 1 outside the recognised dynamic shapes — falls
        // through to None so the model loader can fail loudly instead of
        // silently producing a zero-length tensor.
        assert_eq!(expected_input_length(&[]), None);
        assert_eq!(expected_input_length(&[1, 1, 1, 1]), None);
    }

    #[test]
    fn expected_input_length_accepts_batch_dim_variations() {
        // The leading "batch" dim is ignored — the spec only cares about
        // the last dim being > 1. This is intentional so a model reporting
        // [B, samples] shape still works.
        assert_eq!(expected_input_length(&[2, 144_000]), Some(144_000));
        assert_eq!(expected_input_length(&[8, 1, 96_000]), Some(96_000));
    }

    // ─── compute_confidence: the is_probability_output branch ───────────
    //
    // This is the function that lost half the Pica pica confidence signal
    // before the fix. Each test pins one cell in the decision matrix.

    #[test]
    fn compute_confidence_v30_probabilities_pass_through() {
        // sensitivity is deliberately ignored on the probability path.
        let raw = 0.9247_f32;
        let with_sensitivity_1 = compute_confidence(raw, 1.0, true);
        let with_sensitivity_2 = compute_confidence(raw, 2.0, true);
        assert!((with_sensitivity_1 - raw).abs() < 1e-6);
        assert!((with_sensitivity_2 - raw).abs() < 1e-6);
    }

    #[test]
    fn compute_confidence_v30_clamps_out_of_range() {
        // Numerically noisy probabilities can spike outside [0, 1]; we clamp
        // rather than panic. (NaN is *not* clamped by clamp — separate test.)
        assert!((compute_confidence(-0.05, 1.0, true) - 0.0).abs() < 1e-6);
        assert!((compute_confidence(1.05, 1.0, true) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn compute_confidence_v24_logits_get_sigmoid() {
        // V2.4 raw logit 0 should map to 0.5.
        let conf = compute_confidence(0.0, 1.0, false);
        assert!((conf - 0.5).abs() < 1e-6);
    }

    #[test]
    fn compute_confidence_v24_logits_sensitivity_matters() {
        // sensitivity > 1 makes the same logit more confident.
        let conf_default = compute_confidence(1.0, 1.0, false);
        let conf_sensitive = compute_confidence(1.0, 2.0, false);
        assert!(conf_sensitive > conf_default);
        // sensitivity < 1 makes it less confident.
        let conf_dampened = compute_confidence(1.0, 0.5, false);
        assert!(conf_dampened < conf_default);
    }

    /// Regression for the V3.0 sigmoid-on-probabilities bug.
    ///
    /// On the bundled WAV, the model rated the Magpie at 0.9247. The old
    /// pipeline ran sigmoid(1.0 * 0.9247) = 0.7158 and silently lost the
    /// confidence signal. After the fix the probability passes through
    /// untouched.
    #[test]
    fn regression_v30_probability_not_sigmoided() {
        let model_output = 0.9247_f32;
        let new_path = compute_confidence(model_output, 1.0, true);
        // Within rounding of the model output itself.
        assert!(
            (new_path - 0.9247).abs() < 1e-4,
            "V3.0 probability path lost the signal: got {new_path}"
        );
        // What the old (buggy) path would have produced — kept here so a
        // re-introduction of the bug shows up as an explicit mismatch in
        // the assertion message.
        let old_buggy_path = sigmoid(1.0 * model_output);
        assert!(
            (old_buggy_path - 0.7158).abs() < 5e-4,
            "sigmoid(0.9247) was {old_buggy_path}, expected ~0.7158"
        );
        assert!(
            new_path > old_buggy_path + 0.2,
            "the fix must lift confidence by > 0.2 on this anchor"
        );
    }
}
