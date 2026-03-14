//! Shared application state for the web server.
//!
//! Holds the database connection, WebSocket broadcast channel, and configuration,
//! shared across all request handlers via axum's State extractor.

#[cfg(feature = "analytics")]
use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_core::i18n::I18nManager;
use birdnet_integrations::species_images::ImageCache;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use crate::routes::admin::logs::LogBroadcaster;
use crate::routes::websocket::DetectionBroadcast;

/// Default WebSocket broadcast channel capacity.
const DEFAULT_BROADCAST_CAPACITY: usize = 256;

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
    /// Localization manager for species common names.
    i18n: Option<RwLock<I18nManager>>,
    /// Audio source configuration for live streaming (ALSA device or RTSP URL).
    audio_source: Option<String>,
}

impl AppState {
    /// Create new application state with an open database connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened.
    pub fn new(db_path: PathBuf) -> Result<Self, birdnet_db::sqlite::DbError> {
        let conn = birdnet_db::sqlite::open_or_create(&db_path)?;

        // Run migrations on startup
        if let Err(e) = birdnet_db::migration::migrate(&conn) {
            tracing::warn!(error = %e, "migration warning");
        }

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
                i18n: None,
                audio_source: None,
            }),
        })
    }

    /// Create application state with both `SQLite` and `DuckDB` connections.
    ///
    /// The `DuckDB` database is opened at the given path for behavioral
    /// analytics queries. An initial sync from `SQLite` is performed.
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

        // Run migrations on startup
        if let Err(e) = birdnet_db::migration::migrate(&conn) {
            tracing::warn!(error = %e, "migration warning");
        }

        // Open DuckDB analytics database
        let recording_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recordings");

        let analytics_db = match AnalyticsDb::open(analytics_path) {
            Ok(mut adb) => {
                tracing::info!(path = %analytics_path.display(), "DuckDB analytics database opened");

                // Initial sync from SQLite
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

                // Try to load the behavioral extension (non-fatal if offline)
                if let Err(e) = adb.load_extension() {
                    tracing::warn!(
                        error = %e,
                        "duckdb-behavioral extension not loaded (analytics queries unavailable)"
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
                i18n: None,
                audio_source: None,
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
                i18n: None,
                audio_source: None,
            }),
        }
    }

    /// Set the species image cache.
    ///
    /// Must be called before the state is shared across threads (before server start).
    /// Returns a new `AppState` with the image cache configured.
    ///
    /// # Panics
    ///
    /// Panics if called after the state has been shared (cloned).
    #[must_use]
    pub fn with_image_cache(self, cache: ImageCache) -> Self {
        // We need to recreate the inner since Arc doesn't allow mutation.
        // This is called once during setup, before the state is shared.
        let inner = Arc::try_unwrap(self.inner).unwrap_or_else(|arc| {
            // If there are other references, we need to clone the inner state.
            // This shouldn't happen during startup, but handle gracefully.
            let old = &*arc;
            let db = old
                .db
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // We can't clone the connection, so this path is a programming error.
            // In practice, this is only called once during setup.
            drop(db);
            panic!("with_image_cache called after state was shared");
        });

        Self {
            inner: Arc::new(AppStateInner {
                db: inner.db,
                db_path: inner.db_path,
                recording_dir: inner.recording_dir,
                #[cfg(feature = "analytics")]
                analytics_db: inner.analytics_db,
                image_cache: Some(Arc::new(cache)),
                detection_broadcast: inner.detection_broadcast,
                log_broadcaster: inner.log_broadcaster,
                i18n: inner.i18n,
                audio_source: inner.audio_source,
            }),
        }
    }

    /// Execute a closure with a reference to the `SQLite` database connection.
    ///
    /// The mutex is held for the duration of the closure.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned (indicates a prior panic while holding the lock).
    pub fn with_db<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T,
    {
        let conn = self.inner.db.lock().expect("database mutex poisoned");
        f(&conn)
    }

    /// Execute a closure with a reference to the `DuckDB` analytics database.
    ///
    /// Returns `None` if the analytics database is not available.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    #[cfg(feature = "analytics")]
    pub fn with_analytics<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&AnalyticsDb) -> T,
    {
        self.inner.analytics_db.as_ref().map(|db| {
            let db = db.lock().expect("analytics mutex poisoned");
            f(&db)
        })
    }

    /// Execute a closure with a `TimeSeriesDb` executor backed by the DuckDB connection.
    ///
    /// Returns `None` if the analytics database is not available, or `Some(Err(…))`
    /// if the executor cannot be initialised (e.g. missing view).
    ///
    /// # Panics
    ///
    /// Panics if the analytics mutex is poisoned.
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
            let db = db.lock().expect("analytics mutex poisoned");
            birdnet_timeseries::executor::TimeSeriesDb::new(db.conn()).and_then(f)
        })
    }

    /// Whether the `DuckDB` analytics database is available.
    #[cfg(feature = "analytics")]
    pub fn has_analytics(&self) -> bool {
        self.inner.analytics_db.is_some()
    }

    /// Whether the `DuckDB` analytics database is available.
    ///
    /// Always returns `false` when compiled without the `analytics` feature.
    #[cfg(not(feature = "analytics"))]
    pub const fn has_analytics(&self) -> bool {
        false
    }

    /// Get the database file path.
    pub fn db_path(&self) -> &Path {
        &self.inner.db_path
    }

    /// Get the directory where extracted audio recordings are stored.
    pub fn recording_dir(&self) -> PathBuf {
        self.inner.recording_dir.clone()
    }

    /// Override the recording directory (for testing or custom deployments).
    #[must_use]
    pub fn with_recording_dir(self, dir: PathBuf) -> Self {
        let inner = Arc::try_unwrap(self.inner).unwrap_or_else(|arc| {
            drop(arc.db.lock().ok());
            panic!("with_recording_dir called after state was shared");
        });
        Self {
            inner: Arc::new(AppStateInner {
                db: inner.db,
                db_path: inner.db_path,
                recording_dir: dir,
                #[cfg(feature = "analytics")]
                analytics_db: inner.analytics_db,
                image_cache: inner.image_cache,
                detection_broadcast: inner.detection_broadcast,
                log_broadcaster: inner.log_broadcaster,
                i18n: inner.i18n,
                audio_source: inner.audio_source,
            }),
        }
    }

    /// Set the i18n manager for species name translation.
    ///
    /// Must be called before the state is shared across threads (before server start).
    #[must_use]
    pub fn with_i18n(self, manager: I18nManager) -> Self {
        let inner = Arc::try_unwrap(self.inner).unwrap_or_else(|_| {
            panic!("with_i18n called after state was shared");
        });
        Self {
            inner: Arc::new(AppStateInner {
                db: inner.db,
                db_path: inner.db_path,
                recording_dir: inner.recording_dir,
                #[cfg(feature = "analytics")]
                analytics_db: inner.analytics_db,
                image_cache: inner.image_cache,
                detection_broadcast: inner.detection_broadcast,
                log_broadcaster: inner.log_broadcaster,
                i18n: Some(RwLock::new(manager)),
                audio_source: inner.audio_source,
            }),
        }
    }

    /// Set the audio source for live streaming (ALSA device name or RTSP URL).
    ///
    /// Must be called before the state is shared across threads (before server start).
    #[must_use]
    pub fn with_audio_source(self, source: String) -> Self {
        let inner = Arc::try_unwrap(self.inner).unwrap_or_else(|_| {
            panic!("with_audio_source called after state was shared");
        });
        Self {
            inner: Arc::new(AppStateInner {
                db: inner.db,
                db_path: inner.db_path,
                recording_dir: inner.recording_dir,
                #[cfg(feature = "analytics")]
                analytics_db: inner.analytics_db,
                image_cache: inner.image_cache,
                detection_broadcast: inner.detection_broadcast,
                log_broadcaster: inner.log_broadcaster,
                i18n: inner.i18n,
                audio_source: Some(source),
            }),
        }
    }

    /// Get the species image cache, if configured.
    pub fn image_cache(&self) -> Option<Arc<ImageCache>> {
        self.inner.image_cache.clone()
    }

    /// Get the detection broadcast channel for WebSocket streaming.
    pub fn detection_broadcast(&self) -> DetectionBroadcast {
        self.inner.detection_broadcast.clone()
    }

    /// Get the log broadcaster for SSE admin log streaming.
    pub fn log_broadcaster(&self) -> LogBroadcaster {
        self.inner.log_broadcaster.clone()
    }

    /// Execute a closure with a reference to the i18n manager.
    ///
    /// Returns `None` if no i18n manager is configured.
    pub fn with_i18n_ref<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&I18nManager) -> T,
    {
        self.inner.i18n.as_ref().map(|lock| {
            let mgr = lock.read().expect("i18n rwlock poisoned");
            f(&mgr)
        })
    }

    /// Get the audio source for live streaming, if configured.
    pub fn audio_source(&self) -> Option<&str> {
        self.inner.audio_source.as_deref()
    }
}
