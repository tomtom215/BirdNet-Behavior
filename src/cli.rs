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
    ///
    /// For a single stream. Use `--rtsp-urls` for multiple streams.
    #[arg(long, env = "BIRDNET_RTSP_URL")]
    pub rtsp_url: Option<String>,

    /// Comma-separated RTSP URLs for multi-stream capture.
    ///
    /// Each URL gets its own independent capture pipeline with filenames
    /// prefixed `RTSP_1-`, `RTSP_2-`, etc. Overrides `--rtsp-url` if set.
    #[arg(long, env = "BIRDNET_RTSP_URLS", value_delimiter = ',')]
    pub rtsp_urls: Vec<String>,

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

    /// Path to the metadata ONNX model for species occurrence filtering.
    ///
    /// When set, the metadata model predicts which species are likely present
    /// at the station's location and time of year, filtering out unlikely species.
    #[arg(long, env = "BIRDNET_METADATA_MODEL")]
    pub metadata_model: Option<PathBuf>,

    /// Species frequency threshold for the metadata model filter (0.0-1.0).
    ///
    /// Species with occurrence probability below this threshold are filtered out.
    /// Lower values allow more species through; higher values are more restrictive.
    #[arg(long, default_value = "0.03", env = "BIRDNET_SF_THRESH")]
    pub sf_thresh: f32,

    /// Privacy filter threshold for human voice detection (0.0 = disabled).
    ///
    /// When enabled, audio chunks containing human voice are suppressed along
    /// with adjacent chunks. Typical values: 0.01-0.03.
    #[arg(long, default_value = "0.0", env = "BIRDNET_PRIVACY_THRESHOLD")]
    pub privacy_threshold: f32,

    /// Analysis window overlap in seconds (0.0-2.9, default 0.0).
    ///
    /// Controls how much consecutive 3-second analysis windows overlap.
    /// Higher overlap increases sensitivity at the cost of more CPU time.
    /// BirdNET-Pi equivalent: OVERLAP config option.
    #[arg(long, default_value = "0.0", env = "BIRDNET_OVERLAP")]
    pub overlap: f32,

    /// Custom site name displayed in page titles and header.
    ///
    /// Replaces the default "BirdNet-Behavior" branding in the web UI.
    #[arg(long, env = "BIRDNET_SITENAME")]
    pub site_name: Option<String>,

    /// Language code for species name translation (e.g., "de", "fr", "ja").
    ///
    /// When set, species common names are translated to the specified language
    /// using BirdNET label files. Default: "en" (English).
    #[arg(long, default_value = "en", env = "BIRDNET_LANG")]
    pub lang: String,

    /// Directory containing BirdNET language label files for i18n.
    ///
    /// Label files should be named like `labels_de.txt`, `labels_fr.txt`, etc.
    #[arg(long, env = "BIRDNET_LABELS_DIR")]
    pub labels_dir: Option<PathBuf>,

    /// eBird/AllAboutBirds species info links: "ebird", "allaboutbirds", or "none".
    #[arg(long, default_value = "ebird", env = "BIRDNET_INFO_SITE")]
    pub info_site: String,

    /// Audio format for extracted detection clips: "wav", "mp3", "flac", or "ogg".
    ///
    /// Non-WAV formats require ffmpeg or sox to be installed.
    /// BirdNET-Pi equivalent: AUDIOFMT config option.
    #[arg(long, default_value = "wav", env = "BIRDNET_AUDIO_FORMAT")]
    pub audio_format: String,

    /// Maximum number of extracted recordings kept per species (0 = unlimited).
    ///
    /// When set, the oldest files beyond this limit are deleted automatically.
    /// BirdNET-Pi equivalent: MAX_FILES_SPECIES config option.
    #[arg(long, default_value = "0", env = "BIRDNET_MAX_FILES_PER_SPECIES")]
    pub max_files_per_species: u32,

    /// Comma-separated paths to exclude from disk usage monitoring.
    ///
    /// Files under these paths are never auto-purged.
    #[arg(long, env = "BIRDNET_DISK_EXCLUDE", value_delimiter = ',')]
    pub disk_exclude: Vec<std::path::PathBuf>,

    /// Directory containing custom species images (checked before Wikipedia cache).
    ///
    /// Files should be named `{lowercase_sci_name_with_underscores}.jpg`, e.g.
    /// `turdus_merula.jpg`. BirdNET-Pi equivalent: CUSTOM_IMAGE directory.
    #[arg(long, env = "BIRDNET_CUSTOM_IMAGE_DIR")]
    pub custom_image_dir: Option<PathBuf>,

    /// Path to Apprise config file (alternative/addition to --apprise-url).
    ///
    /// When set, uses the `apprise` CLI tool with `-c <file>` for notifications.
    /// BirdNET-Pi equivalent: APPRISE_CONFIG_FILE config option.
    #[arg(long, env = "BIRDNET_APPRISE_CONFIG")]
    pub apprise_config: Option<PathBuf>,

    /// Weekly report notification schedule.
    ///
    /// Send a weekly summary via Apprise on a fixed weekday.
    /// Values: "monday", "tuesday", ..., "sunday", or "disabled".
    /// BirdNET-Pi equivalent: weekly_report cron job.
    #[arg(long, default_value = "monday", env = "BIRDNET_WEEKLY_REPORT_SCHEDULE")]
    pub weekly_report_schedule: String,
}
