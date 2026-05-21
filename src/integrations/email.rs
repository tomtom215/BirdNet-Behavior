//! Email (SMTP) alert notifier construction from database settings.

use std::sync::Arc;

/// Type alias for the shared email notifier handle.
pub type EmailHandle = Arc<birdnet_integrations::email::EmailNotifier>;

/// Create an email notifier from settings stored in the `SQLite` database.
///
/// Returns `None` if no SMTP host is configured or construction fails.
pub fn create_email_notifier(state: &birdnet_web::state::AppState) -> Option<EmailHandle> {
    use birdnet_db::settings::{ensure_settings_table, get_or};
    use birdnet_integrations::email::{EmailConfig, EmailNotifier};

    // Helper: unwrap a settings Result to String, falling back to the default.
    fn s(r: Result<String, birdnet_db::settings::SettingsError>, default: &str) -> String {
        r.unwrap_or_else(|_| default.to_string())
    }

    let smtp_host: String = state
        .with_db(|conn| {
            ensure_settings_table(conn).ok();
            Ok::<String, birdnet_db::settings::SettingsError>(s(
                get_or(conn, "email_smtp_host", ""),
                "",
            ))
        })
        .unwrap_or_default();
    if smtp_host.is_empty() {
        return None;
    }

    let cfg = state
        .with_db(|conn| {
            let smtp_port = s(get_or(conn, "email_smtp_port", "587"), "587")
                .parse::<u16>()
                .unwrap_or(587);
            let use_starttls = s(get_or(conn, "email_starttls", "true"), "true") != "false";
            let min_confidence = s(get_or(conn, "email_min_confidence", "0.80"), "0.80")
                .parse::<f64>()
                .unwrap_or(0.80);
            let cooldown_secs = s(get_or(conn, "email_cooldown_secs", "300"), "300")
                .parse::<u64>()
                .unwrap_or(300);
            let from_name_str = s(get_or(conn, "email_from_name", ""), "");
            Ok::<EmailConfig, birdnet_db::settings::SettingsError>(EmailConfig {
                smtp_host: smtp_host.clone(),
                smtp_port,
                username: s(get_or(conn, "email_smtp_user", ""), ""),
                password: s(get_or(conn, "email_smtp_pass", ""), ""),
                from_address: s(get_or(conn, "email_from", ""), ""),
                to_address: s(get_or(conn, "email_to", ""), ""),
                from_name: if from_name_str.is_empty() {
                    None
                } else {
                    Some(from_name_str)
                },
                use_starttls,
                min_confidence,
                cooldown_secs,
            })
        })
        .unwrap_or_else(|_| EmailConfig {
            smtp_host: smtp_host.clone(),
            smtp_port: 587,
            username: String::new(),
            password: String::new(),
            from_address: String::new(),
            to_address: String::new(),
            from_name: None,
            use_starttls: true,
            min_confidence: 0.80,
            cooldown_secs: 300,
        });

    match EmailNotifier::new(cfg) {
        Ok(notifier) => {
            tracing::info!(smtp_host = %smtp_host, "email alerts enabled");
            Some(Arc::new(notifier))
        }
        Err(e) => {
            tracing::warn!(error = %e, "email notifier not started (check SMTP settings)");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::create_email_notifier;
    use crate::integrations::test_support::test_state;

    #[test]
    fn email_notifier_is_none_when_no_smtp_host_configured() {
        // Empty settings DB → no smtp host → no notifier.
        let state = test_state();
        assert!(create_email_notifier(&state).is_none());
    }

    #[test]
    fn email_notifier_built_from_settings_table() {
        // Seed the settings rows the helper reads, then prove it
        // constructs a notifier. We don't actually send mail; we only
        // pin that the configuration path builds the handle.
        use birdnet_db::settings::{SettingsCategory, ensure_settings_table, set};
        let state = test_state();
        state.with_db(|conn| {
            ensure_settings_table(conn).unwrap();
            set(
                conn,
                "email_smtp_host",
                "smtp.example.com",
                SettingsCategory::Notifications,
            )
            .unwrap();
            set(
                conn,
                "email_smtp_port",
                "587",
                SettingsCategory::Notifications,
            )
            .unwrap();
            set(
                conn,
                "email_from",
                "birds@example.com",
                SettingsCategory::Notifications,
            )
            .unwrap();
            set(
                conn,
                "email_to",
                "operator@example.com",
                SettingsCategory::Notifications,
            )
            .unwrap();
        });
        let notifier = create_email_notifier(&state);
        assert!(
            notifier.is_some(),
            "email notifier should construct when smtp host is set"
        );
    }
}
