//! `SQLite` query modules grouped by concern.

pub mod analytics;
pub mod correlation;
pub mod detection_reviews;
pub mod detections;
pub mod heatmap;
pub mod images;
pub mod maintenance;
pub mod quarantine;
pub mod species;

pub use analytics::{
    ModelVsReviewRow, QualitySummary, ReviewVerdictDay, confidence_distribution, confidence_trend,
    daily_counts, detection_quality_by_hour, distinct_detection_dates, hourly_activity,
    last_hour_count, latest_detection, latest_detection_full, low_confidence_species,
    model_vs_review_by_species, quality_summary, range_daily_counts, review_verdict_trend,
    today_species_hour_heatmap, weekly_detection_count, weekly_new_species, weekly_top_species,
};
pub use correlation::{companion_species, temporal_cooccurrence, top_cooccurrence_pairs};
pub use detection_reviews::{
    DetectionReview, ReviewStatus, UnreviewedDetection, clear_detection_review,
    detection_review_counts, get_detection_review, recent_detection_reviews, set_detection_review,
    unreviewed_recent_detections,
};
pub use detections::{
    RecordingsFilter, TodayFilter, all_detections, best_detections_for_date,
    concurrent_detections_from_other_sources, delete_detection, detection_count,
    detection_count_for_date, detection_count_for_species_date, detection_dates,
    detections_by_date, detections_by_species, detections_per_day, insert_detection,
    is_detection_locked, lock_detection, locked_file_names, recent_clips, recent_clips_count,
    recent_detections, recent_detections_page, relabel_detection, seconds_since_last_detection,
    species_for_date, todays_detection_count, todays_detections, todays_source_activity,
    unlock_detection,
};
pub use heatmap::{hourly_totals, species_daily_heatmap, weekly_heatmap};
pub use images::{
    ImageBlacklist, add_image_blacklist, blacklisted_urls_for_species, is_image_blacklisted,
    list_image_blacklist, remove_image_blacklist,
};
pub use maintenance::{
    BACKUP_VACUUM_INTERVAL_SECS, DAILY_INTERVAL_SECS, JOB_BACKUP_VACUUM, JOB_INTEGRITY_CHECK,
    JOB_SESSION_PRUNE, JOB_SPECIES_CAP, last_run_unix, record_run,
};
pub use quarantine::{
    QuarantineFilter, QuarantineReason, QuarantineRecord, QuarantineRow, QuarantineStats,
    approve_quarantine, count_quarantine, delete_quarantine, get_quarantine, insert_quarantine,
    list_quarantine, prune_quarantine, quarantine_pending_count, quarantine_stats,
    reject_quarantine,
};
pub use species::{
    SpeciesThreshold, delete_species_threshold, get_species_threshold_map, get_species_thresholds,
    recent_by_species, search_species, set_species_threshold, species_count, species_daily_counts,
    species_first_seen, species_hourly_activity, species_hourly_activity_batch, species_sparklines,
    species_summary, top_species,
};
