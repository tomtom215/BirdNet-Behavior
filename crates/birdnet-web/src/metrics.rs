//! Process-local Prometheus-style counters and latency histograms.
//!
//! The `/api/v2/metrics` endpoint previously exposed only the snapshot
//! gauges that could be computed at scrape time (DB row counts, RSS,
//! uptime). That's enough to know *what state the system is in*, but not
//! enough to answer *did anything happen between the last two scrapes?*
//! For ecological monitoring the answer to the second question is the
//! whole point.
//!
//! This module adds an `Arc<MetricsRegistry>` carried by `AppState` and
//! a small set of typed mutators the detection daemon and DB layer call
//! into:
//!
//! * `inc_detection(species, chunk_offset_secs)` — bumps the
//!   `birdnet_detections_total{species,chunk_offset}` counter.
//! * `observe_inference_seconds(seconds)` — feeds an
//!   `birdnet_inference_duration_seconds` histogram with fixed buckets.
//! * `observe_db_write_seconds(seconds)` — same idea, separate histogram.
//! * `set_source_up(source, up)` — `birdnet_audio_source_up{source}` gauge.
//! * `inc_watchdog_pings()` — `birdnet_watchdog_pings_total` counter.
//! * `inc_detection_write_failed()` — `birdnet_detection_write_failures_total`
//!   counter: detections the pipeline produced and the database refused.
//!
//! All operations are lock-free (`AtomicU64`) on the counters; the
//! per-species and per-source labelled maps live behind a `RwLock`
//! because reads are vastly more common than writes (one writer per
//! detection vs. one reader per Prometheus scrape).
//!
//! The exposition format is hand-rolled — committing to a Prometheus
//! crate would pull in ~6 transitive deps for what is functionally
//! `println!("metric_name{labels} value")`. The render function is
//! pure, tested below, and conforms to the text 0.0.4 format the
//! existing `/api/v2/metrics` endpoint already speaks.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Fixed exponential histogram buckets (seconds).
///
/// Chosen to bracket the observed decode-to-prediction latency on a Pi 5
/// (~300 ms) and the DB write latency on a stressed SQLite (~30 ms)
/// without splitting the distribution finely outside those bands.
/// `+Inf` is implicit (sum/count).
///
/// Edit only with a corresponding update to the dashboard JSON.
pub const LATENCY_BUCKETS_SECS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Container for one histogram instance.
///
/// Pre-allocates the bucket vector at construction so the hot
/// `observe()` path is a series of `AtomicU64::fetch_add`, no
/// allocation. The `+Inf` bucket is implicit: it equals `count`.
#[derive(Debug)]
struct Histogram {
    bucket_bounds: &'static [f64],
    buckets_le: Vec<AtomicU64>, // cumulative counts, one per bound
    sum_micros: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    fn new() -> Self {
        let buckets_le = (0..LATENCY_BUCKETS_SECS.len())
            .map(|_| AtomicU64::new(0))
            .collect();
        Self {
            bucket_bounds: LATENCY_BUCKETS_SECS,
            buckets_le,
            sum_micros: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn observe(&self, seconds: f64) {
        // Cumulative buckets: every bucket ≥ the smallest fitting bound
        // is incremented. The Prometheus exposition expects this shape.
        for (bound, b) in self.bucket_bounds.iter().zip(self.buckets_le.iter()) {
            if seconds <= *bound {
                b.fetch_add(1, Ordering::Relaxed);
            }
        }
        // Track sum in microseconds to keep precision without going to f64.
        // Negative observations are clamped to 0 (they would underflow the
        // u64 cast); 100 ks (≈1.16 days) saturates safely below u64::MAX.
        let clamped = if seconds.is_nan() {
            0.0
        } else {
            seconds.clamp(0.0, 100_000.0)
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let micros = (clamped * 1_000_000.0) as u64;
        self.sum_micros.fetch_add(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> HistogramSnapshot {
        let buckets = self
            .bucket_bounds
            .iter()
            .zip(self.buckets_le.iter())
            .map(|(bound, b)| (*bound, b.load(Ordering::Relaxed)))
            .collect();
        // Reverse the *1_000_000 the observe path applied. Mantissa
        // precision is fine for any plausible latency-sum total.
        #[allow(clippy::cast_precision_loss)]
        let sum_secs = (self.sum_micros.load(Ordering::Relaxed) as f64) / 1_000_000.0;
        let count = self.count.load(Ordering::Relaxed);
        HistogramSnapshot {
            buckets,
            sum_secs,
            count,
        }
    }
}

/// Plain-data snapshot of a histogram — used by the render function so it
/// can format without holding any locks.
#[derive(Debug, Clone)]
pub struct HistogramSnapshot {
    /// `(upper_bound_secs, cumulative_count)` pairs, one per bucket in
    /// [`LATENCY_BUCKETS_SECS`] order. The implicit `+Inf` bucket equals `count`.
    pub buckets: Vec<(f64, u64)>,
    /// Sum of all observed values in seconds.
    pub sum_secs: f64,
    /// Total number of observations recorded.
    pub count: u64,
}

/// The local date (`YYYY-MM-DD`), hour (0–23) and source an effort credit
/// belongs to.
pub type EffortKey = (String, u8, String);

/// Top-level metrics registry shared between the daemon and the web
/// server via `Arc`.
#[derive(Debug)]
pub struct MetricsRegistry {
    detections: RwLock<HashMap<(String, i64), AtomicU64>>,
    inference_duration: Histogram,
    db_write_duration: Histogram,
    audio_source_up: RwLock<HashMap<String, AtomicU64>>,
    watchdog_pings_total: AtomicU64,
    /// Detections that were classified and then failed to store.
    ///
    /// The daemon logs a `warn!` and moves on, which on an unattended station
    /// is the same as not noticing. The known way to reach it is the hour that
    /// daylight-saving repeats each autumn: local wall-clock is this schema's
    /// identity, so the second pass of that hour can collide with the first on
    /// `idx_detections_unique` and be refused. Any storage fault reaches it
    /// too. A counter makes "some detections were lost" a question the station
    /// can answer instead of one an operator has to grep the journal for.
    detection_write_failures_total: AtomicU64,
    outbound_queue_depth: RwLock<HashMap<String, AtomicU64>>,
    /// Seconds since the most recent stored detection, refreshed by the
    /// deadman task. `u64::MAX` = not yet measured / no detections ever.
    detection_silence_secs: AtomicU64,
    /// The deadman's own verdict (AD-4): `0` quiet within its threshold, `1`
    /// tripped (an episode is open), `u64::MAX` = the deadman is off.
    detection_deadman: AtomicU64,
    /// Whether the system reports its clock as synchronised: `0` no, `1` yes,
    /// `u64::MAX` = nothing here can answer, so the series is absent rather
    /// than reporting a container's host clock as broken.
    clock_synced: AtomicU64,
    /// State of the MQTT presence connection: `0` down, `1` up,
    /// `u64::MAX` = MQTT is not configured, so the series is absent rather
    /// than reporting a broker that was never asked for as broken.
    mqtt_connected: AtomicU64,
    /// When the presence session was last seen to be down, Unix seconds; `0`
    /// while it is up or MQTT is not configured. What the station-health
    /// condition measures a broker outage from (DD-22).
    mqtt_disconnected_since: AtomicU64,
    /// The last MQTT connect or keepalive error, for the operator-facing
    /// condition; cleared on reconnect.
    mqtt_last_error: RwLock<Option<String>>,
    /// Classifications the pipeline produced and then discarded, by reason.
    ///
    /// A station that is "detecting nothing" is either hearing nothing or
    /// throwing everything away, and from outside those look identical. The
    /// reason label is what separates them.
    ///
    /// The reasons production actually emits, and what each one means when it
    /// spikes: `confidence` (the model is unsure — a noisy site, or a threshold
    /// set too high), `duplicate` (the same bird inside the deduplication
    /// window, which is normal and only interesting if it dominates),
    /// `quarantine` (below a per-species threshold, so the row is in the review
    /// queue rather than gone), `implausible_hour` (a day bird at 2 a.m. —
    /// worth looking at, and also in the queue), and `implausible_clock` (the
    /// station is recording without knowing what day it is: NTP has not
    /// reached it, and **nothing it records in this state can be filed**).
    ///
    /// And, counted by the daemon as each file's filters run (ACC-02):
    /// `privacy` (removed for a human voice in or next to its window),
    /// `noise` (a watched noise class), `corroboration` (heard too few times
    /// nearby), `occurrence` (out of range here this week, or not scored by
    /// the occurrence model; the scored ones above the station's bar are in
    /// the review queue as "Unlikely here"), and `species_list` (the
    /// operator's include or exclude list). These are counted at the floor
    /// the classifier runs at, so some would have gone for `confidence` had
    /// they got that far; each discard is counted once, by the first filter
    /// that removed it. `quality` appears only in this file's own tests.
    detections_dropped: RwLock<HashMap<String, AtomicU64>>,
    /// Analysis windows a filter masked whole, by reason (`privacy`, `noise`;
    /// ACC-02): audio the station listened to and did not judge.
    windows_masked: RwLock<HashMap<String, AtomicU64>>,
    /// Capture processes the supervisor has restarted, per source.
    /// Audio files the detection pipeline finished analysing, by source.
    ///
    /// The one series that separates "the model is answering nothing" from
    /// "the pipeline is not running". Every other signal is downstream of a
    /// prediction the model actually made, so on a station with a wrong label
    /// file, a wrong sample rate, or a model swapped by a bad update, all of
    /// them are flat and empty — exactly as they are on a station where
    /// inference never started. See `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`
    /// (OB-12).
    files_analysed: RwLock<HashMap<String, AtomicU64>>,
    /// Audio the pipeline finished analysing, in milliseconds, per source.
    ///
    /// The data rate: a source that delivers continuously adds one second of
    /// audio per second, so `rate()` of it is about 1 once no backlog is
    /// waiting (draining one, it is above 1). A count of files cannot
    /// say that — a camera that drops packets still yields a 15-second
    /// segment file, with less audio in it.
    audio_analysed_ms: RwLock<HashMap<String, AtomicU64>>,
    /// Analysed audio not yet written to the effort ledger, in seconds, by
    /// the local date and hour it was recorded in and its source (RES-01).
    /// Not exported: the effort recorder drains it into `recording_effort`
    /// and `recording_effort_hourly` on its five-minute tick, so a write is
    /// not made for every segment.
    effort_pending: std::sync::Mutex<HashMap<EffortKey, f64>>,
    /// Segments the watcher announced that were gone before the pipeline
    /// read them, per source (PR-1 / S-3): recorded audio never analysed.
    segments_dropped: RwLock<HashMap<String, AtomicU64>>,
    /// Segments the shed policy skipped, by reason (PR-2).
    segments_shed: RwLock<HashMap<String, AtomicU64>>,
    /// Segments waiting for analysis after the last sweep (PR-2);
    /// `u64::MAX` = the daemon has not reported yet.
    analysis_queue_depth: AtomicU64,
    /// Each classifier's health as the daemon last reported it, primary
    /// first (`e853776`). Empty until the daemon has analysed a file.
    model_health: RwLock<Vec<(String, birdnet_core::inference::health::ModelHealth)>>,
    /// What each classifier is (window, rate, labels, bar), as the daemon stated
    /// it once the classifiers had loaded, primary first. Empty until then.
    model_info: RwLock<Vec<birdnet_core::inference::registry::ModelInfo>>,
    /// Notifications that never left the station, by why.
    ///
    /// Both guards on the outbound path — the per-destination circuit breaker
    /// and the per-destination rate limit — drop a notification without
    /// failing, and the skip was logged at `debug`, which the default filter
    /// discards. A station could drop every alert it ever tried to send and
    /// leave no evidence anywhere. See `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`
    /// (OB-5).
    notifications_dropped: RwLock<HashMap<String, AtomicU64>>,
    capture_restarts: RwLock<HashMap<String, AtomicU64>>,
    /// Capture processes found alive but producing no segments, per source.
    /// Distinct from a restart: this is the *diagnosis*, and a stall that keeps
    /// recurring on one source is a hardware fault rather than a flaky link.
    capture_stalls: RwLock<HashMap<String, AtomicU64>>,
    /// What the station did with its model files: the detector, every
    /// configured classifier and the occurrence filter, each in exactly one
    /// state (MDL-01 to MDL-16). Written by the binary as start-up decides each
    /// part, read by every surface that says what the models are doing, and
    /// exported as `birdnet_detector_state`, `birdnet_classifier_state` and
    /// `birdnet_occurrence_filter_state`.
    ///
    /// `birdnet_occurrence_filter_active` and `birdnet_occurrence_candidates`
    /// are read from it too, so the one number that would have made the
    /// inert-filter defect visible cannot disagree with the reason given.
    models: RwLock<birdnet_core::inference::model_state::ModelsState>,
    /// Each model file the configuration names and the catalogue knows, and
    /// what the station is doing to keep it the pinned file (MDL-10),
    /// exported as `birdnet_model_file_state`.
    model_files: RwLock<Vec<birdnet_core::inference::model_state::ModelFile>>,
    /// Clips the database referenced and the disk did not have, found and
    /// stamped by the last reconciliation pass (S-14). `u64::MAX` until a
    /// pass has run.
    orphaned_clips: AtomicU64,
    /// Detections the SQLite store accepted and the DuckDB copy refused
    /// (OP-7): each one is a row the behavioural dashboards will not show
    /// until the startup drift check rebuilds the copy.
    analytics_mirror_failures_total: AtomicU64,
    /// Seconds since the Unix epoch of the last such failure; `0` for none.
    analytics_mirror_last_failure: AtomicU64,
    /// The last mirror error's text, for the condition.
    analytics_mirror_last_error: RwLock<Option<String>>,
    /// Set by a disk manager whose last purge removed recordings and lowered
    /// usage by nothing (PR-7): the card is full of something the purge
    /// cannot reach, and it has stopped deleting. Shared with the manager.
    purge_ineffective: Arc<AtomicBool>,
    /// Set by the recordings disk manager while the disk is over the purge
    /// threshold and its last pass found nothing it was allowed to delete
    /// (N25-01): every clip left is protected, or there are none. Not a latch;
    /// cleared by the first pass that deletes something or a reading below the
    /// threshold. Shared with the manager.
    nothing_purgeable: Arc<AtomicBool>,
    /// Set by the recordings disk manager when the disk is full and the station
    /// is set to keep everything (`FULL_DISK=keep`): recording is on hold and
    /// no clip is deleted. Shared with the manager, read by the capture
    /// supervisor.
    recording_held: Arc<AtomicBool>,
    /// HTTP responses served, by status class (`2xx`, `4xx`, …).
    http_responses: RwLock<HashMap<String, AtomicU64>>,
    /// Web request latency.
    http_duration: Histogram,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    /// Create a new, zeroed-out registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            detections: RwLock::new(HashMap::new()),
            inference_duration: Histogram::new(),
            db_write_duration: Histogram::new(),
            audio_source_up: RwLock::new(HashMap::new()),
            watchdog_pings_total: AtomicU64::new(0),
            detection_write_failures_total: AtomicU64::new(0),
            outbound_queue_depth: RwLock::new(HashMap::new()),
            detection_silence_secs: AtomicU64::new(u64::MAX),
            detection_deadman: AtomicU64::new(u64::MAX),
            clock_synced: AtomicU64::new(u64::MAX),
            mqtt_connected: AtomicU64::new(u64::MAX),
            mqtt_disconnected_since: AtomicU64::new(0),
            mqtt_last_error: RwLock::new(None),
            detections_dropped: RwLock::new(HashMap::new()),
            windows_masked: RwLock::new(HashMap::new()),
            files_analysed: RwLock::new(HashMap::new()),
            audio_analysed_ms: RwLock::new(HashMap::new()),
            effort_pending: std::sync::Mutex::new(HashMap::new()),
            segments_dropped: RwLock::new(HashMap::new()),
            segments_shed: RwLock::new(HashMap::new()),
            analysis_queue_depth: AtomicU64::new(u64::MAX),
            model_health: RwLock::new(Vec::new()),
            model_info: RwLock::new(Vec::new()),
            notifications_dropped: RwLock::new(HashMap::new()),
            capture_restarts: RwLock::new(HashMap::new()),
            capture_stalls: RwLock::new(HashMap::new()),
            models: RwLock::new(birdnet_core::inference::model_state::ModelsState::default()),
            model_files: RwLock::new(Vec::new()),
            orphaned_clips: AtomicU64::new(u64::MAX),
            analytics_mirror_failures_total: AtomicU64::new(0),
            analytics_mirror_last_failure: AtomicU64::new(0),
            analytics_mirror_last_error: RwLock::new(None),
            purge_ineffective: Arc::new(AtomicBool::new(false)),
            nothing_purgeable: Arc::new(AtomicBool::new(false)),
            recording_held: Arc::new(AtomicBool::new(false)),
            http_responses: RwLock::new(HashMap::new()),
            http_duration: Histogram::new(),
        }
    }

    /// Bump a labelled counter map, taking the write lock only to insert.
    fn bump(map: &RwLock<HashMap<String, AtomicU64>>, key: &str) {
        Self::add(map, key, 1);
    }

    /// Add `n` to a labelled counter.
    fn add(map: &RwLock<HashMap<String, AtomicU64>>, key: &str, n: u64) {
        if let Ok(m) = map.read()
            && let Some(c) = m.get(key)
        {
            c.fetch_add(n, Ordering::Relaxed);
            return;
        }
        if let Ok(mut m) = map.write() {
            m.entry(key.to_owned())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(n, Ordering::Relaxed);
        }
    }

    /// Read a labelled counter map into a sorted vector.
    fn read_map(map: &RwLock<HashMap<String, AtomicU64>>) -> Vec<(String, u64)> {
        map.read().map_or_else(
            |_| Vec::new(),
            |m| {
                let mut v: Vec<(String, u64)> = m
                    .iter()
                    .map(|(k, c)| (k.clone(), c.load(Ordering::Relaxed)))
                    .collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            },
        )
    }

    /// Record a classification the pipeline discarded.
    ///
    /// `reason` is a small closed vocabulary (`quality`, `privacy`,
    /// `occurrence`, `confidence`, `quarantine`) rather than free text, so the
    /// label cardinality cannot grow with traffic.
    pub fn inc_detection_dropped(&self, reason: &str) {
        Self::bump(&self.detections_dropped, reason);
    }

    /// Record `n` classifications the daemon's filters discarded for
    /// `reason` (ACC-02). Zero records nothing, so a reason no file has
    /// triggered stays absent.
    pub fn add_detections_dropped(&self, reason: &str, n: u64) {
        if n > 0 {
            Self::add(&self.detections_dropped, reason, n);
        }
    }

    /// Record `n` analysis windows masked whole for `reason` (ACC-02). Zero
    /// records nothing.
    pub fn add_windows_masked(&self, reason: &str, n: u64) {
        if n > 0 {
            Self::add(&self.windows_masked, reason, n);
        }
    }

    /// Record that the pipeline finished analysing one audio file.
    ///
    /// A healthy station analysing 15-second segments emits about 5 760 of
    /// these a day per source, so a *flat* counter alongside
    /// `birdnet_audio_source_up == 1` means capture is writing files nothing is
    /// analysing, and a *rising* counter with no detections means the model is
    /// answering nothing. Neither of those was distinguishable from outside.
    pub fn inc_file_analysed(&self, source: &str) {
        Self::bump(&self.files_analysed, source);
    }

    /// Record how much audio one analysed file held. A length that is not a
    /// finite, non-negative number of seconds is not counted.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn add_audio_analysed(&self, source: &str, secs: f64) {
        if secs.is_finite() && secs >= 0.0 {
            Self::add(
                &self.audio_analysed_ms,
                source,
                (secs * 1000.0).round() as u64,
            );
        }
    }

    /// Hold `secs` of analysed audio for the effort ledger, under the local
    /// `date` and `hour` it was recorded in and its `source` (RES-01). A
    /// length that is not a finite, non-negative number is not held.
    pub fn add_effort_pending(&self, date: &str, hour: u8, source: &str, secs: f64) {
        if !(secs.is_finite() && secs >= 0.0) {
            return;
        }
        let mut pending = self
            .effort_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *pending
            .entry((date.to_owned(), hour, source.to_owned()))
            .or_insert(0.0) += secs;
    }

    /// Take every held effort credit, leaving none. The caller writes them,
    /// and gives back with [`Self::add_effort_pending`] any it could not.
    pub fn take_effort_pending(&self) -> HashMap<EffortKey, f64> {
        std::mem::take(
            &mut *self
                .effort_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Record a segment that vanished before the pipeline analysed it
    /// (PR-1 / S-3).
    pub fn inc_segment_dropped(&self, source: &str) {
        Self::bump(&self.segments_dropped, source);
    }

    /// Record a segment the shed policy skipped, by reason (PR-2).
    pub fn inc_segment_shed(&self, reason: &str) {
        Self::bump(&self.segments_shed, reason);
    }

    /// Publish how many segments are waiting for analysis (PR-2).
    pub fn set_analysis_queue_depth(&self, depth: usize) {
        self.analysis_queue_depth.store(
            u64::try_from(depth).unwrap_or(u64::MAX - 1),
            Ordering::Relaxed,
        );
    }

    /// Publish each classifier's health (`e853776`).
    pub fn set_model_health(
        &self,
        health: &[(String, birdnet_core::inference::health::ModelHealth)],
    ) {
        if let Ok(mut current) = self.model_health.write() {
            current.clear();
            current.extend_from_slice(health);
        }
    }

    /// Publish what each classifier is, once they have loaded.
    pub fn set_model_info(&self, info: &[birdnet_core::inference::registry::ModelInfo]) {
        if let Ok(mut current) = self.model_info.write() {
            current.clear();
            current.extend_from_slice(info);
        }
    }

    /// What each classifier is, primary first; empty before the daemon has
    /// loaded them (or in web-only mode, where none is).
    #[must_use]
    pub fn model_info(&self) -> Vec<birdnet_core::inference::registry::ModelInfo> {
        self.model_info
            .read()
            .map_or_else(|_| Vec::new(), |m| m.clone())
    }

    /// Each classifier's health as last reported, primary first; empty before
    /// the daemon has analysed a file.
    #[must_use]
    pub fn model_health(&self) -> Vec<(String, birdnet_core::inference::health::ModelHealth)> {
        self.model_health
            .read()
            .map_or_else(|_| Vec::new(), |m| m.clone())
    }

    /// Segments waiting for analysis after the last sweep; `None` before the
    /// daemon has reported.
    #[must_use]
    pub fn analysis_queue_depth(&self) -> Option<u64> {
        match self.analysis_queue_depth.load(Ordering::Relaxed) {
            u64::MAX => None,
            v => Some(v),
        }
    }

    /// Record a notification that never left the station.
    ///
    /// `reason` is a small closed vocabulary — `circuit_open`, `rate_limited`,
    /// `send_failed`, `no_destination` — rather than free text, so the label
    /// cardinality cannot grow with traffic.
    pub fn inc_notification_dropped(&self, reason: &str) {
        Self::bump(&self.notifications_dropped, reason);
    }

    /// Record that a capture process was restarted.
    pub fn inc_capture_restart(&self, source: &str) {
        Self::bump(&self.capture_restarts, source);
    }

    /// Record that a capture process was found alive but silently stalled.
    pub fn inc_capture_stall(&self, source: &str) {
        Self::bump(&self.capture_stalls, source);
    }

    /// Publish the occurrence filter's state: filtering, with how many species
    /// it admitted when it last ran, or off and why.
    pub fn set_occurrence_filter(&self, state: birdnet_core::inference::model_state::FilterState) {
        self.models
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .occurrence_filter = state;
    }

    /// Publish what became of every configured classifier, the primary first.
    pub fn set_classifier_states(
        &self,
        states: Vec<birdnet_core::inference::model_state::ClassifierState>,
    ) {
        self.models
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .classifiers = states;
    }

    /// Publish the detector's state. One that is not running takes the
    /// classifiers it had loaded and the occurrence filter with it (see
    /// [`birdnet_core::inference::model_state::ModelsState::set_detector`]).
    pub fn set_detector_state(&self, state: birdnet_core::inference::model_state::DetectorState) {
        self.models
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_detector(state);
    }

    /// What the station did with its model files, as last published.
    #[must_use]
    pub fn models_state(&self) -> birdnet_core::inference::model_state::ModelsState {
        // A writer that panicked left a whole state behind (each write
        // replaces one part), so the last one written is read rather than an
        // invented "starting".
        self.models
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Publish what the station is doing about one model file (MDL-10),
    /// replacing what was published for the same path.
    pub fn set_model_file(&self, file: birdnet_core::inference::model_state::ModelFile) {
        let mut files = self
            .model_files
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot) = files.iter_mut().find(|f| f.path == file.path) {
            *slot = file;
        } else {
            files.push(file);
        }
    }

    /// Every model file the station looks after, in the order first
    /// published, with what it is doing about each (MDL-10).
    #[must_use]
    pub fn model_files(&self) -> Vec<birdnet_core::inference::model_state::ModelFile> {
        self.model_files
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The flag a disk manager sets when its purge stopped achieving
    /// anything (PR-7), for the manager to hold.
    #[must_use]
    pub fn purge_ineffective_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.purge_ineffective)
    }

    /// Whether the last purge removed recordings without lowering usage.
    #[must_use]
    pub fn purge_ineffective(&self) -> bool {
        self.purge_ineffective.load(Ordering::Relaxed)
    }

    /// The flag a disk manager sets while the disk is over the purge threshold
    /// and its purge finds nothing it may delete (N25-01), for the manager to
    /// hold.
    #[must_use]
    pub fn nothing_purgeable_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.nothing_purgeable)
    }

    /// Whether the disk is over the purge threshold and the last purge found
    /// nothing it was allowed to delete.
    #[must_use]
    pub fn nothing_purgeable(&self) -> bool {
        self.nothing_purgeable.load(Ordering::Relaxed)
    }

    /// The flag the recordings disk manager raises when the disk is full and
    /// the station keeps everything (`FULL_DISK=keep`), and the capture
    /// supervisor honours by not recording.
    #[must_use]
    pub fn recording_held_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.recording_held)
    }

    /// Whether recording is on hold because the disk is full.
    #[must_use]
    pub fn recording_held(&self) -> bool {
        self.recording_held.load(Ordering::Relaxed)
    }

    /// Record a detection the DuckDB copy refused (OP-7).
    pub fn inc_analytics_mirror_failed(&self, error: &str) {
        self.analytics_mirror_failures_total
            .fetch_add(1, Ordering::Relaxed);
        self.analytics_mirror_last_failure
            .store(unix_now_secs(), Ordering::Relaxed);
        if let Ok(mut e) = self.analytics_mirror_last_error.write() {
            *e = Some(error.to_owned());
        }
    }

    /// Detections the DuckDB copy refused since process start.
    #[must_use]
    pub fn analytics_mirror_failures(&self) -> u64 {
        self.analytics_mirror_failures_total.load(Ordering::Relaxed)
    }

    /// When the last mirror write failed, as seconds since the Unix epoch,
    /// with the error; `None` when none has.
    #[must_use]
    pub fn analytics_mirror_last_failure(&self) -> Option<(u64, String)> {
        match self.analytics_mirror_last_failure.load(Ordering::Relaxed) {
            0 => None,
            at => Some((
                at,
                self.analytics_mirror_last_error
                    .read()
                    .ok()
                    .and_then(|e| e.clone())
                    .unwrap_or_default(),
            )),
        }
    }

    /// Record what the last clip-reconciliation pass found: rows whose clip
    /// the disk no longer had.
    pub fn set_orphaned_clips(&self, found: u64) {
        self.orphaned_clips.store(found, Ordering::Relaxed);
    }

    /// Orphaned clips found by the last reconciliation pass; `None` until one
    /// has run.
    #[must_use]
    pub fn orphaned_clips(&self) -> Option<u64> {
        match self.orphaned_clips.load(Ordering::Relaxed) {
            u64::MAX => None,
            n => Some(n),
        }
    }

    /// The occurrence filter's live state: whether it is running, and how
    /// many species it admits (`None` until it has run, and whenever it is
    /// off). Read from [`Self::models_state`], for the surfaces a person reads.
    pub fn occurrence_filter(&self) -> OccurrenceFilterState {
        let models = self.models_state();
        OccurrenceFilterState {
            active: models.occurrence_filter.is_active(),
            candidates: models.occurrence_filter.candidates(),
        }
    }

    /// Record one served HTTP response.
    pub fn observe_http(&self, status: u16, seconds: f64) {
        let class = match status {
            100..=199 => "1xx",
            200..=299 => "2xx",
            300..=399 => "3xx",
            400..=499 => "4xx",
            _ => "5xx",
        };
        Self::bump(&self.http_responses, class);
        self.http_duration.observe(seconds);
    }

    /// Bump the detection counter for `(species, chunk_offset)`.
    ///
    /// `chunk_offset_secs` is rounded to the nearest integer so the
    /// counter cardinality stays bounded (the V3.0 model picks 4.5 s
    /// chunks → roughly `recording_length / 4.5` distinct values per
    /// recording, capped at ~14 for the default 60 s `recording_length`).
    pub fn inc_detection(&self, species: &str, chunk_offset_secs: f32) {
        // chunk_offset_secs in practice is in [0, 86400] (≤ 1 day's worth
        // of recording chunks). Clamp anyway so a runaway value can't
        // produce an absurd label.
        #[allow(clippy::cast_possible_truncation)]
        let chunk_offset = chunk_offset_secs.round().clamp(0.0, 1.0e9) as i64;
        let key = (species.to_owned(), chunk_offset);
        // Fast path: read lock, find existing entry, bump.
        if let Ok(map) = self.detections.read()
            && let Some(c) = map.get(&key)
        {
            c.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // Slow path: insert a new entry under write lock.
        if let Ok(mut map) = self.detections.write() {
            map.entry(key)
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record one inference latency observation (seconds).
    pub fn observe_inference_seconds(&self, seconds: f64) {
        self.inference_duration.observe(seconds);
    }

    /// Record one DB write latency observation (seconds).
    pub fn observe_db_write_seconds(&self, seconds: f64) {
        self.db_write_duration.observe(seconds);
    }

    /// Set the `audio_source_up{source}` gauge to 0 or 1.
    pub fn set_source_up(&self, source: &str, up: bool) {
        let v = u64::from(up);
        if let Ok(map) = self.audio_source_up.read()
            && let Some(g) = map.get(source)
        {
            g.store(v, Ordering::Relaxed);
            return;
        }
        if let Ok(mut map) = self.audio_source_up.write() {
            map.entry(source.to_owned())
                .or_insert_with(|| AtomicU64::new(0))
                .store(v, Ordering::Relaxed);
        }
    }

    /// Current value of the `audio_source_up{source}` gauge.
    ///
    /// Returns `Some(true)` for an up source, `Some(false)` for a known-down
    /// source, and `None` when no gauge has ever been published under that
    /// label — which the audio-source probe handler treats as "source not
    /// registered with the supervisor" and falls back to the legacy
    /// heuristic.
    #[must_use]
    pub fn source_up(&self, source: &str) -> Option<bool> {
        self.audio_source_up
            .read()
            .ok()
            .and_then(|map| map.get(source).map(|g| g.load(Ordering::Relaxed) == 1))
    }

    /// Set the `outbound_queue_depth{kind}` gauge (store-and-forward backlog).
    pub fn set_outbound_queue_depth(&self, kind: &str, depth: u64) {
        if let Ok(map) = self.outbound_queue_depth.read()
            && let Some(g) = map.get(kind)
        {
            g.store(depth, Ordering::Relaxed);
            return;
        }
        if let Ok(mut map) = self.outbound_queue_depth.write() {
            map.entry(kind.to_owned())
                .or_insert_with(|| AtomicU64::new(0))
                .store(depth, Ordering::Relaxed);
        }
    }

    /// Record seconds elapsed since the most recent stored detection
    /// (deadman freshness signal). Pass `None` when the station has no
    /// detections yet — renders as "unknown" rather than an alarming zero.
    pub fn set_detection_silence_secs(&self, secs: Option<u64>) {
        self.detection_silence_secs
            .store(secs.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Seconds since the most recent detection, when known.
    #[must_use]
    pub fn detection_silence_secs(&self) -> Option<u64> {
        match self.detection_silence_secs.load(Ordering::Relaxed) {
            u64::MAX => None,
            v => Some(v),
        }
    }

    /// Publish the deadman's verdict (AD-4): `None` when it is off,
    /// `Some(true)` while an episode is open.
    pub fn set_detection_deadman(&self, tripped: Option<bool>) {
        self.detection_deadman
            .store(tripped.map_or(u64::MAX, u64::from), Ordering::Relaxed);
    }

    /// The deadman's verdict: `None` off, `Some(false)` quiet within its
    /// threshold, `Some(true)` tripped.
    #[must_use]
    pub fn detection_deadman(&self) -> Option<bool> {
        match self.detection_deadman.load(Ordering::Relaxed) {
            u64::MAX => None,
            v => Some(v == 1),
        }
    }

    /// Record whether the system reports its clock as synchronised.
    ///
    /// `None` means the question could not be answered — no systemd to ask,
    /// which is every Docker deployment — and renders as an absent series
    /// rather than a `0` nobody can act on.
    pub fn set_clock_synced(&self, synced: Option<bool>) {
        self.clock_synced
            .store(synced.map_or(u64::MAX, u64::from), Ordering::Relaxed);
    }

    /// Clock synchronisation state, when the system can report one.
    #[must_use]
    pub fn clock_synced(&self) -> Option<bool> {
        match self.clock_synced.load(Ordering::Relaxed) {
            u64::MAX => None,
            v => Some(v == 1),
        }
    }

    /// Record whether the MQTT presence session is currently connected.
    ///
    /// Only the presence loop calls this, and only on a station with MQTT
    /// configured — so the series exists exactly when there is a broker whose
    /// reachability is a real question.
    pub fn set_mqtt_connected(&self, up: bool) {
        self.mqtt_connected.store(u64::from(up), Ordering::Relaxed);
        if up {
            self.mqtt_disconnected_since.store(0, Ordering::Relaxed);
            if let Ok(mut e) = self.mqtt_last_error.write() {
                *e = None;
            }
        } else if self.mqtt_disconnected_since.load(Ordering::Relaxed) == 0 {
            // The first observation of the outage, not the latest: the
            // condition measures how long the broker has been gone.
            self.mqtt_disconnected_since
                .store(unix_now_secs().max(1), Ordering::Relaxed);
        }
    }

    /// Record why the last MQTT connect or keepalive failed.
    pub fn set_mqtt_error(&self, error: &str) {
        if let Ok(mut e) = self.mqtt_last_error.write() {
            *e = Some(error.chars().take(300).collect());
        }
    }

    /// When the MQTT presence session was first seen down, or `None` while
    /// it is up or MQTT is not configured.
    #[must_use]
    pub fn mqtt_disconnected_since(&self) -> Option<u64> {
        match self.mqtt_disconnected_since.load(Ordering::Relaxed) {
            0 => None,
            t => Some(t),
        }
    }

    /// The last MQTT error, if the session is down.
    #[must_use]
    pub fn mqtt_last_error(&self) -> Option<String> {
        self.mqtt_last_error.read().ok().and_then(|e| e.clone())
    }

    /// MQTT presence connection state, or `None` when MQTT is not configured.
    #[must_use]
    pub fn mqtt_connected(&self) -> Option<bool> {
        match self.mqtt_connected.load(Ordering::Relaxed) {
            u64::MAX => None,
            v => Some(v == 1),
        }
    }

    /// Bump the watchdog ping counter.
    pub fn inc_watchdog_pings(&self) {
        self.watchdog_pings_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Bump the counter of detections the database refused.
    ///
    /// Called from the one place that can know — the event processor's insert
    /// error arm — so the count is of *detections lost*, not of SQL errors.
    pub fn inc_detection_write_failed(&self) {
        self.detection_write_failures_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Sample every metric for a Prometheus scrape.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        let detections = self.detections.read().map_or_else(
            |_| Vec::new(),
            |m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
                    .collect()
            },
        );
        let source_up = self.audio_source_up.read().map_or_else(
            |_| Vec::new(),
            |m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
                    .collect()
            },
        );
        let outbound_queue = self.outbound_queue_depth.read().map_or_else(
            |_| Vec::new(),
            |m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
                    .collect()
            },
        );
        MetricsSnapshot {
            detections,
            inference: self.inference_duration.snapshot(),
            db_write: self.db_write_duration.snapshot(),
            source_up,
            outbound_queue,
            detection_silence_secs: self.detection_silence_secs(),
            clock_synced: self.clock_synced(),
            mqtt_connected: self.mqtt_connected(),
            detections_dropped: Self::read_map(&self.detections_dropped),
            windows_masked: Self::read_map(&self.windows_masked),
            files_analysed: Self::read_map(&self.files_analysed),
            audio_analysed_ms: Self::read_map(&self.audio_analysed_ms),
            segments_dropped: Self::read_map(&self.segments_dropped),
            segments_shed: Self::read_map(&self.segments_shed),
            analysis_queue_depth: self.analysis_queue_depth(),
            model_health: self.model_health(),
            notifications_dropped: Self::read_map(&self.notifications_dropped),
            capture_restarts: Self::read_map(&self.capture_restarts),
            capture_stalls: Self::read_map(&self.capture_stalls),
            models: self.models_state(),
            model_files: self.model_files(),
            orphaned_clips: self.orphaned_clips(),
            analytics_mirror_failures: self.analytics_mirror_failures(),
            disk_full: DiskFullSnapshot {
                purge_ineffective: self.purge_ineffective(),
                nothing_purgeable: self.nothing_purgeable(),
                recording_held: self.recording_held(),
            },
            http_responses: Self::read_map(&self.http_responses),
            http_duration: self.http_duration.snapshot(),
            watchdog_pings: self.watchdog_pings_total.load(Ordering::Relaxed),
            detection_write_failures: self.detection_write_failures_total.load(Ordering::Relaxed),
        }
    }
}

/// What the species occurrence filter is doing right now (ON-12).
///
/// Published to Prometheus since the inert-filter defect, and now to the
/// station page: a filter that admits zero species is a station that records
/// nothing, and an operator without a metrics stack had no way to see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccurrenceFilterState {
    /// Whether the metadata model is loaded and filtering. `false` means every
    /// species the classifier knows is a candidate, wherever the station is.
    pub active: bool,
    /// How many species the filter admitted the last time it ran; `None`
    /// until it has.
    pub candidates: Option<u64>,
}

impl OccurrenceFilterState {
    /// A filter that is running and admits no species at all: nothing the
    /// station hears can be recorded until it is fixed.
    #[must_use]
    pub const fn admits_nothing(self) -> bool {
        self.active && matches!(self.candidates, Some(0))
    }
}

/// What the disk managers last reported about a full disk, for the renderer.
///
/// Three signals, kept together because each answers "what is a full disk
/// doing to this station?" and none implies another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskFullSnapshot {
    /// Whether the last disk purge removed recordings without lowering usage.
    pub purge_ineffective: bool,
    /// Whether the disk is over the purge threshold and the last purge found
    /// nothing it was allowed to delete.
    pub nothing_purgeable: bool,
    /// Whether recording is on hold because the disk is full and the station
    /// keeps everything.
    pub recording_held: bool,
}

