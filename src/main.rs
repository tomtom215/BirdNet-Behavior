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
mod helpers;
mod integrations;
mod weekly_report;

use clap::Parser;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, reload};

use cli::Cli;
use helpers::{
    db_path_from_config, init_audio_source, init_i18n, init_image_cache, init_site_name,
    maybe_install_avahi_service, run_backup, run_integrity_check, start_disk_manager,
};

/// Default log filter when `RUST_LOG` is not set.
const DEFAULT_LOG_FILTER: &str = "info,birdnet_behavior=debug";

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use a reloadable filter so SIGHUP can change the log level at runtime.
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Spawn SIGHUP handler for runtime log level changes.
    // Usage: set RUST_LOG env var then `kill -HUP <pid>`.
    #[cfg(unix)]
    {
        let handle = reload_handle;
        tokio::spawn(async move {
            let mut sighup = tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::hangup(),
            )
            .expect("failed to install SIGHUP handler");
            loop {
                sighup.recv().await;
                let new_filter = EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
                match handle.reload(new_filter) {
                    Ok(()) => tracing::info!("log filter reloaded via SIGHUP"),
                    Err(e) => tracing::error!(error = %e, "failed to reload log filter"),
                }
            }
        });
    }

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
    let state = helpers::build_state_with_analytics(&cli, config.as_ref(), &server_config)?;

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

    // Initialize all optional subsystems.
    let state = init_image_cache(state, &cli, config.as_ref());
    let state = if let Some(ref dir) = cli.custom_image_dir {
        tracing::info!(path = %dir.display(), "custom species image directory configured");
        state.with_custom_image_dir(dir.clone())
    } else {
        state
    };
    let state = init_audio_source(state, &cli, config.as_ref());
    let state = init_site_name(state, &cli, config.as_ref());
    let state = if cli.info_site == "ebird" {
        state
    } else {
        state.with_info_site(cli.info_site.clone())
    };
    let state = init_i18n(state, &cli, config.as_ref());

    let broadcast = state.detection_broadcast();

    // Create integration clients.
    let apprise_client = integrations::create_apprise_client(&cli, config.as_ref());
    let birdweather_client = integrations::create_birdweather_client(&cli, config.as_ref());
    let email_notifier = integrations::create_email_notifier(&state);
    let heartbeat_client = integrations::create_heartbeat_client(&cli, config.as_ref());
    let mqtt_client = integrations::create_mqtt_client(&cli, config.as_ref());
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

    // Start background subsystems.
    let _disk_manager_thread = start_disk_manager(&cli, config.as_ref(), &state);
    let _capture_managers = capture::start_capture_manager(&cli, config.as_ref());

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
            mqtt_client,
            notification_filter,
            notification_template,
        )
    };

    // Register Avahi mDNS service for zero-config local discovery.
    let site_name = cli.site_name.as_deref().unwrap_or("BirdNet-Behavior");
    maybe_install_avahi_service(addr.port(), site_name);

    // Start the web server.
    let auth_config = integrations::create_auth_config(config.as_ref());
    tracing::info!(addr = %addr, "starting web server");
    let app = birdnet_web::server::build_router_with_auth(state, auth_config);

    // Publish Home Assistant MQTT auto-discovery if configured.
    if let Some(ref mqtt) = integrations::get_mqtt_client_ref(&cli, config.as_ref()) {
        integrations::publish_ha_discovery(mqtt, &cli, config.as_ref());
    }

    // Spawn daily auto-update check (logs result, does not auto-apply).
    tokio::spawn(async {
        // Wait 60 seconds after startup before first check.
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let current_version = env!("CARGO_PKG_VERSION");
        loop {
            match tokio::task::spawn_blocking(move || {
                birdnet_integrations::auto_update::check_for_update(current_version)
            })
            .await
            {
                Ok(Ok(info)) => {
                    if info.update_available {
                        tracing::info!(
                            current = %info.current_version,
                            latest = %info.latest_version,
                            "new version available — use the admin panel to update"
                        );
                    } else {
                        tracing::debug!("auto-update check: already up to date");
                    }
                }
                Ok(Err(e)) => {
                    tracing::debug!(error = %e, "auto-update check failed (non-fatal)");
                }
                Err(e) => {
                    tracing::debug!(error = %e, "auto-update check task panicked");
                }
            }
            // Check once every 24 hours.
            tokio::time::sleep(std::time::Duration::from_secs(86_400)).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Use `into_make_service_with_connect_info` so the per-IP rate limiter
    // can read the client socket address from request extensions.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("BirdNet-Behavior stopped");
    Ok(())
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
