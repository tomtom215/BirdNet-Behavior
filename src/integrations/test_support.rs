//! Shared fixtures for the integration-builder unit tests.
//!
//! Each builder is a pure config-to-handle mapping, so the tests across the
//! sibling modules share the same three fixtures: a default CLI, a `Config`
//! built from key/value pairs, and an in-memory `AppState`.

use crate::cli::Cli;
use clap::Parser;

/// A `Cli` with every flag at its documented default.
pub(super) fn default_cli() -> Cli {
    Cli::parse_from(["birdnet-behavior"])
}

/// A `Cli` whose `explicit` set reports the given argument ids as
/// operator-supplied, so precedence rules ("an explicit flag beats the admin
/// setting") can be exercised without going through `clap`.
pub(super) fn cli_with_explicit(ids: &[&str]) -> Cli {
    let mut cli = default_cli();
    cli.explicit = crate::cli::ExplicitArgs::from_ids(ids.iter().copied());
    cli
}

/// A `Config` parsed from the given `KEY=value` entries.
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
