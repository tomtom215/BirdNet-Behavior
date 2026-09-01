//! Push-notification client.
//!
//! Sends notifications when bird detections meet configurable criteria
//! (confidence threshold, species watchlist, cooldown period) over up to three
//! channels, in this order:
//!
//! 1. **Native routes** — [`crate::dispatch`] delivers Discord, Slack,
//!    Telegram, ntfy, Gotify, Pushover and generic JSON webhooks in-process.
//!    No Python, no `apprise` binary, no subprocess per detection.
//! 2. **The `apprise` CLI** — only when the configured Apprise config file
//!    contains a scheme with no native sender. When every URL in it is
//!    natively supported the CLI is never invoked, so a station configured
//!    only for the schemes above needs no Apprise installation at all.
//! 3. **An Apprise API server** — when `APPRISE_URL` names one; its channels
//!    are configured inside that server, so there is nothing to route here.
//!
//! The URL syntax stays Apprise's throughout, because that is what operators
//! already have written down.

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Default request timeout for the Apprise server.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default cooldown between notifications for the same species (5 minutes).
const DEFAULT_COOLDOWN_SECS: u64 = 300;

/// Default ceiling on sends per destination per minute.
///
/// Twelve is above what a station produces once the per-species cooldown is
/// applied — a dawn chorus of twelve distinct species in one minute is a good
/// morning — and an order of magnitude below every service's own limit.
///
/// Public so callers that build [`NotifyConfig`] field by field rather than
/// from [`Default`] use this value rather than a copy of it.
pub const DEFAULT_RATE_PER_MINUTE: u32 = 12;

/// Total request attempts (initial + retries) before a send is abandoned.
const MAX_ATTEMPTS: u32 = 3;

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
    /// A native (non-Apprise) delivery failed.
    Native(String),
}

