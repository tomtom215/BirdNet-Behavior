//! Core migration traits.
//!
//! These traits define the contract between the migration engine and
//! source-specific implementations.  Each BirdNET-Pi version (or any
//! future supported source) implements `SchemaDetector`, `Migrator`,
//! and optionally `Validator`.

use std::path::Path;

use crate::error::MigrateError;
use crate::progress::ProgressHandle;
use crate::schema::DetectedSchema;

/// Detects whether a SQLite file uses a known source schema.
pub trait SchemaDetector: Send + Sync {
    /// Test whether the database at `path` matches this schema.
    ///
    /// Implementations open the file read-only and inspect table names,
    /// column names, and row counts.  They must **not** modify the file.
    ///
    /// # Errors
    ///
    /// Returns `MigrateError::SourceOpen` if the file cannot be read.
    /// Returns `MigrateError::UnknownSchema` if the file does not match.
    fn detect(&self, path: &Path) -> Result<DetectedSchema, MigrateError>;
}

/// Validates a source database before or after migration.
pub trait Validator: Send + Sync {
    /// Run pre-migration checks on the source database.
    ///
    /// Returns `Ok(ValidationReport)` even if some checks fail — failures
    /// are recorded inside the report and the caller decides whether to
    /// proceed.
    ///
    /// # Errors
    ///
    /// Returns `MigrateError` only if the database cannot be opened at all.
    fn validate_source(&self, source_path: &Path) -> Result<ValidationReport, MigrateError>;

    /// Run post-migration checks comparing source and destination row counts.
    ///
    /// # Errors
    ///
    /// Returns `MigrateError` on database access failures.
    fn validate_destination(
        &self,
        source_path: &Path,
        dest_path: &Path,
    ) -> Result<ValidationReport, MigrateError>;
}

/// Imports data from a source database into the destination.
pub trait Migrator: Send + Sync {
    /// Import all detections from `source_path` into `dest_path`.
    ///
    /// - The source file is **never modified**.
    /// - The destination receives only rows that do not already exist.
    /// - Progress is reported via `progress`.
    ///
    /// # Errors
    ///
    /// Returns `MigrateError` on any database or I/O failure.
    fn migrate(
        &self,
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
    ) -> Result<MigrationSummary, MigrateError>;
}

/// Report from a validation run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationReport {
    /// Whether all checks passed.
    pub passed: bool,
    /// Individual check results.
    pub checks: Vec<ValidationCheck>,
    /// Number of source rows found.
    pub source_rows: u64,
    /// Schema name/version.
    pub schema_name: String,
}

impl ValidationReport {
    /// Create a new report, computing `passed` from the checks.
    pub fn new(
        schema_name: impl Into<String>,
        source_rows: u64,
        checks: Vec<ValidationCheck>,
    ) -> Self {
        let passed = checks.iter().all(|c| c.passed || !c.required);
        Self {
            passed,
            checks,
            source_rows,
            schema_name: schema_name.into(),
        }
    }
}

/// A single validation check result.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationCheck {
    /// Check name.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Whether this check must pass for migration to proceed.
    pub required: bool,
    /// Human-readable detail.
    pub detail: String,
}

impl ValidationCheck {
    /// Create a passing check.
    pub fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            required: true,
            detail: detail.into(),
        }
    }

    /// Create a failing check.
    pub fn fail(name: impl Into<String>, detail: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            passed: false,
            required,
            detail: detail.into(),
        }
    }
}

/// Summary returned after a completed migration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MigrationSummary {
    /// Number of rows read from source.
    pub source_rows: u64,
    /// Number of rows written to destination.
    pub imported_rows: u64,
    /// Number of rows skipped (duplicates or filtered).
    pub skipped_rows: u64,
    /// Schema name/version that was migrated.
    pub schema_name: String,
    /// Source file path.
    pub source_path: String,
}
