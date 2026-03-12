//! BirdNet-Behavior: Real-time acoustic bird classification with behavioral analytics.
//!
//! Single binary entry point that starts all subsystems:
//! - Detection pipeline (audio capture -> ML inference -> reporting)
//! - Web server (REST API, WebSocket, HTMX)
//! - Database management (`SQLite` operational + `DuckDB` analytics)
//! - Behavioral analytics (duckdb-behavioral extension)
//! - External integrations (`BirdWeather`, notifications)

use clap::Parser;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use tracing_subscriber::EnvFilter;

/// BirdNet-Behavior bird detection and analytics system.
#[derive(Parser, Debug)]
#[command(name = "birdnet-behavior", version, about)]
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    /// Path to configuration file.
    #[arg(
        short,
        long,
        default_value = "/etc/birdnet/birdnet.conf",
        env = "BIRDNET_CONFIG"
    )]
    config: PathBuf,

    /// Web server listen address.
    #[arg(long, default_value = "127.0.0.1:8502", env = "BIRDNET_LISTEN")]
    listen: String,

    /// Run only the web server (skip analysis daemon).
    #[arg(long)]
    web_only: bool,

    /// Run database integrity check and exit.
    #[arg(long)]
    check_db: bool,

    /// Create database backup and exit.
    #[arg(long)]
    backup_db: bool,

    /// Path to the ONNX model file (overrides config).
    #[arg(long, env = "BIRDNET_MODEL")]
    model: Option<PathBuf>,

    /// Path to the species labels file (overrides config).
    #[arg(long, env = "BIRDNET_LABELS")]
    labels: Option<PathBuf>,

    /// Directory to watch for new audio files (overrides config).
    #[arg(long, env = "BIRDNET_WATCH_DIR")]
    watch_dir: Option<PathBuf>,

    /// Process audio files already present in watch directory on startup.
    #[arg(long)]
    process_existing: bool,

    /// Path to the `DuckDB` analytics database file (enables behavioral analytics).
    ///
    /// When set, a file-backed `DuckDB` database is opened at this path for
    /// behavioral analytics queries (sessionize, retention, funnel, etc.).
    /// The file is created if it doesn't exist.
    #[arg(long, env = "BIRDNET_ANALYTICS_DB")]
    analytics_db: Option<PathBuf>,

    /// Apprise notification server URL (e.g., `http://localhost:8000`).
    ///
    /// When set, push notifications are sent for high-confidence detections
    /// via the Apprise REST API. Configure notification channels in Apprise.
    #[arg(long, env = "BIRDNET_APPRISE_URL")]
    apprise_url: Option<String>,

    /// Minimum confidence threshold for Apprise notifications (0.0 - 1.0).
    #[arg(long, default_value = "0.8", env = "BIRDNET_NOTIFY_CONFIDENCE")]
    notify_confidence: f32,

    /// `BirdWeather` station token for uploading detections.
    ///
    /// When set, every detection is posted to `app.birdweather.com`.
    /// Get a station token from the `BirdWeather` app settings.
    #[arg(long, env = "BIRDNET_BIRDWEATHER_TOKEN")]
    birdweather_token: Option<String>,

    /// Station latitude for `BirdWeather` uploads.
    #[arg(long, env = "BIRDNET_LATITUDE")]
    latitude: Option<f64>,

    /// Station longitude for `BirdWeather` uploads.
    #[arg(long, env = "BIRDNET_LONGITUDE")]
    longitude: Option<f64>,

    /// Directory for caching species images from Wikipedia.
    ///
    /// When set, species thumbnail images are fetched from Wikipedia and
    /// cached locally for offline/air-gapped display on the dashboard.
    #[arg(long, env = "BIRDNET_IMAGE_CACHE_DIR")]
    image_cache_dir: Option<PathBuf>,

    /// ALSA device for microphone capture (e.g., `plughw:1,0`).
    ///
    /// When set, BirdNet-Behavior manages audio recording directly using
    /// `arecord`, producing segments in the watch directory for analysis.
    #[arg(long, env = "BIRDNET_ALSA_DEVICE")]
    alsa_device: Option<String>,

    /// RTSP URL for audio capture (e.g., `rtsp://camera.local:554/stream`).
    ///
    /// When set, BirdNet-Behavior captures audio from the RTSP stream via
    /// `ffmpeg`, producing segments in the watch directory for analysis.
    #[arg(long, env = "BIRDNET_RTSP_URL")]
    rtsp_url: Option<String>,

    /// Duration of each recording segment in seconds (default: 15).
    #[arg(long, default_value = "15", env = "BIRDNET_SEGMENT_DURATION")]
    segment_duration: u32,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
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

    // Load configuration (optional -- may not exist in fresh installs)
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

    // Database maintenance commands
    if cli.check_db {
        return run_integrity_check(config.as_ref());
    }
    if cli.backup_db {
        return run_backup(config.as_ref());
    }

    // Database resilience: check and recover on startup
    let db_path = db_path_from_config(config.as_ref());
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");

    // Only run recovery if the database file exists
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
                tracing::error!(error = %e, "database recovery failed");
            }
        }
    }

    // Start web server
    let addr: std::net::SocketAddr = cli.listen.parse()?;
    let server_config = birdnet_web::server::ServerConfig {
        addr,
        db_path: db_path.clone(),
    };

    // Create app state (includes WebSocket broadcast channel and optional DuckDB)
    #[cfg(feature = "analytics")]
    let state = {
        // Resolve analytics DB path from CLI, config, or derive from SQLite path
        let analytics_path = cli
            .analytics_db
            .clone()
            .or_else(|| config.as_ref()?.get("ANALYTICS_DB_PATH").map(PathBuf::from));

        if let Some(ref analytics_path) = analytics_path {
            tracing::info!(path = %analytics_path.display(), "enabling DuckDB analytics");
            birdnet_web::state::AppState::new_with_analytics(
                server_config.db_path.clone(),
                analytics_path,
            )
            .map_err(|e| format!("database error: {e}"))?
        } else {
            birdnet_web::state::AppState::new(server_config.db_path.clone())
                .map_err(|e| format!("database error: {e}"))?
        }
    };

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

    // Initialize species image cache (if configured)
    let state = if let Some(ref cache_dir) = cli
        .image_cache_dir
        .clone()
        .or_else(|| config.as_ref()?.get("IMAGE_CACHE_DIR").map(PathBuf::from))
    {
        match birdnet_integrations::species_images::ImageCache::new(cache_dir) {
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
    } else {
        state
    };

    let broadcast = state.detection_broadcast();

    // Create integration clients (if configured)
    let apprise_client = create_apprise_client(&cli, config.as_ref());
    let birdweather_client = create_birdweather_client(&cli, config.as_ref());

    // Start audio capture if configured (managed recording)
    let _capture_manager = start_capture_manager(&cli, config.as_ref());

    // Start detection daemon if not in web-only mode and model is available
    let _daemon_handle = if cli.web_only {
        tracing::info!("running in web-only mode (no detection daemon)");
        None
    } else {
        start_detection_daemon(
            &cli,
            config.as_ref(),
            state.clone(),
            broadcast,
            apprise_client,
            birdweather_client,
        )
    };

    // Configure authentication (optional)
    let auth_config = create_auth_config(config.as_ref());

    // Start the web server (blocks until shutdown)
    tracing::info!(addr = %addr, "starting web server");
    let app = birdnet_web::server::build_router_with_auth(state, auth_config);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("BirdNet-Behavior stopped");
    Ok(())
}

