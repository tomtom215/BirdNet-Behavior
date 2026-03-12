//! Time-series analytics for bird detection temporal patterns.
//!
//! Implements DuckDB window function analytics (tumbling, hopping, sliding,
//! and session windows) for bird activity data. Designed to complement the
//! behavioural extension in `birdnet-behavioral` with pure-SQL temporal queries.
//!
//! # Window Types
//!
//! | Window    | Use Case                                    |
//! |-----------|---------------------------------------------|
//! | Tumbling  | Hourly/daily/weekly detection summaries     |
//! | Hopping   | Peak activity detection (overlapping)       |
//! | Sliding   | Moving averages and smooth trend lines      |
//! | Session   | Activity gaps and continuous-presence spans |
//!
//! # Features
//!
//! Enable the `analytics` feature to compile the [`executor`] module which
//! runs queries against a live DuckDB connection. Without it, only the query
//! builders, type definitions, and window specifications are available.
//!
//! ```toml
//! birdnet-timeseries = { path = "…", features = ["analytics"] }
//! ```

pub mod error;
pub mod queries;
pub mod types;
pub mod window;

#[cfg(feature = "analytics")]
pub mod executor;

pub use error::TimeSeriesError;
pub use types::{params, results};
pub use window::WindowSpec;
