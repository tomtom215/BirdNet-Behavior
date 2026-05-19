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
/// Chosen to bracket the observed per-chunk inference latency on a Pi 5
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
    pub buckets: Vec<(f64, u64)>,
    pub sum_secs: f64,
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
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detections: RwLock::new(HashMap::new()),
            inference_duration: Histogram::new(),
            db_write_duration: Histogram::new(),
            audio_source_up: RwLock::new(HashMap::new()),
            watchdog_pings_total: AtomicU64::new(0),
        }
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

    /// Bump the watchdog ping counter.
    pub fn inc_watchdog_pings(&self) {
        self.watchdog_pings_total.fetch_add(1, Ordering::Relaxed);
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
        MetricsSnapshot {
            detections,
            inference: self.inference_duration.snapshot(),
            db_write: self.db_write_duration.snapshot(),
            source_up,
            watchdog_pings: self.watchdog_pings_total.load(Ordering::Relaxed),
        }
    }
}

/// Plain snapshot of every metric, returned to the renderer.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub detections: Vec<((String, i64), u64)>,
    pub inference: HistogramSnapshot,
    pub db_write: HistogramSnapshot,
    pub source_up: Vec<(String, u64)>,
    pub watchdog_pings: u64,
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

    out.push_str("# HELP birdnet_inference_duration_seconds Per-chunk inference latency (decode-to-prediction) in seconds.\n");
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

    out.push_str("# HELP birdnet_watchdog_pings_total Total successful WATCHDOG=1 notifications sent to systemd since process start.\n");
    out.push_str("# TYPE birdnet_watchdog_pings_total counter\n");
    let _ = writeln!(out, "birdnet_watchdog_pings_total {}", snap.watchdog_pings);

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
fn escape_label(s: &str) -> String {
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