/// Start the detection daemon in a background thread.
///
/// Returns the daemon handle, or None if the model/labels are not configured.
fn start_detection_daemon(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
) -> Option<birdnet_core::detection::daemon::DaemonHandle> {
    // Resolve paths from CLI flags or config
    let model_path = cli
        .model
        .clone()
        .or_else(|| config?.get("MODEL_PATH").map(PathBuf::from));

    let labels_path = cli
        .labels
        .clone()
        .or_else(|| config?.get("LABELS_PATH").map(PathBuf::from));

    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from));

    let (Some(model_path), Some(labels_path), Some(watch_dir)) =
        (model_path, labels_path, watch_dir)
    else {
        tracing::info!("detection daemon not started: model, labels, or watch_dir not configured");
        tracing::info!(
            "use --model, --labels, --watch-dir flags or set MODEL_PATH, LABELS_PATH, RECS_DIR in config"
        );
        return None;
    };

    // Build daemon config
    let sensitivity = config
        .and_then(|c| c.get_parsed::<f32>("SENSITIVITY").ok())
        .unwrap_or(1.0);

    let confidence = config
        .and_then(|c| c.get_parsed::<f32>("CONFIDENCE").ok())
        .unwrap_or(0.25);

    let daemon_config = birdnet_core::detection::daemon::DaemonConfig {
        watch_dir: watch_dir.clone(),
        model_path,
        labels_path,
        pipeline: birdnet_core::detection::pipeline::PipelineConfig {
            watch_dir,
            ..birdnet_core::detection::pipeline::PipelineConfig::default()
        },
        model: birdnet_core::inference::model::ModelConfig {
            sensitivity,
            confidence_threshold: confidence,
            ..birdnet_core::inference::model::ModelConfig::default()
        },
        process_existing: cli.process_existing,
    };

    // Create event channel
    let (event_tx, event_rx) = mpsc::channel();

    // Start daemon
    match birdnet_core::detection::daemon::run_daemon(&daemon_config, event_tx) {
        Ok(handle) => {
            tracing::info!("detection daemon started");

            // Capture the tokio runtime handle for async notification sends
            let rt_handle = tokio::runtime::Handle::current();

            // Spawn event processor on a blocking thread (it uses std::mpsc::recv)
            tokio::task::spawn_blocking(move || {
                event_processor(event_rx, state, broadcast, apprise, birdweather, rt_handle);
            });

            Some(handle)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to start detection daemon");
            None
        }
    }
}

