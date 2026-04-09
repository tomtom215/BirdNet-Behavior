//! SMTP email notification integration.
//!
//! Sends rich HTML + plain-text bird detection alerts via any SMTP server
//! (`Gmail`, `SendGrid`, local Postfix, etc.).  Supports per-species cooldown to
//! avoid notification spam.
//!
//! # Quick start
//!
//! ```ignore
//! use birdnet_integrations::email::{EmailConfig, EmailNotifier};
//!
//! let config = EmailConfig {
//!     smtp_host: "smtp.gmail.com".into(),
//!     smtp_port: 587,
//!     username: "you@gmail.com".into(),
//!     password: "app-password".into(),
//!     from_address: "you@gmail.com".into(),
//!     to_address: "alerts@example.com".into(),
//!     from_name: Some("BirdNet-Behavior".into()),
//!     use_starttls: true,
//!     min_confidence: 0.8,
//!     cooldown_secs: 300,
//! };
//!
//! let notifier = EmailNotifier::new(config);
//! // notifier.notify(&detection).await?;
//! ```

pub mod smtp;
pub mod templates;
pub mod types;

pub use types::{DetectionEmail, EmailConfig, EmailError};

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tracing::debug;

/// Email notifier with per-species cooldown tracking.
///
/// Thread-safe: the cooldown map is protected by a `Mutex`.
#[derive(Debug)]
pub struct EmailNotifier {
    config: EmailConfig,
    /// Maps `common_name` → last email sent time.
    last_sent: Mutex<HashMap<String, Instant>>,
}

impl EmailNotifier {
    /// Create a new notifier from the given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`EmailError::Config`] if the configuration is invalid.
    pub fn new(config: EmailConfig) -> Result<Self, EmailError> {
        config.validate()?;
        Ok(Self {
            config,
            last_sent: Mutex::new(HashMap::new()),
        })
    }

    /// Attempt to send an email for a detection.
    ///
    /// Returns `Ok(true)` if an email was sent, `Ok(false)` if suppressed by
    /// confidence threshold or cooldown.
    ///
    /// # Errors
    ///
    /// Returns [`EmailError`] if the SMTP send fails.
    pub async fn notify(&self, detection: &DetectionEmail) -> Result<bool, EmailError> {
        if detection.confidence < self.config.min_confidence {
            debug!(
                species = %detection.common_name,
                confidence = detection.confidence,
                threshold = self.config.min_confidence,
                "email suppressed: below confidence threshold"
            );
            return Ok(false);
        }

        if self.is_in_cooldown(&detection.common_name) {
            debug!(
                species = %detection.common_name,
                "email suppressed: cooldown active"
            );
            return Ok(false);
        }

        smtp::send_detection_email(&self.config, detection).await?;
        self.record_sent(&detection.common_name);
        Ok(true)
    }

    /// Check whether a species is currently in the cooldown window.
    fn is_in_cooldown(&self, species: &str) -> bool {
        let cooldown = Duration::from_secs(self.config.cooldown_secs);
        if cooldown.is_zero() {
            return false;
        }
        let map = self
            .last_sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(species)
            .is_some_and(|last| last.elapsed() < cooldown)
    }

    /// Record that an email was sent for `species` right now.
    ///
    /// Also prunes stale cooldown entries (older than 2x the cooldown period)
    /// to prevent unbounded memory growth over long deployments.
    fn record_sent(&self, species: &str) {
        let mut map = self
            .last_sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Prune entries whose cooldown has long expired (2x cooldown = guaranteed stale).
        let prune_threshold = Duration::from_secs(self.config.cooldown_secs * 2);
        if map.len() > 100 {
            map.retain(|_, instant| instant.elapsed() < prune_threshold);
        }
        map.insert(species.to_owned(), Instant::now());
    }

    /// Reset the cooldown for a specific species (useful for testing).
    pub fn reset_cooldown(&self, species: &str) {
        let mut map = self
            .last_sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.remove(species);
    }

    /// Return the configured minimum confidence threshold.
    pub const fn min_confidence(&self) -> f64 {
        self.config.min_confidence
    }

    /// Return the configured cooldown in seconds.
    pub const fn cooldown_secs(&self) -> u64 {
        self.config.cooldown_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_cooldown(secs: u64, min_confidence: f64) -> EmailConfig {
        EmailConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            username: "u".into(),
            password: "p".into(),
            from_address: "from@example.com".into(),
            to_address: "to@example.com".into(),
            from_name: None,
            use_starttls: true,
            min_confidence,
            cooldown_secs: secs,
        }
    }

    fn detection(confidence: f64) -> DetectionEmail {
        DetectionEmail {
            common_name: "Robin".into(),
            scientific_name: "Erithacus rubecula".into(),
            confidence,
            date: "2026-03-13".into(),
            time: "07:00:00".into(),
            station_name: None,
            detection_url: None,
        }
    }

    #[test]
    fn notifier_creation_succeeds() {
        let n = EmailNotifier::new(config_with_cooldown(300, 0.8));
        assert!(n.is_ok());
    }

    #[test]
    fn invalid_config_rejected() {
        let mut cfg = config_with_cooldown(300, 0.8);
        cfg.smtp_host = String::new();
        let n = EmailNotifier::new(cfg);
        assert!(matches!(n, Err(EmailError::Config(_))));
    }

    #[test]
    fn cooldown_initially_inactive() {
        let n = EmailNotifier::new(config_with_cooldown(300, 0.0)).unwrap();
        assert!(!n.is_in_cooldown("Robin"));
    }

    #[test]
    fn cooldown_active_after_record() {
        let n = EmailNotifier::new(config_with_cooldown(300, 0.0)).unwrap();
        n.record_sent("Robin");
        assert!(n.is_in_cooldown("Robin"));
    }

    #[test]
    fn cooldown_reset_clears() {
        let n = EmailNotifier::new(config_with_cooldown(300, 0.0)).unwrap();
        n.record_sent("Robin");
        n.reset_cooldown("Robin");
        assert!(!n.is_in_cooldown("Robin"));
    }

    #[test]
    fn zero_cooldown_always_inactive() {
        let n = EmailNotifier::new(config_with_cooldown(0, 0.0)).unwrap();
        n.record_sent("Robin");
        assert!(!n.is_in_cooldown("Robin"));
    }

    #[test]
    fn confidence_accessor() {
        let n = EmailNotifier::new(config_with_cooldown(300, 0.75)).unwrap();
        assert!((n.min_confidence() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn cooldown_accessor() {
        let n = EmailNotifier::new(config_with_cooldown(600, 0.8)).unwrap();
        assert_eq!(n.cooldown_secs(), 600);
    }

    #[tokio::test]
    async fn low_confidence_suppresses_without_smtp() {
        let n = EmailNotifier::new(config_with_cooldown(0, 0.9)).unwrap();
        // confidence 0.5 < 0.9 threshold → no SMTP call, returns Ok(false)
        let result = n.notify(&detection(0.5)).await;
        assert!(matches!(result, Ok(false)));
    }

    #[tokio::test]
    async fn cooldown_suppresses_without_smtp() {
        let n = EmailNotifier::new(config_with_cooldown(3600, 0.0)).unwrap();
        n.record_sent("Robin");
        // In cooldown → Ok(false) without SMTP call
        let result = n.notify(&detection(1.0)).await;
        assert!(matches!(result, Ok(false)));
    }
}
