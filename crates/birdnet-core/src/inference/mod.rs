//! ML inference for bird species classification.
//!
//! Supports ONNX models via `tract` (pure Rust, zero C dependencies).
//! Designed for the `BirdNET` model family and compatible architectures.

pub mod labels;
pub mod model;
pub mod species_filter;