/// Bridge detection events from the daemon to database inserts and WebSocket broadcasts.
///
/// Takes ownership of `state` and `broadcast` because this function runs on a
/// `spawn_blocking` thread and needs to own the `Arc`-backed handles.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
fn event_processor(
    event_rx: mpsc::Receiver<birdnet_core::detection::daemon::DetectionEvent>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    rt_handle: tokio::runtime::Handle,
) {
    tracing::debug!("event processor started");

    loop {
        // Receive from std mpsc (blocking -- but this is in a spawned task)
        let Ok(event) = event_rx.recv() else {
            tracing::info!("event channel closed, stopping event processor");
            break;
        };

        let detection = &event.detection;

        // Insert into database
        let week_str = detection.week.to_string();
        let file_str = event.source_file.to_string_lossy();
        let record = birdnet_db::sqlite::DetectionRecord {
            date: &detection.date,
            time: &detection.time,
            sci_name: &detection.scientific_name,
            com_name: &detection.common_name,
            confidence: f64::from(detection.confidence),
            lat: "",
            lon: "",
            cutoff: "",
            week: &week_str,
            sensitivity: "",
            overlap: "",
            file_name: &file_str,
        };

        let db_result = state.with_db(|conn| birdnet_db::sqlite::insert_detection(conn, &record));

        if let Err(e) = db_result {
            tracing::warn!(error = %e, "failed to insert detection into database");
        }

        // Also insert into DuckDB analytics (if enabled)
        #[cfg(feature = "analytics")]
        if state.has_analytics() {
            let insert_result = state.with_analytics(|adb| {
                adb.insert_detection(
                    &detection.date,
                    &detection.time,
                    &detection.scientific_name,
                    &detection.common_name,
                    f64::from(detection.confidence),
                    &file_str,
                )
            });
            if let Some(Err(e)) = insert_result {
                tracing::warn!(error = %e, "failed to insert detection into DuckDB");
            }
        }

        // Broadcast to WebSocket clients
        let ws_event = birdnet_web::routes::websocket::WsDetectionEvent {
            event: "detection",
            common_name: detection.common_name.clone(),
            scientific_name: detection.scientific_name.clone(),
            confidence: detection.confidence,
            date: detection.date.clone(),
            time: detection.time.clone(),
            start: detection.start,
            stop: detection.stop,
        };

        broadcast.send(&ws_event);

        // Send Apprise push notification (if configured and detection qualifies)
        if let Some(ref apprise) = apprise {
            let should_send = apprise
                .blocking_lock()
                .should_notify(&detection.common_name, detection.confidence);

            if should_send {
                let species = detection.common_name.clone();
                let confidence = detection.confidence;
                let date = detection.date.clone();
                let time = detection.time.clone();
                let client = Arc::clone(apprise);

                rt_handle.spawn(async move {
                    let result = client
                        .lock()
                        .await
                        .notify_detection(&species, confidence, &date, &time)
                        .await;

                    if let Err(e) = result {
                        tracing::warn!(
                            error = %e,
                            species = %species,
                            "failed to send Apprise notification"
                        );
                    } else {
                        tracing::debug!(species = %species, "Apprise notification sent");
                    }
                });
            }
        }

        // Post detection to BirdWeather (if configured)
        if let Some(ref bw) = birdweather {
            let post = birdnet_integrations::birdweather::DetectionPost {
                timestamp: format!("{}T{}Z", detection.date, detection.time),
                common_name: detection.common_name.clone(),
                scientific_name: detection.scientific_name.clone(),
                confidence: detection.confidence,
                lat: bw.coordinates().0,
                lon: bw.coordinates().1,
            };
            let client = bw.clone();

            rt_handle.spawn(async move {
                if let Err(e) = client.post_detection(&post).await {
                    tracing::warn!(
                        error = %e,
                        species = %post.common_name,
                        "failed to post detection to BirdWeather"
                    );
                } else {
                    tracing::debug!(
                        species = %post.common_name,
                        "detection posted to BirdWeather"
                    );
                }
            });
        }

        tracing::debug!(
            species = %detection.common_name,
            confidence = format!("{:.0}%", detection.confidence * 100.0),
            latency_ms = event.latency_ms,
            ws_clients = broadcast.client_count(),
            "event processed"
        );
    }
}

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
            tracing::error!("database integrity check FAILED - corruption detected");
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

