//! Error types for the migration subsystem.

use std::fmt;

/// Errors that can occur during migration.
#[derive(Debug)]
pub enum MigrateError {
    /// Source database cannot be opened or read.
    SourceOpen(rusqlite::Error),
    /// Destination database cannot be opened or written.
    DestinationOpen(rusqlite::Error),
    /// Source file does not exist or is not a file.
    SourceNotFound(String),
    /// Source database schema is not recognised.
    UnknownSchema(String),
    /// Source database schema is recognised but not supported.
    UnsupportedSchema(String),
    /// Data query or insert failed during migration.
    DataTransfer(rusqlite::Error),
    /// Pre-migration validation failed.
    ValidationFailed(String),
    /// Migration was aborted.
    Aborted(String),
    /// I/O error (file copy, temp file, etc.).
    Io(std::io::Error),
    /// A query or aggregation failed during report generation.
    Query(String),
    /// CSV/TSV parse error.
    CsvParse(String),
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceOpen(e) => write!(f, "cannot open source database: {e}"),
            Self::DestinationOpen(e) => write!(f, "cannot open destination database: {e}"),
            Self::SourceNotFound(p) => write!(f, "source file not found: {p}"),
            Self::UnknownSchema(msg) => write!(f, "unknown source schema: {msg}"),
            Self::UnsupportedSchema(msg) => write!(f, "unsupported schema version: {msg}"),
            Self::DataTransfer(e) => write!(f, "data transfer error: {e}"),
            Self::ValidationFailed(msg) => write!(f, "validation failed: {msg}"),
            Self::Aborted(reason) => write!(f, "migration aborted: {reason}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Query(msg) => write!(f, "query error: {msg}"),
            Self::CsvParse(msg) => write!(f, "CSV parse error: {msg}"),
        }
    }
}

impl std::error::Error for MigrateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SourceOpen(e) | Self::DestinationOpen(e) | Self::DataTransfer(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MigrateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
