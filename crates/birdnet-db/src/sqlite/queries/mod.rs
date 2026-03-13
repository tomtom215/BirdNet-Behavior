//! SQLite query modules grouped by concern.

pub mod analytics;
pub mod detections;
pub mod species;

pub use analytics::{
    confidence_distribution, daily_counts, hourly_activity, latest_detection,
};
pub use detections::{
    all_detections, detection_count, detections_by_date, detections_by_species,
    insert_detection, recent_detections, recent_detections_page,
};
pub use species::{
    recent_by_species, search_species, species_count, species_daily_counts,
    species_hourly_activity, species_summary, top_species,
};