/// Plain snapshot of every metric, returned to the renderer.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    /// Per-`(species, chunk_offset_secs)` detection counts.
    pub detections: Vec<((String, i64), u64)>,
    /// Snapshot of the BirdNET decode-to-prediction latency histogram.
    ///
    /// Observed once per **stored detection** (`daemon::processor`, in the
    /// `Accept` arm after `insert_detection`) — not once per analysed audio
    /// chunk. Its count therefore tracks bird activity, not throughput, and it
    /// cannot be used to derive what fraction of captured audio was analysed.
    pub inference: HistogramSnapshot,
    /// Snapshot of the SQLite detection-row write latency histogram.
    pub db_write: HistogramSnapshot,
    /// Per-source `audio_source_up` gauge values (0 = down, 1 = up).
    pub source_up: Vec<(String, u64)>,
    /// Total `WATCHDOG=1` pings sent to systemd since process start.
    pub watchdog_pings: u64,
    /// Detections classified but refused by the database since process start.
    pub detection_write_failures: u64,
    /// Store-and-forward backlog per channel kind.
    pub outbound_queue: Vec<(String, u64)>,
    /// Seconds since the most recent stored detection (`None` = unknown).
    pub detection_silence_secs: Option<u64>,
    /// Clock synchronisation state (`None` = the system cannot say).
    pub clock_synced: Option<bool>,
    /// MQTT presence connection state (`None` = MQTT not configured).
    pub mqtt_connected: Option<bool>,
    /// Discarded classifications by reason.
    pub detections_dropped: Vec<(String, u64)>,
    /// Analysis windows masked whole, by reason.
    pub windows_masked: Vec<(String, u64)>,
    /// Audio files the pipeline finished analysing, per source.
    pub files_analysed: Vec<(String, u64)>,
    /// Audio analysed, in milliseconds, per source.
    pub audio_analysed_ms: Vec<(String, u64)>,
    /// Segments gone before the pipeline read them, per source.
    pub segments_dropped: Vec<(String, u64)>,
    /// Segments the shed policy skipped, by reason.
    pub segments_shed: Vec<(String, u64)>,
    /// Segments waiting for analysis, when the daemon has reported.
    pub analysis_queue_depth: Option<u64>,
    /// Each classifier's health, primary first.
    pub model_health: Vec<(String, birdnet_core::inference::health::ModelHealth)>,
    /// Notifications that never left the station, by reason.
    pub notifications_dropped: Vec<(String, u64)>,
    /// Capture restarts per source.
    pub capture_restarts: Vec<(String, u64)>,
    /// Silent stalls diagnosed per source.
    pub capture_stalls: Vec<(String, u64)>,
    /// What the station did with its model files: the detector, each
    /// classifier and the occurrence filter.
    pub models: birdnet_core::inference::model_state::ModelsState,
    /// The model files the station looks after (MDL-10).
    pub model_files: Vec<birdnet_core::inference::model_state::ModelFile>,
    /// Orphaned clips found by the last reconciliation pass; `None` until one
    /// has run.
    pub orphaned_clips: Option<u64>,
    /// Detections the DuckDB copy refused since process start.
    pub analytics_mirror_failures: u64,
    /// What the disk managers last reported about a full disk.
    pub disk_full: DiskFullSnapshot,
    /// HTTP responses by status class.
    pub http_responses: Vec<(String, u64)>,
    /// Web request latency.
    pub http_duration: HistogramSnapshot,
}

