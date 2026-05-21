//! Heartbeat / uptime-monitor ping client construction.

use std::sync::Arc;

use crate::cli::Cli;

/// Type alias for the heartbeat client handle.
pub type HeartbeatHandle = Arc<birdnet_integrations::heartbeat::HeartbeatClient>;

/// Create a heartbeat client from CLI flags and/or config file values.
///
/// Returns `None` if no heartbeat URL is configured.
pub fn create_heartbeat_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<HeartbeatHandle> {
    let url = cli
        .heartbeat_url
        .clone()
        .or_else(|| config?.get("HEARTBEAT_URL").map(String::from))?;

    match birdnet_integrations::heartbeat::HeartbeatClient::new(&url) {
        Ok(client) => {
            tracing::info!(url = %url, "heartbeat monitoring enabled");
            Some(Arc::new(client))
        }
        Err(e) => {
            tracing::warn!(error = %e, "heartbeat client not created");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::create_heartbeat_client;
    use crate::integrations::test_support::{config_with, default_cli};

    #[test]
    fn heartbeat_is_none_without_url() {
        let cli = default_cli();
        assert!(create_heartbeat_client(&cli, None).is_none());
    }

    #[test]
    fn heartbeat_built_from_cli_url() {
        let mut cli = default_cli();
        cli.heartbeat_url = Some("https://heartbeat.example/ping".to_owned());
        assert!(create_heartbeat_client(&cli, None).is_some());
    }

    #[test]
    fn heartbeat_built_from_config_url() {
        let cli = default_cli();
        let cfg = config_with(&[("HEARTBEAT_URL", "https://hb.example/ping")]);
        assert!(create_heartbeat_client(&cli, Some(&cfg)).is_some());
    }
}
