//! Audio processing pipeline.
//!
//! Pure Rust audio pipeline: decode (symphonia) -> resample (rubato) -> spectrogram.
//! Replaces librosa, soundfile, and sox with zero C dependencies.

pub mod capture;
pub mod decode;
pub mod extraction;
pub mod resample;
pub mod spectrogram;
