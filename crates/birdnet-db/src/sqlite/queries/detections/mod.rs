//! Detection CRUD queries, split by concern.
//!
//! - `filter` — the composable `DetectionFilter` the searchable list runs on.
//! - `read` — counts, listings, pagination, today's feed, corroboration.
//! - `write` — insert, delete, relabel.
//! - `search` — free-text search-term parsing (pure, no database).
//! - `locks` — lock/unlock a clip against the disk purge.
//!
//! Every query is re-exported here so callers keep using the flat
//! `queries::detections::<fn>` path.

mod filter;
mod locks;
mod read;
mod search;
mod write;

#[cfg(test)]
mod test_support;

pub use filter::{
    DateRange, DetectionFilter, HourWindow, LockFilter, SortOrder, VerdictFilter, known_sources,
    search_detection_count, search_detections,
};
pub use locks::{is_detection_locked, lock_detection, locked_file_names, unlock_detection};
pub use read::{
    CLIP_AVAILABLE, RecordingsFilter, TodayFilter, all_detections, analytic_detection_count,
    analytic_detection_count_for_date, analytic_species_count_for_date, best_detections_for_date,
    concurrent_detections_from_other_sources, detected_at_utc_for, detection_count,
    detection_count_for_date, detection_count_for_species_date, detection_dates,
    detections_by_date, detections_by_species, detections_per_day, recent_clips,
    recent_clips_count, recent_detections, recent_detections_page, seconds_since_last_detection,
    species_for_date, todays_detection_count, todays_detections, todays_source_activity,
    unstamped_detection_count,
};
pub use search::{SearchTerm, parse_search_term};
pub use write::{delete_detection, insert_detection, relabel_detection};
