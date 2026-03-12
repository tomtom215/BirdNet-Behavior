//! SQL query builders for time-series analytics goals.
//!
//! Each sub-module produces a complete, runnable DuckDB SQL string
//! targeted at a specific analytics question. All queries read from the
//! `detections_ts` view (requires `birdnet-behavioral` to have set up
//! the DuckDB database).
//!
//! Sub-modules are organised by *analytics goal*, not by window type:
//!
//! | Module      | Goal                                        |
//! |-------------|---------------------------------------------|
//! | `activity`  | Detection counts over time                  |
//! | `diversity` | Species richness and diversity indices      |
//! | `trend`     | Moving averages and long-range trends       |
//! | `peak`      | Identifying the busiest intervals           |
//! | `gap`       | Inactivity gap and absence detection        |

pub mod activity;
pub mod diversity;
pub mod gap;
pub mod peak;
pub mod trend;

/// SQL to ensure the `detections_ts` view exists.
///
/// Called by the executor before running any query that depends on the view.
/// Safe to call multiple times (`CREATE OR REPLACE`).
pub const ENSURE_TS_VIEW: &str = "
CREATE OR REPLACE VIEW detections_ts AS
SELECT *,
    TRY_CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp,
    TRY_CAST(Date AS DATE)                     AS detection_date
FROM detections;
";

/// A trait for query builders that produce a single runnable SQL string.
///
/// All query builder structs implement this to support uniform test patterns
/// and potential future query compilation / caching.
pub trait QueryPlan {
    /// Build and return the complete SQL query string.
    fn sql(&self) -> String;
}