/// The station's models as state-set gauges (MDL-01 to MDL-16): for the
/// detector, each configured classifier and the occurrence filter, one series
/// per state it can be in, the one it is in at 1 and the rest at 0. Every
/// state is listed, so an alert on `…{state="failed"} == 1` has a series to
/// read before the first failure as well as after it.
///
/// `birdnet_occurrence_filter_active` is 1 only for a filter that is
/// filtering: one configured with no coordinates, or whose files are missing
/// or unusable, is 0 (MDL-04). Its candidate count is exported only while it
/// is filtering and has run, never invented for one that is off.
fn push_models_series(
    out: &mut String,
    models: &birdnet_core::inference::model_state::ModelsState,
) {
    use birdnet_core::inference::model_state::{ClassifierStatus, DetectorState, FilterState};
    use std::fmt::Write as _;

    out.push_str("# HELP birdnet_detector_state The detection daemon's state: 1 for the state it is in (starting, running, web_only, not_configured, failed, stopped), 0 for the others.\n");
    out.push_str("# TYPE birdnet_detector_state gauge\n");
    for code in DetectorState::CODES {
        let _ = writeln!(
            out,
            "birdnet_detector_state{{state=\"{code}\"}} {}",
            u8::from(*code == models.detector.code())
        );
    }
    if !models.classifiers.is_empty() {
        out.push_str("# HELP birdnet_classifier_state Each configured classifier's state: 1 for the state it is in (judging; unrouted, loaded and judging nothing; or left_out_<why>), 0 for the others.\n");
        out.push_str("# TYPE birdnet_classifier_state gauge\n");
        for classifier in &models.classifiers {
            let id = escape_label(&classifier.id);
            for code in ClassifierStatus::CODES {
                let _ = writeln!(
                    out,
                    "birdnet_classifier_state{{classifier=\"{id}\",state=\"{code}\"}} {}",
                    u8::from(*code == classifier.status.code())
                );
            }
        }
    }
    out.push_str("# HELP birdnet_occurrence_filter_state The species occurrence filter's state: 1 for the state it is in (active, or off_<why>), 0 for the others.\n");
    out.push_str("# TYPE birdnet_occurrence_filter_state gauge\n");
    for code in FilterState::CODES {
        let _ = writeln!(
            out,
            "birdnet_occurrence_filter_state{{state=\"{code}\"}} {}",
            u8::from(*code == models.occurrence_filter.code())
        );
    }

    out.push_str("# HELP birdnet_occurrence_filter_active Whether species occurrence filtering is running (1) or every species the classifier knows is admitted (0).\n");
    out.push_str("# TYPE birdnet_occurrence_filter_active gauge\n");
    let _ = writeln!(
        out,
        "birdnet_occurrence_filter_active {}",
        u8::from(models.occurrence_filter.is_active())
    );
    if let Some(n) = models.occurrence_filter.candidates() {
        out.push_str("# HELP birdnet_occurrence_candidates Species the occurrence filter currently admits.\n");
        out.push_str("# TYPE birdnet_occurrence_candidates gauge\n");
        let _ = writeln!(out, "birdnet_occurrence_candidates {n}");
    }
}

