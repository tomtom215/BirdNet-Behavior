//! `DuckDB` behavioral analytics for bird detection patterns.
//!
//! Applies tomtom215's `duckdb-behavioral` extension functions
//! to bird activity data:
//! - `sessionize`: Group continuous activity into sessions
//! - `retention`: Track species return patterns
//! - `window_funnel`: Analyze dawn chorus sequences
//! - `sequence_match`: Find specific activity patterns
//! - `sequence_next_node`: Predict next species
//!
//! TODO(phase1): Integrate `DuckDB` with behavioral extension.
