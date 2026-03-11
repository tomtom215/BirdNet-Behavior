//! Shared application state for the web server.
//!
//! Holds the database connection and configuration, shared across
//! all request handlers via axum's State extractor.

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    /// Path to the database file (for diagnostics).
    db_path: PathBuf,
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

        Ok(Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                db_path,
            }),
        })
    }

    /// Create application state from an existing connection (for testing).
    pub fn from_connection(conn: Connection, db_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db: Mutex::new(conn),
                db_path,
            }),
        }
    }

    /// Execute a closure with a reference to the database connection.
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

    /// Get the database file path.
    pub fn db_path(&self) -> &PathBuf {
        &self.inner.db_path
    }
}
