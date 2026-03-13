//! `SQLite` operational database.
//!
//! Connection management (WAL mode, PRAGMAs), row types, and query helpers
//! for the birds.db detection database, organized by concern.
//!
//! # Module layout
//!
//! | Sub-module           | Contents                                                    |
//! |----------------------|-------------------------------------------------------------|
//! | `connection`         | `DbError`, `open_connection`, `open_or_create`, `quick_check` |
//! | `types`              | `DetectionRecord`, `DetectionRow`, `SpeciesCount`, …       |
//! | `queries::detections`| Insert, count, paginate, filter detection rows             |
//! | `queries::species`   | Per-species aggregates, summaries, and activity            |
//! | `queries::analytics` | Hourly, daily, confidence distribution, latest             |

pub mod connection;
pub mod queries;
pub mod types;

// Flat re-exports so existing call-sites (`birdnet_db::sqlite::foo`) continue
// to compile without modification.
pub use connection::{DbError, open_connection, open_or_create, quick_check};
pub use queries::{
    all_detections, confidence_distribution, daily_counts, detection_count,
    detections_by_date, detections_by_species, hourly_activity, insert_detection,
    latest_detection, recent_by_species, recent_detections, recent_detections_page,
    search_species, species_count, species_daily_counts, species_hourly_activity,
    species_summary, top_species,
};
pub use types::{
    DailyCount, DetectionRecord, DetectionRow, HourlyCount, SpeciesCount, SpeciesSummary,
};
