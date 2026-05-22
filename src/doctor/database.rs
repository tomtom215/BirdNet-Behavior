//! Database checks: parent-directory writability and integrity.

use birdnet_core::config::Config;

use super::{Check, writable};
use crate::cli::Cli;
use crate::helpers::db_path_from_config;

pub(super) fn check_database(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let db_path = db_path_from_config(config);
    let mut out = Vec::new();

    let Some(parent) = db_path.parent() else {
        out.push(Check::fail(
            "Database directory",
            format!("{} has no parent directory", db_path.display()),
            "set DB_PATH in the config to an absolute path with a writable parent",
        ));
        return out;
    };

    if parent.exists() {
        if writable(parent) {
            out.push(Check::pass(
                "Database directory",
                format!("{} is writable", parent.display()),
            ));
        } else {
            out.push(Check::fail(
                "Database directory",
                format!("{} is not writable", parent.display()),
                "ensure the running user owns this directory (chown / chmod u+w)",
            ));
        }
    } else {
        out.push(Check::warn(
            "Database directory",
            format!(
                "{} does not exist yet — will be created on first run",
                parent.display()
            ),
            "no action needed unless you want to pre-create it with `mkdir -p`",
        ));
    }

    if db_path.exists() {
        let _ = cli; // cli unused beyond db_path; keep symmetric signature
        match birdnet_db::resilience::full_integrity_check(&db_path) {
            Ok(true) => out.push(Check::pass(
                "Database integrity",
                format!("{} passes integrity check", db_path.display()),
            )),
            Ok(false) => out.push(Check::fail(
                "Database integrity",
                format!("{} reports corruption", db_path.display()),
                "run `birdnet-behavior --backup-db` then restore from the most recent backup",
            )),
            Err(e) => out.push(Check::fail(
                "Database integrity",
                format!("{} could not be opened: {e}", db_path.display()),
                "verify the file is a valid SQLite database; restore from backup if not",
            )),
        }
    } else {
        out.push(Check::skip(
            "Database integrity",
            "no database file yet — will be created on first run",
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn dir_pass_and_integrity_skip_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::parse(&format!("DB_PATH={}/birds.db", dir.path().display())).unwrap();
        let checks = check_database(&cli(), Some(&cfg));
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Database directory") && c.status == Status::Pass)
        );
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Database integrity") && c.status == Status::Skip)
        );
    }

    #[test]
    fn dir_warn_when_parent_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = format!("{}/no-such-subdir/birds.db", dir.path().display());
        let cfg = Config::parse(&format!("DB_PATH={missing}")).unwrap();
        let checks = check_database(&cli(), Some(&cfg));
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Database directory") && c.status == Status::Warn)
        );
    }

    #[test]
    fn integrity_pass_for_valid_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("birds.db");
        // A real, migrated SQLite database passes the integrity check.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        drop(conn);
        let cfg = Config::parse(&format!("DB_PATH={}", db_path.display())).unwrap();
        let checks = check_database(&cli(), Some(&cfg));
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Database integrity") && c.status == Status::Pass)
        );
    }
}
