//! A tiny in-memory TTL cache for rendered analytics fragments.
//!
//! The heavy analytics partials (activity streamgraph, dawn chorus, seasonal
//! phenology, co-occurrence, …) run multi-second aggregate queries that
//! previously executed on *every* page visit and *every* HTMX poll. On a
//! Raspberry Pi 4 that made jumping between analytics pages feel sluggish.
//!
//! This cache stores each fragment's rendered HTML for a short time-to-live so
//! repeat visits, the periodic `every Ns` polls, and the background pre-warmer
//! all serve an instant copy. Live, must-be-fresh surfaces (the dashboard
//! detection feed and the stat tiles) are deliberately *not* routed through it.
//!
//! Dependency-free by design (no `moka`/`lru`): a `Mutex<HashMap>` with a coarse
//! TTL and a bounded entry count is more than enough for the handful of distinct
//! analytics keys a single station serves, and it keeps the air-gapped Pi build
//! lean.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default freshness window for cached analytics fragments.
///
/// The cached surfaces are multi-day / yearly aggregates (streamgraph, hour ×
/// day grid, phenology, co-occurrence) and load once per page visit rather than
/// polling, so several minutes of staleness is imperceptible. A longer window
/// also lets the background pre-warmer keep them hot by running the heavy
/// queries only every few minutes — gentle on a Raspberry Pi — instead of
/// continuously. Periodically-polled live surfaces (the dashboard feed and stat
/// tiles) are deliberately not routed through this cache.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Hard cap on distinct cached keys so an adversarial spread of query
/// parameters (e.g. `?days=`) cannot grow the map without bound.
const MAX_ENTRIES: usize = 256;

/// One cached fragment plus the instant it was stored.
#[derive(Debug)]
struct Entry {
    value: String,
    stored: Instant,
}

/// In-memory time-to-live cache keyed by an opaque fragment key.
#[derive(Debug)]
pub struct AnalyticsCache {
    entries: Mutex<HashMap<String, Entry>>,
    ttl: Duration,
}

impl AnalyticsCache {
    /// Create a cache with the given freshness window.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// The freshness window applied to stored entries.
    #[must_use]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Fetch a still-fresh value for `key`, or `None` if it is absent or has
    /// expired. An expired hit is dropped opportunistically on the way out.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        let now = Instant::now();
        let mut map = self.lock();
        match map.get(key) {
            Some(entry) if now.duration_since(entry.stored) <= self.ttl => {
                Some(entry.value.clone())
            }
            Some(_) => {
                map.remove(key);
                None
            }
            None => None,
        }
    }

    /// Store `value` under `key`, replacing any existing entry. When the cache
    /// is at capacity and the key is new, one entry is evicted first —
    /// preferring an already-expired entry, otherwise the oldest.
    pub fn put(&self, key: String, value: String) {
        let now = Instant::now();
        let mut map = self.lock();
        if map.len() >= MAX_ENTRIES && !map.contains_key(&key) {
            if let Some(victim) = Self::eviction_candidate(&map, now, self.ttl) {
                map.remove(&victim);
            }
        }
        map.insert(key, Entry { value, stored: now });
    }

    /// Get a cached fragment or compute, store, and return it.
    ///
    /// `compute` runs only on a miss. It returns `Some(html)` to cache a
    /// successful render or `None` to skip caching (e.g. a query error), in
    /// which case the returned string is passed straight through uncached.
    pub fn get_or_store<F>(&self, key: &str, compute: F) -> String
    where
        F: FnOnce() -> (Option<String>, String),
    {
        if let Some(hit) = self.get(key) {
            return hit;
        }
        let (cacheable, body) = compute();
        if let Some(value) = cacheable {
            self.put(key.to_string(), value);
        }
        body
    }

    /// Number of entries currently held (fresh or stale). Diagnostic aid.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lock the inner map, recovering from a poisoned mutex rather than
    /// propagating the panic — a cache is best-effort and never worth crashing
    /// a request over.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Pick a key to evict: an expired entry if one exists, else the oldest.
    fn eviction_candidate(
        map: &HashMap<String, Entry>,
        now: Instant,
        ttl: Duration,
    ) -> Option<String> {
        map.iter()
            .find(|(_, e)| now.duration_since(e.stored) > ttl)
            .map(|(k, _)| k.clone())
            .or_else(|| {
                map.iter()
                    .min_by_key(|(_, e)| e.stored)
                    .map(|(k, _)| k.clone())
            })
    }
}

impl Default for AnalyticsCache {
    fn default() -> Self {
        Self::new(DEFAULT_TTL)
    }
}

/// Serve a heavy-analytics fragment from cache, computing it on the blocking
/// pool on a miss.
///
/// `compute` runs only on a miss and returns `Some(html)` to cache and serve a
/// successful render, or `None` to fall through to `fallback` *without* caching
/// (e.g. on a query error — we never want to pin an error fragment for the TTL).
/// This is the single integration point the analytics partials and the
/// background pre-warmer share, so both populate the same store under the same
/// keys.
pub async fn cached_fragment<F>(
    state: &crate::state::AppState,
    key: String,
    fallback: &'static str,
    compute: F,
) -> String
where
    F: FnOnce(&crate::state::AppState) -> Option<String> + Send + 'static,
{
    if let Some(hit) = state.analytics_cache().get(&key) {
        return hit;
    }
    let state_for_blocking = state.clone();
    let computed = tokio::task::spawn_blocking(move || compute(&state_for_blocking))
        .await
        .ok()
        .flatten();
    match computed {
        Some(html) => {
            state.analytics_cache().put(key, html.clone());
            html
        }
        None => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miss_returns_none() {
        let cache = AnalyticsCache::new(DEFAULT_TTL);
        assert_eq!(cache.get("absent"), None);
        assert!(cache.is_empty());
    }

    #[test]
    fn put_then_get_round_trips() {
        let cache = AnalyticsCache::new(DEFAULT_TTL);
        cache.put("k".to_string(), "<svg/>".to_string());
        assert_eq!(cache.get("k"), Some("<svg/>".to_string()));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn expired_entry_is_not_returned() {
        let cache = AnalyticsCache::new(Duration::from_millis(5));
        cache.put("k".to_string(), "v".to_string());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(cache.get("k"), None, "stale entry must miss");
    }

    #[test]
    fn get_or_store_computes_then_serves_cached() {
        let cache = AnalyticsCache::new(DEFAULT_TTL);
        let first = cache.get_or_store("k", || (Some("fresh".to_string()), "fresh".to_string()));
        assert_eq!(first, "fresh");
        // Second call must NOT invoke compute (it would panic if it did).
        let second = cache.get_or_store("k", || panic!("should have hit the cache"));
        assert_eq!(second, "fresh");
    }

    #[test]
    fn get_or_store_does_not_cache_when_compute_opts_out() {
        let cache = AnalyticsCache::new(DEFAULT_TTL);
        let body = cache.get_or_store("k", || (None, "error".to_string()));
        assert_eq!(body, "error");
        assert_eq!(cache.get("k"), None, "opt-out result must not be cached");
    }

    #[test]
    fn capacity_is_bounded() {
        let cache = AnalyticsCache::new(DEFAULT_TTL);
        for i in 0..(MAX_ENTRIES + 50) {
            cache.put(format!("k{i}"), "v".to_string());
        }
        assert!(
            cache.len() <= MAX_ENTRIES,
            "cache must not exceed {MAX_ENTRIES} entries, got {}",
            cache.len()
        );
    }
}
