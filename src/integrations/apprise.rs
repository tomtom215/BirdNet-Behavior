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
    // Blank is "not configured", not "configured to nothing". Every surface that
    // can supply these produces a blank rather than an absent value when the
    // operator declines the feature: `docker-compose.yml` interpolates
    // `BIRDNET_APPRISE_URL: ${BIRDNET_APPRISE_URL:-}` whether or not the
    // operator set it, `.env.example` ships the key with an empty value, clap
    // hands an empty environment variable through as `Some("")`, and the
    // settings page's own hint reads "leave blank to disable HTTP push
    // notifications". Treating those as present is what let a blank URL with no
    // config file reach an `.expect` and abort the process during startup.
    let nonblank = |s: String| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_owned())
    };
    let apprise_url = cli
        .apprise_url
        .clone()
        .or_else(|| config?.get("APPRISE_URL").map(String::from))
        .and_then(nonblank);

    let apprise_config_file = cli
        .apprise_config
        .clone()
        .or_else(|| {
            config?
                .get("APPRISE_CONFIG_FILE")
                .map(std::path::PathBuf::from)
        })
        .filter(|p| !p.as_os_str().is_empty());

    // Need at least one of: URL or config file.
    if apprise_url.is_none() && apprise_config_file.is_none() {
        return None;
    }

    // Use the URL if present, or a placeholder for CLI-only mode.
    let url = apprise_url.unwrap_or_default();

    // Asking clap which arguments were really supplied replaces the old
    // "does this differ from 0.8 by more than an epsilon?" test, which treated
    // an operator who explicitly passed the default as having passed nothing.
    let min_confidence = crate::helpers::resolve::setting::<f32>(
        cli,
        "notify_confidence",
        cli.notify_confidence,
        config,
        "APPRISE_MIN_CONFIDENCE",
    );

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
        // CLI-only mode: no HTTP server configured. The guard above already
        // returned `None` when neither a URL nor a config file survived
        // blank-trimming, so a blank URL here means a config file is present —
        // but say so with a `let ... else` rather than an `.expect`, because
        // the previous `.expect` was reachable and aborted startup.
        #[allow(clippy::redundant_clone)] // else branch also borrows apprise_config_file
        let cfg_path = apprise_config_file.clone()?;
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
    fn apprise_url_typed_in_the_admin_ui_actually_builds_a_client() {
        // The settings overlay writes the UI's `apprise_url` to `APPRISE_URL`
        // before this runs, so this is the shape the web form produces. Before
        // `apprise_url` was bridged the row was stored, the /admin "Test"
        // button read it and sent successfully, and the detection path — which
        // only ever looked at the CLI flag — built no client at all, so no
        // detection notification was ever sent.
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_URL", "http://localhost:8000")]);
        assert!(
            create_apprise_client(&cli, Some(&cfg)).is_some(),
            "an Apprise URL from the settings table must produce a live client"
        );
    }

    /// Whether the built client would notify at `confidence`.
    ///
    /// Asserts through the behaviour the station actually depends on rather
    /// than reaching into the client's fields.
    fn would_notify(
        cli: &crate::cli::Cli,
        cfg: &birdnet_core::config::Config,
        confidence: f32,
    ) -> bool {
        let handle = create_apprise_client(cli, Some(cfg)).expect("client built");
        let mut client = handle.blocking_lock();
        client.should_notify("Turdus merula", confidence)
    }

    #[test]
    fn notify_confidence_from_settings_is_used_when_the_flag_is_default() {
        let mut cli = default_cli();
        cli.apprise_url = Some("http://localhost:8000".to_owned());
        let cfg = config_with(&[("APPRISE_MIN_CONFIDENCE", "0.95")]);
        assert!(
            !would_notify(&cli, &cfg, 0.90),
            "a 0.95 threshold from the settings page must suppress a 0.90 detection"
        );
        assert!(would_notify(&cli, &cfg, 0.96));
    }

    #[test]
    fn explicit_notify_confidence_flag_beats_the_settings() {
        // Replaces the old epsilon-vs-0.8 sentinel, which treated an operator
        // who explicitly passed the documented default as having passed
        // nothing — so `--notify-confidence 0.8` silently lost to the config.
        let mut cli = crate::integrations::test_support::cli_with_explicit(&["notify_confidence"]);
        cli.notify_confidence = 0.8;
        cli.apprise_url = Some("http://localhost:8000".to_owned());
        let cfg = config_with(&[("APPRISE_MIN_CONFIDENCE", "0.95")]);
        assert!(
            would_notify(&cli, &cfg, 0.85),
            "an explicit --notify-confidence 0.8 must win over the settings value"
        );
    }

    /// A blank Apprise URL means "not configured", not "crash on start".
    ///
    /// Three surfaces produce a blank one and none of them is exotic:
    /// `docker-compose.yml` interpolates `BIRDNET_APPRISE_URL: ${BIRDNET_APPRISE_URL:-}`
    /// unconditionally, `.env.example` ships the key with an empty value, and
    /// the admin settings page tells the operator in as many words to "leave
    /// blank to disable HTTP push notifications". Before this, all three
    /// reached `.expect("config file required when no URL")` and aborted the
    /// process during startup — which, under the shipped unit's
    /// `Restart=always` + `StartLimitBurst=5`, leaves an unattended station
    /// permanently `failed` rather than merely un-notified.
    #[test]
    fn a_blank_apprise_url_disables_notifications_instead_of_panicking() {
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_URL", "")]);
        assert!(
            create_apprise_client(&cli, Some(&cfg)).is_none(),
            "a blank APPRISE_URL must disable Apprise, not abort startup"
        );
    }

    /// The same value arriving on the CLI/env side rather than the config file.
    /// clap hands an empty environment variable through as `Some("")`, so this
    /// is the exact shape `docker compose up` produces.
    #[test]
    fn a_blank_apprise_url_from_the_cli_side_also_disables_notifications() {
        let mut cli = default_cli();
        cli.apprise_url = Some(String::new());
        assert!(
            create_apprise_client(&cli, None).is_none(),
            "a blank --apprise-url must disable Apprise, not abort startup"
        );
    }

    /// Whitespace is blank too: an operator who typed a space into the settings
    /// field, or a `.env` line with a trailing space, must not be a crash.
    #[test]
    fn a_whitespace_only_apprise_url_is_treated_as_blank() {
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_URL", "   ")]);
        assert!(create_apprise_client(&cli, Some(&cfg)).is_none());
    }

    /// The counterpart, so the guard above cannot degrade into "Apprise never
    /// builds": a blank URL alongside a real config file must still give the
    /// CLI-only client, which is a supported configuration.
    #[test]
    fn a_blank_url_with_a_config_file_still_builds_the_cli_only_client() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut cli = default_cli();
        cli.apprise_url = Some(String::new());
        cli.apprise_config = Some(tmp.path().to_path_buf());
        assert!(
            create_apprise_client(&cli, None).is_some(),
            "blank URL + config file is CLI-only mode, not 'disabled'"
        );
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
