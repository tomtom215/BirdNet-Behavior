//! Apprise notification client.
//!
//! Sends push notifications via an [Apprise](https://github.com/caronc/apprise-api)
//! server when bird detections meet configurable criteria (confidence threshold,
//! species watchlist, cooldown period).
//!
//! Apprise aggregates 80+ notification services (Telegram, Slack, Discord, email,
//! Pushover, etc.) behind a single REST API, so users configure their notification
//! channels in Apprise and this client simply posts to its `/notify` endpoint.

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Default request timeout for the Apprise server.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default cooldown between notifications for the same species (5 minutes).
const DEFAULT_COOLDOWN_SECS: u64 = 300;

/// Maximum retry attempts for failed requests.
const MAX_RETRIES: u32 = 2;

/// Apprise client errors.
#[derive(Debug)]
pub enum AppriseError {
    /// HTTP request failed.
    Http(String),
    /// Apprise server returned an error.
    Server(String),
    /// No Apprise URL configured.
    NoUrl,
    /// Apprise CLI invocation failed.
    Cli(String),
}

impl fmt::Display for AppriseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "Apprise HTTP error: {msg}"),
            Self::Server(msg) => write!(f, "Apprise server error: {msg}"),
            Self::NoUrl => write!(f, "Apprise server URL not configured"),
            Self::Cli(msg) => write!(f, "Apprise CLI error: {msg}"),
        }
    }
}

impl std::error::Error for AppriseError {}

/// Notification type (maps to Apprise message types).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyType {
    /// Informational notification.
    Info,
    /// Warning notification.
    Warning,
    /// Success notification.
    Success,
}

/// Configuration for notification filtering.
#[derive(Debug, Clone)]
pub struct NotifyConfig {
    /// Minimum confidence to trigger a notification (0.0 - 1.0).
    pub min_confidence: f32,
    /// Species include-list (empty = notify for all species).
    ///
    /// When non-empty, only species in this list trigger notifications.
    /// BirdNET-Pi equivalent: `APPRISE_ONLY_NOTIFY_SPECIES_NAMES`.
    pub species_watchlist: Vec<String>,
    /// Species exclude-list — species that should never trigger notifications.
    ///
    /// Applied after `species_watchlist` (exclusion wins). Supports the
    /// dual-filter pattern: notify only for watchlist species except excluded ones.
    /// BirdNET-Pi equivalent: `APPRISE_WATCHLIST_EXCLUDE` (BirdNet-Behavior addition).
    pub species_notify_exclude: Vec<String>,
    /// Default cooldown period between notifications for the same species.
    pub cooldown: Duration,
    /// Per-species cooldown overrides (scientific name → duration).
    pub per_species_cooldown: HashMap<String, Duration>,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.8,
            species_watchlist: Vec::new(),
            species_notify_exclude: Vec::new(),
            cooldown: Duration::from_secs(DEFAULT_COOLDOWN_SECS),
            per_species_cooldown: HashMap::new(),
        }
    }
}

/// Apprise notification client.
///
/// Sends notifications to an Apprise API server (or via the `apprise` CLI
/// when `--apprise-config` is configured). Includes a per-species cooldown
/// to prevent notification flooding during active bird sessions.
///
/// BirdNET-Pi equivalent: `birdnet_analysis.sh` invokes `apprise -c <file>`.
#[derive(Debug)]
pub struct Client {
    /// Apprise API server base URL (e.g., `http://localhost:8000`).
    base_url: String,
    /// HTTP client.
    http: reqwest::Client,
    /// Notification filtering configuration.
    config: NotifyConfig,
    /// Per-species last-notification timestamps for cooldown.
    last_notified: HashMap<String, Instant>,
    /// Optional path to an Apprise config file (uses `apprise` CLI).
    ///
    /// When set, `send_notification` invokes `apprise -c <path> -t <title> -b <body>`
    /// in addition to (or instead of) the HTTP server.
    /// BirdNET-Pi equivalent: `APPRISE_CONFIG_FILE` setting.
    config_file: Option<PathBuf>,
}

