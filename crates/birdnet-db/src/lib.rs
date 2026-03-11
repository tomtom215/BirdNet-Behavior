//! BirdNET-Pi database layer.
//!
//! SQLite-based operational database with WAL mode enforcement,
//! backup/restore via the SQLite backup API, integrity checking,
//! and corruption recovery.

pub mod migration;
pub mod resilience;
pub mod sqlite;
