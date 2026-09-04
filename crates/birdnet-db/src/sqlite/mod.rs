//! `SQLite` operational database.
//!
//! Connection management (WAL mode, PRAGMAs), row types, and query helpers
//! for the birds.db detection database, organized by concern.
//!
//! # Module layout
//!
//! | Sub-module              | Contents                                                    |
//! |-------------------------|-------------------------------------------------------------|
//! | `connection`            | `DbError`, `open_connection`, `open_or_create`, `open_readonly`, `quick_check` |
//! | `types`                 | `DetectionRecord`, `DetectionRow`, `SpeciesCount`, …       |
//! | `queries::detections`   | Insert, count, paginate, filter detection rows             |
//! | `queries::species`      | Per-species aggregates, summaries, and activity            |
//! | `queries::analytics`    | Hourly, daily, confidence distribution, quality metrics    |
//! | `queries::heatmap`      | Hour × day-of-week activity heatmap                        |
//! | `queries::correlation`  | Species co-occurrence and companion species                |
//! | `queries::quarantine`   | Rare-bird quarantine queue CRUD and review workflow        |
//! | `queries::maintenance`  | Restart-durable last-run times for the maintenance schedule |

pub mod connection;
pub mod queries;
pub mod types;

// Flat re-exports so existing call-sites (`birdnet_db::sqlite::foo`) continue
// to compile without modification.
pub use connection::{DbError, open_connection, open_or_create, open_readonly, quick_check};
pub use queries::correlation::{FollowOn, SpeciesPair};
pub use queries::detection_reviews::{
    DetectionReview, ReviewStatus, UnreviewedDetection, clear_detection_review,
    detection_review_counts, detection_review_total, detection_reviews_page, get_detection_review,
    recent_detection_reviews, rejected_detection_count, set_detection_review,
    unreviewed_recent_detections,
};
pub use queries::heatmap::{HeatmapCell, HourTotal};
pub use queries::imports::{
    DIFFERENT_SITE_KM, ImportBatch, delete_import_batch, import_batch_row_count,
    imported_detection_count, list_import_batches,
};
pub use queries::maintenance::{
    AUDIT_RETENTION_DAYS, BACKUP_VACUUM_INTERVAL_SECS, DAILY_INTERVAL_SECS, JOB_BACKUP_VACUUM,
    JOB_INTEGRITY_CHECK, JOB_LOG_RETENTION, JOB_OFFSITE_BACKUP, JOB_SESSION_PRUNE, JOB_SPECIES_CAP,
    JOB_SUMMARY_AUDIT, NOTIFICATION_RETENTION_DAYS, QUARANTINE_RETENTION_DAYS,
    SOUND_LEVEL_RETENTION_DAYS, last_run_result, last_run_unix, record_run, record_run_result,
};
pub use queries::{
    ALL_QUARANTINE_REASONS, CLIP_AVAILABLE, DateRange, DetectionFilter, HourWindow, ImageBlacklist,
    LockFilter, ModelVsReviewRow, QualitySummary, QuarantineFilter, QuarantineReason,
    QuarantineRecord, QuarantineRow, QuarantineStats, RecordingsFilter, ReviewVerdictDay,
    SortOrder, SpeciesThreshold, TodayFilter, VerdictFilter, add_image_blacklist, all_detections,
    analytic_detection_count, analytic_detection_count_for_date, analytic_species_count_for_date,
    approve_quarantine, best_detections_for_date, blacklisted_urls_for_species, com_name_for,
    companion_species, concurrent_detections_from_other_sources, confidence_distribution,
    confidence_trend, count_quarantine, daily_counts, delete_detection, delete_quarantine,
    delete_species_threshold, detected_at_utc_for, detection_at, detection_count,
    detection_count_for_date, detection_count_for_species_date, detection_dates,
    detection_quality_by_hour, detections_by_date, detections_by_species, detections_per_day,
    distinct_detection_dates, get_quarantine, get_species_threshold_map, get_species_thresholds,
    hourly_activity, hourly_totals, insert_detection, insert_quarantine, is_detection_locked,
    is_image_blacklisted, known_sources, last_hour_count, latest_detection, latest_detection_full,
    list_image_blacklist, list_quarantine, lock_detection, locked_file_names,
    low_confidence_species, model_vs_review_by_species, prune_quarantine, quality_summary,
    quarantine_pending_count, quarantine_stats, range_daily_counts, recent_by_species,
    recent_clips, recent_clips_count, recent_detections, recent_detections_page, reject_quarantine,
    relabel_detection, remove_image_blacklist, review_verdict_trend,
    reviewed_detections_by_species, search_detection_count, search_detections, search_species,
    seconds_since_last_detection, set_species_threshold, species_count, species_daily_counts,
    species_daily_heatmap, species_first_detection, species_first_seen, species_for_date,
    species_hourly_activity, species_hourly_activity_batch, species_shapes, species_sparklines,
    species_summary, temporal_cooccurrence, today_species_hour_heatmap, todays_detection_count,
    todays_detections, todays_source_activity, top_cooccurrence_pairs, top_species,
    total_recording_seconds, unlock_detection, unstamped_detection_count, weekly_detection_count,
    weekly_heatmap, weekly_new_species, weekly_top_species,
};
pub use types::{
    ConcurrentDetection, DailyCount, DayCount, DetectionRecord, DetectionRow, HourlyCount,
    SourceActivity, SpeciesCount, SpeciesSummary,
};
