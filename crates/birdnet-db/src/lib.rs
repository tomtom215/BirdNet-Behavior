//! BirdNET-Pi database layer.
//!
//! SQLite-based operational database with WAL mode enforcement,
//! backup/restore via the `SQLite` backup API, integrity checking,
//! and corruption recovery.

pub mod alert_rules;
pub mod migration;
pub mod notifications;
pub mod resilience;
pub mod settings;
pub mod sqlite;
