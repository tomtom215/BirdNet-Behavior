//! Apprise push-notification client construction.

use std::sync::Arc;

use crate::cli::Cli;

/// Type alias for the shared Apprise client handle.
pub type AppriseHandle = Arc<tokio::sync::Mutex<birdnet_integrations::apprise::Client>>;

/// Create an Apprise notification client from CLI flags and/or config file values.
///
/// Returns `None` if neither an Apprise URL nor config file is configured.
pub fn create_apprise_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<AppriseHandle> {
    let apprise_url = cli
        .apprise_url
        .clone()
        .or_else(|| config?.get("APPRISE_URL").map(String::from));

    let apprise_config_file = cli.apprise_config.clone().or_else(|| {
        config?
            .get("APPRISE_CONFIG_FILE")
            .map(std::path::PathBuf::from)
    });

    // Need at least one of: URL or config file.
    if apprise_url.is_none() && apprise_config_file.is_none() {
        return None;
    }

    // Use the URL if present, or a placeholder for CLI-only mode.
    let url = apprise_url.unwrap_or_default();

    let min_confidence = if (cli.notify_confidence - 0.8).abs() > f32::EPSILON {
        cli.notify_confidence
    } else {
        config
            .and_then(|c| c.get_parsed::<f32>("APPRISE_MIN_CONFIDENCE").ok())
            .unwrap_or(cli.notify_confidence)
    };

    let cooldown_secs = config
        .and_then(|c| c.get_parsed::<u64>("APPRISE_COOLDOWN").ok())
        .unwrap_or(300);

    // Helper to split a comma-separated config value into a Vec<String>.
    let parse_species_list = |list: &str| -> Vec<String> {
        list.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let species_watchlist = config
        .and_then(|c| c.get("APPRISE_WATCHLIST"))
        .map(parse_species_list)
        .unwrap_or_default();

    // Dual-filter: exclude list from config file OR CLI --notify-species-exclude.
    let species_notify_exclude = {
        let from_config = config
            .and_then(|c| c.get("APPRISE_WATCHLIST_EXCLUDE"))
            .map(parse_species_list)
            .unwrap_or_default();
        let from_cli = cli
            .notify_species_exclude
            .as_deref()
            .map(parse_species_list)
            .unwrap_or_default();
        // Merge both sources; dedup not strictly necessary but keeps it clean.
        let mut merged = from_config;
        for s in from_cli {
            if !merged.contains(&s) {
                merged.push(s);
            }
        }
        merged
    };

    let notify_config = birdnet_integrations::apprise::NotifyConfig {
        min_confidence,
        species_watchlist,
        species_notify_exclude,
        cooldown: std::time::Duration::from_secs(cooldown_secs),
        per_species_cooldown: std::collections::HashMap::new(),
    };

    let client_result = if url.is_empty() {
        // CLI-only mode: no HTTP server configured.
        #[allow(clippy::redundant_clone)] // else branch also borrows apprise_config_file
        let cfg_path = apprise_config_file
            .clone()
            .expect("config file required when no URL");
        tracing::info!(
            path = %cfg_path.display(),
            "Apprise CLI-only notifications enabled"
        );
        birdnet_integrations::apprise::Client::new_cli_only(cfg_path, notify_config)
    } else {
        birdnet_integrations::apprise::Client::new(&url, notify_config).map(|c| {
            if let Some(ref cfg_path) = apprise_config_file {
                tracing::info!(
                    url = %url,
                    path = %cfg_path.display(),
                    min_confidence = %min_confidence,
                    cooldown_secs,
                    "Apprise notifications enabled (HTTP + CLI config)"
                );
                c.with_config_file(cfg_path.clone())
            } else {
                tracing::info!(
                    url = %url,
                    min_confidence = %min_confidence,
                    cooldown_secs,
                    "Apprise notifications enabled"
                );
                c
            }
        })
    };

    match client_result {
        Ok(client) => Some(Arc::new(tokio::sync::Mutex::new(client))),
        Err(e) => {
            tracing::error!(error = %e, "failed to create Apprise client");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::create_apprise_client;
    use crate::integrations::test_support::{config_with, default_cli};

    #[test]
    fn apprise_is_none_when_neither_url_nor_config_file_set() {
        let cli = default_cli();
        assert!(create_apprise_client(&cli, None).is_none());
    }

    #[test]
    fn apprise_built_from_cli_url() {
        let mut cli = default_cli();
        cli.apprise_url = Some("http://localhost:8000".to_owned());
        assert!(create_apprise_client(&cli, None).is_some());
    }

    #[test]
    fn apprise_built_from_config_url() {
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_URL", "http://broker:8000")]);
        assert!(create_apprise_client(&cli, Some(&cfg)).is_some());
    }

    #[test]
    fn apprise_built_in_cli_only_mode_when_config_file_set() {
        // CLI-only mode: no HTTP URL, only an Apprise config file. The
        // helper still returns a handle; notifications fan out via the
        // `apprise` CLI rather than the HTTP API.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut cli = default_cli();
        cli.apprise_config = Some(tmp.path().to_path_buf());
        assert!(create_apprise_client(&cli, None).is_some());
    }
}
