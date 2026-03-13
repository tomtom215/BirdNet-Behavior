//! Migration progress tracking.
//!
//! Provides a callback-based progress mechanism so the web UI can poll
//! or stream progress events during long-running imports.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Current stage of a migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStage {
    /// Not yet started.
    Pending,
    /// Detecting source schema.
    Detecting,
    /// Running pre-migration validation.
    Validating,
    /// Importing rows from source.
    Importing,
    /// Running post-migration validation.
    Verifying,
    /// Migration completed successfully.
    Complete,
    /// Migration failed.
    Failed,
    /// Migration was cancelled.
    Cancelled,
}

/// Snapshot of migration progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    /// Current stage.
    pub stage: MigrationStage,
    /// Rows imported so far.
    pub rows_imported: u64,
    /// Total rows to import (0 if unknown).
    pub rows_total: u64,
    /// Human-readable status message.
    pub message: String,
    /// Error message (only set when stage is `Failed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl MigrationProgress {
    /// Create a pending (not started) progress snapshot.
    pub fn pending() -> Self {
        Self {
            stage: MigrationStage::Pending,
            rows_imported: 0,
            rows_total: 0,
            message: "Waiting to start".to_string(),
            error: None,
        }
    }

    /// Percentage complete (0–100).  Returns 0 if total is unknown.
    pub fn percent(&self) -> u8 {
        if self.rows_total == 0 {
            return 0;
        }
        #[allow(clippy::cast_precision_loss)]
        let pct = (self.rows_imported as f64 / self.rows_total as f64 * 100.0) as u8;
        pct.min(100)
    }

    /// Whether the migration is finished (success or failure).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.stage,
            MigrationStage::Complete | MigrationStage::Failed | MigrationStage::Cancelled
        )
    }
}

/// Shared, cloneable progress handle.
///
/// A `ProgressHandle` wraps an `Arc<Mutex<MigrationProgress>>` so it can be
/// shared between the background migration thread and the HTTP handlers that
/// poll progress.
#[derive(Debug, Clone)]
pub struct ProgressHandle(Arc<Mutex<MigrationProgress>>);

impl ProgressHandle {
    /// Create a new handle initialised to `Pending`.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(MigrationProgress::pending())))
    }

    /// Read a snapshot of the current progress.
    pub fn snapshot(&self) -> MigrationProgress {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Update progress (called from the migration worker).
    pub fn update(&self, progress: MigrationProgress) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = progress;
    }

    /// Convenience: update stage and message without changing row counts.
    pub fn set_stage(&self, stage: MigrationStage, message: impl Into<String>) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.stage = stage;
        guard.message = message.into();
    }

    /// Convenience: increment imported rows and update message.
    pub fn advance(&self, rows_imported: u64, message: impl Into<String>) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.rows_imported = rows_imported;
        guard.message = message.into();
    }

    /// Mark as failed with an error message.
    pub fn fail(&self, error: impl Into<String>) {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let msg = error.into();
        guard.error = Some(msg.clone());
        guard.stage = MigrationStage::Failed;
        guard.message = msg;
    }
}

impl Default for ProgressHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_percent_zero_total() {
        let p = MigrationProgress::pending();
        assert_eq!(p.percent(), 0);
    }

    #[test]
    fn progress_percent_calculation() {
        let p = MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 50,
            rows_total: 200,
            message: String::new(),
            error: None,
        };
        assert_eq!(p.percent(), 25);
    }

    #[test]
    fn progress_handle_update() {
        let handle = ProgressHandle::new();
        assert_eq!(handle.snapshot().stage, MigrationStage::Pending);

        handle.set_stage(MigrationStage::Importing, "importing rows");
        assert_eq!(handle.snapshot().stage, MigrationStage::Importing);
    }

    #[test]
    fn progress_is_terminal() {
        let mut p = MigrationProgress::pending();
        assert!(!p.is_terminal());
        p.stage = MigrationStage::Complete;
        assert!(p.is_terminal());
        p.stage = MigrationStage::Failed;
        assert!(p.is_terminal());
    }

    #[test]
    fn progress_handle_fail() {
        let handle = ProgressHandle::new();
        handle.fail("disk full");
        let snap = handle.snapshot();
        assert_eq!(snap.stage, MigrationStage::Failed);
        assert!(snap.error.is_some());
    }
}