/// Each model file the station looks after (MDL-10) as a state-set gauge:
/// one series per state, the one it is in at 1. Nothing is exported before
/// the first file is published, so a station with no catalogue file
/// configured has no series that would read as "checked".
fn push_model_file_series(
    out: &mut String,
    files: &[birdnet_core::inference::model_state::ModelFile],
) {
    use birdnet_core::inference::model_state::RepairState;
    use std::fmt::Write as _;
    if files.is_empty() {
        return;
    }
    out.push_str("# HELP birdnet_model_file_state Each model file the configuration names and the catalogue knows: 1 for the state it is in (checking, verified, fetching, will_retry, not_fetched, repaired), 0 for the others.\n");
    out.push_str("# TYPE birdnet_model_file_state gauge\n");
    for file in files {
        let name = escape_label(&file.path.file_name().map_or_else(
            || file.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ));
        for code in RepairState::CODES {
            let _ = writeln!(
                out,
                "birdnet_model_file_state{{file=\"{name}\",state=\"{code}\"}} {}",
                u8::from(*code == file.repair.code())
            );
        }
    }
}

/// Render the runtime metrics as Prometheus text 0.0.4.
///
/// Each classifier's health (`e853776`): whether it is failing, its run of
/// failed windows, the windows it has run, and when it last answered.
///
/// The last-success timestamp is omitted for a classifier that has never
/// answered, rather than exported as 0 — a timestamp of the epoch would read
/// as measured.
fn push_model_health_series(
    out: &mut String,
    health: &[(String, birdnet_core::inference::health::ModelHealth)],
) {
    use std::fmt::Write as _;
    if health.is_empty() {
        return;
    }
    out.push_str("# HELP birdnet_model_failing Whether a classifier is failing (1): it has failed at least 10 analysis windows in a row and answered none since. 0 while it answers.\n");
    out.push_str("# TYPE birdnet_model_failing gauge\n");
    for (model, h) in health {
        let _ = writeln!(
            out,
            "birdnet_model_failing{{model=\"{}\"}} {}",
            escape_label(model),
            u8::from(h.is_failing())
        );
    }
    out.push_str("# HELP birdnet_model_consecutive_failed_windows Analysis windows a classifier has failed since it last answered one.\n");
    out.push_str("# TYPE birdnet_model_consecutive_failed_windows gauge\n");
    for (model, h) in health {
        let _ = writeln!(
            out,
            "birdnet_model_consecutive_failed_windows{{model=\"{}\"}} {}",
            escape_label(model),
            h.consecutive_failures
        );
    }
    out.push_str("# HELP birdnet_model_windows_total Analysis windows a classifier has run since it was loaded, answered or failed.\n");
    out.push_str("# TYPE birdnet_model_windows_total counter\n");
    for (model, h) in health {
        let _ = writeln!(
            out,
            "birdnet_model_windows_total{{model=\"{}\"}} {}",
            escape_label(model),
            h.windows
        );
    }
    out.push_str("# HELP birdnet_model_inference_seconds_total Seconds a classifier has spent inside inference on windows it answered, since it was loaded. Divide its rate by the rate of birdnet_model_inference_windows_total for the mean time per window. Absent for one that has answered none.\n");
    out.push_str("# TYPE birdnet_model_inference_seconds_total counter\n");
    for (model, h) in health {
        if h.timed_windows > 0 {
            #[allow(clippy::cast_precision_loss)]
            let secs = h.answered_micros as f64 / 1_000_000.0;
            let _ = writeln!(
                out,
                "birdnet_model_inference_seconds_total{{model=\"{}\"}} {secs}",
                escape_label(model)
            );
        }
    }
    out.push_str("# HELP birdnet_model_inference_windows_total Windows a classifier answered and timed, since it was loaded. The count behind birdnet_model_inference_seconds_total.\n");
    out.push_str("# TYPE birdnet_model_inference_windows_total counter\n");
    for (model, h) in health {
        if h.timed_windows > 0 {
            let _ = writeln!(
                out,
                "birdnet_model_inference_windows_total{{model=\"{}\"}} {}",
                escape_label(model),
                h.timed_windows
            );
        }
    }
    out.push_str("# HELP birdnet_model_last_success_timestamp_seconds When a classifier last answered a window cleanly, in seconds since the Unix epoch. Absent for one that has not.\n");
    out.push_str("# TYPE birdnet_model_last_success_timestamp_seconds gauge\n");
    for (model, h) in health {
        if let Some(at) = h.last_success_at {
            let _ = writeln!(
                out,
                "birdnet_model_last_success_timestamp_seconds{{model=\"{}\"}} {at}",
                escape_label(model)
            );
        }
    }
}

