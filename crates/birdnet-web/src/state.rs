//! Shared application state for the web server.
//!
//! Holds the database connection, WebSocket broadcast channel, and configuration,
//! shared across all request handlers via axum's State extractor.

#[cfg(feature = "analytics")]
use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_core::i18n::I18nManager;

use crate::analytics_cache::AnalyticsCache;
use birdnet_integrations::species_images::ImageCache;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::watch;

use crate::metrics::{self, SharedMetrics};
use crate::routes::admin::logs::LogBroadcaster;
use crate::routes::spectrogram_ws::SpectrogramBroadcast;
use crate::routes::websocket::DetectionBroadcast;

/// Default WebSocket broadcast channel capacity.
///
/// Sized for the detection stream, whose events are small JSON objects (a
/// species, confidence and timestamps — a few hundred bytes), so a 256-deep
/// backlog for a briefly-lagging client is only tens of KB.
const DEFAULT_BROADCAST_CAPACITY: usize = 256;

/// Broadcast capacity for the live spectrogram stream.
///
/// Each frame carries the full mel matrix (up to 128 × 256 floats), so a frame
/// serialises to a few hundred KB — three orders of magnitude larger than a
/// detection event. At the detection capacity (256) a single lagging client
/// could pin ~75 MB of frames in the ring; on a 2–4 GB Pi that is a real
/// back-pressure hazard. A live view only needs the most recent frames (new
/// recordings arrive seconds apart, and a client further behind than this
/// should jump to the latest — the receivers already drop on `Lagged`), so a
/// shallow ring is both correct and bounds worst-case retention to a few MB.
const SPECTROGRAM_BROADCAST_CAPACITY: usize = 16;

/// Shared application state.
#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

/// Inner state (wrapped in Arc for sharing).
#[derive(Debug)]
struct AppStateInner {
    /// `SQLite` connection protected by a mutex for thread safety.
    db: Mutex<Connection>,
    /// Path to the `SQLite` database file (for diagnostics).
    db_path: PathBuf,
    /// Directory containing extracted audio recording clips.
    recording_dir: PathBuf,
    /// `DuckDB` analytics database (file-backed, for behavioral queries).
    #[cfg(feature = "analytics")]
    analytics_db: Option<Mutex<AnalyticsDb>>,
    /// Species image cache (Wikipedia/Wikimedia Commons).
    image_cache: Option<Arc<ImageCache>>,
    /// Broadcast channel for live detection WebSocket streaming.
    detection_broadcast: DetectionBroadcast,
    /// Broadcast channel for live log SSE streaming.
    log_broadcaster: LogBroadcaster,
    /// Broadcast channel for live spectrogram WebSocket streaming.
    spectrogram_broadcast: SpectrogramBroadcast,
    /// Shutdown latch. Flips from `false` to `true` once, when the server
    /// begins graceful shutdown. Long-lived streaming handlers (detection and
    /// spectrogram WebSocket streams, the admin log SSE stream) watch this and
    /// close promptly so axum's connection drain finishes, instead of holding
    /// the socket open until the `SHUTDOWN_GRACE` backstop force-exits.
    shutdown: watch::Sender<bool>,
    /// Localization manager for species common names.
    i18n: Option<RwLock<I18nManager>>,
    /// Custom site name for branding.
    site_name: Option<String>,
    /// Species info link site: "ebird", "allaboutbirds", or "none".
    info_site: String,
    /// Custom species image directory (checked before Wikipedia cache).
    custom_image_dir: Option<PathBuf>,
    /// Path to the active configuration file, threaded from the CLI so the
    /// in-UI diagnostics page can re-read and validate it. `None` in tests.
    config_path: Option<PathBuf>,
    /// Runtime metrics registry. Shared with the detection daemon (via the
    /// detection-event pipeline) and the metrics endpoint. Process-local;
    /// values are reset when the process restarts.
    metrics: SharedMetrics,
    /// Set once at startup to whether the detection daemon actually came up, so
    /// the health endpoint can distinguish a capturing-and-classifying station
    /// from one that booted web-only or failed to start the daemon (e.g. a
    /// misconfigured model/labels/watch dir). An `Arc<AtomicBool>` so the
    /// orchestrator can flip it after the state has been cloned and shared.
    detection_daemon_running: Arc<AtomicBool>,
    /// Short-TTL cache for rendered heavy-analytics fragments (streamgraph,
    /// dawn chorus, phenology, co-occurrence, time-series). Shared so a
    /// background pre-warmer and the request handlers populate the same store.
    analytics_cache: Arc<AnalyticsCache>,
}

