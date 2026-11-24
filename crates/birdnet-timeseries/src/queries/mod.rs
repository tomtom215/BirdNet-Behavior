//! SQL query builders for time-series analytics goals.
//!
//! Each sub-module produces a complete, runnable `DuckDB` SQL string
//! targeted at a specific analytics question. All queries read from the
//! `detections_ts` view (requires `birdnet-behavioral` to have set up
//! the `DuckDB` database); the moving average, year-over-year, anomaly and
//! quiet-day queries also read `recording_effort`, the analytics copy of the
//! station's listening ledger, to tell a day it was off from a quiet one.
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
/// `make_timestamp(microseconds)` yields a plain `TIMESTAMP` holding the UTC
/// instant, the same type as `detection_timestamp`, so the two are
/// interchangeable as arguments to the extension's functions. It was
/// `CAST(to_timestamp(..) AS TIMESTAMP)`, which is not the same thing: once ICU
/// is loaded that cast converts into the session `TimeZone`, which defaults to
/// the system zone. On any station not set to UTC both passes through a
/// repeated autumn hour then read as the same local time, and every elapsed
/// time across the change came out an hour short — while CI, running in UTC,
/// stayed green (`tests/two_clocks.rs` now runs in `Europe/Berlin`). Rows with no
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
    make_timestamp(detected_at_utc * 1000000) AS detection_instant,
    TRY_CAST(Date AS DATE) AS detection_date
FROM detections
WHERE review_verdict IS DISTINCT FROM 'rejected';
";

/// "The last `days` days": `days` calendar dates ending with today.
///
/// `detection_date >= CURRENT_DATE - INTERVAL n DAYS` — the spelling every
/// query here used — includes both ends, so "the last 3 days" was four dates.
/// A strict `>` makes N days mean N dates. Today is included, partial as it
/// is: for a list of counts, the newest row being the day in progress is what
/// a reader expects. Anything that *averages* over days must use
/// [`last_complete_days`] instead.
pub(crate) fn last_days(days: u32) -> String {
    format!("detection_date > CURRENT_DATE - INTERVAL {days} DAYS")
}

/// The `days` complete dates before today — today, a few hours in, is not a
/// day yet, and averaging it in drags every per-day figure toward zero each
/// morning.
pub(crate) fn last_complete_days(days: u32) -> String {
    format!(
        "detection_date >= CURRENT_DATE - INTERVAL {days} DAYS AND detection_date < CURRENT_DATE"
    )
}

/// Seconds of recorded listening below which a day counts as one the station
/// was **not listening** on.
///
/// The same floor as the "not listening" marks the web charts draw from the
/// same ledger (`birdnet-web`'s `pages::listening`, which reads this constant),
/// so a chart and the analytics beside it never disagree about a day. It is a
/// floor for *absent*, not a measure of a full day: the recorder credits five
/// minutes per sample, so any day with one sample clears it. A day that cleared
/// it with a fraction of its usual listening is told apart, where a query can
/// say so, by [`PARTIAL_EFFORT_FRACTION`].
///
/// Why not half the scheduled recording window, as the audit suggested: the
/// schedule (solar or fixed, per source) lives in `SQLite` settings and the
/// scheduler crate, not in the analytics store these queries run in, and the
/// station's own typical day — the median of the listened days before it — is
/// available here and follows the seasons the same way.
pub const MIN_LISTENED_SECS: f64 = 60.0;

/// Fraction of its typical day below which a listened day is *partial*.
///
/// The typical day is the median effort of the listened days in the window
/// before it; the anomaly detector labels a partial day rather than judging it.
pub const PARTIAL_EFFORT_FRACTION: f64 = 0.5;

/// The effort ledger as two CTEs, `effort_days (day, seconds)` and
/// `ledger (first_day)`, for a query's `WITH` list.
///
/// Effort is the `DuckDB` copy of the station's `recording_effort` table
/// (`AnalyticsDb::sync_recording_effort` at startup, then the last two days
/// after every sample), **summed** over sources: detections are stored one row
/// per source per detection, so the effort they are counted against is the
/// listening of every source together.
///
/// `ledger = false` (no `recording_effort` table on this connection) yields an
/// empty ledger, so every day is *unknown* — see [`listened_sql`].
pub(crate) const fn effort_ctes(ledger: bool) -> &'static str {
    if ledger {
        "effort_days AS (
    SELECT TRY_CAST(date AS DATE) AS day, SUM(seconds) AS seconds
    FROM recording_effort
    WHERE TRY_CAST(date AS DATE) IS NOT NULL
    GROUP BY 1
),
ledger AS (SELECT MIN(day) AS first_day FROM effort_days)"
    } else {
        "effort_days AS (SELECT NULL::DATE AS day, NULL::DOUBLE AS seconds WHERE false),
ledger AS (SELECT NULL::DATE AS first_day)"
    }
}

/// SQL for the seconds the station listened on `day` (a `DATE` expression),
/// `NULL` where the ledger cannot say: before its first row, or with no ledger
/// at all — a station imported from BirdNET-Pi, or one that predates the
/// recorder. Needs `effort_days` joined as `e` on `e.day = {day}` and `ledger`
/// cross-joined as `l`.
pub(crate) fn effort_seconds_sql(day: &str) -> String {
    format!(
        "CASE WHEN l.first_day IS NOT NULL AND {day} >= l.first_day \
         THEN COALESCE(e.seconds, 0) END"
    )
}

/// SQL true when the station was listening on `day`, or the ledger cannot say.
///
/// Unknown counts as listening, because that is what every query here assumed
/// before it read the ledger: on a station without one, nothing changes. A day
/// the ledger covers and credits with less than [`MIN_LISTENED_SECS`] is not
/// listened. Same joins as [`effort_seconds_sql`].
pub(crate) fn listened_sql(day: &str) -> String {
    format!(
        "COALESCE({} >= {MIN_LISTENED_SECS:.1}, TRUE)",
        effort_seconds_sql(day)
    )
}

/// A trait for query builders that produce a single runnable SQL string.
///
/// All query builder structs implement this to support uniform test patterns
/// and potential future query compilation / caching.
pub trait QueryPlan {
    /// Build and return the complete SQL query string.
    fn sql(&self) -> String;
}