impl Client {
    /// Create a new Apprise notification client with an HTTP server URL.
    ///
    /// # Errors
    ///
    /// Returns `AppriseError::NoUrl` if the URL is empty.
    pub fn new(base_url: &str, config: NotifyConfig) -> Result<Self, AppriseError> {
        if base_url.is_empty() {
            return Err(AppriseError::NoUrl);
        }

        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| AppriseError::Http(e.to_string()))?;

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
            config,
            last_notified: HashMap::new(),
            config_file: None,
        })
    }

    /// Create a CLI-only Apprise client (no HTTP server URL).
    ///
    /// Used when only `--apprise-config` is set (no `--apprise-url`).
    /// All notifications are sent via `apprise -c <config_file>`.
    ///
    /// # Errors
    ///
    /// Returns `AppriseError` if the HTTP client cannot be built.
    pub fn new_cli_only(
        config_file: PathBuf,
        notify_config: NotifyConfig,
    ) -> Result<Self, AppriseError> {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| AppriseError::Http(e.to_string()))?;

        Ok(Self {
            base_url: String::new(), // no HTTP server
            http,
            config: notify_config,
            last_notified: HashMap::new(),
            config_file: Some(config_file),
        })
    }

    /// Check if a detection should trigger a notification.
    ///
    /// Returns `false` if:
    /// - Confidence is below the threshold
    /// - Species is not on the watchlist (when watchlist is non-empty)
    /// - The species was notified recently (within cooldown period)
    pub fn should_notify(&mut self, species: &str, confidence: f32) -> bool {
        // Confidence threshold
        if confidence < self.config.min_confidence {
            return false;
        }

        // Species include-list (empty = all species pass)
        if !self.config.species_watchlist.is_empty()
            && !self.config.species_watchlist.iter().any(|s| s == species)
        {
            return false;
        }

        // Species exclude-list — exclusion always wins, even for watchlist members
        if self
            .config
            .species_notify_exclude
            .iter()
            .any(|s| s == species)
        {
            return false;
        }

        // Per-species cooldown (use species-specific override if available)
        let cooldown = self
            .config
            .per_species_cooldown
            .get(species)
            .copied()
            .unwrap_or(self.config.cooldown);
        let now = Instant::now();
        if let Some(last) = self.last_notified.get(species)
            && now.duration_since(*last) < cooldown
        {
            return false;
        }

        // Prune stale cooldown entries (older than 2x cooldown) to prevent
        // unbounded memory growth over long field deployments.
        if self.last_notified.len() > 100 {
            let prune_after = cooldown * 2;
            self.last_notified
                .retain(|_, instant| now.duration_since(*instant) < prune_after);
        }

        // Update last-notified timestamp
        self.last_notified.insert(species.to_string(), now);
        true
    }

    /// Send a bird detection notification.
    ///
    /// Formats a human-readable message and sends it to the Apprise server.
    ///
    /// # Errors
    ///
    /// Returns `AppriseError` on network or server failure.
    pub async fn notify_detection(
        &self,
        species: &str,
        confidence: f32,
        date: &str,
        time: &str,
    ) -> Result<(), AppriseError> {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let conf_pct = (confidence * 100.0) as u32;

        let body = format!("{species} detected ({conf_pct}% confidence) at {time} on {date}");
        let title = format!("Bird Detection: {species}");

        self.send_notification(&title, &body, NotifyType::Info)
            .await
    }

    /// Send a generic notification to the Apprise server.
    ///
    /// # Errors
    ///
    /// Returns `AppriseError` on network or server failure.
    pub async fn send_notification(
        &self,
        title: &str,
        body: &str,
        notify_type: NotifyType,
    ) -> Result<(), AppriseError> {
        self.send_notification_with_image(title, body, notify_type, None)
            .await
    }

    /// Send a notification with an optional image attachment.
    ///
    /// If a config file is configured, also sends via `apprise` CLI.
    ///
    /// # Errors
    ///
    /// Returns `AppriseError` on network or server failure.
    pub async fn send_notification_with_image(
        &self,
        title: &str,
        body: &str,
        notify_type: NotifyType,
        image_url: Option<&str>,
    ) -> Result<(), AppriseError> {
        // Send via CLI if config file is configured.
        if self.config_file.is_some()
            && let Err(e) = self.send_via_cli(title, body).await
        {
            tracing::warn!(error = %e, "Apprise CLI notification failed");
        }

        // If no HTTP server URL, we're done.
        if self.base_url.is_empty() {
            return Ok(());
        }

        let url = format!("{}/notify", self.base_url);

        let mut payload = serde_json::json!({
            "title": title,
            "body": body,
            "type": notify_type,
        });

        if let Some(img) = image_url {
            payload["image"] = serde_json::json!(img);
        }

        self.post_with_retry(&url, &payload).await
    }

    /// Configure an Apprise config file for CLI-based notifications.
    ///
    /// When set, notifications are sent via `apprise -c <path> -t <title> -b <body>`
    /// in addition to the HTTP server (if a URL is also configured).
    /// BirdNET-Pi equivalent: `APPRISE_CONFIG_FILE` config option.
    #[must_use]
    pub fn with_config_file(mut self, path: PathBuf) -> Self {
        self.config_file = Some(path);
        self
    }

    /// Send a notification via the `apprise` CLI tool.
    ///
    /// Invokes `apprise -c <config_file> -t <title> -b <body>`.
    ///
    /// # Errors
    ///
    /// Returns `AppriseError::Cli` if the command fails or is not available.
    pub async fn send_via_cli(&self, title: &str, body: &str) -> Result<(), AppriseError> {
        let Some(ref config_path) = self.config_file else {
            return Err(AppriseError::Cli("no config file configured".into()));
        };

        let config_path = config_path.clone();
        let title = title.to_string();
        let body = body.to_string();

        tokio::task::spawn_blocking(move || {
            let output = std::process::Command::new("apprise")
                .arg("-c")
                .arg(&config_path)
                .arg("-t")
                .arg(&title)
                .arg("-b")
                .arg(&body)
                .output()
                .map_err(|e| AppriseError::Cli(format!("apprise CLI not found: {e}")))?;

            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(AppriseError::Cli(format!(
                    "apprise CLI exited {}: {stderr}",
                    output.status
                )))
            }
        })
        .await
        .map_err(|e| AppriseError::Cli(e.to_string()))?
    }

    /// Whether an Apprise config file is configured.
    pub const fn has_config_file(&self) -> bool {
        self.config_file.is_some()
    }

    /// Get the configured base URL.
    pub fn url(&self) -> &str {
        &self.base_url
    }

    /// Get the notification configuration.
    pub const fn config(&self) -> &NotifyConfig {
        &self.config
    }

    /// Clear all cooldown timers (useful for testing or config changes).
    pub fn reset_cooldowns(&mut self) {
        self.last_notified.clear();
    }

    /// POST with retry on failure.
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<(), AppriseError> {
        let mut last_error = AppriseError::Http("no attempts made".into());

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(2_u64.pow(attempt));
                tracing::debug!(
                    attempt,
                    delay_secs = delay.as_secs(),
                    "retrying Apprise POST"
                );
                tokio::time::sleep(delay).await;
            }

            match self.http.post(url).json(body).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(());
                    }
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    last_error = AppriseError::Server(format!("{status}: {text}"));
                }
                Err(e) => {
                    last_error = AppriseError::Http(e.to_string());
                }
            }
        }

        Err(last_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_returns_error() {
        let result = Client::new("", NotifyConfig::default());
        assert!(matches!(result, Err(AppriseError::NoUrl)));
    }

    #[test]
    fn client_stores_url() {
        let client = Client::new("http://localhost:8000", NotifyConfig::default()).unwrap();
        assert_eq!(client.url(), "http://localhost:8000");
    }

    #[test]
    fn trailing_slash_stripped() {
        let client = Client::new("http://localhost:8000/", NotifyConfig::default()).unwrap();
        assert_eq!(client.url(), "http://localhost:8000");
    }

    #[test]
    fn should_notify_respects_confidence_threshold() {
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.8,
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        assert!(!client.should_notify("European Robin", 0.5));
        assert!(!client.should_notify("European Robin", 0.79));
        assert!(client.should_notify("European Robin", 0.8));
        assert!(client.should_notify("Great Tit", 0.95));
    }

    #[test]
    fn should_notify_respects_watchlist() {
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.5,
                species_watchlist: vec!["European Robin".into(), "Great Tit".into()],
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        assert!(client.should_notify("European Robin", 0.9));
        assert!(!client.should_notify("Eurasian Blackbird", 0.9)); // not on watchlist
    }

    #[test]
    fn empty_watchlist_allows_all() {
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.5,
                species_watchlist: vec![],
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        assert!(client.should_notify("Any Species", 0.9));
    }

    #[test]
    fn should_notify_exclude_list_blocks_notification() {
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.5,
                species_notify_exclude: vec!["European Starling".into()],
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        assert!(!client.should_notify("European Starling", 0.99)); // excluded
        assert!(client.should_notify("European Robin", 0.9)); // not excluded
    }

    #[test]
    fn should_notify_exclude_wins_over_watchlist() {
        // Species on both watchlist and exclude list → excluded wins.
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.5,
                species_watchlist: vec!["European Starling".into()],
                species_notify_exclude: vec!["European Starling".into()],
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        assert!(!client.should_notify("European Starling", 0.99));
    }

    #[test]
    fn should_notify_respects_cooldown() {
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.5,
                cooldown: Duration::from_secs(300),
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        // First notification: allowed
        assert!(client.should_notify("European Robin", 0.9));
        // Second notification: blocked by cooldown
        assert!(!client.should_notify("European Robin", 0.9));
        // Different species: allowed
        assert!(client.should_notify("Great Tit", 0.9));
    }

    #[test]
    fn reset_cooldowns_clears_state() {
        let mut client = Client::new(
            "http://localhost:8000",
            NotifyConfig {
                min_confidence: 0.5,
                cooldown: Duration::from_secs(300),
                ..NotifyConfig::default()
            },
        )
        .unwrap();

        assert!(client.should_notify("European Robin", 0.9));
        assert!(!client.should_notify("European Robin", 0.9));

        client.reset_cooldowns();
        assert!(client.should_notify("European Robin", 0.9));
    }

    #[test]
    fn default_notify_config() {
        let config = NotifyConfig::default();
        assert!((config.min_confidence - 0.8).abs() < f32::EPSILON);
        assert!(config.species_watchlist.is_empty());
        assert_eq!(config.cooldown, Duration::from_secs(300));
    }
}
