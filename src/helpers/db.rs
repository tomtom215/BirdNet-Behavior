//! Database-path resolution and the run-and-exit maintenance commands.

use std::path::PathBuf;

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
    use super::db_path_from_config;
    use crate::helpers::test_support::config_with;
    use std::path::PathBuf;

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
