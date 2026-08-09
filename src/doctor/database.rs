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
            // Deliberately a WARNING, not an error, even though corruption is
            // serious. The installed unit gates startup on
            //   ExecStartPre=... --doctor ... || [ $? -le 1 ]
            // so an error here (exit 2) stops systemd from starting the daemon
            // — and the daemon is what owns the recovery: `app.rs` runs
            // `check_and_recover`, restores from the newest backup that
            // verifies, and failing that quarantines the corrupt file and
            // starts fresh, refusing only if it cannot move the file aside.
            //
            // Failing here therefore blocked the exact code path that fixes
            // this, and `Restart=always` then burned StartLimitBurst=5 in under
            // a minute and parked the unit in `failed` — an unattended station
            // dead, with good backups on disk that nothing would ever restore.
            // Measured on a Raspberry Pi 4: corrupting the header left the
            // station down until a human ran `systemctl reset-failed`.
            //
            // Exit code 2 means "errors that will prevent operation". A corrupt
            // database does not prevent operation, so it is not one.
            Ok(false) => out.push(Check::warn(
                "Database integrity",
                format!(
                    "{} reports corruption — it will be quarantined and recovered from backup at startup",
                    db_path.display()
                ),
                "no action needed to restart; to inspect first, run `birdnet-behavior --check-db` \
                 and see the backups directory beside the database",
            )),
            Err(e) => out.push(Check::warn(
                "Database integrity",
                format!(
                    "{} could not be opened ({e}) — startup will attempt recovery, then quarantine it and start fresh",
                    db_path.display()
                ),
                "if this repeats, verify the file is a valid SQLite database and check the \
                 directory's ownership and permissions",
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

    /// The regression that killed a station on real hardware.
    ///
    /// A corrupt database must warn, never fail. The installed unit gates
    /// startup on `--doctor ... || [ $? -le 1 ]`, so a failure here (exit 2)
    /// prevents systemd from starting the daemon — and the daemon is what
    /// quarantines the corrupt file and recovers from backup. Reported as an
    /// error, the diagnostic blocks its own remedy.
    #[test]
    fn corrupt_database_warns_so_startup_can_recover_it() {
        use std::io::{Seek, SeekFrom, Write};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("birds.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        drop(conn);

        // Scribble over the header the way a failing SD card does.
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&db_path)
            .unwrap();
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(&[0xAB; 8192]).unwrap();
        f.sync_all().unwrap();
        drop(f);

        let cfg = Config::parse(&format!("DB_PATH={}", db_path.display())).unwrap();
        let checks = check_database(&cli(), Some(&cfg));
        let integrity = checks
            .iter()
            .find(|c| c.name.contains("Database integrity"))
            .expect("integrity check present");

        assert_ne!(
            integrity.status,
            Status::Pass,
            "corruption must still be reported: {}",
            integrity.message
        );
        assert_eq!(
            integrity.status,
            Status::Warn,
            "must be a warning (exit 1) so ExecStartPre lets the daemon start and recover, \
             not an error (exit 2) which blocks it: {}",
            integrity.message
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