/// The pipeline's own throughput series: files analysed, segments dropped
/// before analysis, segments shed, and the queue's depth.
fn push_pipeline_series(out: &mut String, snap: &MetricsSnapshot) {
    use std::fmt::Write as _;
    out.push_str("# HELP birdnet_files_analysed_total Audio files the detection pipeline finished analysing, per source. Flat while birdnet_audio_source_up is 1 means capture is writing files nothing analyses; rising with no detections means the model is answering nothing.\n");
    out.push_str("# TYPE birdnet_files_analysed_total counter\n");
    for (source, count) in &snap.files_analysed {
        let _ = writeln!(
            out,
            "birdnet_files_analysed_total{{source=\"{}\"}} {count}",
            escape_label(source)
        );
    }

    out.push_str("# HELP birdnet_audio_analysed_seconds_total Seconds of audio the detection pipeline finished analysing, per source. Once nothing is waiting (birdnet_analysis_queue_depth), its rate is about 1 for a source that delivers continuously; above 1 while a backlog is analysed; below 1 when audio is missing — dropouts inside segments, or segments dropped or shed (see those counters).\n");
    out.push_str("# TYPE birdnet_audio_analysed_seconds_total counter\n");
    for (source, ms) in &snap.audio_analysed_ms {
        let _ = writeln!(
            out,
            "birdnet_audio_analysed_seconds_total{{source=\"{}\"}} {}.{:03}",
            escape_label(source),
            ms / 1000,
            ms % 1000
        );
    }

    out.push_str("# HELP birdnet_segments_dropped_total Raw segments the watcher announced that were gone before the pipeline read them, per source: recorded audio never analysed. Rising means the stream directory is being drained faster than inference keeps up.\n");
    out.push_str("# TYPE birdnet_segments_dropped_total counter\n");
    for (source, count) in &snap.segments_dropped {
        let _ = writeln!(
            out,
            "birdnet_segments_dropped_total{{source=\"{}\"}} {count}",
            escape_label(source)
        );
    }

    out.push_str("# HELP birdnet_segments_shed_total Raw segments the shed policy chose not to analyse, by reason: backlog (more segments waiting than the threshold) or thermal (the board at its limit). One in two is skipped while either holds.\n");
    out.push_str("# TYPE birdnet_segments_shed_total counter\n");
    for (reason, count) in &snap.segments_shed {
        let _ = writeln!(
            out,
            "birdnet_segments_shed_total{{reason=\"{}\"}} {count}",
            escape_label(reason)
        );
    }

    if let Some(depth) = snap.analysis_queue_depth {
        out.push_str("# HELP birdnet_analysis_queue_depth Raw segments waiting for analysis after the daemon's last sweep. Climbing means inference is slower than real time; above the shed threshold the daemon analyses one segment in two.\n");
        out.push_str("# TYPE birdnet_analysis_queue_depth gauge\n");
        let _ = writeln!(out, "birdnet_analysis_queue_depth {depth}");
    }

    out.push_str("# HELP birdnet_notifications_dropped_total Notifications that never left the station, by reason: circuit_open (the destination is considered down), rate_limited (over the configured per-minute budget), send_failed (the destination refused or was unreachable), no_destination (nothing configured to send to).\n");
    out.push_str("# TYPE birdnet_notifications_dropped_total counter\n");
    for (reason, count) in &snap.notifications_dropped {
        let _ = writeln!(
            out,
            "birdnet_notifications_dropped_total{{reason=\"{}\"}} {count}",
            escape_label(reason)
        );
    }

    if let Some(n) = snap.orphaned_clips {
        out.push_str("# HELP birdnet_orphaned_clips Detections whose clip the disk no longer had, found and stamped by the last reconciliation pass.\n");
        out.push_str("# TYPE birdnet_orphaned_clips gauge\n");
        let _ = writeln!(out, "birdnet_orphaned_clips {n}");
    }

    out.push_str("# HELP birdnet_capture_restarts_total Capture processes restarted by the supervisor, per source.\n");
    out.push_str("# TYPE birdnet_capture_restarts_total counter\n");
    for (source, count) in &snap.capture_restarts {
        let _ = writeln!(
            out,
            "birdnet_capture_restarts_total{{source=\"{}\"}} {count}",
            escape_label(source)
        );
    }

    out.push_str("# HELP birdnet_capture_stalls_total Capture processes found alive but producing no segments, per source.\n");
    out.push_str("# TYPE birdnet_capture_stalls_total counter\n");
    for (source, count) in &snap.capture_stalls {
        let _ = writeln!(
            out,
            "birdnet_capture_stalls_total{{source=\"{}\"}} {count}",
            escape_label(source)
        );
    }
}

