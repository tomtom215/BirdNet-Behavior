//! Helper functions for the main binary entry point.
//!
//! Extracted from `main.rs` for modularity. Contains database utilities,
//! startup initialization, and system integration helpers.

use std::path::PathBuf;

use crate::cli::Cli;

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

/// Build app state with DuckDB analytics (feature-gated).
#[cfg(feature = "analytics")]
pub fn build_state_with_analytics(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    server_config: &birdnet_web::server::ServerConfig,
) -> Result<birdnet_web::state::AppState, Box<dyn std::error::Error>> {
    let analytics_path = cli
        .analytics_db
        .clone()
        .or_else(|| config?.get("ANALYTICS_DB_PATH").map(PathBuf::from));

    if let Some(ref analytics_path) = analytics_path {
        tracing::info!(path = %analytics_path.display(), "enabling DuckDB analytics");
        birdnet_web::state::AppState::new_with_analytics(
            server_config.db_path.clone(),
            analytics_path,
        )
        .map_err(|e| format!("database error: {e}").into())
    } else {
        birdnet_web::state::AppState::new(server_config.db_path.clone())
            .map_err(|e| format!("database error: {e}").into())
    }
}

/// Initialize the species image cache.
pub fn init_image_cache(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let cache_dir = cli
        .image_cache_dir
        .clone()
        .or_else(|| config?.get("IMAGE_CACHE_DIR").map(PathBuf::from));

    let Some(ref cache_dir) = cache_dir else {
        return state;
    };

    match birdnet_integrations::species_images::ImageCache::with_wikipedia(cache_dir) {
        Ok(cache) => {
            tracing::info!(
                path = %cache_dir.display(),
                cached = cache.cached_count(),
                "species image cache enabled"
            );
            state.with_image_cache(cache)
        }
        Err(e) => {
            tracing::warn!(error = %e, "species image cache not available (non-fatal)");
            state
        }
    }
}

/// Initialize i18n language settings.
pub fn init_i18n(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let lang = if cli.lang == "en" {
        config
            .and_then(|c| c.get("DATABASE_LANG"))
            .map_or_else(|| "en".to_string(), |v| v.to_string())
    } else {
        cli.lang.clone()
    };

    if lang == "en" {
        return state;
    }

    let labels_dir = cli
        .labels_dir
        .clone()
        .or_else(|| config?.get("LABELS_DIR").map(PathBuf::from));

    let Some(labels_dir) = labels_dir else {
        tracing::warn!(lang = %lang, "language set but no --labels-dir configured");
        return state;
    };

    let mut mgr = birdnet_core::i18n::I18nManager::new(&lang);
    match mgr.load_language(&lang, &labels_dir) {
        Ok(()) => {
            tracing::info!(lang = %lang, "i18n language loaded");
            state.with_i18n(mgr)
        }
        Err(e) => {
            tracing::warn!(lang = %lang, error = %e, "failed to load language pack");
            state
        }
    }
}

/// Initialize audio source for live streaming.
pub fn init_audio_source(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let source = cli
        .rtsp_url
        .clone()
        .or_else(|| cli.alsa_device.clone())
        .or_else(|| config?.get("RTSP_STREAM").map(String::from))
        .or_else(|| config?.get("REC_CARD").map(String::from));

    match source {
        Some(src) => {
            tracing::info!(source = %src, "live audio stream source configured");
            state.with_audio_source(src)
        }
        None => state,
    }
}

/// Initialize custom site name.
pub fn init_site_name(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let name = cli
        .site_name
        .clone()
        .or_else(|| config?.get("SITENAME").map(String::from));

    match name {
        Some(n) if !n.is_empty() => {
            tracing::info!(site_name = %n, "custom site name configured");
            state.with_site_name(n)
        }
        _ => state,
    }
}

/// Start the disk manager as a background thread.
pub fn start_disk_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) -> Option<std::thread::JoinHandle<()>> {
    use birdnet_core::audio::capture::{DiskManager, DiskManagerConfig, FullDiskAction};

    let monitored_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))?;

    let max_files_per_species = if cli.max_files_per_species > 0 {
        cli.max_files_per_species
    } else {
        config
            .and_then(|c| c.get_parsed::<u32>("MAX_FILES_SPECIES").ok())
            .unwrap_or(0)
    };

    let purge_threshold = config
        .and_then(|c| c.get_parsed::<u8>("DISK_PURGE_THRESHOLD").ok())
        .unwrap_or(95);

    let locked_file_names =
        state.with_db(|conn| birdnet_db::sqlite::locked_file_names(conn).unwrap_or_default());

    let config_obj = DiskManagerConfig {
        monitored_dir: monitored_dir.clone(),
        purge_threshold,
        full_disk_action: FullDiskAction::Purge,
        max_files_per_species,
        check_interval_secs: 60,
        exclude_paths: cli.disk_exclude.clone(),
        locked_file_names,
    };

    tracing::info!(
        dir = %monitored_dir.display(),
        max_files_per_species,
        purge_threshold,
        excluded_paths = cli.disk_exclude.len(),
        "disk manager configured"
    );

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let manager = DiskManager::new(config_obj);

    let handle = std::thread::spawn(move || {
        manager.run(&stop_rx);
    });

    std::mem::forget(stop_tx);
    Some(handle)
}

/// Generate an Avahi mDNS service file for local network discovery.
pub fn maybe_install_avahi_service(port: u16, site_name: &str) {
    let avahi_dir = std::path::Path::new("/etc/avahi/services");
    if !avahi_dir.exists() {
        return;
    }

    let service_file = avahi_dir.join("birdnet-behavior.service");
    if service_file.exists() {
        return;
    }

    let name = if site_name.is_empty() || site_name == "BirdNet-Behavior" {
        "BirdNet-Behavior".to_string()
    } else {
        site_name.to_string()
    };

    let xml = format!(
        r#"<?xml version="1.0" standalone='no'?>
<!DOCTYPE service-group SYSTEM "avahi-service.dtd">
<service-group>
  <name replace-wildcards="yes">{name} on %h</name>
  <service>
    <type>_http._tcp</type>
    <port>{port}</port>
    <txt-record>path=/</txt-record>
    <txt-record>software=BirdNet-Behavior</txt-record>
  </service>
</service-group>
"#
    );

    match std::fs::write(&service_file, xml) {
        Ok(()) => tracing::info!(
            path = %service_file.display(),
            "Avahi mDNS service file written — station discoverable as birdnet.local"
        ),
        Err(e) => tracing::debug!(
            error = %e,
            "Could not write Avahi service file (non-fatal, run as root to enable mDNS)"
        ),
    }
}
