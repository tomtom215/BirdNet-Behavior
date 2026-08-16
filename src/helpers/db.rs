//! Database-path resolution and the run-and-exit maintenance commands.

use std::path::{Path, PathBuf};

/// Create the directory that will hold the operational `SQLite` database.
///
/// `SQLite` will not create a missing parent directory — it fails the open with
/// `unable to open database file`, which says nothing about what is wrong — so
/// a station whose `DB_PATH` points somewhere that does not exist yet simply
/// refuses to start.
///
/// Every sibling directory is already created on demand: the recordings
/// directory ([`crate::helpers::system`]), the watch directory
/// ([`crate::daemon`]), the capture output and tmpfs mounts, and the `DuckDB`
/// analytics store two modules over in `birdnet-behavioral`. This one was the
/// exception, and it is the only one whose absence is fatal — while
/// `--doctor` reported *"will be created on first run → no action needed"* and
/// exited 0.
///
/// The realistic trigger is not a fresh install (the installer pre-creates the
/// directory) but the move `docs/FIELD_DEPLOYMENT.md` actively recommends:
/// relocating storage off the SD card, which fails after ~6 months of WAL
/// churn. `RECS_DIR=/data/recordings` works because it is auto-created;
/// `DB_PATH=/data/birdnet/birds.db` did not.
///
/// # Errors
///
/// Returns a message naming the directory and the underlying cause when it
/// cannot be created — a read-only mount or a permissions problem, which are
/// genuinely the operator's to fix and must not be silently swallowed.
pub fn ensure_db_dir(db_path: &Path) -> Result<(), String> {
    // A bare relative filename (`birds.db`) has an empty parent, which is the
    // current directory and therefore already exists. Nothing to do.
    let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    if parent.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "could not create the database directory {}: {e}. Create it and make it writable by \
             the user running birdnet-behavior (`sudo mkdir -p {0} && sudo chown $USER {0}`), or \
             point DB_PATH somewhere writable.",
            parent.display()
        )
    })?;
    tracing::info!(path = %parent.display(), "created database directory");
    Ok(())
}

/// Resolve the database path from config, falling back to a default location.
pub fn db_path_from_config(config: Option<&birdnet_core::config::Config>) -> PathBuf {
    config.and_then(|c| c.get("DB_PATH")).map_or_else(
        || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/pi".into());
            PathBuf::from(format!("{home}/BirdNet-Behavior/birds.db"))
        },
        PathBuf::from,
    )
}

/// Report what the pending data-rewriting migrations would do, and exit.
///
/// Opens the database read-only so the report cannot itself be the thing that
/// migrates: `migrate()` runs on the normal startup path, and an operator
/// asking what an upgrade *would* do must not trigger it by asking.
///
/// # Errors
///
/// Returns an error if the database cannot be opened or the preview queries
/// fail. A database that does not exist yet is not an error — there is no
/// history to rewrite.
pub fn run_migration_report(
    config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = db_path_from_config(config);
    if !db_path.exists() {
        println!(
            "No database at {} yet — nothing to migrate.",
            db_path.display()
        );
        return Ok(());
    }

    // Read-only, and explicitly so. `Connection::open` would create and then
    // silently migrate on some paths; this cannot.
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;

    let current = birdnet_db::migration::current_version(&conn)?;
    let previews = birdnet_db::migration::preview_pending(&conn)?;

    println!("Database   {}", db_path.display());
    println!("Schema     version {current}\n");

    if previews.is_empty() {
        println!(
            "No pending migration rewrites existing detections.\n\n  \
             Schema-only migrations may still be pending; those add columns and \
             indexes\n  and leave the rows already on disk alone."
        );
        return Ok(());
    }

    for p in &previews {
        println!("Migration {} — {}", p.version, p.description);
        if p.rows.is_empty() {
            println!("  (nothing on disk for it to change)");
        }
        for (label, value) in &p.rows {
            println!("  {label:<46} {value}");
        }
        println!();
    }
    println!(
        "This changed nothing. The rewrite runs on the next normal start, and the\n\
         database is copied to <db>.pre-migration-<version>.backup immediately\n\
         beforehand — restoring it is a file move. The copy needs about as much\n\
         free space as the database itself."
    );
    Ok(())
}

