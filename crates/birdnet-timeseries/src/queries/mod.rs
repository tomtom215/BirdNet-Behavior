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
///
/// # Two clocks, named apart
///
/// `detection_timestamp` is the station's **local wall clock** and stays that
/// way: it is what hour-of-day filters, calendar-date grouping and every
/// displayed session time are asked in, and all three are local questions.
/// `detection_instant` is the same detection as a **point in time**, from
/// `detected_at_utc` (migration 32).
///
/// The distinction is not decorative. Local wall clock is not monotonic — one
/// hour repeats every autumn and one never happens every spring — so anything
/// that measures *elapsed time* or *order* on it is wrong across a transition:
/// a 30-minute sessionisation gap sees a **negative 55-minute** gap on the
/// autumn night and splits a session that never broke, and reads **two hours**
/// for one real hour on the spring one. Both were live in every `sessionize`,
/// `window_funnel`, `sequence_match` and gap query here.
///
/// So the rule this view exists to make expressible, and which the queries
/// downstream follow: **elapsed time and ordering ask `detection_instant`;
/// clock position, calendar date and anything shown to a human ask
/// `detection_timestamp`.** Naming the two apart is the same move
/// `DailySchedule::clock()` made for the recording gate — a caller that has to
/// remember which clock a bare `ts` means will eventually not.
///
/// `to_timestamp` yields a `TIMESTAMP WITH TIME ZONE`; the cast to plain
/// `TIMESTAMP` keeps it the same type as `detection_timestamp` so the two are
/// interchangeable as arguments to the extension's functions. Rows with no
/// instant — a history predating migration 32, or one whose wall clock names no
/// point in time — yield NULL and drop out of ordered and bucketed results
/// exactly as they already do for `detection_timestamp`.
///
/// This must stay byte-comparable in meaning with
/// `birdnet_behavioral::queries::CREATE_DETECTIONS_TS_VIEW`: both crates run
/// against the same connection and the last `CREATE OR REPLACE` wins, so a
/// column present in one and missing from the other is a query that breaks
/// depending on which page was opened first.
pub const ENSURE_TS_VIEW: &str = "
CREATE OR REPLACE VIEW detections_ts AS
SELECT *,
    TRY_CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp,
    CAST(to_timestamp(detected_at_utc) AS TIMESTAMP) AS detection_instant,
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
