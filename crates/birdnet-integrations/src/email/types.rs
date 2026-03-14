//! Email notification types, errors, and configuration.

use std::fmt;

/// Configuration for the SMTP email integration.
#[derive(Debug, Clone)]
pub struct EmailConfig {
    /// SMTP server hostname (e.g. `"smtp.gmail.com"`).
    pub smtp_host: String,
    /// SMTP server port (587 = STARTTLS, 465 = TLS, 25 = plain).
    pub smtp_port: u16,
    /// Authentication username.
    pub username: String,
    /// Authentication password or app-specific password.
    pub password: String,
    /// Sender address (`From:` header).
    pub from_address: String,
    /// Recipient address (`To:` header).
    pub to_address: String,
    /// Optional sender display name.
    pub from_name: Option<String>,
    /// Use STARTTLS (port 587) vs implicit TLS (port 465).
    pub use_starttls: bool,
    /// Minimum confidence for triggering an email (0.0–1.0).
    pub min_confidence: f64,
    /// Minimum seconds between emails for the same species.
    pub cooldown_secs: u64,
}

impl EmailConfig {
    /// Validate configuration fields.
    ///
    /// # Errors
    ///
    /// Returns an [`EmailError::Config`] if required fields are empty or
    /// the confidence value is out of range.
    pub fn validate(&self) -> Result<(), EmailError> {
        if self.smtp_host.is_empty() {
            return Err(EmailError::Config("smtp_host is required".into()));
        }
        if self.from_address.is_empty() {
            return Err(EmailError::Config("from_address is required".into()));
        }
        if self.to_address.is_empty() {
            return Err(EmailError::Config("to_address is required".into()));
        }
        if !(0.0..=1.0).contains(&self.min_confidence) {
            return Err(EmailError::Config(
                "min_confidence must be between 0.0 and 1.0".into(),
            ));
        }
        Ok(())
    }
}

/// A detection event to be emailed.
#[derive(Debug, Clone)]
pub struct DetectionEmail {
    /// Species common name.
    pub common_name: String,
    /// Species scientific name.
    pub scientific_name: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f64,
    /// Detection date (YYYY-MM-DD).
    pub date: String,
    /// Detection time (HH:MM:SS).
    pub time: String,
    /// Optional station name.
    pub station_name: Option<String>,
    /// Optional link to the detection in the web UI.
    pub detection_url: Option<String>,
}

/// Errors produced by the email integration.
#[derive(Debug)]
pub enum EmailError {
    /// Invalid configuration.
    Config(String),
    /// SMTP transport error (message contains provider text).
    Transport(String),
    /// Address parsing failure.
    Address(String),
    /// Message building failure.
    Build(String),
}

impl fmt::Display for EmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "email config error: {msg}"),
            Self::Transport(msg) => write!(f, "smtp transport error: {msg}"),
            Self::Address(msg) => write!(f, "email address error: {msg}"),
            Self::Build(msg) => write!(f, "email build error: {msg}"),
        }
    }
}

impl std::error::Error for EmailError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> EmailConfig {
        EmailConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            username: "user".into(),
            password: "pass".into(),
            from_address: "bird@example.com".into(),
            to_address: "me@example.com".into(),
            from_name: Some("BirdNet-Behavior".into()),
            use_starttls: true,
            min_confidence: 0.8,
            cooldown_secs: 300,
        }
    }

    #[test]
    fn valid_config() {
        assert!(sample_config().validate().is_ok());
    }

    #[test]
    fn empty_host_is_invalid() {
        let mut cfg = sample_config();
        cfg.smtp_host = String::new();
        assert!(matches!(cfg.validate(), Err(EmailError::Config(_))));
    }

    #[test]
    fn invalid_confidence_range() {
        let mut cfg = sample_config();
        cfg.min_confidence = 1.5;
        assert!(matches!(cfg.validate(), Err(EmailError::Config(_))));
    }

    #[test]
    fn empty_to_address_is_invalid() {
        let mut cfg = sample_config();
        cfg.to_address = String::new();
        assert!(matches!(cfg.validate(), Err(EmailError::Config(_))));
    }
}
