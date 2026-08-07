//! Shared fixtures for the helper unit tests.

use crate::cli::Cli;
use clap::Parser;

/// A `Cli` with every flag at its documented default.
///
/// `Cli::parse_from` with just the binary name materialises every
/// `default_value` and leaves Options at None / Vecs empty — exactly the
/// "user passed no flags" baseline a config has to override.
pub fn default_cli() -> Cli {
    Cli::parse_from(["birdnet-behavior"])
}

/// A `Cli` whose `explicit` set reports the given argument ids as
/// operator-supplied, so precedence rules ("an explicit flag beats the admin
/// setting") can be exercised without going through `clap`.
pub fn cli_with_explicit(ids: &[&str]) -> Cli {
    let mut cli = default_cli();
    cli.explicit = crate::cli::ExplicitArgs::from_ids(ids.iter().copied());
    cli
}

/// A `Config` parsed from the given `KEY=value` entries (no file I/O).
pub fn config_with(entries: &[(&str, &str)]) -> birdnet_core::config::Config {
    let content = entries
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    birdnet_core::config::Config::parse(&content).unwrap()
}

/// An `AppState` backed by a fresh in-memory, migrated `SQLite` database.
pub fn test_state() -> birdnet_web::state::AppState {
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
pub fn test_state_in(dir: &std::path::Path) -> birdnet_web::state::AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    birdnet_web::state::AppState::from_connection(conn, dir.join("birds.db"))
}