/// Type alias for the shared Apprise client handle.
type AppriseHandle = Arc<tokio::sync::Mutex<birdnet_integrations::apprise::Client>>;

/// Create an Apprise notification client from CLI flags and/or config file values.
///
/// Returns `None` if no Apprise URL is configured, meaning notifications are disabled.
fn create_apprise_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<AppriseHandle> {
    let apprise_url = cli
        .apprise_url
        .clone()
        .or_else(|| config?.get("APPRISE_URL").map(String::from));

    let url = apprise_url?;

    // Build notification filter config from CLI and config file
    let min_confidence = if (cli.notify_confidence - 0.8).abs() > f32::EPSILON {
        // CLI flag was explicitly set (differs from default)
        cli.notify_confidence
    } else {
        config
            .and_then(|c| c.get_parsed::<f32>("APPRISE_MIN_CONFIDENCE").ok())
            .unwrap_or(cli.notify_confidence)
    };

    let cooldown_secs = config
        .and_then(|c| c.get_parsed::<u64>("APPRISE_COOLDOWN").ok())
        .unwrap_or(300);

    let species_watchlist = config
        .and_then(|c| c.get("APPRISE_WATCHLIST"))
        .map(|list| {
            list.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let notify_config = birdnet_integrations::apprise::NotifyConfig {
        min_confidence,
        species_watchlist,
        cooldown: std::time::Duration::from_secs(cooldown_secs),
    };

    match birdnet_integrations::apprise::Client::new(&url, notify_config) {
        Ok(client) => {
            tracing::info!(
                url = %url,
                min_confidence = %min_confidence,
                cooldown_secs = cooldown_secs,
                "Apprise notifications enabled"
            );
            Some(Arc::new(tokio::sync::Mutex::new(client)))
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to create Apprise client");
            None
        }
    }
}

/// Create a `BirdWeather` client from CLI flags and/or config file values.
///
/// Returns `None` if no station token is configured.
fn create_birdweather_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_integrations::birdweather::Client> {
    let token = cli
        .birdweather_token
        .clone()
        .or_else(|| config?.get("BIRDWEATHER_TOKEN").map(String::from));

    let token = token?;

    let lat = cli
        .latitude
        .or_else(|| config?.get_parsed::<f64>("LATITUDE").ok())
        .unwrap_or(0.0);

    let lon = cli
        .longitude
        .or_else(|| config?.get_parsed::<f64>("LONGITUDE").ok())
        .unwrap_or(0.0);

    match birdnet_integrations::birdweather::Client::new(&token, lat, lon) {
        Ok(client) => {
            tracing::info!(lat = lat, lon = lon, "BirdWeather uploads enabled");
            Some(client)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to create BirdWeather client");
            None
        }
    }
}

/// Start a managed audio capture process from CLI/config settings.
///
/// Returns the `CaptureManager` handle (keeps recording alive until dropped).
fn start_capture_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_core::audio::capture::CaptureManager> {
    use birdnet_core::audio::capture::{
        AudioFormat, CaptureManager, CaptureSource, RecordingConfig,
    };

    // Determine output directory (same as watch_dir)
    let output_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp/StreamData"));

    // Determine capture source from CLI flags
    let alsa_device = cli
        .alsa_device
        .clone()
        .or_else(|| config?.get("ALSA_CARD").map(String::from));

    let rtsp_url = cli
        .rtsp_url
        .clone()
        .or_else(|| config?.get("RTSP_URL").map(String::from));

    let source = alsa_device.map_or_else(
        || {
            rtsp_url.map(|url| CaptureSource::Rtsp {
                url,
                stream_id: "rtsp".to_string(),
            })
        },
        |device| {
            Some(CaptureSource::Microphone {
                device,
                sample_rate: 48000,
                channels: 1,
            })
        },
    );

    let source = source?;

    let recording_config = RecordingConfig {
        source,
        output_dir,
        segment_duration_secs: cli.segment_duration,
        format: AudioFormat::Wav,
    };

    let mut manager = CaptureManager::new(recording_config);

    match manager.start() {
        Ok(()) => {
            tracing::info!("audio capture started");
            Some(manager)
        }
        Err(e) => {
            tracing::warn!(error = %e, "audio capture not started (non-fatal)");
            None
        }
    }
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

/// Create an authentication config from the config file.
///
/// Looks for `CADDY_PWD` (password) and defaults username to "birdnet"
/// to match the BirdNET-Pi Caddy setup. Returns `None` if no password is set.
fn create_auth_config(
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_web::auth::AuthConfig> {
    let password = config?.get("CADDY_PWD")?;
    let username = config
        .and_then(|c| c.get("CADDY_USER"))
        .unwrap_or("birdnet");

    let auth = birdnet_web::auth::AuthConfig::new(username, password)?;

    tracing::info!(username = %username, "basic auth enabled");
    Some(auth)
}
