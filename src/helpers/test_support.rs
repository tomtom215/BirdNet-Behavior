//! Shared fixtures for the helper unit tests.

use crate::cli::Cli;
use clap::Parser;

/// A `Cli` with every flag at its documented default.
///
/// `Cli::parse_from` with just the binary name materialises every
/// `default_value` and leaves Options at None / Vecs empty — exactly the
/// "user passed no flags" baseline a config has to override.
pub(super) fn default_cli() -> Cli {
    Cli::parse_from(["birdnet-behavior"])
}

/// A `Config` parsed from the given `KEY=value` entries (no file I/O).
pub(super) fn config_with(entries: &[(&str, &str)]) -> birdnet_core::config::Config {
    let content = entries
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    birdnet_core::config::Config::parse(&content).unwrap()
}

/// An `AppState` backed by a fresh in-memory, migrated `SQLite` database.
pub(super) fn test_state() -> birdnet_web::state::AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    birdnet_web::state::AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}
