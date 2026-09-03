//! BirdNET-Pi database layer.
//!
//! SQLite-based operational database with WAL mode enforcement,
//! backup/restore via the `SQLite` backup API, integrity checking,
//! and corruption recovery.

pub mod accounts;
pub mod alert_rules;
pub mod audio_levels;
pub mod audio_sources;
pub mod clock;
pub mod dynamic_thresholds;
pub mod migration;
pub mod notifications;
pub mod outbound_queue;
pub mod phantoms;
pub mod resilience;
pub mod settings;
pub mod sound_levels;
pub mod species_tracking;
pub mod sqlite;
pub mod thresholds;
pub mod weather;