/// Run a database integrity check and exit.
pub fn run_integrity_check(
    config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = db_path_from_config(config);
    tracing::info!(path = %db_path.display(), "running integrity check");
    match birdnet_db::resilience::full_integrity_check(&db_path) {
        Ok(true) => {
            tracing::info!("database integrity check PASSED");
            Ok(())
        }
        Ok(false) => {
            tracing::error!("database integrity check FAILED — corruption detected");
            std::process::exit(1);
        }
        Err(e) => Err(Box::new(e)),
    }
}

/// Create a database backup and exit.
pub fn run_backup(
    config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = db_path_from_config(config);
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");
    tracing::info!(path = %db_path.display(), "creating database backup");
    let backup_path = birdnet_db::resilience::backup_database(&db_path, &backup_dir)?;
    tracing::info!(backup = %backup_path.display(), "backup created successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{db_path_from_config, ensure_db_dir};
    use crate::helpers::test_support::config_with;
    use std::path::PathBuf;

    // ── ensure_db_dir ──────────────────────────────────────────────────

    #[test]
    fn creates_a_missing_database_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("birdnet/birds.db");
        assert!(!db.parent().unwrap().exists());

        ensure_db_dir(&db).expect("should create the directory");
        assert!(db.parent().unwrap().is_dir());
    }

    #[test]
    fn creates_every_missing_level() {
        // The storage relocation docs/FIELD_DEPLOYMENT.md recommends lands
        // several levels deep on a fresh mount.
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("mnt/ssd/birdnet/data/birds.db");

        ensure_db_dir(&db).expect("should create every level");
        assert!(db.parent().unwrap().is_dir());
    }

    #[test]
    fn is_idempotent_when_the_directory_already_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("birds.db");

        ensure_db_dir(&db).expect("first call");
        ensure_db_dir(&db).expect("second call must not fail");
        assert!(tmp.path().is_dir());
    }

    #[test]
    fn accepts_a_bare_relative_filename() {
        // `DB_PATH=birds.db` has an empty parent, which is the current
        // directory — already there, nothing to create, and definitely not a
        // reason to fail startup.
        ensure_db_dir(&PathBuf::from("birds.db")).expect("bare filename is fine");
    }

    #[test]
    fn reports_an_actionable_error_when_the_directory_cannot_be_created() {
        // Rooting the path at a regular file makes create_dir_all fail with
        // ENOTDIR for any user, including root — where a permissions test
        // would not fail at all.
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("not-a-directory");
        std::fs::write(&file, b"x").expect("write file");

        let err = ensure_db_dir(&file.join("nested/birds.db"))
            .expect_err("creating a directory under a file must fail");
        assert!(
            err.contains("could not create the database directory"),
            "error should name what failed: {err}"
        );
        assert!(
            err.contains("nested"),
            "error should name the directory: {err}"
        );
        assert!(
            err.contains("DB_PATH"),
            "error should tell the operator which knob to change: {err}"
        );
    }

    #[test]
    fn db_path_uses_config_value_when_present() {
        let cfg = config_with(&[("DB_PATH", "/srv/birds.db")]);
        assert_eq!(
            db_path_from_config(Some(&cfg)),
            PathBuf::from("/srv/birds.db")
        );
    }

    #[test]
    fn db_path_falls_back_to_home_when_config_absent() {
        // No DB_PATH in the config — the helper should construct
        // $HOME/BirdNet-Behavior/birds.db. We assert the suffix to
        // avoid coupling the test to the current HOME value.
        let cfg = config_with(&[("SOMETHING_ELSE", "irrelevant")]);
        let path = db_path_from_config(Some(&cfg));
        assert!(
            path.ends_with("BirdNet-Behavior/birds.db"),
            "expected default to end with BirdNet-Behavior/birds.db; got {}",
            path.display()
        );
    }

    #[test]
    fn db_path_falls_back_when_config_is_none() {
        // No config at all: same default-construction path.
        let path = db_path_from_config(None);
        assert!(path.ends_with("BirdNet-Behavior/birds.db"));
    }
}
