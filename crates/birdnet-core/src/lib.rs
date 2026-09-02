//! BirdNET-Pi core detection pipeline.
//!
//! Provides audio processing, ML inference, detection types, and configuration
//! parsing for the BirdNET-Pi bird classification system.

pub mod audio;
pub mod civil;
pub mod config;
pub mod detection;
mod file_settle;
pub mod i18n;
pub mod inference;
pub mod season;
