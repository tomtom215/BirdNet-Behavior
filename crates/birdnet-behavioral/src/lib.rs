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
//! Uses a file-based `DuckDB` database for durable analytics storage,
//! with data synced from the operational `SQLite` database. The behavioral
//! extension is loaded at runtime for advanced analytical queries.
//!
//! Enable the `analytics` feature to compile the `DuckDB` connection module.
//! Without it, only the query builders and types are available (useful for
//! SQL generation and type definitions without the heavy `DuckDB` C++ dependency).
//!
//! The [`phenology`] module provides SQL query builders for migration timing,
//! seasonal abundance indices, and inter-annual trend analysis.  These queries
//! are compatible with both `SQLite` and `DuckDB` (see module-level docs for
//! per-function compatibility notes).

#[cfg(feature = "analytics")]
pub mod connection;
pub mod phenology;
pub mod queries;
pub mod types;
