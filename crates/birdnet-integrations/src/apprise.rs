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
}

impl fmt::Display for AppriseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "Apprise HTTP error: {msg}"),
            Self::Server(msg) => write!(f, "Apprise server error: {msg}"),
            Self::NoUrl => write!(f, "Apprise server URL not configured"),
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
    /// Species watchlist (empty = notify for all species).
    pub species_watchlist: Vec<String>,
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
            cooldown: Duration::from_secs(DEFAULT_COOLDOWN_SECS),
            per_species_cooldown: HashMap::new(),
        }
    }
}

/// Apprise notification client.
///
/// Sends notifications to an Apprise API server. Includes a per-species
/// cooldown to prevent notification flooding during active bird sessions.
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
}

impl Client {
    /// Create a new Apprise notification client.
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

        // Species watchlist filter (empty = all species)
        if !self.config.species_watchlist.is_empty()
            && !self.config.species_watchlist.iter().any(|s| s == species)
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
        if let Some(last) = self.last_notified.get(species) {
            if now.duration_since(*last) < cooldown {
                return false;
            }
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