impl fmt::Display for AppriseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "Apprise HTTP error: {msg}"),
            Self::Server(msg) => write!(f, "Apprise server error: {msg}"),
            Self::NoUrl => write!(f, "Apprise server URL not configured"),
            Self::Cli(msg) => write!(f, "Apprise CLI error: {msg}"),
            Self::Native(msg) => write!(f, "notification delivery failed: {msg}"),
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
    /// Ceiling on sends per destination per minute; `0` disables it.
    ///
    /// Not about our own load — it is about the services'. Discord allows five
    /// requests a second per webhook, Telegram about thirty, and Pushover
    /// **ten thousand messages a month**. A station with fifty species active
    /// can exhaust the last one in a fortnight without a cap.
    pub rate_per_minute: u32,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.8,
            species_watchlist: Vec::new(),
            species_notify_exclude: Vec::new(),
            cooldown: Duration::from_secs(DEFAULT_COOLDOWN_SECS),
            per_species_cooldown: HashMap::new(),
            rate_per_minute: DEFAULT_RATE_PER_MINUTE,
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
    /// Destinations delivered in-process by [`crate::dispatch`].
    native: Vec<crate::dispatch::Route>,
    /// One delivery gate — circuit breaker plus rate limit — per entry of
    /// `native`, by index.
    guards: Vec<crate::dispatch::Gate>,
    /// Sends skipped because a destination's circuit was open.
    skipped_circuit_open: u64,
    /// Sends skipped because a destination was over its rate limit.
    skipped_rate_limited: u64,
    /// Whether the `apprise` CLI still has to run for `config_file`.
    ///
    /// False once every URL in that file has a native sender — which is the
    /// point of the native senders. Conservatively true whenever a config file
    /// is set without the caller having said otherwise, so a config file whose
    /// contents were never read is still delivered rather than dropped.
    cli_needed: bool,
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
            native: Vec::new(),
            guards: Vec::new(),
            skipped_circuit_open: 0,
            skipped_rate_limited: 0,
            cli_needed: true,
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
            native: Vec::new(),
            guards: Vec::new(),
            skipped_circuit_open: 0,
            skipped_rate_limited: 0,
            cli_needed: true,
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
        &mut self,
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
        &mut self,
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
        &mut self,
        title: &str,
        body: &str,
        notify_type: NotifyType,
        image_url: Option<&str>,
    ) -> Result<(), AppriseError> {
        let mut first_error: Option<AppriseError> = None;
        let mut delivered = 0_usize;

        // Native routes first: no subprocess, no Python, and a real error
        // rather than an exit status when a URL is wrong.
        let message = crate::dispatch::Message {
            title: title.to_string(),
            body: body.to_string(),
            severity: notify_type.into(),
            image_url: image_url.map(ToString::to_string),
        };
        for (index, route) in self.native.iter().enumerate() {
            // A destination that is down is skipped outright, apart from one
            // probe per open period. Without this the station spends an
            // attempt (or three, with backoff) on a retired webhook for every
            // detection, all day — and it is the retries, not the sends, that
            // get an address rate-limited. The interaction rules live in
            // `dispatch::Gate`, where they are testable with injected time.
            match self.guards[index].admit(Instant::now()) {
                crate::dispatch::Admission::Send | crate::dispatch::Admission::Probe => {}
                crate::dispatch::Admission::CircuitOpen(wait) => {
                    self.skipped_circuit_open += 1;
                    tracing::debug!(
                        target_label = %route.label,
                        retry_in_secs = wait.as_secs(),
                        "skipping a destination whose circuit is open"
                    );
                    continue;
                }
                crate::dispatch::Admission::RateLimited => {
                    self.skipped_rate_limited += 1;
                    tracing::warn!(
                        target_label = %route.label,
                        "dropping a notification: destination is over its rate limit"
                    );
                    continue;
                }
            }

            match crate::dispatch::send_with_retry(&self.http, &route.target, &message).await {
                Ok(()) => {
                    self.guards[index].on_success();
                    delivered += 1;
                }
                Err(e) => {
                    self.guards[index].on_failure(Instant::now());
                    // `route.label` is credential-free by construction; the
                    // URL it came from is not, and must never be logged.
                    tracing::warn!(target_label = %route.label, error = %e, "notification failed");
                    if first_error.is_none() {
                        first_error = Some(AppriseError::Native(e.to_string()));
                    }
                }
            }
        }

        // The CLI runs only for a config file that still has a scheme without
        // a native sender — otherwise every URL in it would be sent twice.
        if self.config_file.is_some() && self.cli_needed {
            match self.send_via_cli(title, body).await {
                Ok(()) => delivered += 1,
                Err(e) => {
                    tracing::warn!(error = %e, "Apprise CLI notification failed");
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }

        // If no HTTP server URL, we're done.
        if self.base_url.is_empty() {
            return match (delivered, first_error) {
                (0, Some(e)) => Err(e),
                _ => Ok(()),
            };
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

    /// Deliver `routes` in-process, and say whether the `apprise` CLI is still
    /// needed for the configured config file.
    ///
    /// `cli_fallback` must be true when at least one URL in that file has no
    /// native sender. Passing false when it does would silently drop those
    /// channels; passing true when it does not would send every natively
    /// routed URL twice.
    #[must_use]
    pub fn with_native_routes(
        mut self,
        routes: Vec<crate::dispatch::Route>,
        cli_fallback: bool,
    ) -> Self {
        let now = Instant::now();
        self.guards = routes
            .iter()
            .map(|_| crate::dispatch::Gate::new(self.config.rate_per_minute, now))
            .collect();
        self.native = routes;
        self.cli_needed = cli_fallback;
        self
    }

    /// How many sends have been skipped because a destination's circuit was
    /// open, and how many because it was over its rate limit.
    #[must_use]
    pub const fn skip_counts(&self) -> (u64, u64) {
        (self.skipped_circuit_open, self.skipped_rate_limited)
    }

    /// Credential-free labels for the natively delivered destinations.
    ///
    /// Safe to log or render in the admin UI.
    #[must_use]
    pub fn native_labels(&self) -> Vec<&str> {
        self.native.iter().map(|r| r.label.as_str()).collect()
    }

    /// Whether the `apprise` CLI would be invoked for the configured file.
    #[must_use]
    pub const fn needs_apprise_cli(&self) -> bool {
        self.config_file.is_some() && self.cli_needed
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

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                // Jittered, capped exponential backoff so concurrent retries —
                // and many stations hitting the same endpoint — don't
                // synchronise into a thundering herd.
                let delay = crate::retry::backoff_delay(attempt, crate::retry::jitter_frac());
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
                    // A 4xx (other than 429) is a deterministic client error —
                    // a malformed Apprise URL/payload won't succeed on retry, so
                    // fail fast rather than burning the backoff budget.
                    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        return Err(last_error);
                    }
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
