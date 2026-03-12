//! Audio resampling via rubato.
//!
//! Resamples audio to the target sample rate required by the ML model.
//! `BirdNET` models typically expect 48kHz; Perch expects 32kHz.
//!
//! Uses rubato 1.0's `Async` polynomial resampler for high-quality
//! sample rate conversion with the `audioadapter` buffer system.

use std::fmt;

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::audioadapter::Adapter;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

/// Errors during resampling.
#[derive(Debug)]
pub enum ResampleError {
    /// Invalid parameters (e.g., zero sample rate).
    InvalidParams(String),
    /// Resampling computation failed.
    Process(String),
}

impl fmt::Display for ResampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParams(msg) => write!(f, "invalid resample params: {msg}"),
            Self::Process(msg) => write!(f, "resample error: {msg}"),
        }
    }
}

impl std::error::Error for ResampleError {}

/// Resample mono audio samples from `from_rate` to `to_rate`.
///
/// Returns the resampled samples. If rates are equal, returns input unchanged.
///
/// # Errors
///
/// Returns `ResampleError` if sample rates are zero or resampling fails.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, ResampleError> {
    if from_rate == 0 || to_rate == 0 {
        return Err(ResampleError::InvalidParams(
            "sample rates must be non-zero".into(),
        ));
    }

    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }

    let ratio = f64::from(to_rate) / f64::from(from_rate);
    let chunk_size = 1024;

    let mut resampler = Async::<f32>::new_poly(
        ratio,
        1.1,
        PolynomialDegree::Septic,
        chunk_size,
        1, // mono
        FixedAsync::Input,
    )
    .map_err(|e| ResampleError::Process(e.to_string()))?;

    let estimated_output_len = (samples.len() as f64 * ratio) as usize + chunk_size;
    let mut output = Vec::with_capacity(estimated_output_len);
    let input_frames_needed = resampler.input_frames_next();
    let mut pos = 0;

    // Process full chunks using InterleavedSlice adapter (mono: channels=1)
    while pos + input_frames_needed <= samples.len() {
        let chunk = &samples[pos..pos + input_frames_needed];
        let adapter = InterleavedSlice::new(chunk, 1, input_frames_needed)
            .map_err(|e| ResampleError::Process(e.to_string()))?;
        let result = resampler
            .process(&adapter, 0, None)
            .map_err(|e: rubato::ResampleError| ResampleError::Process(e.to_string()))?;
        let frames = result.frames();
        for i in 0..frames {
            if let Some(sample) = result.read_sample(0, i) {
                output.push(sample);
            }
        }
        pos += input_frames_needed;
    }

    // Process remaining samples padded with zeros
    if pos < samples.len() {
        let remaining = samples.len() - pos;
        let mut last_chunk = vec![0.0_f32; input_frames_needed];
        last_chunk[..remaining].copy_from_slice(&samples[pos..]);
        let adapter = InterleavedSlice::new(&last_chunk[..], 1, input_frames_needed)
            .map_err(|e| ResampleError::Process(e.to_string()))?;
        let result = resampler
            .process(&adapter, 0, None)
            .map_err(|e: rubato::ResampleError| ResampleError::Process(e.to_string()))?;
        let output_frames = (remaining as f64 * ratio) as usize;
        let available = result.frames();
        let take = output_frames.min(available);
        for i in 0..take {
            if let Some(sample) = result.read_sample(0, i) {
                output.push(sample);
            }
        }
    }

    Ok(output)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_returns_input() {
        let samples = vec![0.1, 0.2, 0.3];
        let result = resample(&samples, 48000, 48000).unwrap();
        assert_eq!(result, samples);
    }

    #[test]
    fn zero_rate_returns_error() {
        let samples = vec![0.1, 0.2];
        assert!(resample(&samples, 0, 48000).is_err());
        assert!(resample(&samples, 48000, 0).is_err());
    }

    #[test]
    fn downsample_produces_output() {
        // 2048 samples at 48kHz -> resample to 16kHz
        let samples: Vec<f32> = (0..2048).map(|i| (i as f32 / 48000.0).sin()).collect();
        let result = resample(&samples, 48000, 16000).unwrap();
        assert!(!result.is_empty());
        assert!(result.len() < samples.len());
    }
}
