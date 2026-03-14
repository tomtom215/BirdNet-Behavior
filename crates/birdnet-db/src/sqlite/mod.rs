//! `SQLite` operational database.
//!
//! Connection management (WAL mode, PRAGMAs), row types, and query helpers
//! for the birds.db detection database, organized by concern.
//!
//! # Module layout
//!
//! | Sub-module              | Contents                                                    |
//! |-------------------------|-------------------------------------------------------------|
//! | `connection`            | `DbError`, `open_connection`, `open_or_create`, `quick_check` |
//! | `types`                 | `DetectionRecord`, `DetectionRow`, `SpeciesCount`, …       |
//! | `queries::detections`   | Insert, count, paginate, filter detection rows             |
//! | `queries::species`      | Per-species aggregates, summaries, and activity            |
//! | `queries::analytics`    | Hourly, daily, confidence distribution, latest             |
//! | `queries::heatmap`      | Hour × day-of-week activity heatmap                        |
//! | `queries::correlation`  | Species co-occurrence and companion species                |

pub mod connection;
pub mod queries;
pub mod types;

// Flat re-exports so existing call-sites (`birdnet_db::sqlite::foo`) continue
// to compile without modification.
pub use connection::{DbError, open_connection, open_or_create, quick_check};
pub use queries::correlation::{FollowOn, SpeciesPair};
pub use queries::heatmap::{HeatmapCell, HourTotal};
pub use queries::{
    SpeciesThreshold, all_detections, companion_species, confidence_distribution, daily_counts,
    delete_detection, delete_species_threshold, detection_count, detection_count_for_date,
    detection_dates, detections_by_date, detections_by_species, distinct_detection_dates,
    get_species_threshold_map, get_species_thresholds, hourly_activity, hourly_totals,
    insert_detection, latest_detection, range_daily_counts, recent_by_species, recent_detections,
    recent_detections_page, relabel_detection, search_species, set_species_threshold,
    species_count, species_daily_counts, species_daily_heatmap, species_first_seen,
    species_for_date, species_hourly_activity, species_sparklines, species_summary,
    temporal_cooccurrence, todays_detection_count, todays_detections, top_cooccurrence_pairs,
    top_species, weekly_detection_count, weekly_heatmap, weekly_new_species, weekly_top_species,
};
pub use types::{
    DailyCount, DetectionRecord, DetectionRow, HourlyCount, SpeciesCount, SpeciesSummary,
};
