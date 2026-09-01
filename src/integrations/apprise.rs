//! Apprise push-notification client construction.

use std::path::Path;
use std::sync::Arc;

use crate::cli::Cli;

/// Native destinations resolved from the operator's configuration.
pub struct NativeRoutes {
    /// Destinations delivered in-process.
    pub routes: Vec<birdnet_integrations::dispatch::Route>,
    /// Whether the `apprise` CLI must still run for the config file.
    pub cli_fallback: bool,
}

/// Work out what can be delivered natively and what still needs Apprise.
///
/// `--notify-urls` is always native: a scheme with no native sender there is
/// reported and dropped, because that flag exists precisely to avoid Apprise.
///
/// An Apprise **config file** is all-or-nothing. If every URL in it has a
/// native sender the file is delivered in-process and the `apprise` CLI is
/// never invoked. If even one does not, the whole file goes to the CLI — the
/// CLI has no way to be told "send to all but these", so routing part of the
/// file natively *and* running the CLI would deliver the rest twice.
///
/// A file that cannot be read is left to the CLI rather than assumed empty.
fn native_routes(notify_urls: Option<&str>, config_file: Option<&Path>) -> NativeRoutes {
    use birdnet_integrations::dispatch;

    let mut routes = Vec::new();

    if let Some(urls) = notify_urls {
        let parsed = dispatch::routes(urls);
        if !parsed.deferred.is_empty() {
            tracing::warn!(
                schemes = %parsed.deferred.join(", "),
                "--notify-urls has no native sender for these schemes; \
                 configure them in an Apprise config file instead"
            );
        }
        for route in &parsed.native {
            tracing::info!(destination = %route.label, "native notification destination");
        }
        routes.extend(parsed.native);
    }

    let Some(path) = config_file else {
        return NativeRoutes {
            routes,
            cli_fallback: false,
        };
    };

    let Ok(contents) = std::fs::read_to_string(path) else {
        tracing::debug!(
            path = %path.display(),
            "Apprise config file not readable here; leaving it to the apprise CLI"
        );
        return NativeRoutes {
            routes,
            cli_fallback: true,
        };
    };

    let parsed = dispatch::routes(&contents);
    if parsed.deferred.is_empty() && parsed.unparseable == 0 && !parsed.native.is_empty() {
        for route in &parsed.native {
            tracing::info!(destination = %route.label, "native notification destination");
        }
        routes.extend(parsed.native);
        tracing::info!(
            path = %path.display(),
            "every URL in the Apprise config file is delivered natively; \
             the apprise CLI will not be invoked"
        );
        return NativeRoutes {
            routes,
            cli_fallback: false,
        };
    }

    if !parsed.deferred.is_empty() {
        tracing::info!(
            path = %path.display(),
            schemes = %parsed.deferred.join(", "),
            "Apprise config file contains schemes without a native sender; \
             delivering the whole file through the apprise CLI"
        );
    }
    NativeRoutes {
        routes,
        cli_fallback: true,
    }
}

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

    // Notification URLs delivered in-process. Apprise's syntax, none of its
    // runtime: a station configured only with these needs no Python at all.
    let notify_urls = cli
        .notify_urls
        .clone()
        .or_else(|| config?.get("NOTIFY_URLS").map(String::from))
        .and_then(nonblank);

    // Need at least one of: native URLs, an Apprise server, or a config file.
    if apprise_url.is_none() && apprise_config_file.is_none() && notify_urls.is_none() {
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

    let native = native_routes(notify_urls.as_deref(), apprise_config_file.as_deref());

    let client_result = if url.is_empty() {
        // CLI-only mode: no HTTP server configured. The guard above already
        // returned `None` when neither a URL nor a config file survived
        // blank-trimming, so a blank URL here means a config file is present —
        // but say so with a `let ... else` rather than an `.expect`, because
        // the previous `.expect` was reachable and aborted startup.
        // No Apprise server. Either a config file, native URLs, or both.
        // `new_cli_only` wants a path, so hand it the config file when there
        // is one and a placeholder when the operator supplied only URLs — the
        // CLI is not invoked in that case, because `cli_fallback` is false.
        let cfg_path = apprise_config_file.unwrap_or_default();
        if cfg_path.as_os_str().is_empty() {
            tracing::info!(
                destinations = native.routes.len(),
                "native notifications enabled (no Apprise involved)"
            );
        } else {
            tracing::info!(
                path = %cfg_path.display(),
                native = native.routes.len(),
                apprise_cli = native.cli_fallback,
                "Apprise CLI-only notifications enabled"
            );
        }
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
        Ok(client) => Some(Arc::new(tokio::sync::Mutex::new(
            client.with_native_routes(native.routes, native.cli_fallback),
        ))),
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

    // ── native routing vs. the apprise CLI ──────────────────────────────
    //
    // The rule these pin: an Apprise config file is all-or-nothing. Getting
    // this wrong is silent in both directions — half the file delivered twice,
    // or half of it never delivered at all.

    use std::io::Write as _;

    /// Write `contents` to a temp file and resolve its routes.
    fn routes_for_file(contents: &str) -> super::NativeRoutes {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f.flush().unwrap();
        super::native_routes(None, Some(f.path()))
    }

    #[test]
    fn a_config_file_of_only_native_schemes_does_not_need_the_apprise_cli() {
        let r = routes_for_file(
            "discord://1234/abcd\n\
             ntfy://my-garden\n\
             # a comment\n\
             tgram://999:secret/12315544\n",
        );
        assert_eq!(r.routes.len(), 3);
        assert!(
            !r.cli_fallback,
            "with every URL delivered natively the apprise CLI must not run too"
        );
    }

    #[test]
    fn one_unsupported_scheme_sends_the_whole_file_through_the_cli() {
        // Counterpart to the gate above, and the reason it is all-or-nothing:
        // `apprise -c file` sends to every URL in the file. Routing the two
        // native ones here *and* running the CLI would deliver them twice.
        let r = routes_for_file(
            "discord://1234/abcd\n\
             ntfy://my-garden\n\
             matrix://user:pass@matrix.org/#birds\n",
        );
        assert!(
            r.routes.is_empty(),
            "nothing may be routed natively when the CLI still has to run"
        );
        assert!(r.cli_fallback);
    }

    #[test]
    fn an_unreadable_config_file_is_left_to_the_cli() {
        // Assuming an empty file would silently disable every channel in it.
        let r = super::native_routes(None, Some(std::path::Path::new("/nonexistent/apprise.yml")));
        assert!(r.routes.is_empty());
        assert!(r.cli_fallback, "an unread file must not be assumed empty");
    }

    #[test]
    fn notify_urls_alone_never_invokes_the_cli() {
        let r = super::native_routes(Some("discord://1234/abcd,ntfy://garden"), None);
        assert_eq!(r.routes.len(), 2);
        assert!(!r.cli_fallback);
    }

    #[test]
    fn an_unsupported_scheme_in_notify_urls_is_dropped_not_smuggled_to_the_cli() {
        // `--notify-urls` exists to avoid Apprise, so it cannot quietly turn
        // the CLI back on; the startup warning is what tells the operator.
        let r = super::native_routes(Some("mailto://user:pass@gmail.com"), None);
        assert!(r.routes.is_empty());
        assert!(!r.cli_fallback);
    }

    #[test]
    fn notify_urls_alone_builds_a_client() {
        // Before this, the constructor returned `None` unless an Apprise URL
        // or config file was set, so a station configured only with native
        // URLs would have sent nothing at all.
        let mut cli = default_cli();
        cli.notify_urls = Some("ntfy://my-garden".to_owned());
        assert!(create_apprise_client(&cli, None).is_some());
    }

    #[test]
    fn the_client_reports_whether_apprise_is_needed() {
        let mut cli = default_cli();
        cli.notify_urls = Some("discord://1234/abcd".to_owned());
        let handle = create_apprise_client(&cli, None).expect("client");
        let (labels, needs_cli) = {
            let client = handle.blocking_lock();
            (
                client
                    .native_labels()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                client.needs_apprise_cli(),
            )
        };
        assert_eq!(labels, ["discord"]);
        assert!(
            !needs_cli,
            "a station configured only with native URLs must not shell out"
        );
    }
}
