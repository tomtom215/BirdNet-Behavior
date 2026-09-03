//! Audio processing pipeline.
//!
//! Pure Rust audio pipeline: decode (symphonia) -> resample (rubato) -> spectrogram.
//! Replaces librosa, soundfile, and sox with zero C dependencies.
//!
//! The [`quality`] module pre-screens audio chunks for SNR, spectral flatness,
//! and environmental interference (rain/wind) before ML inference.
//!
//! The [`soundlevel`] module measures the site rather than the chunk: a level
//! in each ISO 266 third-octave band, kept as a series. The two are easy to
//! confuse and answer different questions — see that module's own header.

pub mod biquad;
pub mod capture;
pub mod decode;
pub mod eq;
pub mod extraction;
pub mod quality;
pub mod resample;
pub mod soundlevel;
pub mod spectrogram;
