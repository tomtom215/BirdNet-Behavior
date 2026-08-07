//! CLI argument definitions for BirdNet-Behavior.

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, parser::ValueSource};
// Brings `.map` into scope for the custom `image_cache_dir` value parser.
use clap::builder::TypedValueParser as _;
use std::collections::HashSet;
use std::path::PathBuf;

/// The set of arguments the operator actually supplied, as opposed to the ones
/// `clap` filled in from `default_value`.
///
/// This exists because a flag with a `default_value` is indistinguishable, after
/// parsing, from one the operator typed: `cli.segment_duration` is `15` whether
/// that came from `--segment-duration 15` or from nobody saying anything. That
/// matters for every setting the admin UI can also set. The runtime resolves
/// those in the order *explicit CLI flag / env → admin settings (already layered
/// onto the config by `helpers::overlay_db_settings`) → config file → built-in
/// default*, and without this the first step always wins, so a value chosen in
/// the web form could never take effect.
///
/// The existing workarounds were per-flag sentinels — `disk_purge_threshold > 0`,
/// `(notify_confidence - 0.8).abs() > f32::EPSILON` — which only work when the
/// default happens to be an otherwise-invalid value, and silently do the wrong
/// thing when the operator explicitly types the default. Asking `clap` which
/// source a value came from is exact and works for every flag.
#[derive(Debug, Default, Clone)]
pub struct ExplicitArgs(HashSet<String>);

impl ExplicitArgs {
    /// Whether `id` was supplied on the command line or through its environment
    /// variable (as opposed to coming from `default_value`).
    ///
    /// `id` is the clap argument id, which for the derive API is the field name
    /// (`"segment_duration"`, not `"--segment-duration"`).
    #[must_use]
    pub fn has(&self, id: &str) -> bool {
        self.0.contains(id)
    }

    /// Build the set from parsed [`ArgMatches`].
    fn from_matches(matches: &ArgMatches) -> Self {
        Self(
            matches
                .ids()
                .map(clap::Id::as_str)
                .filter(|id| {
                    matches!(
                        matches.value_source(id),
                        Some(ValueSource::CommandLine | ValueSource::EnvVariable)
                    )
                })
                .map(ToOwned::to_owned)
                .collect(),
        )
    }

    /// Construct directly from argument ids. Test-only: production code always
    /// derives the set from real [`ArgMatches`].
    #[cfg(test)]
    pub(crate) fn from_ids<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(ids.into_iter().map(Into::into).collect())
    }
}

