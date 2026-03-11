//! `DuckDB` behavioral analytics for bird detection patterns.
//!
//! Applies tomtom215's [`duckdb-behavioral`](https://github.com/tomtom215/duckdb-behavioral)
//! extension functions to bird activity data:
//! - `sessionize`: Group continuous activity into sessions
//! - `retention`: Track species return patterns
//! - `window_funnel`: Analyze dawn chorus sequences
//! - `sequence_match`: Find specific activity patterns
//! - `sequence_next_node`: Predict next species
//!
//! This crate provides query builders and result types for the behavioral
//! analytics API. The actual `DuckDB` connection and extension loading will
//! be integrated when the `duckdb` crate is added as a dependency.

pub mod queries;
pub mod types;
