//! Phenology analytics for bird activity patterns.
//!
//! Phenology is the scientific study of cyclic and seasonal natural
//! phenomena — in ornithology, this covers:
//!
//! - **Migration timing:** First/last detection dates, arrival windows.
//! - **Abundance indices:** How detection frequency varies through the year.
//! - **Inter-annual trends:** Year-over-year changes in species presence.
//!
//! ## SQL target
//!
//! Every builder here emits **`DuckDB`** SQL and reads the `detections_ts`
//! view, which is what this crate's `AnalyticsDb` creates and what carries a
//! properly typed `detection_date`.
//!
//! These builders previously advertised a compatibility matrix in which most of
//! them ran on `SQLite` *and* `DuckDB`. That was never true of `DuckDB`, the
//! engine this crate actually talks to: they emitted `strftime('%Y', Date)`,
//! which is `SQLite`'s `strftime(format, value)` argument order, so `DuckDB`
//! rejected them with "Could not choose a best candidate function for the
//! function call `strftime(STRING_LITERAL, VARCHAR)`". `phenology_timing_sql`
//! additionally used `julianday`, which `DuckDB` does not have. Ten of the
//! eleven queries failed to bind. The claim survived because every test asserted
//! on the generated *text* — `sql.contains("month")` and the like — which a
//! query no engine will run passes just as well as one that works.
//!
//! `tests/phenology_execute.rs` now runs all of them against a real store.
//! Restoring `SQLite` support would mean a second set of builders; nothing in
//! the tree asked for one, so the dual claim was dropped rather than doubled.
//!
//! ## Example
//!
//! ```rust
//! use birdnet_behavioral::phenology::{timing, AbundanceParams, PhenologyParams};
//!
//! let params = PhenologyParams {
//!     species: Some("Common Swift".to_string()),
//!     year_start: Some(2024),
//!     ..PhenologyParams::default()
//! };
//! let sql = timing::phenology_timing_sql(&params);
//! // Execute sql against the DuckDB analytics store…
//! ```

pub mod abundance;
pub mod timing;
pub mod types;

pub use abundance::{
    effort_corrected_abundance_sql, monthly_totals_sql, peak_weeks_sql, weekly_abundance_sql,
    weekly_richness_sql,
};
pub use timing::{
    first_detection_sql, interannual_trend_sql, migration_window_sql, phenology_timing_sql,
};
pub use types::{
    AbundanceParams, MigrationWindow, PhenologyParams, PhenologyRecord, WeeklyAbundance,
};
