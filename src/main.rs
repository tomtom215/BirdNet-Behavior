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
use tracing_subscriber::EnvFilter;

/// BirdNet-Behavior bird detection and analytics system.
#[derive(Parser, Debug)]
#[command(name = "birdnet-behavior", version, about)]
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
}

#[tokio::main]
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

    // Load configuration
    let config = birdnet_core::config::Config::load_from(&cli.config)?;
    tracing::info!(
        model = config.get_or("MODEL", "unknown"),
        "configuration loaded"
    );

    // Database maintenance commands
    if cli.check_db {
        return run_integrity_check(&config);
    }
    if cli.backup_db {
        return run_backup(&config);
    }

    // Database resilience: check and recover on startup
    let db_path = db_path_from_config(&config);
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");

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

    // Start web server
    let addr: std::net::SocketAddr = cli.listen.parse()?;
    let server_config = birdnet_web::server::ServerConfig {
        addr,
        db_path: db_path.clone(),
    };

    tracing::info!(addr = %addr, "starting web server");
    birdnet_web::server::start(server_config).await?;

    Ok(())
}

fn db_path_from_config(config: &birdnet_core::config::Config) -> PathBuf {
    config.get("DB_PATH").map_or_else(
        || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/pi".into());
            PathBuf::from(format!("{home}/BirdNet-Behavior/birds.db"))
        },
        PathBuf::from,
    )
}

fn run_integrity_check(
    config: &birdnet_core::config::Config,
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

fn run_backup(config: &birdnet_core::config::Config) -> Result<(), Box<dyn std::error::Error>> {
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
