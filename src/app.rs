//! Server orchestration — everything the binary does once it has decided to
//! run the detection daemon + web server.
//!
//! Split out of `main.rs` so the entry point stays a thin, decision-only
//! shell. The decision (`dispatch_subcommand`) is pure and unit-tested; the
//! orchestration here is inherently I/O-bound — it binds a TCP listener,
//! spawns background tasks, and serves until a shutdown signal — so it can
//! only be exercised by the end-to-end / subprocess layers (ADR-16 layers
//! 3–4), never a unit test. Keeping the two apart is what lets `src/main.rs`
//! carry real unit-test coverage instead of being a 0 %-covered black hole.

use crate::cli::Cli;
use crate::{capture, daemon, helpers, integrations, maintenance, sd_notify, weekly_report};

/// Run the detection daemon and web server until a shutdown signal arrives.
///
/// Called from `main` only for [`crate::Action::RunServer`]; the maintenance
/// and doctor short-circuits are handled before this is reached. Takes
/// ownership of the parsed `cli` and the loaded `config` (already resolved by
/// `main`).
#[allow(clippy::too_many_lines)]
pub async fn run(
    cli: Cli,
    config: Option<birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Startup database resilience check.
    let db_path = helpers::db_path_from_config(config.as_ref());
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
            Err(e) => {
                // Corrupt and unrecoverable (no good backup). Never write to a
                // corrupt database: quarantine it for offline recovery and
                // start fresh, so an unattended station keeps recording rather
                // than refusing to boot. Only refuse to start if we cannot even
                // move the corrupt file aside.
                tracing::error!(
                    error = %e,
                    path = %db_path.display(),
                    "database is corrupt and no good backup exists; quarantining before starting fresh"
                );
                let quarantined = birdnet_db::resilience::quarantine_corrupt_database(&db_path)
                    .map_err(|qe| {
                        format!(
                            "database is corrupt and could not be quarantined ({qe}); refusing to \
                             start to avoid writing to a corrupt database — restore a backup or move \
                             {} aside manually",
                            db_path.display()
                        )
                    })?;
                tracing::error!(
                    quarantined_to = %quarantined.display(),
                    "corrupt database quarantined; starting with a fresh database. Restore a backup \
                     or recover the quarantined file to keep historical detections"
                );
            }
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
    let state = helpers::init_image_cache(state, &cli, config.as_ref());
    let state = if let Some(ref dir) = cli.custom_image_dir {
        tracing::info!(path = %dir.display(), "custom species image directory configured");
        state.with_custom_image_dir(dir.clone())
    } else {
        state
    };
    let state = helpers::init_audio_source(state, &cli, config.as_ref());
    let state = helpers::init_site_name(state, &cli, config.as_ref());
    let state = if cli.info_site == "ebird" {
        state
    } else {
        state.with_info_site(cli.info_site.clone())
    };
    let state = helpers::init_i18n(state, &cli, config.as_ref());

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
    let _disk_manager_thread = helpers::start_disk_manager(&cli, config.as_ref(), &state);
    let _capture_handle = capture::start_capture_manager(&cli, config.as_ref(), state.metrics());

    let daemon_handle = if cli.web_only {
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
    helpers::maybe_install_avahi_service(addr.port(), site_name);

    // Start the web server.
    let auth_config = integrations::create_auth_config(config.as_ref());
    tracing::info!(addr = %addr, "starting web server");
    let metrics_for_watchdog = state.metrics();
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

    // Periodic database maintenance (VACUUM, integrity check, backup rotation).
    // No-op when the DB does not exist yet.
    maintenance::spawn_database_maintenance(db_path.clone(), backup_dir.clone());

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // The web server is bound — notify systemd that startup is complete and
    // begin pinging the watchdog. If we are not running under systemd these
    // are no-ops.
    sd_notify::ready();
    // Gate the watchdog on detection-loop progress: if the pipeline hangs, the
    // heartbeat stops advancing, the pinger withholds WATCHDOG=1, and systemd
    // restarts us instead of leaving a frozen daemon running. In web-only mode
    // there is no detection loop, so the pinger falls back to unconditional pings.
    let detection_heartbeat = daemon_handle
        .as_ref()
        .map(birdnet_core::detection::daemon::DaemonHandle::heartbeat);
    sd_notify::spawn_watchdog_pinger(Some(metrics_for_watchdog), detection_heartbeat);

    // Use `into_make_service_with_connect_info` so the per-IP rate limiter
    // can read the client socket address from request extensions.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    sd_notify::stopping();
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
