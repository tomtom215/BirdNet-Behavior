//! BirdNet-Behavior: Real-time acoustic bird classification with behavioral analytics.
//!
//! Single binary entry point that starts all subsystems:
//! - Detection pipeline (audio capture → ML inference → reporting)
//! - Web server (REST API, WebSocket, HTMX, admin panel)
//! - Database management (`SQLite` operational + `DuckDB` analytics)
//! - External integrations (`BirdWeather`, notifications)
//! - BirdNET-Pi migration tooling

mod capture;
mod cli;
mod daemon;
mod integrations;
mod weekly_report;

use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

use cli::Cli;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,birdnet_behavior=debug")),
        )
        .init();

    let cli = Cli::parse();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %cli.config.display(),
        "starting BirdNet-Behavior"
    );

    // Load configuration (optional — may not exist on fresh installs).
    let config = match birdnet_core::config::Config::load_from(&cli.config) {
        Ok(c) => {
            tracing::info!(model = c.get_or("MODEL", "unknown"), "configuration loaded");
            Some(c)
        }
        Err(e) => {
            tracing::warn!(error = %e, "config not loaded, using defaults");
            None
        }
    };

    // Database maintenance commands (run and exit).
    if cli.check_db {
        return run_integrity_check(config.as_ref());
    }
    if cli.backup_db {
        return run_backup(config.as_ref());
    }

    // Startup database resilience check.
    let db_path = db_path_from_config(config.as_ref());
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");

    if db_path.exists() {
        match birdnet_db::resilience::check_and_recover(&db_path, &backup_dir) {
            Ok(result) => {
                if result.action == birdnet_db::resilience::RecoveryAction::Recovered {
                    tracing::warn!(details = %result.details, "database recovered");
                } else {
                    tracing::info!(details = %result.details, "database healthy");
                }
            }
            Err(e) => tracing::error!(error = %e, "database recovery failed"),
        }
    }

    // Build app state.
    let addr: std::net::SocketAddr = cli.listen.parse()?;
    let server_config = birdnet_web::server::ServerConfig {
        addr,
        db_path: db_path.clone(),
    };

    #[cfg(feature = "analytics")]
    let state = build_state_with_analytics(&cli, config.as_ref(), &server_config)?;

    #[cfg(not(feature = "analytics"))]
    let state = {
        if cli.analytics_db.is_some() {
            tracing::warn!(
                "DuckDB analytics requested but not compiled in. Rebuild with --features analytics"
            );
        }
        birdnet_web::state::AppState::new(server_config.db_path.clone())
            .map_err(|e| format!("database error: {e}"))?
    };

    // Initialize species image cache.
    let state = init_image_cache(state, &cli, config.as_ref());

    // Wire custom image directory (shown before Wikipedia cache).
    let state = if let Some(ref dir) = cli.custom_image_dir {
        tracing::info!(path = %dir.display(), "custom species image directory configured");
        state.with_custom_image_dir(dir.clone())
    } else {
        state
    };

    // Wire audio source for live streaming (/stream endpoint).
    let state = init_audio_source(state, &cli, config.as_ref());

    // Wire custom site name (displayed in page titles).
    let state = init_site_name(state, &cli, config.as_ref());

    // Wire species info link site (eBird/AllAboutBirds).
    let state = if cli.info_site != "ebird" {
        state.with_info_site(cli.info_site.clone())
    } else {
        state
    };

    // Wire i18n language if not English.
    let state = init_i18n(state, &cli, config.as_ref());

    let broadcast = state.detection_broadcast();

    // Create integration clients.
    let apprise_client = integrations::create_apprise_client(&cli, config.as_ref());
    let birdweather_client = integrations::create_birdweather_client(&cli, config.as_ref());
    let email_notifier = integrations::create_email_notifier(&state);
    let heartbeat_client = integrations::create_heartbeat_client(&cli, config.as_ref());
    let notification_filter = integrations::create_notification_filter(&cli);
    let notification_template = integrations::create_notification_template(&cli, config.as_ref());

    // Start weekly report scheduler (if Apprise is configured).
    if let Some(ref apprise) = apprise_client {
        weekly_report::start_weekly_report_scheduler(
            &cli.weekly_report_schedule,
            std::sync::Arc::clone(apprise),
            state.clone(),
        );
    }

    // Start disk manager (monitors and purges old recordings).
    let _disk_manager_thread = start_disk_manager(&cli, config.as_ref(), &state);

    // Start audio capture (with recording schedule integration).
    let _capture_managers = capture::start_capture_manager(&cli, config.as_ref());

    // Start detection daemon (unless in web-only mode).
    let _daemon_handle = if cli.web_only {
        tracing::info!("running in web-only mode (no detection daemon)");
        None
    } else {
        daemon::start_detection_daemon(
            &cli,
            config.as_ref(),
            state.clone(),
            broadcast,
            apprise_client,
            birdweather_client,
            email_notifier,
            heartbeat_client,
            notification_filter,
            notification_template,
        )
    };

    // Start the web server.
    let auth_config = integrations::create_auth_config(config.as_ref());
    tracing::info!(addr = %addr, "starting web server");
    let app = birdnet_web::server::build_router_with_auth(state, auth_config);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("BirdNet-Behavior stopped");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn db_path_from_config(config: Option<&birdnet_core::config::Config>) -> PathBuf {
    config.and_then(|c| c.get("DB_PATH")).map_or_else(
        || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/pi".into());
            PathBuf::from(format!("{home}/BirdNet-Behavior/birds.db"))
        },
        PathBuf::from,
    )
}

fn run_integrity_check(
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

fn run_backup(
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

#[cfg(feature = "analytics")]
fn build_state_with_analytics(
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

fn init_image_cache(
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

fn init_i18n(
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

fn init_audio_source(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    // Prefer RTSP URL, then ALSA device, then config values.
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

fn init_site_name(
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
///
/// Resolves the monitored directory from CLI/config, populates `exclude_paths`
/// and `locked_file_names` from CLI flags and the database, then starts a
/// background thread running the disk manager loop.
///
/// Returns the thread handle (kept alive until dropped).
fn start_disk_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) -> Option<std::thread::JoinHandle<()>> {
    use birdnet_core::audio::capture::{DiskManager, DiskManagerConfig, FullDiskAction};

    let monitored_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))?;

    // Resolve per-species limit from CLI or config.
    let max_files_per_species = if cli.max_files_per_species > 0 {
        cli.max_files_per_species
    } else {
        config
            .and_then(|c| c.get_parsed::<u32>("MAX_FILES_SPECIES").ok())
            .unwrap_or(0)
    };

    // Resolve purge threshold from config (default 95).
    let purge_threshold = config
        .and_then(|c| c.get_parsed::<u8>("DISK_PURGE_THRESHOLD").ok())
        .unwrap_or(95);

    // Load locked file names from the database to protect them from purge.
    let locked_file_names = state
        .with_db(|conn| birdnet_db::sqlite::locked_file_names(conn).unwrap_or_default());

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

    // Leak the sender so the manager runs until process exit.
    std::mem::forget(stop_tx);

    Some(handle)
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received Ctrl+C"),
        () = terminate => tracing::info!("received SIGTERM"),
    }
}