/// Unwrap the `Arc<AppStateInner>`, aborting if shared (called during setup only).
///
/// Builder methods (`with_*`) must be called before the `AppState` is cloned
/// and shared with request handlers.  If this is violated (programming error),
/// the process aborts with a clear error message rather than silently ignoring
/// the mutation.  Since `panic = "abort"` is set in the release profile, this
/// is equivalent to the previous `panic!()` but with a documented rationale.
fn unwrap_inner(inner: Arc<AppStateInner>, method: &str) -> AppStateInner {
    Arc::try_unwrap(inner).unwrap_or_else(|_| {
        // This is a programming error (builder called after state was shared).
        // Abort rather than silently dropping the configuration change.
        tracing::error!(
            method,
            "AppState builder method called after state was shared — this is a bug"
        );
        std::process::abort();
    })
}

/// Rebuild `AppStateInner` from parts, applying one field mutation via a closure.
fn rebuild_inner<F>(old: AppStateInner, mutate: F) -> Arc<AppStateInner>
where
    F: FnOnce(&mut AppStateInner),
{
    let mut inner = old;
    mutate(&mut inner);
    Arc::new(inner)
}

impl AppState {
    /// Create new application state with an open database connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened.
    pub fn new(db_path: PathBuf) -> Result<Self, birdnet_db::sqlite::DbError> {
        let conn = birdnet_db::sqlite::open_or_create(&db_path)?;

        // A migration failure is fatal: each migration is now atomic, so a
        // failure leaves the DB at the last fully-applied version. Serving an
        // under-migrated schema to code that expects newer columns only yields
        // confusing runtime errors, so fail fast and let systemd surface it.
        birdnet_db::migration::migrate(&conn)?;

        let recording_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recordings");

        Ok(Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                db_path,
                recording_dir,
                #[cfg(feature = "analytics")]
                analytics_db: None,
                image_cache: None,
                detection_broadcast: DetectionBroadcast::new(DEFAULT_BROADCAST_CAPACITY),
                log_broadcaster: LogBroadcaster::new(),
                spectrogram_broadcast: SpectrogramBroadcast::new(SPECTROGRAM_BROADCAST_CAPACITY),
                shutdown: watch::channel(false).0,
                i18n: None,
                site_name: None,
                info_site: "ebird".to_string(),
                custom_image_dir: None,
                config_path: None,
                metrics: metrics::new_shared(),
                detection_daemon_running: Arc::new(AtomicBool::new(false)),
                analytics_cache: Arc::new(AnalyticsCache::default()),
            }),
        })
    }

    /// Create application state with both `SQLite` and `DuckDB` connections.
    ///
    /// # Errors
    ///
    /// Returns an error if either database cannot be opened.
    #[cfg(feature = "analytics")]
    pub fn new_with_analytics(
        db_path: PathBuf,
        analytics_path: &Path,
    ) -> Result<Self, birdnet_db::sqlite::DbError> {
        let conn = birdnet_db::sqlite::open_or_create(&db_path)?;

        // A migration failure is fatal: each migration is now atomic, so a
        // failure leaves the DB at the last fully-applied version. Serving an
        // under-migrated schema to code that expects newer columns only yields
        // confusing runtime errors, so fail fast and let systemd surface it.
        birdnet_db::migration::migrate(&conn)?;

        let recording_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recordings");

        let analytics_db = match AnalyticsDb::open(analytics_path) {
            Ok(mut adb) => {
                tracing::info!(path = %analytics_path.display(), "DuckDB analytics database opened");

                match adb.sync_from_sqlite(&conn) {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!(rows = count, "initial SQLite → DuckDB sync complete");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "initial DuckDB sync failed (non-fatal)");
                    }
                }

                if let Err(e) = adb.load_extension() {
                    tracing::warn!(
                        error = %e,
                        "duckdb-behavioral extension not loaded (analytics queries unavailable)"
                    );
                } else {
                    tracing::info!(
                        duckdb = adb.duckdb_version().as_deref().unwrap_or("unknown"),
                        extension = adb.extension_version().as_deref().unwrap_or("unknown"),
                        "duckdb-behavioral extension loaded"
                    );
                }

                Some(Mutex::new(adb))
            }
            Err(e) => {
                tracing::warn!(error = %e, "DuckDB analytics database not available (non-fatal)");
                None
            }
        };

        Ok(Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                db_path,
                recording_dir,
                analytics_db,
                image_cache: None,
                detection_broadcast: DetectionBroadcast::new(DEFAULT_BROADCAST_CAPACITY),
                log_broadcaster: LogBroadcaster::new(),
                spectrogram_broadcast: SpectrogramBroadcast::new(SPECTROGRAM_BROADCAST_CAPACITY),
                shutdown: watch::channel(false).0,
                i18n: None,
                site_name: None,
                info_site: "ebird".to_string(),
                custom_image_dir: None,
                config_path: None,
                metrics: metrics::new_shared(),
                detection_daemon_running: Arc::new(AtomicBool::new(false)),
                analytics_cache: Arc::new(AnalyticsCache::default()),
            }),
        })
    }

    /// Create application state from an existing connection (for testing).
    pub fn from_connection(conn: Connection, db_path: PathBuf) -> Self {
        let recording_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recordings");
        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                db_path,
                recording_dir,
                #[cfg(feature = "analytics")]
                analytics_db: None,
                image_cache: None,
                detection_broadcast: DetectionBroadcast::new(DEFAULT_BROADCAST_CAPACITY),
                log_broadcaster: LogBroadcaster::new(),
                spectrogram_broadcast: SpectrogramBroadcast::new(SPECTROGRAM_BROADCAST_CAPACITY),
                shutdown: watch::channel(false).0,
                i18n: None,
                site_name: None,
                info_site: "ebird".to_string(),
                custom_image_dir: None,
                config_path: None,
                metrics: metrics::new_shared(),
                detection_daemon_running: Arc::new(AtomicBool::new(false)),
                analytics_cache: Arc::new(AnalyticsCache::default()),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Builder methods (called once during startup, before sharing)
    // -----------------------------------------------------------------------

    /// Set the species image cache.
    #[must_use]
    pub fn with_image_cache(self, cache: ImageCache) -> Self {
        let inner = unwrap_inner(self.inner, "with_image_cache");
        Self {
            inner: rebuild_inner(inner, |s| s.image_cache = Some(Arc::new(cache))),
        }
    }

    /// Override the recording directory.
    #[must_use]
    pub fn with_recording_dir(self, dir: PathBuf) -> Self {
        let inner = unwrap_inner(self.inner, "with_recording_dir");
        Self {
            inner: rebuild_inner(inner, |s| s.recording_dir = dir),
        }
    }

    /// Set the i18n manager for species name translation.
    #[must_use]
    pub fn with_i18n(self, manager: I18nManager) -> Self {
        let inner = unwrap_inner(self.inner, "with_i18n");
        Self {
            inner: rebuild_inner(inner, |s| s.i18n = Some(RwLock::new(manager))),
        }
    }

    /// Set the custom site name for branding.
    #[must_use]
    pub fn with_site_name(self, name: String) -> Self {
        let inner = unwrap_inner(self.inner, "with_site_name");
        Self {
            inner: rebuild_inner(inner, |s| s.site_name = Some(name)),
        }
    }

    /// Set the species info link site.
    #[must_use]
    pub fn with_info_site(self, site: String) -> Self {
        let inner = unwrap_inner(self.inner, "with_info_site");
        Self {
            inner: rebuild_inner(inner, |s| s.info_site = site),
        }
    }

    /// Set the custom species image directory.
    #[must_use]
    pub fn with_custom_image_dir(self, dir: PathBuf) -> Self {
        let inner = unwrap_inner(self.inner, "with_custom_image_dir");
        Self {
            inner: rebuild_inner(inner, |s| s.custom_image_dir = Some(dir)),
        }
    }

    /// Set the path to the active configuration file (for the diagnostics page).
    #[must_use]
    pub fn with_config_path(self, path: PathBuf) -> Self {
        let inner = unwrap_inner(self.inner, "with_config_path");
        Self {
            inner: rebuild_inner(inner, |s| s.config_path = Some(path)),
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Execute a closure with a reference to the `SQLite` database connection.
    ///
    /// Recovers from a poisoned mutex via [`std::sync::PoisonError::into_inner`]
    /// rather than panicking: a panic inside the closure only borrows the
    /// `Connection` (it can't tear it), so the connection is still usable. The
    /// previous `.expect()` turned one bad query (e.g. a panic in any handler
    /// or background task that took this lock) into a permanent server brick,
    /// since every later `with_db` would panic too. This matches the
    /// recover-and-continue policy used elsewhere in the workspace (see
    /// `birdnet-integrations::species_images::cache`).
    pub fn with_db<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T,
    {
        let conn = self
            .inner
            .db
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&conn)
    }

    /// Execute a closure with a reference to the `DuckDB` analytics database.
    ///
    /// Recovers from a poisoned mutex; see [`Self::with_db`] for rationale.
    #[cfg(feature = "analytics")]
    pub fn with_analytics<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&AnalyticsDb) -> T,
    {
        self.inner.analytics_db.as_ref().map(|db| {
            let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            f(&db)
        })
    }

    /// Execute a closure with a `TimeSeriesDb` executor backed by the DuckDB connection.
    ///
    /// Recovers from a poisoned mutex; see [`Self::with_db`] for rationale.
    #[cfg(feature = "analytics")]
    pub fn with_timeseries<F, T>(
        &self,
        f: F,
    ) -> Option<Result<T, birdnet_timeseries::TimeSeriesError>>
    where
        F: FnOnce(
            birdnet_timeseries::executor::TimeSeriesDb<'_>,
        ) -> Result<T, birdnet_timeseries::TimeSeriesError>,
    {
        self.inner.analytics_db.as_ref().map(|db| {
            let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            birdnet_timeseries::executor::TimeSeriesDb::new(db.conn()).and_then(f)
        })
    }

    /// Rebuild the `DuckDB` analytics copy from the full `SQLite` detections
    /// table.
    ///
    /// The startup sync is incremental (only rows newer than the latest already
    /// in `DuckDB`), so a bulk historical import — whose rows are back-dated —
    /// would otherwise never reach the behavioural / time-series analytics. This
    /// runs a full rebuild so imported history appears with its original
    /// timestamps. Returns `None` when analytics is not enabled, otherwise the
    /// number of rows loaded (or the rebuild error).
    ///
    /// Recovers from a poisoned mutex; see [`Self::with_db`] for rationale.
    #[cfg(feature = "analytics")]
    pub fn resync_analytics_full(
        &self,
    ) -> Option<Result<u64, birdnet_behavioral::connection::AnalyticsError>> {
        let analytics = self.inner.analytics_db.as_ref()?;
        // Lock the SQLite connection first, then analytics — the only ordering
        // used elsewhere is sequential (the processor writes SQLite then DuckDB
        // without nesting), so this cannot deadlock. Recover from poison rather
        // than crash the process, matching `with_db` / `with_timeseries`.
        let conn = self
            .inner
            .db
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let adb = analytics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(adb.full_resync_from_sqlite(&conn))
    }

    /// Whether the `DuckDB` analytics database is available.
    #[cfg(feature = "analytics")]
    pub fn has_analytics(&self) -> bool {
        self.inner.analytics_db.is_some()
    }

    /// Whether the `DuckDB` analytics database is available.
    #[cfg(not(feature = "analytics"))]
    pub const fn has_analytics(&self) -> bool {
        false
    }

    /// Get the database file path.
    pub fn db_path(&self) -> &Path {
        &self.inner.db_path
    }

    /// Path to the active configuration file, if threaded from the CLI.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> {
        self.inner.config_path.as_deref()
    }

    /// Get the directory where extracted audio recordings are stored.
    pub fn recording_dir(&self) -> PathBuf {
        self.inner.recording_dir.clone()
    }

    /// Get the species image cache, if configured.
    pub fn image_cache(&self) -> Option<Arc<ImageCache>> {
        self.inner.image_cache.clone()
    }

    /// Get the detection broadcast channel for WebSocket streaming.
    pub fn detection_broadcast(&self) -> DetectionBroadcast {
        self.inner.detection_broadcast.clone()
    }

    /// Get the shared runtime metrics registry.
    ///
    /// Daemon-side code calls `inc_detection`, `observe_*`, etc. on this
    /// handle; the `/api/v2/metrics` route reads from it. Both sides share
    /// the same `Arc` so updates from any thread are visible immediately.
    #[must_use]
    pub fn metrics(&self) -> SharedMetrics {
        Arc::clone(&self.inner.metrics)
    }

    /// The shared short-TTL cache for heavy-analytics fragments.
    #[must_use]
    pub fn analytics_cache(&self) -> &AnalyticsCache {
        &self.inner.analytics_cache
    }

    /// Get the log broadcaster for SSE admin log streaming.
    pub fn log_broadcaster(&self) -> LogBroadcaster {
        self.inner.log_broadcaster.clone()
    }

    /// Get the spectrogram broadcast channel for WebSocket streaming.
    pub fn spectrogram_broadcast(&self) -> SpectrogramBroadcast {
        self.inner.spectrogram_broadcast.clone()
    }

    /// Subscribe to the shutdown latch.
    ///
    /// The returned receiver's value flips to `true` exactly once, when the
    /// server begins graceful shutdown. Long-lived streaming handlers select on
    /// this so they stop and let axum's connection drain finish, instead of
    /// holding the socket open until the `SHUTDOWN_GRACE` backstop force-exits.
    #[must_use]
    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.inner.shutdown.subscribe()
    }

    /// Signal every live connection to close. Called once when shutdown begins.
    ///
    /// Uses `send_replace` so the latch is set even when no receiver is
    /// currently subscribed: a connection still mid-upgrade when the signal
    /// arrives subscribes afterwards and observes the latched value, so it
    /// closes promptly rather than wedging the drain.
    pub fn begin_shutdown(&self) {
        self.inner.shutdown.send_replace(true);
    }

    /// Execute a closure with a reference to the i18n manager.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    pub fn with_i18n_ref<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&I18nManager) -> T,
    {
        self.inner.i18n.as_ref().map(|lock| {
            let mgr = lock.read().expect("i18n rwlock poisoned");
            f(&mgr)
        })
    }

    /// Get the custom species image directory, if configured.
    pub fn custom_image_dir(&self) -> Option<&Path> {
        self.inner.custom_image_dir.as_deref()
    }

    /// Get the custom site name, defaulting to "BirdNet-Behavior".
    pub fn site_name(&self) -> &str {
        self.inner
            .site_name
            .as_deref()
            .unwrap_or("BirdNet-Behavior")
    }

    /// Get the species info link site ("ebird", "allaboutbirds", or "none").
    pub fn info_site(&self) -> &str {
        &self.inner.info_site
    }

    /// Shared handle to the detection-daemon-running flag, for the orchestrator
    /// to set once it knows whether the daemon started (after the state has been
    /// cloned and shared).
    #[must_use]
    pub fn detection_status_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.inner.detection_daemon_running)
    }

    /// Whether the detection daemon is running, as recorded at startup.
    #[must_use]
    pub fn detection_daemon_running(&self) -> bool {
        self.inner.detection_daemon_running.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        AppState::from_connection(
            Connection::open_in_memory().expect("open in-memory sqlite"),
            PathBuf::from("test.db"),
        )
    }

    #[tokio::test]
    async fn shutdown_latch_starts_closed_then_latches_open() {
        let state = test_state();
        let mut rx = state.subscribe_shutdown();
        assert!(!*rx.borrow(), "latch should start closed");

        state.begin_shutdown();

        // Value is already `true`, so `wait_for` resolves immediately — this is
        // exactly what the streaming handlers observe to close on shutdown.
        // Copy the bool out so the borrow guard isn't held past this line.
        let opened = *rx.wait_for(|&v| v).await.expect("sender stays alive");
        assert!(opened, "latch should be open after begin_shutdown");
    }

    #[tokio::test]
    async fn subscribe_after_shutdown_observes_latched_value() {
        let state = test_state();
        // No subscribers yet — `send_replace` must still set the latch so a
        // connection that subscribes during the drain window closes promptly.
        state.begin_shutdown();

        let mut rx = state.subscribe_shutdown();
        assert!(
            *rx.borrow_and_update(),
            "late subscriber must observe the latched shutdown value"
        );
    }
}