/// Pure function; safe to test by feeding in a hand-built snapshot.
#[must_use]
pub fn render_runtime_metrics(snap: &MetricsSnapshot) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(2048);

    out.push_str("# HELP birdnet_detections_total Total bird detections observed since process start, by species and integer chunk offset (s).\n");
    out.push_str("# TYPE birdnet_detections_total counter\n");
    // Sort for deterministic output (helps with snapshot testing).
    let mut sorted = snap.detections.clone();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for ((species, chunk_offset), count) in &sorted {
        let _ = writeln!(
            out,
            "birdnet_detections_total{{species=\"{}\",chunk_offset=\"{}\"}} {}",
            escape_label(species),
            chunk_offset,
            count
        );
    }

    out.push_str("# HELP birdnet_inference_duration_seconds Decode-to-prediction latency in seconds, observed once per stored detection (not per analysed audio chunk).\n");
    out.push_str("# TYPE birdnet_inference_duration_seconds histogram\n");
    render_histogram(
        &mut out,
        "birdnet_inference_duration_seconds",
        &snap.inference,
    );

    out.push_str("# HELP birdnet_db_write_duration_seconds SQLite insert latency for one detection row in seconds.\n");
    out.push_str("# TYPE birdnet_db_write_duration_seconds histogram\n");
    render_histogram(
        &mut out,
        "birdnet_db_write_duration_seconds",
        &snap.db_write,
    );

    out.push_str("# HELP birdnet_detections_dropped_total Classifications produced and then discarded, by reason.\n");
    out.push_str("# TYPE birdnet_detections_dropped_total counter\n");
    for (reason, count) in &snap.detections_dropped {
        let _ = writeln!(
            out,
            "birdnet_detections_dropped_total{{reason=\"{}\"}} {count}",
            escape_label(reason)
        );
    }

    out.push_str("# HELP birdnet_windows_masked_total Analysis windows a filter masked whole (audio listened to and not judged), by reason.\n");
    out.push_str("# TYPE birdnet_windows_masked_total counter\n");
    for (reason, count) in &snap.windows_masked {
        let _ = writeln!(
            out,
            "birdnet_windows_masked_total{{reason=\"{}\"}} {count}",
            escape_label(reason)
        );
    }

    push_pipeline_series(&mut out, snap);
    push_model_health_series(&mut out, &snap.model_health);

    push_models_series(&mut out, &snap.models);
    push_model_file_series(&mut out, &snap.model_files);

    out.push_str("# HELP birdnet_http_responses_total Web responses served, by status class.\n");
    out.push_str("# TYPE birdnet_http_responses_total counter\n");
    for (class, count) in &snap.http_responses {
        let _ = writeln!(
            out,
            "birdnet_http_responses_total{{class=\"{}\"}} {count}",
            escape_label(class)
        );
    }

    out.push_str(
        "# HELP birdnet_http_request_duration_seconds Web request handling latency in seconds.\n",
    );
    out.push_str("# TYPE birdnet_http_request_duration_seconds histogram\n");
    render_histogram(
        &mut out,
        "birdnet_http_request_duration_seconds",
        &snap.http_duration,
    );

    out.push_str("# HELP birdnet_audio_source_up Whether an audio source is currently producing samples (1 = up, 0 = down).\n");
    out.push_str("# TYPE birdnet_audio_source_up gauge\n");
    let mut sources = snap.source_up.clone();
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    for (source, up) in &sources {
        let _ = writeln!(
            out,
            "birdnet_audio_source_up{{source=\"{}\"}} {}",
            escape_label(source),
            up
        );
    }

    out.push_str("# HELP birdnet_outbound_queue_depth Store-and-forward payloads parked for replay after upload failures, by channel.\n");
    out.push_str("# TYPE birdnet_outbound_queue_depth gauge\n");
    let mut queued = snap.outbound_queue.clone();
    queued.sort_by(|a, b| a.0.cmp(&b.0));
    for (kind, depth) in &queued {
        let _ = writeln!(
            out,
            "birdnet_outbound_queue_depth{{kind=\"{}\"}} {}",
            escape_label(kind),
            depth
        );
    }

    // Emitted only once measured: an absent series reads as "unknown" in
    // Prometheus, which is the truth before the deadman task's first pass
    // (and on a station that has never detected anything).
    if let Some(synced) = snap.clock_synced {
        out.push_str(
            "# HELP birdnet_clock_synced Whether the system reports its clock as synchronised to a time source (1) or not \
             (0). Absent when nothing can answer, which is every container without systemd. A 0 here means timestamps are \
             free-running: the dates stay plausible and every count and chart looks healthy while the hours drift.\n",
        );
        out.push_str("# TYPE birdnet_clock_synced gauge\n");
        let _ = writeln!(out, "birdnet_clock_synced {}", u8::from(synced));
    }
    if let Some(up) = snap.mqtt_connected {
        out.push_str(
            "# HELP birdnet_mqtt_connected Whether the MQTT presence session is connected to the broker (1) or not (0). \
             Absent when MQTT is not configured. While this is 0 the broker has published the station's last will, so \
             Home Assistant already shows it offline.\n",
        );
        out.push_str("# TYPE birdnet_mqtt_connected gauge\n");
        let _ = writeln!(out, "birdnet_mqtt_connected {}", u8::from(up));
    }
    if let Some(secs) = snap.detection_silence_secs {
        out.push_str("# HELP birdnet_detection_silence_seconds Seconds since the most recent stored detection (end-to-end audio\u{2192}detection freshness).\n");
        out.push_str("# TYPE birdnet_detection_silence_seconds gauge\n");
        let _ = writeln!(out, "birdnet_detection_silence_seconds {secs}");
    }

    out.push_str("# HELP birdnet_watchdog_pings_total Total successful WATCHDOG=1 notifications sent to systemd since process start.\n");
    out.push_str("# TYPE birdnet_watchdog_pings_total counter\n");
    let _ = writeln!(out, "birdnet_watchdog_pings_total {}", snap.watchdog_pings);

    out.push_str("# HELP birdnet_purge_ineffective 1 when the last disk-full purge removed recordings without lowering usage — the card is full of something the purge cannot reach, and it has stopped deleting.\n");
    out.push_str("# TYPE birdnet_purge_ineffective gauge\n");
    let _ = writeln!(
        out,
        "birdnet_purge_ineffective {}",
        u8::from(snap.disk_full.purge_ineffective)
    );

    out.push_str("# HELP birdnet_purge_nothing_deletable 1 when the disk is over the purge threshold and the last disk-full purge found no recording it could delete: every one left is protected (locked, reviewed, at its species floor, excluded) or failed to delete, or there are none. The disk goes on filling.\n");
    out.push_str("# TYPE birdnet_purge_nothing_deletable gauge\n");
    let _ = writeln!(
        out,
        "birdnet_purge_nothing_deletable {}",
        u8::from(snap.disk_full.nothing_purgeable)
    );

    out.push_str("# HELP birdnet_recording_held 1 when the disk is full and the station is set to keep everything (FULL_DISK=keep): recording is on hold and no clip is being deleted.\n");
    out.push_str("# TYPE birdnet_recording_held gauge\n");
    let _ = writeln!(
        out,
        "birdnet_recording_held {}",
        u8::from(snap.disk_full.recording_held)
    );

    out.push_str("# HELP birdnet_analytics_mirror_failures_total Detections the database accepted and the DuckDB analytics copy refused since process start.\n");
    out.push_str("# TYPE birdnet_analytics_mirror_failures_total counter\n");
    let _ = writeln!(
        out,
        "birdnet_analytics_mirror_failures_total {}",
        snap.analytics_mirror_failures
    );

    out.push_str("# HELP birdnet_detection_write_failures_total Detections classified by the model and refused by the database since process start.\n");
    out.push_str("# TYPE birdnet_detection_write_failures_total counter\n");
    let _ = writeln!(
        out,
        "birdnet_detection_write_failures_total {}",
        snap.detection_write_failures
    );

    out
}

fn render_histogram(out: &mut String, name: &str, h: &HistogramSnapshot) {
    use std::fmt::Write as _;
    for (bound, count) in &h.buckets {
        let _ = writeln!(out, "{name}_bucket{{le=\"{bound}\"}} {count}");
    }
    let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {}", h.count);
    let _ = writeln!(out, "{name}_sum {}", h.sum_secs);
    let _ = writeln!(out, "{name}_count {}", h.count);
}

/// Escape `"` and `\` per Prometheus label value rules.
pub(crate) fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Seconds since the Unix epoch, `0` before it.
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Wrap the registry in `Arc` so it can live in `AppState`.
pub type SharedMetrics = Arc<MetricsRegistry>;

/// Convenience constructor.
#[must_use]
pub fn new_shared() -> SharedMetrics {
    Arc::new(MetricsRegistry::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detections_counter_increments_per_key() {
        let m = MetricsRegistry::new();
        m.inc_detection("Pica pica", 4.5);
        m.inc_detection("Pica pica", 4.5);
        m.inc_detection("Pica pica", 9.0);
        m.inc_detection("Corvus corax", 4.5);

        let snap = m.snapshot();
        let mut by_key: HashMap<(String, i64), u64> = HashMap::new();
        for (k, v) in &snap.detections {
            by_key.insert(k.clone(), *v);
        }
        assert_eq!(by_key.get(&("Pica pica".into(), 5)), Some(&2));
        assert_eq!(by_key.get(&("Pica pica".into(), 9)), Some(&1));
        assert_eq!(by_key.get(&("Corvus corax".into(), 5)), Some(&1));
    }

    #[test]
    fn histogram_observe_buckets_are_cumulative() {
        let h = Histogram::new();
        h.observe(0.003); // 3 ms — falls into the 0.005 bucket and up
        let snap = h.snapshot();
        // 0.001 bound: 0 (3 ms > 1 ms); 0.005 and onward: 1.
        for (bound, count) in &snap.buckets {
            if *bound < 0.003 {
                assert_eq!(*count, 0, "bucket {bound} should be empty");
            } else {
                assert_eq!(*count, 1, "bucket {bound} should hold 1 observation");
            }
        }
        assert_eq!(snap.count, 1);
        assert!(
            (snap.sum_secs - 0.003).abs() < 1e-5,
            "sum_secs was {}",
            snap.sum_secs
        );
    }

    #[test]
    fn histogram_handles_overflow_bucket() {
        let h = Histogram::new();
        h.observe(60.0); // 60 s — bigger than the largest bucket bound.
        let snap = h.snapshot();
        // No bucket should reflect it, but the count must.
        for (_bound, count) in &snap.buckets {
            assert_eq!(*count, 0);
        }
        assert_eq!(snap.count, 1);
        assert!((snap.sum_secs - 60.0).abs() < 1e-3);
    }

    #[test]
    fn audio_source_up_gauge_toggles() {
        let m = MetricsRegistry::new();
        m.set_source_up("alsa:plughw:1,0", true);
        m.set_source_up("rtsp://camera.local", false);

        let snap = m.snapshot();
        let mut by_source: HashMap<String, u64> = HashMap::new();
        for (k, v) in &snap.source_up {
            by_source.insert(k.clone(), *v);
        }
        assert_eq!(by_source.get("alsa:plughw:1,0"), Some(&1));
        assert_eq!(by_source.get("rtsp://camera.local"), Some(&0));

        // Toggle: bringing the RTSP source up replaces the value, not appends.
        m.set_source_up("rtsp://camera.local", true);
        let snap2 = m.snapshot();
        let mut by_source2: HashMap<String, u64> = HashMap::new();
        for (k, v) in &snap2.source_up {
            by_source2.insert(k.clone(), *v);
        }
        assert_eq!(by_source2.get("rtsp://camera.local"), Some(&1));
        assert_eq!(by_source2.len(), 2, "no duplicates expected");
    }

    #[test]
    fn watchdog_counter_accumulates() {
        let m = MetricsRegistry::new();
        for _ in 0..7 {
            m.inc_watchdog_pings();
        }
        let snap = m.snapshot();
        assert_eq!(snap.watchdog_pings, 7);
    }

    #[test]
    fn render_includes_all_metric_names() {
        let m = MetricsRegistry::new();
        m.inc_detection("Pica pica", 0.0);
        m.observe_inference_seconds(0.123);
        m.observe_db_write_seconds(0.001);
        m.set_source_up("alsa", true);
        m.inc_watchdog_pings();

        let text = render_runtime_metrics(&m.snapshot());
        assert!(text.contains("# TYPE birdnet_detections_total counter"));
        assert!(text.contains("# TYPE birdnet_inference_duration_seconds histogram"));
        assert!(text.contains("# TYPE birdnet_db_write_duration_seconds histogram"));
        assert!(text.contains("# TYPE birdnet_audio_source_up gauge"));
        assert!(text.contains("# TYPE birdnet_watchdog_pings_total counter"));
        assert!(text.contains("birdnet_inference_duration_seconds_bucket"));
        assert!(text.contains("birdnet_inference_duration_seconds_sum"));
        assert!(text.contains("birdnet_inference_duration_seconds_count"));
        assert!(text.contains("birdnet_watchdog_pings_total 1"));
    }

    /// The counter exists so a station can *say* it lost detections. A
    /// metric that is defined and never rendered is the same silence in a
    /// different place, so assert the exposition, not just the accessor.
    #[test]
    fn refused_detection_writes_are_counted_and_exported() {
        let m = MetricsRegistry::new();
        let text = render_runtime_metrics(&m.snapshot());
        assert!(
            text.contains("birdnet_detection_write_failures_total 0"),
            "a healthy station must publish the zero, or a scrape cannot tell \
             'none lost' from 'not instrumented': {text}"
        );
        m.inc_detection_write_failed();
        m.inc_detection_write_failed();
        let text = render_runtime_metrics(&m.snapshot());
        assert!(
            text.contains("# TYPE birdnet_detection_write_failures_total counter"),
            "missing TYPE line: {text}"
        );
        assert!(
            text.contains("birdnet_detection_write_failures_total 2"),
            "counter did not reach the exposition: {text}"
        );
    }

    /// `e853776`: each classifier's health reaches the exposition, and a
    /// classifier that has never answered has no last-success timestamp
    /// rather than the epoch.
    #[test]
    fn classifier_health_is_exported_per_model() {
        use birdnet_core::inference::health::{ErrorClass, FAILING_AFTER, ModelHealth};
        let m = MetricsRegistry::new();
        assert!(
            !render_runtime_metrics(&m.snapshot()).contains("birdnet_model_"),
            "nothing to say before the daemon reports"
        );
        let mut good = ModelHealth::default();
        good.succeeded(1_700_000_000);
        let mut bad = ModelHealth::default();
        for i in 0..FAILING_AFTER {
            bad.failed(ErrorClass::Inference, u64::from(i));
        }
        m.set_model_health(&[("birdnet".to_owned(), good), ("bats".to_owned(), bad)]);
        let text = render_runtime_metrics(&m.snapshot());
        for line in [
            "birdnet_model_failing{model=\"birdnet\"} 0",
            "birdnet_model_failing{model=\"bats\"} 1",
            "birdnet_model_consecutive_failed_windows{model=\"birdnet\"} 0",
            "birdnet_model_consecutive_failed_windows{model=\"bats\"} 10",
            "birdnet_model_windows_total{model=\"birdnet\"} 1",
            "birdnet_model_windows_total{model=\"bats\"} 10",
            "birdnet_model_last_success_timestamp_seconds{model=\"birdnet\"} 1700000000",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line}: {text}");
        }
        assert!(
            !text.contains("birdnet_model_last_success_timestamp_seconds{model=\"bats\"}"),
            "a model that never answered has no last success: {text}"
        );
        // A later report replaces the earlier one rather than adding to it.
        m.set_model_health(&[("birdnet".to_owned(), good)]);
        assert_eq!(m.model_health().len(), 1);
    }

    /// The time a classifier spent answering is exported as a counter pair so a
    /// scraper can compute the mean over any interval. A classifier that has
    /// answered nothing has no time, which is omitted rather than exported as
    /// zero: zero seconds over zero windows would read as a very fast model.
    #[test]
    fn the_time_a_classifier_spent_is_exported_only_once_it_has_answered() {
        use birdnet_core::inference::health::ModelHealth;
        use std::time::Duration;
        let m = MetricsRegistry::new();
        let mut timed = ModelHealth::default();
        timed.succeeded(10);
        timed.record_inference_time(Duration::from_millis(300));
        timed.record_inference_time(Duration::from_millis(500));
        let untimed = ModelHealth::default();
        m.set_model_health(&[("perch".to_owned(), timed), ("new".to_owned(), untimed)]);
        let text = render_runtime_metrics(&m.snapshot());
        for line in [
            "birdnet_model_inference_seconds_total{model=\"perch\"} 0.8",
            "birdnet_model_inference_windows_total{model=\"perch\"} 2",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line}: {text}");
        }
        assert!(
            !text.contains("birdnet_model_inference_seconds_total{model=\"new\"}")
                && !text.contains("birdnet_model_inference_windows_total{model=\"new\"}"),
            "a classifier that answered nothing has no time: {text}"
        );
    }

    #[test]
    fn what_each_classifier_is_replaces_rather_than_accumulates() {
        use birdnet_core::inference::registry::ModelInfo;
        let m = MetricsRegistry::new();
        assert!(
            m.model_info().is_empty(),
            "nothing until the daemon states it"
        );
        let info = |id: &str| ModelInfo {
            id: id.to_owned(),
            window_secs: 3.0,
            sample_rate: 48_000,
            labels: 6_522,
            bar: 0.7,
        };
        m.set_model_info(&[info("a"), info("b")]);
        assert_eq!(m.model_info().len(), 2);
        m.set_model_info(&[info("a")]);
        assert_eq!(m.model_info().len(), 1);
    }

    #[test]
    fn render_escapes_quote_in_label() {
        let m = MetricsRegistry::new();
        // Adversarial label content. We must never produce malformed
        // Prometheus output that breaks the scraper's parser.
        // The input has a literal `"` and a literal `\`.
        m.set_source_up("weird\"source\\with", true);
        let text = render_runtime_metrics(&m.snapshot());
        // Quotes become \" and backslashes become \\, and the rendered
        // line wraps the value in `source="..."`. Pin the exact byte
        // sequence the scraper has to parse.
        assert!(
            text.contains(r#"source="weird\"source\\with""#),
            "rendered output did not escape correctly:\n{text}"
        );
    }

    #[test]
    fn render_is_sorted_for_determinism() {
        let m = MetricsRegistry::new();
        m.inc_detection("Zonotrichia leucophrys", 0.0);
        m.inc_detection("Apus apus", 0.0);
        m.inc_detection("Buteo buteo", 0.0);

        let text = render_runtime_metrics(&m.snapshot());
        // Alphabetical species order in the text output:
        let apus = text.find("Apus apus").expect("Apus apus present");
        let buteo = text.find("Buteo buteo").expect("Buteo buteo present");
        let zono = text.find("Zonotrichia").expect("Zonotrichia present");
        assert!(
            apus < buteo && buteo < zono,
            "rendered output not sorted alphabetically"
        );
    }
}