/// BirdNet-Behavior bird detection and analytics system.
#[derive(Parser, Debug)]
#[command(name = "birdnet-behavior", version, about)]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// Which arguments the operator actually supplied — see [`ExplicitArgs`].
    ///
    /// `#[arg(skip)]` keeps this out of the parser surface entirely: it is not a
    /// flag, takes no value, and never appears in `--help`. It is populated by
    /// [`Cli::parse_tracked`] / [`Cli::parse_tracked_from`] after parsing.
    #[arg(skip)]
    pub explicit: ExplicitArgs,

    /// Path to configuration file.
    #[arg(
        short,
        long,
        default_value = "/etc/birdnet/birdnet.conf",
        env = "BIRDNET_CONFIG"
    )]
    pub config: PathBuf,

    /// Web server listen address. Defaults to all interfaces so the dashboard
    /// is reachable on the LAN out of the box; the `/admin` panel is gated by a
    /// password (set `CADDY_PWD`). Bind `127.0.0.1:8502` to restrict to this host.
    #[arg(long, default_value = "0.0.0.0:8502", env = "BIRDNET_LISTEN")]
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

    /// Run the preflight diagnostic and exit.
    ///
    /// Validates the configuration, audio source, model file, database
    /// integrity, disk space, and tool dependencies. Returns:
    ///   - 0 — all checks passed
    ///   - 1 — at least one warning (system will run, features degraded)
    ///   - 2 — at least one error (system will not work until fixed)
    ///
    /// Run after install and any time the system behaves unexpectedly.
    #[arg(long, visible_alias = "preflight")]
    pub doctor: bool,

    /// Run the preflight diagnostic and emit a single-line JSON document.
    ///
    /// Same checks and exit codes as `--doctor`, but the output is a
    /// machine-readable JSON object with `summary` and `checks` fields.
    /// Use for monitoring scripts (Nagios / Zabbix / Prometheus textfile
    /// collector / Home Assistant command sensor).
    #[arg(long)]
    pub doctor_json: bool,

    /// Attempt safe automatic repairs, then run the diagnostic.
    ///
    /// Implies `--doctor` (use with `--doctor-json` for machine-readable
    /// output). Repairs are idempotent and never destructive — currently they
    /// create any missing configured directories (the recordings/watch
    /// directory and the image-cache directory), the most common cause of
    /// "service runs but nothing is recorded" (e.g. after a tmpfs reset on
    /// reboot). The diagnostic then runs as usual and reflects the repaired
    /// state. Anything that needs root (ownership, packages) is reported, not
    /// changed.
    #[arg(long)]
    pub fix: bool,

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

    /// Reinstall the behavioral `DuckDB` extension and exit.
    ///
    /// Force-downloads the latest `behavioral` build for the bundled `DuckDB`
    /// version from the community registry, loads it to verify, then exits.
    /// Requires `--analytics-db` (or `ANALYTICS_DB_PATH`) and network access.
    #[arg(long)]
    pub refresh_extension: bool,

    /// Make no unsolicited outbound connections.
    ///
    /// Turns off every network call the station would otherwise make on its
    /// own: the daily update check against `api.github.com` and the Wikipedia
    /// species-image downloads. Integrations you configure explicitly — Apprise,
    /// `BirdWeather`, MQTT, SMTP, the heartbeat ping, the weather poll — are
    /// left alone, because asking for one is a decision the operator already
    /// made.
    ///
    /// For metered or cellular links, air-gapped deployments, and institutional
    /// review, where "which hosts does this contact?" needs one answer rather
    /// than a per-feature audit. See `docs/book/getting-started/configuration.md`
    /// for the full egress list.
    #[arg(long, env = "BIRDNET_OFFLINE")]
    pub offline: bool,

    /// Skip the daily check for a new release.
    ///
    /// The station otherwise contacts `api.github.com` 60 seconds after start
    /// and every 24 hours after that, purely to log whether a newer version
    /// exists. It never installs anything on its own — updates are applied only
    /// from the admin panel. Implied by `--offline`.
    #[arg(long, env = "BIRDNET_NO_UPDATE_CHECK")]
    pub no_update_check: bool,

    /// Apprise notification server URL (e.g., `http://localhost:8000`).
    #[arg(long, env = "BIRDNET_APPRISE_URL")]
    pub apprise_url: Option<String>,

    /// Minimum confidence threshold for Apprise notifications (0.0–1.0).
    #[arg(long, default_value = "0.8", env = "BIRDNET_NOTIFY_CONFIDENCE")]
    pub notify_confidence: f32,

    /// Hours of zero detections before the deadman watchdog raises an alert.
    ///
    /// The end-to-end "is the station actually detecting?" check: when no
    /// detection has been recorded for this many hours, a loud warning is
    /// logged and (if Apprise is configured) a notification is sent — once
    /// per quiet episode, with a recovery notice when detections resume.
    /// `0` disables alerting; the freshness gauge
    /// (`birdnet_detection_silence_seconds`) is exported regardless.
    /// Defaults to 24; config-file key: `DEADMAN_HOURS`.
    #[arg(long, env = "BIRDNET_DEADMAN_HOURS")]
    pub deadman_hours: Option<u32>,

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
    ///
    /// Defaults to an `images/` directory beside the database when unset, so a
    /// stock install shows species photos out of the box. Pass an empty value
    /// (`--image-cache-dir ""`) to disable image caching entirely — no
    /// Wikipedia fetches, e.g. for air-gapped deployments.
    //
    // The explicit OsString→PathBuf parser is load-bearing: clap's stock
    // PathBuf parser rejects empty values, which made the documented
    // empty-string opt-out (and `BIRDNET_IMAGE_CACHE_DIR=`) unreachable —
    // `init_image_cache` treats an empty path as "disable" and could never
    // see one from the CLI/env.
    #[arg(
        long,
        env = "BIRDNET_IMAGE_CACHE_DIR",
        value_parser = clap::builder::OsStringValueParser::new().map(PathBuf::from)
    )]
    pub image_cache_dir: Option<PathBuf>,

    /// ALSA device for microphone capture (e.g., `plughw:1,0`).
    ///
    /// For a single microphone. Use `--alsa-devices` to capture from several
    /// local mics at once.
    #[arg(long, env = "BIRDNET_ALSA_DEVICE")]
    pub alsa_device: Option<String>,

    /// ALSA devices for multi-microphone capture (semicolon-separated, or
    /// repeat the flag).
    ///
    /// Each device gets its own independent capture pipeline with filenames
    /// prefixed `MIC_1-`, `MIC_2-`, etc. (and matching per-source health
    /// metrics). Overrides `--alsa-device` when set. A semicolon — not a comma —
    /// separates devices because ALSA names themselves contain commas (e.g.
    /// `plughw:1,0`). Config-file equivalent: `ALSA_CARDS` (semicolon-separated);
    /// `ALSA_CARD` remains the single-mic key.
    #[arg(long, env = "BIRDNET_ALSA_DEVICES", value_delimiter = ';')]
    pub alsa_devices: Vec<String>,

    /// PipeWire/PulseAudio source for microphone capture.
    ///
    /// Uses `ffmpeg -f pulse` which works with both native `PulseAudio` and
    /// `PipeWire` (via `pipewire-pulse`). Leave empty or use `default` for the
    /// system default source. Takes precedence over `--alsa-device` when set.
    /// BirdNET-Pi equivalent: ALSA device pointing to a `PulseAudio` sink.
    #[arg(long, env = "BIRDNET_PIPEWIRE_DEVICE")]
    pub pipewire_device: Option<String>,

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
    ///
    /// Applies to both ends of the day. Use `--pre-sunrise-offset` /
    /// `--post-sunset-offset` to set them independently; either one overrides
    /// this for its own end.
    #[arg(long, default_value = "30", env = "BIRDNET_TWILIGHT_OFFSET")]
    pub twilight_offset: u32,

    /// Minutes *before* sunrise at which recording starts.
    ///
    /// Overrides `--twilight-offset` for the morning end only. Dawn and dusk
    /// activity are rarely symmetric — the dawn chorus starts well before first
    /// light while evening song tails off quickly — so a station usually wants a
    /// longer pre-sunrise window than post-sunset one. Unset falls back to
    /// `--twilight-offset`.
    #[arg(long, env = "BIRDNET_PRE_SUNRISE_OFFSET")]
    pub pre_sunrise_offset: Option<u32>,

    /// Minutes *after* sunset at which recording stops.
    ///
    /// Overrides `--twilight-offset` for the evening end only. Unset falls back
    /// to `--twilight-offset`.
    #[arg(long, env = "BIRDNET_POST_SUNSET_OFFSET")]
    pub post_sunset_offset: Option<u32>,

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
    /// using `BirdNET` label files. Default: "en" (English).
    #[arg(long, default_value = "en", env = "BIRDNET_LANG")]
    pub lang: String,

    /// Directory containing `BirdNET` language label files for i18n.
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
    /// `BirdNET-Pi` equivalent: `MAX_FILES_SPECIES` config option.
    #[arg(long, default_value = "0", env = "BIRDNET_MAX_FILES_PER_SPECIES")]
    pub max_files_per_species: u32,

    /// Comma-separated paths to exclude from disk usage monitoring.
    ///
    /// Files under these paths are never auto-purged.
    #[arg(long, env = "BIRDNET_DISK_EXCLUDE", value_delimiter = ',')]
    pub disk_exclude: Vec<std::path::PathBuf>,

    /// Reclaim the audio of detections older than this many days (0 = keep
    /// audio forever, the default).
    ///
    /// The age-based half of retention, alongside `--max-files-per-species` and
    /// the disk-full purge. Locked clips are exempt, and the detection records
    /// themselves are always kept — counts, species lists, trends and exports
    /// are unaffected; only the audio is reclaimed.
    #[arg(long, default_value = "0", env = "BIRDNET_CLIP_RETENTION_DAYS")]
    pub clip_retention_days: u32,

    /// Disk-usage percentage at which the oldest recordings start being purged.
    ///
    /// The safety net that keeps a 24/7 station from filling its card: once the
    /// data disk crosses this, the oldest clips are deleted first, and locked
    /// clips are never touched. `BirdNET-Pi` equivalent: `DISK_PURGE_THRESHOLD`.
    ///
    /// `0` leaves the resolution to the config file / admin settings, then the
    /// 95 % default — the flag is only "set" when given, so it never silently
    /// overrides a value chosen in the UI.
    #[arg(long, default_value = "0", env = "BIRDNET_DISK_PURGE_THRESHOLD")]
    pub disk_purge_threshold: u8,

    /// Seconds a raw capture segment is kept in the transient stream directory.
    ///
    /// Raw segments are read by the detector and never needed again; draining
    /// them is what keeps the RAM-backed stream dir from filling. Far longer
    /// than the pipeline needs, so an unprocessed segment is never removed.
    /// `0` = resolve from config / admin settings, then the 600 s default.
    #[arg(long, default_value = "0", env = "BIRDNET_STREAM_RETENTION_SECS")]
    pub stream_retention_secs: u64,

    /// Hard ceiling in mebibytes on the transient stream directory.
    ///
    /// Backstop for many-stream or backed-up runs; the oldest segments drop
    /// first. `0` = resolve from config / admin settings, then the 512 MiB
    /// default.
    #[arg(long, default_value = "0", env = "BIRDNET_STREAM_MAX_MB")]
    pub stream_max_mb: u64,

    /// Directory containing custom species images (checked before Wikipedia cache).
    ///
    /// Files should be named `{lowercase_sci_name_with_underscores}.jpg`, e.g.
    /// `turdus_merula.jpg`. `BirdNET-Pi` equivalent: `CUSTOM_IMAGE` directory.
    #[arg(long, env = "BIRDNET_CUSTOM_IMAGE_DIR")]
    pub custom_image_dir: Option<PathBuf>,

    /// Path to Apprise config file (alternative/addition to --apprise-url).
    ///
    /// When set, uses the `apprise` CLI tool with `-c <file>` for notifications.
    /// `BirdNET-Pi` equivalent: `APPRISE_CONFIG_FILE` config option.
    #[arg(long, env = "BIRDNET_APPRISE_CONFIG")]
    pub apprise_config: Option<PathBuf>,

    /// Weekly report notification schedule.
    ///
    /// Send a weekly summary via Apprise on a fixed weekday.
    /// Values: "monday", "tuesday", ..., "sunday", or "disabled".
    /// `BirdNET-Pi` equivalent: `weekly_report` cron job.
    #[arg(long, default_value = "monday", env = "BIRDNET_WEEKLY_REPORT_SCHEDULE")]
    pub weekly_report_schedule: String,

    /// Frequency shift applied to extracted audio clips in Hz (0 = disabled).
    ///
    /// Shifts the pitch of saved detection clips upward by the given number of Hz,
    /// making high-frequency bird calls accessible to people with high-frequency
    /// hearing loss. Requires ffmpeg or sox. Typical values: 1000–4000 Hz.
    /// `BirdNET-Pi` equivalent: `FREQ_SHIFT` + `FREQ_SHIFT_AMOUNT` config options.
    #[arg(long, default_value = "0", env = "BIRDNET_FREQ_SHIFT_HZ")]
    pub freq_shift_hz: i32,

    // -----------------------------------------------------------------------
    // MQTT integration
    // -----------------------------------------------------------------------
    /// MQTT broker hostname or IP address for `IoT` detection events.
    ///
    /// When set, each bird detection is published as a JSON payload to the
    /// configured broker.  Compatible with Home Assistant, Node-RED, Mosquitto,
    /// and any MQTT 3.1.1-compatible broker.
    /// Example: `--mqtt-host mqtt.local`
    #[arg(long, env = "BIRDNET_MQTT_HOST")]
    pub mqtt_host: Option<String>,

    /// MQTT broker port (default: 1883; TLS: 8883).
    #[arg(long, default_value = "1883", env = "BIRDNET_MQTT_PORT")]
    pub mqtt_port: u16,

    /// MQTT client identifier published in CONNECT packets.
    ///
    /// Must be unique per active connection on the broker.
    #[arg(
        long,
        default_value = "birdnet-behavior",
        env = "BIRDNET_MQTT_CLIENT_ID"
    )]
    pub mqtt_client_id: String,

    /// MQTT broker username for authentication.
    #[arg(long, env = "BIRDNET_MQTT_USERNAME")]
    pub mqtt_username: Option<String>,

    /// MQTT broker password for authentication.
    #[arg(long, env = "BIRDNET_MQTT_PASSWORD")]
    pub mqtt_password: Option<String>,

    /// MQTT topic prefix for published messages (default: "birdnet").
    ///
    /// Detections are published to `{prefix}/detection/{species}`.
    /// Status heartbeat is published to `{prefix}/status`.
    #[arg(long, default_value = "birdnet", env = "BIRDNET_MQTT_TOPIC_PREFIX")]
    pub mqtt_topic_prefix: String,

    /// Set the RETAIN flag on MQTT detection messages.
    ///
    /// When set, the broker stores the last detection per topic and delivers
    /// it immediately to new subscribers.  Useful for Home Assistant sensors.
    #[arg(long, env = "BIRDNET_MQTT_RETAIN")]
    pub mqtt_retain: bool,

    /// Publish Home Assistant MQTT auto-discovery messages at startup.
    ///
    /// When enabled, BirdNet-Behavior publishes MQTT discovery payloads to
    /// `homeassistant/<component>/<uid>/config` so Home Assistant automatically
    /// creates sensors without manual `configuration.yaml` entries.
    ///
    /// Requires `--mqtt-host` to be set.  Harmless if HA is not running.
    #[arg(long, env = "BIRDNET_MQTT_HA_DISCOVERY")]
    pub mqtt_ha_discovery: bool,

    /// Enable audio quality pre-filtering before ML inference.
    ///
    /// When enabled, audio chunks are assessed for SNR, spectral flatness,
    /// and rain/wind interference before being passed to the ML model.
    /// Chunks below the quality threshold are discarded, reducing false
    /// positives in noisy environments at the cost of some sensitivity.
    #[arg(long, env = "BIRDNET_QUALITY_FILTER")]
    pub quality_filter: bool,

    /// Minimum SNR (dB) for audio quality pre-filtering (default: 3.0).
    ///
    /// Only used when --quality-filter is set.
    #[arg(long, default_value = "3.0", env = "BIRDNET_QUALITY_MIN_SNR")]
    pub quality_min_snr_db: f32,
}

