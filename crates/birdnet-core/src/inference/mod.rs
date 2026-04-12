//! ML inference for bird species classification.
//!
//! Loads ONNX models through the `ort` crate (ONNX Runtime) and runs
//! inference on audio chunks. Designed for the `BirdNET` model family and
//! compatible architectures.

pub mod labels;
pub mod model;
pub mod species_filter;
