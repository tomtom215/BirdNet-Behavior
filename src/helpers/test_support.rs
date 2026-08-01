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

/// An `AppState` whose database path lives under `dir`, so
/// `AppState::recording_dir` resolves inside the caller's temp directory.
///
/// `test_state`'s `":memory:"` path makes `recording_dir` the *relative* path
/// `recordings`, which any code that creates that directory would materialise
/// in the test process's working directory. Tests that exercise the recordings
/// directory take this instead.
pub(super) fn test_state_in(dir: &std::path::Path) -> birdnet_web::state::AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    birdnet_web::state::AppState::from_connection(conn, dir.join("birds.db"))
}