impl Cli {
    /// Parse from the process arguments, recording which of them the operator
    /// actually supplied.
    ///
    /// Equivalent to [`clap::Parser::parse`] except that [`Cli::explicit`] is
    /// populated. Prefer this everywhere in production: a `Cli` parsed the plain
    /// way carries an empty `explicit` set, which reads as "the operator gave no
    /// flags" and hands every contested setting to the admin UI.
    #[must_use]
    pub fn parse_tracked() -> Self {
        Self::from_matches(&Self::command().get_matches())
    }

    /// Same as [`Cli::parse_tracked`], but from an explicit argument iterator.
    ///
    /// Test-only: production parses the real process arguments through
    /// [`Cli::parse_tracked`]. Kept so the explicit-source behaviour can be
    /// exercised against a synthesised argv rather than only asserted.
    #[cfg(test)]
    pub fn parse_tracked_from<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::from_matches(&Self::command().get_matches_from(args))
    }

    /// Build a `Cli` from already-parsed matches, attaching the explicit-source
    /// set.
    ///
    /// `from_arg_matches` only fails on a mismatch between the derive and the
    /// command it just produced, which cannot happen for a single generated
    /// type; `exit` renders clap's own diagnostic in the unreachable case rather
    /// than panicking with a less useful one.
    fn from_matches(matches: &ArgMatches) -> Self {
        let explicit = ExplicitArgs::from_matches(matches);
        let mut cli = Self::from_arg_matches(matches).unwrap_or_else(|e| e.exit());
        cli.explicit = explicit;
        cli
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn explicit_set_distinguishes_supplied_flags_from_defaults() {
        // `--segment-duration` carries a `default_value`, so the parsed value is
        // 15 either way. Only the explicit set can tell the two apart — which is
        // what lets an admin-UI value win over a default but not over a flag the
        // operator really typed.
        let given = Cli::parse_tracked_from(["birdnet-behavior", "--segment-duration", "15"]);
        assert_eq!(given.segment_duration, 15);
        assert!(given.explicit.has("segment_duration"));

        let defaulted = Cli::parse_tracked_from(["birdnet-behavior"]);
        assert_eq!(defaulted.segment_duration, 15);
        assert!(!defaulted.explicit.has("segment_duration"));
    }

    #[test]
    fn explicit_set_covers_flags_without_defaults() {
        let cli = Cli::parse_tracked_from(["birdnet-behavior", "--apprise-url", "http://x:8000"]);
        assert!(cli.explicit.has("apprise_url"));
        assert!(!cli.explicit.has("birdweather_token"));
    }

    #[test]
    fn explicit_set_records_boolean_flags_only_when_passed() {
        let off = Cli::parse_tracked_from(["birdnet-behavior"]);
        assert!(!off.explicit.has("night_inhibit"));

        let on = Cli::parse_tracked_from(["birdnet-behavior", "--night-inhibit"]);
        assert!(on.night_inhibit);
        assert!(on.explicit.has("night_inhibit"));
    }

    #[test]
    fn plain_parse_leaves_the_explicit_set_empty() {
        // `#[arg(skip)]` means the field defaults rather than being parsed, so a
        // `Cli` built the plain way claims nothing was supplied. Pinned because
        // production code must use `parse_tracked` to get correct precedence.
        let cli = Cli::parse_from(["birdnet-behavior", "--segment-duration", "30"]);
        assert_eq!(cli.segment_duration, 30);
        assert!(!cli.explicit.has("segment_duration"));
    }

    /// The documented air-gapped opt-out: an explicitly empty
    /// `--image-cache-dir` must parse (clap's stock `PathBuf` parser
    /// rejects empty values, which silently broke this) and arrive as the
    /// empty path that `init_image_cache` interprets as "disabled".
    #[test]
    fn empty_image_cache_dir_parses_as_opt_out() {
        let cli = Cli::parse_from(["birdnet-behavior", "--image-cache-dir", ""]);
        assert_eq!(cli.image_cache_dir, Some(std::path::PathBuf::new()));

        let cli = Cli::parse_from(["birdnet-behavior", "--image-cache-dir="]);
        assert_eq!(cli.image_cache_dir, Some(std::path::PathBuf::new()));
    }

    /// A non-empty value still parses as a normal path.
    #[test]
    fn non_empty_image_cache_dir_parses_as_path() {
        let cli = Cli::parse_from(["birdnet-behavior", "--image-cache-dir", "/var/cache/img"]);
        assert_eq!(
            cli.image_cache_dir,
            Some(std::path::PathBuf::from("/var/cache/img"))
        );
    }
}
