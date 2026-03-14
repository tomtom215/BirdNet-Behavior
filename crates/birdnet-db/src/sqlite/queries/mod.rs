//! SQLite query modules grouped by concern.

pub mod analytics;
pub mod correlation;
pub mod detections;
pub mod heatmap;
pub mod species;

pub use analytics::{
    confidence_distribution, daily_counts, distinct_detection_dates, hourly_activity,
    latest_detection, range_daily_counts, weekly_detection_count, weekly_new_species,
    weekly_top_species,
};
pub use correlation::{companion_species, temporal_cooccurrence, top_cooccurrence_pairs};
pub use detections::{
    all_detections, delete_detection, detection_count, detection_count_for_date, detection_dates,
    detections_by_date, detections_by_species, insert_detection, recent_detections,
    recent_detections_page, relabel_detection, species_for_date, todays_detection_count,
    todays_detections,
};
pub use heatmap::{hourly_totals, species_daily_heatmap, weekly_heatmap};
pub use species::{
    recent_by_species, search_species, species_count, species_daily_counts, species_first_seen,
    species_hourly_activity, species_summary, top_species,
};