/// Axum middleware that records one `birdnet_http_responses_total` sample and
/// one latency observation per request.
///
/// Placed at the very outside of the stack, so it times what the client
/// experienced — including compression and the header layer — rather than the
/// handler alone. The status *class* is recorded rather than the code: a
/// per-code counter grows a label value for every 4xx a scanner can provoke,
/// and the operational question ("is anything 5xx-ing?") does not need more.
pub async fn http_metrics_middleware(
    metrics: SharedMetrics,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    metrics.observe_http(response.status().as_u16(), start.elapsed().as_secs_f64());
    response
}

// ── the metrics added for operational visibility ────────────────────────
//
// A station you cannot SSH into is diagnosed from its metrics or not at all.
// Each family below answers a question the parity audit raised and the
// existing surface could not: is the occurrence filter running, is a source
// flapping, is the pipeline discarding what it hears, is the web layer
// erroring.
#[cfg(test)]
mod operational_metrics_tests {
    use super::*;

    fn rendered(f: impl FnOnce(&MetricsRegistry)) -> String {
        let reg = MetricsRegistry::new();
        f(&reg);
        render_runtime_metrics(&reg.snapshot())
    }

    /// The one series that separates "the model is answering nothing" from
    /// "the pipeline is not running".
    ///
    /// Nothing counted throughput. `birdnet_inference_duration_seconds` is
    /// observed once per **stored detection** — its own `# HELP` says so — so
    /// on a station with a wrong label file, a wrong sample rate, or a model
    /// swapped by a bad update, every latency series is flat and empty, exactly
    /// as it is on a station where inference never started. The four
    /// drop-reason labels do not separate them either: all of them live
    /// downstream of a prediction the model actually made.
    ///
    /// Observed failing before `inc_file_analysed` existed: the exposition
    /// carried no `birdnet_files_analysed_total` at all.
    #[test]
    fn analysed_files_are_counted_per_source() {
        let out = rendered(|r| {
            r.inc_file_analysed("local");
            r.inc_file_analysed("local");
            r.inc_file_analysed("cam1");
        });
        assert!(
            out.contains(r#"birdnet_files_analysed_total{source="local"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_files_analysed_total{source="cam1"} 1"#),
            "the per-source label is what tells a dead microphone on a \
             two-source station from a dead pipeline: {out}"
        );
        assert!(
            out.contains("# TYPE birdnet_files_analysed_total counter"),
            "it must be declared, and as a counter: {out}"
        );
    }

    /// The data rate (upstream birdnet-go `712d4c2`): seconds of audio per
    /// source, which a file count cannot give — a camera that drops packets
    /// still yields one segment file, holding less audio. Observed failing
    /// with `add_audio_analysed` an empty body: no series at all.
    #[test]
    fn analysed_audio_is_counted_in_seconds_per_source() {
        let out = rendered(|r| {
            r.add_audio_analysed("local", 15.0);
            r.add_audio_analysed("local", 14.25);
            r.add_audio_analysed("cam1", 3.0005);
            // Not a length: counted as nothing rather than as a wild value.
            r.add_audio_analysed("cam1", f64::NAN);
            r.add_audio_analysed("cam1", -1.0);
        });
        assert!(
            out.contains(r#"birdnet_audio_analysed_seconds_total{source="local"} 29.250"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_audio_analysed_seconds_total{source="cam1"} 3.001"#),
            "{out}"
        );
        assert!(
            out.contains("# TYPE birdnet_audio_analysed_seconds_total counter"),
            "{out}"
        );
    }

    /// The only number that says an alert never left the station.
    ///
    /// Both guards on the outbound path drop a notification *without failing* —
    /// the circuit breaker when a destination is considered down, the token
    /// bucket when it is over its per-minute budget — and the send then
    /// returned `Ok(())`. The skip was logged at `debug`, which the default
    /// filter discards, so a station could drop every alert it ever tried to
    /// send and leave no evidence at all. See
    /// `docs/UNATTENDED_DEPLOYMENT_AUDIT.md` (OB-5).
    ///
    /// Observed failing before `inc_notification_dropped` existed: the
    /// exposition carried no `birdnet_notifications_dropped_total` at all.
    #[test]
    fn dropped_notifications_are_counted_by_reason() {
        let out = rendered(|r| {
            r.inc_notification_dropped("rate_limited");
            r.inc_notification_dropped("rate_limited");
            r.inc_notification_dropped("circuit_open");
        });
        assert!(
            out.contains(r#"birdnet_notifications_dropped_total{reason="rate_limited"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_notifications_dropped_total{reason="circuit_open"} 1"#),
            "the reason is what separates a dead destination from a busy one, \
             which need different fixes: {out}"
        );
        assert!(
            out.contains("# TYPE birdnet_notifications_dropped_total counter"),
            "it must be declared, and as a counter: {out}"
        );
    }

    /// The counterpart: a station whose notifications all go out must not
    /// publish a series suggesting otherwise.
    #[test]
    fn a_station_that_drops_nothing_publishes_no_drop_samples() {
        let out = rendered(|_| {});
        assert!(
            out.contains("# TYPE birdnet_notifications_dropped_total counter"),
            "the family is always declared, so a dashboard panel is never \
             missing: {out}"
        );
        assert!(
            !out.contains("birdnet_notifications_dropped_total{"),
            "no drops means no samples, not a zero for every reason: {out}"
        );
    }

    /// The discrimination this series exists for, spelled out as an assertion:
    /// a station that analysed files and detected nothing must look different
    /// from one that analysed nothing.
    #[test]
    fn analysing_without_detecting_looks_different_from_not_analysing() {
        let answering_nothing = rendered(|r| {
            for _ in 0..10 {
                r.inc_file_analysed("local");
            }
            // No detections, no latency observations — the exact state a wrong
            // label file produces.
        });
        let not_running = rendered(|_| {});

        assert!(answering_nothing.contains(r#"birdnet_files_analysed_total{source="local"} 10"#));
        assert!(
            !not_running.contains("birdnet_files_analysed_total{"),
            "a station that analysed nothing must emit no sample for it"
        );
        assert_ne!(
            answering_nothing, not_running,
            "these two stations were indistinguishable from every series the \
             exposition carried, which is the whole finding"
        );
    }

    /// The gauge that would have made the inert-filter defect visible. Zero is
    /// the dangerous state, so it must be emitted rather than omitted — a
    /// missing series and a healthy one look identical on a dashboard.
    #[test]
    fn the_occurrence_gauge_is_emitted_even_when_the_filter_is_off() {
        let out = rendered(|_| {});
        assert!(
            out.contains("birdnet_occurrence_filter_active 0"),
            "the off state must be reported, not omitted: {out}"
        );
        assert!(
            !out.contains("birdnet_occurrence_candidates"),
            "a candidate count must not be invented before the filter has run"
        );
    }

    #[test]
    fn the_occurrence_gauge_reports_a_running_filter_and_its_candidates() {
        let out = rendered(|r| {
            r.set_occurrence_filter(birdnet_core::inference::model_state::FilterState::Active {
                candidates: Some(287),
            });
        });
        assert!(out.contains("birdnet_occurrence_filter_active 1"), "{out}");
        assert!(out.contains("birdnet_occurrence_candidates 287"), "{out}");
        assert!(
            out.contains("birdnet_occurrence_filter_state{state=\"active\"} 1"),
            "{out}"
        );
    }

    /// MDL-04: a filter that is configured and cannot filter — here, no
    /// coordinates — is not "on". It was reported active with every species as
    /// a candidate, and Station Health said "on · admitting".
    #[test]
    fn a_filter_that_is_off_is_not_active_whatever_it_was_told_to_admit() {
        use birdnet_core::inference::model_state::{FilterOff, FilterState};
        let r = MetricsRegistry::new();
        r.set_occurrence_filter(FilterState::off(FilterOff::NoCoordinates, "no position"));
        assert_eq!(
            r.occurrence_filter(),
            OccurrenceFilterState {
                active: false,
                candidates: None
            }
        );
        let out = render_runtime_metrics(&r.snapshot());
        assert!(out.contains("birdnet_occurrence_filter_active 0"), "{out}");
        assert!(
            out.contains("birdnet_occurrence_filter_state{state=\"off_no_coordinates\"} 1"),
            "{out}"
        );
        assert!(
            out.contains("birdnet_occurrence_filter_state{state=\"active\"} 0"),
            "{out}"
        );
        assert!(!out.contains("birdnet_occurrence_candidates"), "{out}");
    }

    /// Each classifier gets one series per state, the one it is in at 1, and
    /// the detector its own; a station that has published no classifier yet
    /// has no classifier family, rather than an invented one.
    /// Each model file the station looks after is one state-set family
    /// (MDL-10), replaced in place when its state changes; none is exported
    /// before the first is published.
    #[test]
    fn each_model_file_is_exported_in_the_state_it_is_in() {
        use birdnet_core::inference::model_state::{ModelFile, RepairState};
        let r = MetricsRegistry::new();
        assert!(!render_runtime_metrics(&r.snapshot()).contains("birdnet_model_file_state"));
        let file = |repair| ModelFile {
            catalogue_id: "birdnet-v24".to_owned(),
            path: "/m/birdnet-v24.onnx".into(),
            repair,
        };
        r.set_model_file(file(RepairState::Checking));
        r.set_model_file(file(RepairState::WillRetry {
            at_unix: 9,
            attempts: 1,
            why: "refused".to_owned(),
        }));
        assert_eq!(
            r.model_files().len(),
            1,
            "the same path is replaced, not added"
        );
        let out = render_runtime_metrics(&r.snapshot());
        assert!(
            out.contains(
                "birdnet_model_file_state{file=\"birdnet-v24.onnx\",state=\"will_retry\"} 1"
            ),
            "{out}"
        );
        assert!(
            out.contains(
                "birdnet_model_file_state{file=\"birdnet-v24.onnx\",state=\"checking\"} 0"
            ),
            "{out}"
        );
    }

    #[test]
    fn the_models_state_is_exported_one_series_per_state() {
        use birdnet_core::inference::model_state::{
            ClassifierFault, ClassifierState, ClassifierStatus, DetectorState,
        };
        let r = MetricsRegistry::new();
        let out = render_runtime_metrics(&r.snapshot());
        assert!(
            out.contains("birdnet_detector_state{state=\"starting\"} 1"),
            "{out}"
        );
        assert!(!out.contains("birdnet_classifier_state"), "{out}");

        r.set_classifier_states(vec![
            ClassifierState {
                id: "birdnet".to_owned(),
                primary: true,
                model_path: "/m/v24.onnx".into(),
                labels_path: Some("/m/v24.txt".into()),
                status: ClassifierStatus::Judging {
                    sources: Vec::new(),
                    unrouted_sources: true,
                },
            },
            ClassifierState {
                id: "perch".to_owned(),
                primary: false,
                model_path: "/m/perch.onnx".into(),
                labels_path: Some("/m/perch.csv".into()),
                status: ClassifierStatus::left_out(ClassifierFault::WillNotLoad, "truncated"),
            },
        ]);
        r.set_detector_state(DetectorState::Running);
        let out = render_runtime_metrics(&r.snapshot());
        assert!(
            out.contains("birdnet_detector_state{state=\"running\"} 1"),
            "{out}"
        );
        assert!(
            out.contains("birdnet_detector_state{state=\"starting\"} 0"),
            "{out}"
        );
        assert!(
            out.contains(
                "birdnet_classifier_state{classifier=\"perch\",state=\"left_out_will_not_load\"} 1"
            ),
            "{out}"
        );
        assert!(
            out.contains("birdnet_classifier_state{classifier=\"perch\",state=\"judging\"} 0"),
            "{out}"
        );
        assert!(
            out.contains("birdnet_classifier_state{classifier=\"birdnet\",state=\"judging\"} 1"),
            "{out}"
        );
        let per_classifier = out
            .lines()
            .filter(|l| l.starts_with("birdnet_classifier_state{classifier=\"perch\""))
            .count();
        assert_eq!(
            per_classifier,
            birdnet_core::inference::model_state::ClassifierStatus::CODES.len()
        );
    }

    /// "Detecting nothing" and "discarding everything" look identical from
    /// outside. The reason label is what separates them.
    #[test]
    fn dropped_detections_are_counted_by_reason() {
        let out = rendered(|r| {
            r.inc_detection_dropped("confidence");
            r.inc_detection_dropped("confidence");
            r.inc_detection_dropped("quarantine");
        });
        assert!(
            out.contains(r#"birdnet_detections_dropped_total{reason="confidence"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_detections_dropped_total{reason="quarantine"} 1"#),
            "{out}"
        );
    }

    /// ACC-02: the daemon's filter discards reach the exposition by reason,
    /// and masked windows have their own series; a zero adds no series.
    #[test]
    fn filter_discards_and_masked_windows_are_exported() {
        let out = rendered(|r| {
            r.add_detections_dropped("privacy", 3);
            r.add_detections_dropped("privacy", 2);
            r.add_detections_dropped("noise", 0);
            r.add_windows_masked("privacy", 4);
        });
        assert!(
            out.contains(r#"birdnet_detections_dropped_total{reason="privacy"} 5"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_windows_masked_total{reason="privacy"} 4"#),
            "{out}"
        );
        assert!(!out.contains(r#"reason="noise""#), "{out}");
    }

    /// A restart and a stall are different diagnoses: a stall that keeps
    /// recurring on one source is a hardware fault, a restart that does not is
    /// a flaky link. Counting them together would lose that.
    #[test]
    fn capture_restarts_and_stalls_are_separate_series() {
        let out = rendered(|r| {
            r.inc_capture_restart("mic");
            r.inc_capture_restart("mic");
            r.inc_capture_stall("mic");
        });
        assert!(
            out.contains(r#"birdnet_capture_restarts_total{source="mic"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_capture_stalls_total{source="mic"} 1"#),
            "{out}"
        );
    }

    /// Status *class*, not status code: a per-code counter grows a label value
    /// for every 4xx a scanner can provoke, and "is anything 5xx-ing?" does
    /// not need more.
    #[test]
    fn http_responses_are_bucketed_by_class_not_code() {
        let out = rendered(|r| {
            r.observe_http(200, 0.01);
            r.observe_http(204, 0.01);
            r.observe_http(404, 0.01);
            r.observe_http(503, 0.2);
        });
        assert!(
            out.contains(r#"birdnet_http_responses_total{class="2xx"} 2"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_http_responses_total{class="4xx"} 1"#),
            "{out}"
        );
        assert!(
            out.contains(r#"birdnet_http_responses_total{class="5xx"} 1"#),
            "{out}"
        );
        assert!(
            !out.contains(r#"class="204""#),
            "individual codes must not become label values: {out}"
        );
        assert!(
            out.contains("birdnet_http_request_duration_seconds_count 4"),
            "every response must also feed the latency histogram: {out}"
        );
    }

    /// The whole rendered document has to stay valid exposition. A stray
    /// newline or an unescaped label would make Prometheus drop the *scrape*,
    /// not just the offending series.
    #[test]
    fn the_rendered_document_is_well_formed() {
        let out = rendered(|r| {
            r.set_occurrence_filter(birdnet_core::inference::model_state::FilterState::Active {
                candidates: Some(1),
            });
            r.set_classifier_states(vec![
                birdnet_core::inference::model_state::ClassifierState {
                    id: r#"we"ird"#.to_owned(),
                    primary: true,
                    model_path: "/m/a.onnx".into(),
                    labels_path: None,
                    status: birdnet_core::inference::model_state::ClassifierStatus::Unrouted {
                        primary: "birdnet".to_owned(),
                    },
                },
            ]);
            r.inc_detection_dropped("quality");
            r.inc_capture_restart(r#"weird"source"#);
            r.observe_http(200, 0.01);
        });
        for line in out.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let (_, value) = line
                .rsplit_once(' ')
                .unwrap_or_else(|| panic!("no value on `{line}`"));
            assert!(
                value.parse::<f64>().is_ok(),
                "`{line}` does not end in a number"
            );
        }
        assert!(
            out.contains(r#"source="weird\"source""#),
            "label values must be escaped: {out}"
        );
    }

    /// OP-7: a refused mirror write is counted, timed and exposed.
    #[test]
    fn a_refused_mirror_write_is_counted_and_exposed() {
        let r = MetricsRegistry::new();
        assert_eq!(r.analytics_mirror_failures(), 0);
        assert_eq!(r.analytics_mirror_last_failure(), None);
        r.inc_analytics_mirror_failed("Constraint Error: NOT NULL");
        r.inc_analytics_mirror_failed("IO Error");
        assert_eq!(r.analytics_mirror_failures(), 2);
        let (at, error) = r.analytics_mirror_last_failure().expect("timed");
        assert!(at > 0);
        assert_eq!(error, "IO Error");
        let out = render_runtime_metrics(&r.snapshot());
        assert!(
            out.contains("birdnet_analytics_mirror_failures_total 2"),
            "{out}"
        );
    }

    /// N25-01: the flag the disk manager holds reaches the exposition, as `0`
    /// until it is raised (a missing series reads like a healthy one), and it
    /// is its own series, not the PR-7 latch.
    #[test]
    fn a_purge_that_finds_nothing_deletable_is_exported() {
        // Whole sample lines: the HELP text itself begins "1 when", so a
        // substring match on "… 1" is satisfied by the HELP line alone.
        let sample = |out: &str, line: &str| out.lines().any(|l| l == line);
        let out = rendered(|_| {});
        assert!(sample(&out, "birdnet_purge_nothing_deletable 0"), "{out}");
        let out = rendered(|r| {
            r.nothing_purgeable_flag()
                .store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(r.nothing_purgeable());
        });
        assert!(sample(&out, "birdnet_purge_nothing_deletable 1"), "{out}");
        assert!(sample(&out, "birdnet_purge_ineffective 0"), "{out}");
    }
}
