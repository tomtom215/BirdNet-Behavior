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
use crate::routes::spectrogram_ws::SpectrogramBroadcast;
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
    /// Broadcast channel for live spectrogram WebSocket streaming.
    spectrogram_broadcast: SpectrogramBroadcast,
    /// Localization manager for species common names.
    i18n: Option<RwLock<I18nManager>>,
    /// Audio source configuration for live streaming (ALSA device or RTSP URL).
    audio_source: Option<String>,
    /// Custom site name for branding.
    site_name: Option<String>,
    /// Species info link site: "ebird", "allaboutbirds", or "none".
    info_site: String,
    /// Custom species image directory (checked before Wikipedia cache).
    custom_image_dir: Option<PathBuf>,
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
                spectrogram_broadcast: SpectrogramBroadcast::new(DEFAULT_BROADCAST_CAPACITY),
                i18n: None,
                audio_source: None,
                site_name: None,
                info_site: "ebird".to_string(),
                custom_image_dir: None,
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

        if let Err(e) = birdnet_db::migration::migrate(&conn) {
            tracing::warn!(error = %e, "migration warning");
        }

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
                spectrogram_broadcast: SpectrogramBroadcast::new(DEFAULT_BROADCAST_CAPACITY),
                i18n: None,
                audio_source: None,
                site_name: None,
                info_site: "ebird".to_string(),
                custom_image_dir: None,
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
                spectrogram_broadcast: SpectrogramBroadcast::new(DEFAULT_BROADCAST_CAPACITY),
                i18n: None,
                audio_source: None,
                site_name: None,
                info_site: "ebird".to_string(),
                custom_image_dir: None,
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

    /// Set the audio source for live streaming.
    #[must_use]
    pub fn with_audio_source(self, source: String) -> Self {
        let inner = unwrap_inner(self.inner, "with_audio_source");
        Self {
            inner: rebuild_inner(inner, |s| s.audio_source = Some(source)),
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

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Execute a closure with a reference to the `SQLite` database connection.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    pub fn with_db<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T,
    {
        let conn = self.inner.db.lock().expect("database mutex poisoned");
        f(&conn)
    }

    /// Execute a closure with a reference to the `DuckDB` analytics database.
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
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
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

    /// Get the spectrogram broadcast channel for WebSocket streaming.
    pub fn spectrogram_broadcast(&self) -> SpectrogramBroadcast {
        self.inner.spectrogram_broadcast.clone()
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

    /// Get the audio source for live streaming, if configured.
    pub fn audio_source(&self) -> Option<&str> {
        self.inner.audio_source.as_deref()
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
}
