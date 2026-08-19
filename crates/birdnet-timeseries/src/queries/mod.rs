//! SQL query builders for time-series analytics goals.
//!
//! Each sub-module produces a complete, runnable `DuckDB` SQL string
//! targeted at a specific analytics question. All queries read from the
//! `detections_ts` view (requires `birdnet-behavioral` to have set up
//! the `DuckDB` database).
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
///
/// # This must stay identical to `birdnet_behavioral::queries::CREATE_DETECTIONS_TS_VIEW`
///
/// Both crates run against the *same* `DuckDB` connection — `birdnet-web`'s
/// `AppState::with_timeseries` hands `AnalyticsDb::conn()` straight to
/// `TimeSeriesDb::new`, which runs this statement — and both name the view
/// `detections_ts`. `CREATE OR REPLACE` therefore means the last one to run
/// wins for the rest of the connection's life.
///
/// When this definition omitted the `review_verdict` filter that the
/// behavioural one carries, that was not "the time-series pages ignore
/// rejections": opening a single time-series page silently replaced the
/// behavioural view too, so a rejected detection reappeared in sessionize,
/// retention, funnel, next-species and co-occurrence until the next full sync
/// put the other definition back. Measured on a three-detection fixture with
/// one rejection, `SELECT COUNT(*) FROM detections_ts` went from 2 to 3 across
/// one `quiet_days` call. A reviewer's verdict being undone by an unrelated
/// page view is the worst shape this bug class takes, because nothing reports
/// it and the number that is wrong depends on browsing order.
///
/// `IS DISTINCT FROM` rather than `<> 'rejected'`: SQL three-valued logic makes
/// `NULL <> 'rejected'` evaluate to NULL, which a `WHERE` treats as false, so
/// the plain comparison would drop every *unreviewed* detection — almost all of
/// them.
///
/// `tests/analytics_view_ownership.rs` gates both the texts and the behaviour.
pub const ENSURE_TS_VIEW: &str = "
CREATE OR REPLACE VIEW detections_ts AS
SELECT *,
    TRY_CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp,
    TRY_CAST(Date AS DATE) AS detection_date
FROM detections
WHERE review_verdict IS DISTINCT FROM 'rejected';
";

/// A trait for query builders that produce a single runnable SQL string.
///
/// All query builder structs implement this to support uniform test patterns
/// and potential future query compilation / caching.
pub trait QueryPlan {
    /// Build and return the complete SQL query string.
    fn sql(&self) -> String;
}
