//! SMTP email sender using the `lettre` crate.
//!
//! Connects via STARTTLS (port 587) or implicit TLS (port 465) depending on
//! the configuration.  Credentials are sent using PLAIN auth.
//!
//! Note: TLS transport is handled via the `lettre` crate's `SmtpTransport`
//! which uses rustls under the hood (no native-tls / OpenSSL dependency).

use lettre::message::{Mailbox, MultiPart, SinglePart, header};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::types::{DetectionEmail, EmailConfig, EmailError};
use super::templates;

/// Send a detection email using SMTP.
///
/// Builds both plain-text and HTML MIME parts and sends via STARTTLS or TLS.
///
/// # Errors
///
/// Returns [`EmailError`] if the message cannot be built or the SMTP
/// connection/send fails.
pub async fn send_detection_email(
    config: &EmailConfig,
    detection: &DetectionEmail,
) -> Result<(), EmailError> {
    config.validate()?;

    let message = build_message(config, detection)?;
    let transport = build_transport(config)?;

    transport
        .send(message)
        .await
        .map_err(|e| EmailError::Transport(e.to_string()))?;

    tracing::info!(
        species = %detection.common_name,
        to = %config.to_address,
        "detection email sent"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_message(config: &EmailConfig, detection: &DetectionEmail) -> Result<Message, EmailError> {
    let from_addr: Address = config
        .from_address
        .parse()
        .map_err(|_| EmailError::Address(format!("invalid from address: {}", config.from_address)))?;

    let to_addr: Address = config
        .to_address
        .parse()
        .map_err(|_| EmailError::Address(format!("invalid to address: {}", config.to_address)))?;

    let from_mailbox = if let Some(ref name) = config.from_name {
        Mailbox::new(Some(name.clone()), from_addr)
    } else {
        Mailbox::new(None, from_addr)
    };

    let to_mailbox = Mailbox::new(None, to_addr);

    let subject = templates::subject(detection);
    let plain = templates::plain_body(detection);
    let html = templates::html_body(detection);

    Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_PLAIN)
                        .body(plain),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_HTML)
                        .body(html),
                ),
        )
        .map_err(|e| EmailError::Build(e.to_string()))
}

fn build_transport(
    config: &EmailConfig,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, EmailError> {
    let creds = Credentials::new(config.username.clone(), config.password.clone());

    let transport = if config.use_starttls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
            .map_err(|e| EmailError::Transport(e.to_string()))?
            .port(config.smtp_port)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&config.smtp_host)
            .map_err(|e| EmailError::Transport(e.to_string()))?
            .port(config.smtp_port)
            .credentials(creds)
            .build()
    };

    Ok(transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::EmailConfig;

    fn sample_config() -> EmailConfig {
        EmailConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            username: "user@example.com".into(),
            password: "secret".into(),
            from_address: "bird@example.com".into(),
            to_address: "notify@example.com".into(),
            from_name: Some("BirdNet Alerts".into()),
            use_starttls: true,
            min_confidence: 0.75,
            cooldown_secs: 300,
        }
    }

    fn sample_detection() -> DetectionEmail {
        DetectionEmail {
            common_name: "European Robin".into(),
            scientific_name: "Erithacus rubecula".into(),
            confidence: 0.92,
            date: "2026-03-13".into(),
            time: "07:12:00".into(),
            station_name: Some("Garden".into()),
            detection_url: None,
        }
    }

    #[test]
    fn build_message_succeeds() {
        let msg = build_message(&sample_config(), &sample_detection());
        assert!(msg.is_ok(), "message build should succeed: {msg:?}");
    }

    #[test]
    fn invalid_from_address_errors() {
        let mut cfg = sample_config();
        cfg.from_address = "not-an-email".into();
        let result = build_message(&cfg, &sample_detection());
        assert!(matches!(result, Err(EmailError::Address(_))));
    }

    #[test]
    fn invalid_to_address_errors() {
        let mut cfg = sample_config();
        cfg.to_address = "also-not-valid".into();
        let result = build_message(&cfg, &sample_detection());
        assert!(matches!(result, Err(EmailError::Address(_))));
    }

    #[test]
    fn build_transport_starttls() {
        // Should succeed in constructing the transport object (no network call yet)
        let result = build_transport(&sample_config());
        assert!(result.is_ok());
    }
}
