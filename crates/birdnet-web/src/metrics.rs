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
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// This comment used to name `quality` and `occurrence` instead, and
    /// explain what a spike in each would mean. Neither label is ever emitted
    /// in production — both appear only in this file's own tests — so both
    /// readings it taught were unavailable. See `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`.
    detections_dropped: RwLock<HashMap<String, AtomicU64>>,
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
    capture_restarts: RwLock<HashMap<String, AtomicU64>>,
    /// Capture processes found alive but producing no segments, per source.
    /// Distinct from a restart: this is the *diagnosis*, and a stall that keeps
    /// recurring on one source is a hardware fault rather than a flaky link.
    capture_stalls: RwLock<HashMap<String, AtomicU64>>,
    /// Whether the species occurrence filter is actually running (1) or the
    /// station is admitting every species the classifier knows (0).
    ///
    /// The one number that would have made the inert-filter defect visible
    /// from a dashboard rather than from reading the code.
    occurrence_filter_active: AtomicU64,
    /// How many species the occurrence filter currently admits. `u64::MAX`
    /// until the filter has run once.
    occurrence_candidates: AtomicU64,
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
            detections_dropped: RwLock::new(HashMap::new()),
            files_analysed: RwLock::new(HashMap::new()),
            capture_restarts: RwLock::new(HashMap::new()),
            capture_stalls: RwLock::new(HashMap::new()),
            occurrence_filter_active: AtomicU64::new(0),
            occurrence_candidates: AtomicU64::new(u64::MAX),
            http_responses: RwLock::new(HashMap::new()),
            http_duration: Histogram::new(),
        }
    }

    /// Bump a labelled counter map, taking the write lock only to insert.
    fn bump(map: &RwLock<HashMap<String, AtomicU64>>, key: &str) {
        if let Ok(m) = map.read()
            && let Some(c) = m.get(key)
        {
            c.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if let Ok(mut m) = map.write() {
            m.entry(key.to_owned())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
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

    /// Record that a capture process was restarted.
    pub fn inc_capture_restart(&self, source: &str) {
        Self::bump(&self.capture_restarts, source);
    }

    /// Record that a capture process was found alive but silently stalled.
    pub fn inc_capture_stall(&self, source: &str) {
        Self::bump(&self.capture_stalls, source);
    }

    /// Publish whether occurrence filtering is running, and over how many
    /// species. `None` candidates means "the filter has not run yet".
    pub fn set_occurrence_filter(&self, active: bool, candidates: Option<u64>) {
        self.occurrence_filter_active
            .store(u64::from(active), Ordering::Relaxed);
        self.occurrence_candidates
            .store(candidates.unwrap_or(u64::MAX), Ordering::Relaxed);
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
            detections_dropped: Self::read_map(&self.detections_dropped),
            files_analysed: Self::read_map(&self.files_analysed),
            capture_restarts: Self::read_map(&self.capture_restarts),
            capture_stalls: Self::read_map(&self.capture_stalls),
            occurrence_filter_active: self.occurrence_filter_active.load(Ordering::Relaxed) == 1,
            occurrence_candidates: match self.occurrence_candidates.load(Ordering::Relaxed) {
                u64::MAX => None,
                n => Some(n),
            },
            http_responses: Self::read_map(&self.http_responses),
            http_duration: self.http_duration.snapshot(),
            watchdog_pings: self.watchdog_pings_total.load(Ordering::Relaxed),
            detection_write_failures: self.detection_write_failures_total.load(Ordering::Relaxed),
        }
    }
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
    /// Discarded classifications by reason.
    pub detections_dropped: Vec<(String, u64)>,
    /// Audio files the pipeline finished analysing, per source.
    pub files_analysed: Vec<(String, u64)>,
    /// Capture restarts per source.
    pub capture_restarts: Vec<(String, u64)>,
    /// Silent stalls diagnosed per source.
    pub capture_stalls: Vec<(String, u64)>,
    /// Whether occurrence filtering is running.
    pub occurrence_filter_active: bool,
    /// Species the occurrence filter admits (`None` = not yet run).
    pub occurrence_candidates: Option<u64>,
    /// HTTP responses by status class.
    pub http_responses: Vec<(String, u64)>,
    /// Web request latency.
    pub http_duration: HistogramSnapshot,
}

/// Render the runtime metrics as Prometheus text 0.0.4.
///
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

    out.push_str("# HELP birdnet_files_analysed_total Audio files the detection pipeline finished analysing, per source. Flat while birdnet_audio_source_up is 1 means capture is writing files nothing analyses; rising with no detections means the model is answering nothing.\n");
    out.push_str("# TYPE birdnet_files_analysed_total counter\n");
    for (source, count) in &snap.files_analysed {
        let _ = writeln!(
            out,
            "birdnet_files_analysed_total{{source=\"{}\"}} {count}",
            escape_label(source)
        );
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

    out.push_str("# HELP birdnet_occurrence_filter_active Whether species occurrence filtering is running (1) or every species the classifier knows is admitted (0).\n");
    out.push_str("# TYPE birdnet_occurrence_filter_active gauge\n");
    let _ = writeln!(
        out,
        "birdnet_occurrence_filter_active {}",
        u8::from(snap.occurrence_filter_active)
    );

    if let Some(n) = snap.occurrence_candidates {
        out.push_str("# HELP birdnet_occurrence_candidates Species the occurrence filter currently admits.\n");
        out.push_str("# TYPE birdnet_occurrence_candidates gauge\n");
        let _ = writeln!(out, "birdnet_occurrence_candidates {n}");
    }

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
    if let Some(secs) = snap.detection_silence_secs {
        out.push_str("# HELP birdnet_detection_silence_seconds Seconds since the most recent stored detection (end-to-end audio\u{2192}detection freshness).\n");
        out.push_str("# TYPE birdnet_detection_silence_seconds gauge\n");
        let _ = writeln!(out, "birdnet_detection_silence_seconds {secs}");
    }

    out.push_str("# HELP birdnet_watchdog_pings_total Total successful WATCHDOG=1 notifications sent to systemd since process start.\n");
    out.push_str("# TYPE birdnet_watchdog_pings_total counter\n");
    let _ = writeln!(out, "birdnet_watchdog_pings_total {}", snap.watchdog_pings);

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
        let out = rendered(|r| r.set_occurrence_filter(true, Some(287)));
        assert!(out.contains("birdnet_occurrence_filter_active 1"), "{out}");
        assert!(out.contains("birdnet_occurrence_candidates 287"), "{out}");
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
            r.set_occurrence_filter(true, Some(1));
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
}
