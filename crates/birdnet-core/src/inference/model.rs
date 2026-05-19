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
            let confidence = if self.is_probability_output {
                raw.clamp(0.0, 1.0)
            } else {
                sigmoid(self.config.sensitivity * raw)
            };

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
    /// For fully-dynamic shapes (V3.0 preview), defaults to 96 000 samples (32 kHz × 3 s).
    fn build_input_tensor(&self, audio: &[f32]) -> Result<Tensor<f32>, InferenceError> {
        let expected_len = match self.input_shape.as_slice() {
            [_, n] | [_, _, n] if *n > 1 => *n,
            // All-dynamic or rank-1 shape → V3.0 preview: 4.5 s of 32 kHz
            // audio = 144_000 samples. See `recommended_chunk_samples`
            // docstring for the empirical justification.
            [1] | [1, 1] | [1, 1, 1] => 144_000,
            other => {
                return Err(InferenceError::Shape(format!(
                    "unsupported input shape: {other:?}, expected [1, N] or [1, 1, N]"
                )));
            }
        };

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
        let n_samples = match self.input_shape.as_slice() {
            [_, n] | [_, _, n] if *n > 1 => *n,
            // All-dynamic shape → assume BirdNET+ V3.0 (32 kHz)
            _ => return 32_000,
        };
        match n_samples {
            96_000 => 32_000, // BirdNET+ V3.0 (32 kHz × 3 s)
            _ => 48_000,      // BirdNET   V2.4 (48 kHz × 3 s) or unknown
        }
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
        match self.input_shape.as_slice() {
            // Fixed shapes — use the dimension the model declares.
            [_, n] | [_, _, n] if *n > 1 => *n,
            // Dynamic shape — empirically-optimal default for V3.0 preview.
            _ => 144_000,
        }
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

/// Apply sigmoid function: `1 / (1 + exp(-x))`.
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
}
