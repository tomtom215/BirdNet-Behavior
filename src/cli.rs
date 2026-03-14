//! CLI argument definitions for BirdNet-Behavior.

use clap::Parser;
use std::path::PathBuf;

/// BirdNet-Behavior bird detection and analytics system.
#[derive(Parser, Debug)]
#[command(name = "birdnet-behavior", version, about)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// Path to configuration file.
    #[arg(
        short,
        long,
        default_value = "/etc/birdnet/birdnet.conf",
        env = "BIRDNET_CONFIG"
    )]
    pub config: PathBuf,

    /// Web server listen address.
    #[arg(long, default_value = "127.0.0.1:8502", env = "BIRDNET_LISTEN")]
    pub listen: String,

    /// Run only the web server (skip analysis daemon).
    #[arg(long)]
    pub web_only: bool,

    /// Run database integrity check and exit.
    #[arg(long)]
    pub check_db: bool,

    /// Create database backup and exit.
    #[arg(long)]
    pub backup_db: bool,

    /// Path to the ONNX model file (overrides config).
    #[arg(long, env = "BIRDNET_MODEL")]
    pub model: Option<PathBuf>,

    /// Path to the species labels file (overrides config).
    #[arg(long, env = "BIRDNET_LABELS")]
    pub labels: Option<PathBuf>,

    /// Directory to watch for new audio files (overrides config).
    #[arg(long, env = "BIRDNET_WATCH_DIR")]
    pub watch_dir: Option<PathBuf>,

    /// Process audio files already present in watch directory on startup.
    #[arg(long)]
    pub process_existing: bool,

    /// Path to the `DuckDB` analytics database file (enables behavioral analytics).
    ///
    /// When set, a file-backed `DuckDB` database is opened at this path for
    /// behavioral analytics queries.  The file is created if it doesn't exist.
    #[arg(long, env = "BIRDNET_ANALYTICS_DB")]
    pub analytics_db: Option<PathBuf>,

    /// Apprise notification server URL (e.g., `http://localhost:8000`).
    #[arg(long, env = "BIRDNET_APPRISE_URL")]
    pub apprise_url: Option<String>,

    /// Minimum confidence threshold for Apprise notifications (0.0–1.0).
    #[arg(long, default_value = "0.8", env = "BIRDNET_NOTIFY_CONFIDENCE")]
    pub notify_confidence: f32,

    /// `BirdWeather` station token for uploading detections.
    #[arg(long, env = "BIRDNET_BIRDWEATHER_TOKEN")]
    pub birdweather_token: Option<String>,

    /// Station latitude for `BirdWeather` uploads.
    #[arg(long, env = "BIRDNET_LATITUDE")]
    pub latitude: Option<f64>,

    /// Station longitude for `BirdWeather` uploads.
    #[arg(long, env = "BIRDNET_LONGITUDE")]
    pub longitude: Option<f64>,

    /// Directory for caching species images from Wikipedia.
    #[arg(long, env = "BIRDNET_IMAGE_CACHE_DIR")]
    pub image_cache_dir: Option<PathBuf>,

    /// ALSA device for microphone capture (e.g., `plughw:1,0`).
    #[arg(long, env = "BIRDNET_ALSA_DEVICE")]
    pub alsa_device: Option<String>,

    /// RTSP URL for audio capture (e.g., `rtsp://camera.local:554/stream`).
    #[arg(long, env = "BIRDNET_RTSP_URL")]
    pub rtsp_url: Option<String>,

    /// Duration of each recording segment in seconds (default: 15).
    #[arg(long, default_value = "15", env = "BIRDNET_SEGMENT_DURATION")]
    pub segment_duration: u32,

    /// Recording schedule mode: "all-day" (24/7), "solar" (sunrise-to-sunset),
    /// or "fixed:HH:MM-HH:MM" (e.g., "fixed:06:00-20:00").
    #[arg(long, default_value = "all-day", env = "BIRDNET_RECORDING_SCHEDULE")]
    pub recording_schedule: String,

    /// Inhibit recording during night hours (requires --latitude and --longitude).
    #[arg(long, env = "BIRDNET_NIGHT_INHIBIT")]
    pub night_inhibit: bool,

    /// Minutes offset from sunrise/sunset for twilight recording (default: 30).
    #[arg(long, default_value = "30", env = "BIRDNET_TWILIGHT_OFFSET")]
    pub twilight_offset: u32,

    /// Heartbeat URL to ping after each analysis cycle (e.g., uptime monitoring).
    #[arg(long, env = "BIRDNET_HEARTBEAT_URL")]
    pub heartbeat_url: Option<String>,

    /// Notification trigger mode: "each", "new-species", "new-species-daily".
    #[arg(long, default_value = "each", env = "BIRDNET_NOTIFY_TRIGGER")]
    pub notify_trigger: String,

    /// Species to exclude from notifications (comma-separated scientific names).
    #[arg(long, env = "BIRDNET_NOTIFY_SPECIES_EXCLUDE")]
    pub notify_species_exclude: Option<String>,

    /// Only notify for these species (comma-separated scientific names).
    #[arg(long, env = "BIRDNET_NOTIFY_SPECIES_ONLY")]
    pub notify_species_only: Option<String>,

    /// Custom notification title template (supports $comname, $sciname, $confidence, etc.).
    #[arg(long, env = "BIRDNET_NOTIFY_TITLE_TEMPLATE")]
    pub notify_title_template: Option<String>,

    /// Custom notification body template (supports $comname, $sciname, $confidence, etc.).
    #[arg(long, env = "BIRDNET_NOTIFY_BODY_TEMPLATE")]
    pub notify_body_template: Option<String>,
}
