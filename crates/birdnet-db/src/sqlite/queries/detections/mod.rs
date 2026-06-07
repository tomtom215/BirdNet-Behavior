//! Detection CRUD queries, split by concern.
//!
//! - `read` — counts, listings, pagination, today's feed, corroboration.
//! - `write` — insert, delete, relabel.
//! - `search` — free-text search-term parsing (pure, no database).
//! - `locks` — lock/unlock a clip against the disk purge.
//!
//! Every query is re-exported here so callers keep using the flat
//! `queries::detections::<fn>` path.

mod locks;
mod read;
mod search;
mod write;

#[cfg(test)]
mod test_support;

pub use locks::{is_detection_locked, lock_detection, locked_file_names, unlock_detection};
pub use read::{
    all_detections, best_detections_for_date, concurrent_detections_from_other_sources,
    detection_count, detection_count_for_date, detection_count_for_species_date, detection_dates,
    detections_by_date, detections_by_species, recent_detections, recent_detections_page,
    species_for_date, todays_detection_count, todays_detections,
};
pub use search::{SearchTerm, parse_search_term};
pub use write::{delete_detection, insert_detection, relabel_detection};
