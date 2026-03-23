//! Phenology analytics for bird activity patterns.
//!
//! Phenology is the scientific study of cyclic and seasonal natural
//! phenomena — in ornithology, this covers:
//!
//! - **Migration timing:** First/last detection dates, arrival windows.
//! - **Abundance indices:** How detection frequency varies through the year.
//! - **Inter-annual trends:** Year-over-year changes in species presence.
//!
//! ## SQL compatibility
//!
//! | Query function                    | `SQLite` | `DuckDB` |
//! |-----------------------------------|----------|----------|
//! | [`timing::phenology_timing_sql`]  | ✓      | ✓      |
//! | [`timing::first_detection_sql`]   | ✓      | ✓      |
//! | [`timing::migration_window_sql`]  | ✗      | ✓      |
//! | [`timing::interannual_trend_sql`] | ✗      | ✓      |
//! | [`abundance::weekly_abundance_sql`]        | ✓  | ✓  |
//! | [`abundance::monthly_totals_sql`]          | ✓  | ✓  |
//! | [`abundance::weekly_richness_sql`]         | ✓  | ✓  |
//! | [`abundance::effort_corrected_abundance_sql`] | ✗ | ✓ |
//!
//! SQLite-compatible queries use only `strftime` and `julianday`.
//! DuckDB-only queries use `percentile_cont` and `LAG` window functions.
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
//! // Execute sql against SQLite or DuckDB…
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
