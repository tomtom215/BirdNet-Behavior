//! Audio processing pipeline.
//!
//! Pure Rust audio pipeline: decode (symphonia) -> resample (rubato) -> spectrogram.
//! Replaces librosa, soundfile, and sox with zero C dependencies.
//!
//! The [`quality`] module pre-screens audio chunks for SNR, spectral flatness,
//! and environmental interference (rain/wind) before ML inference.

pub mod capture;
pub mod decode;
pub mod extraction;
pub mod quality;
pub mod resample;
pub mod spectrogram;
