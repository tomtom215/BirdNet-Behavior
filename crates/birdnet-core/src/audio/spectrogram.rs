//! Mel spectrogram generation.
//!
//! Will use the `mel_spec` crate for librosa-compatible output.
//! Critical: BirdNET models were trained on librosa spectrograms,
//! so numerical equivalence is required.
//!
//! TODO(phase2): Implement mel spectrogram pipeline with `mel_spec` crate.
//! Must validate output matches librosa within 1e-4 tolerance.
