//! ONNX model loading and inference via tract (pure Rust).
//!
//! Loads BirdNET ONNX models, runs inference on audio chunks (raw f32 samples),
//! and returns species classification results with confidence scores.
//!
//! The inference pipeline:
//! 1. Accept raw audio f32 samples (already resampled to model sample rate)
//! 2. Feed into tract ONNX model
//! 3. Apply sigmoid with sensitivity adjustment
//! 4. Return top-N species above confidence threshold

use std::fmt;
use std::path::Path;

use tract_onnx::prelude::*;

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

/// Configuration for the BirdNET model.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Sensitivity adjustment for sigmoid (higher = more sensitive, range 0.5-1.5).
    pub sensitivity: f32,
    /// Minimum confidence to include in results.
    pub confidence_threshold: f32,
    /// Maximum number of detections per chunk.
    pub top_n: usize,
    /// Number of inference threads (for tract thread pool).
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

/// A loaded BirdNET ONNX model ready for inference.
pub struct BirdNetModel {
    model: SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>,
    labels: LabelSet,
    config: ModelConfig,
    input_shape: Vec<usize>,
}

impl fmt::Debug for BirdNetModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BirdNetModel")
            .field("labels_count", &self.labels.len())
            .field("config", &self.config)
            .field("input_shape", &self.input_shape)
            .finish()
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

        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .into_optimized()
            .map_err(|e| InferenceError::Model(format!("optimization failed: {e}")))?
            .into_runnable()
            .map_err(|e| InferenceError::Model(format!("plan creation failed: {e}")))?;

        // Extract input shape from the model
        let input_fact = model
            .model()
            .input_fact(0)
            .map_err(|e| InferenceError::Model(format!("cannot read input shape: {e}")))?;

        let input_shape: Vec<usize> = input_fact
            .shape
            .dims()
            .iter()
            .map(|d: &TDim| d.to_i64().unwrap_or(0) as usize)
            .collect();

        tracing::info!(
            input_shape = ?input_shape,
            "model loaded successfully"
        );

        Ok(Self {
            model,
            labels,
            config,
            input_shape,
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
        let cursor = std::io::Cursor::new(bytes);

        let model = tract_onnx::onnx()
            .model_for_read(&mut cursor.clone())
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .into_optimized()
            .map_err(|e| InferenceError::Model(format!("optimization failed: {e}")))?
            .into_runnable()
            .map_err(|e| InferenceError::Model(format!("plan creation failed: {e}")))?;

        let input_fact = model
            .model()
            .input_fact(0)
            .map_err(|e| InferenceError::Model(format!("cannot read input shape: {e}")))?;

        let input_shape: Vec<usize> = input_fact
            .shape
            .dims()
            .iter()
            .map(|d: &TDim| d.to_i64().unwrap_or(0) as usize)
            .collect();

        Ok(Self {
            model,
            labels,
            config,
            input_shape,
        })
    }

    /// Run inference on raw audio samples.
    ///
    /// The `audio` slice should be mono f32 samples at the model's expected
    /// sample rate and duration. For BirdNET V2.4, that's 48kHz x 3s = 144,000 samples.
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
        &self,
        audio: &[f32],
        date: &str,
        time: &str,
        start_secs: f32,
        end_secs: f32,
        week: u32,
    ) -> Result<Vec<Detection>, InferenceError> {
        // Build input tensor matching the model's expected shape
        let input_tensor = self.build_input_tensor(audio)?;

        // Run inference
        let outputs = self
            .model
            .run(tvec![input_tensor.into()])
            .map_err(|e| InferenceError::Runtime(e.to_string()))?;

        // Extract logits from first output
        let logits = outputs[0]
            .to_array_view::<f32>()
            .map_err(|e| InferenceError::Runtime(format!("cannot extract logits: {e}")))?;

        // Apply sigmoid with sensitivity and collect results
        let mut detections = Vec::new();
        let flat_logits = logits
            .as_slice()
            .ok_or_else(|| InferenceError::Runtime("logits not contiguous".into()))?;

        for (i, &logit) in flat_logits.iter().enumerate() {
            let confidence = sigmoid(self.config.sensitivity * logit);

            if confidence >= self.config.confidence_threshold {
                if let Some(label) = self.labels.get(i) {
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
    /// Handles shape matching: if the model expects [1, N], wraps accordingly.
    /// Pads or truncates audio to match expected input length.
    fn build_input_tensor(&self, audio: &[f32]) -> Result<Tensor, InferenceError> {
        match self.input_shape.as_slice() {
            // Shape [batch, samples] -- most BirdNET models
            [1, expected_len] => {
                let expected = *expected_len;
                let mut padded = vec![0.0_f32; expected];
                let copy_len = audio.len().min(expected);
                padded[..copy_len].copy_from_slice(&audio[..copy_len]);

                let tensor = tract_ndarray::Array2::from_shape_vec((1, expected), padded)
                    .map_err(|e| InferenceError::Shape(e.to_string()))?;

                Ok(tensor.into_tensor())
            }
            // Shape [batch, channels, samples] -- some model variants
            [1, 1, expected_len] => {
                let expected = *expected_len;
                let mut padded = vec![0.0_f32; expected];
                let copy_len = audio.len().min(expected);
                padded[..copy_len].copy_from_slice(&audio[..copy_len]);

                let tensor = tract_ndarray::Array3::from_shape_vec((1, 1, expected), padded)
                    .map_err(|e| InferenceError::Shape(e.to_string()))?;

                Ok(tensor.into_tensor())
            }
            // Dynamic shape (0 means dynamic dimension)
            shape if shape.len() == 2 && shape[0] <= 1 => {
                let len = audio.len();
                let tensor = tract_ndarray::Array2::from_shape_vec((1, len), audio.to_vec())
                    .map_err(|e| InferenceError::Shape(e.to_string()))?;

                Ok(tensor.into_tensor())
            }
            other => Err(InferenceError::Shape(format!(
                "unsupported input shape: {other:?}, expected [1, N] or [1, 1, N]"
            ))),
        }
    }

    /// Get the model's expected input shape.
    pub fn input_shape(&self) -> &[usize] {
        &self.input_shape
    }

    /// Get the label set.
    pub fn labels(&self) -> &LabelSet {
        &self.labels
    }

    /// Get the model configuration.
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Update the sensitivity value.
    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        self.config.sensitivity = sensitivity;
    }

    /// Update the confidence threshold.
    pub fn set_confidence_threshold(&mut self, threshold: f32) {
        self.config.confidence_threshold = threshold;
    }
}

/// Apply sigmoid function: `1 / (1 + exp(-x))`.
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Apply softmax to a slice of logits (in-place friendly).
///
/// Useful for models that output raw logits requiring softmax normalization.
#[allow(dead_code)]
fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    exps.iter().map(|&e| e / sum).collect()
}

#[cfg(test)]
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
    fn softmax_sums_to_one() {
        let logits = vec![1.0, 2.0, 3.0, 4.0];
        let probs = softmax(&logits);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "softmax sum: {sum}");
    }

    #[test]
    fn softmax_preserves_ordering() {
        let logits = vec![1.0, 3.0, 2.0];
        let probs = softmax(&logits);
        assert!(probs[1] > probs[2]);
        assert!(probs[2] > probs[0]);
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
