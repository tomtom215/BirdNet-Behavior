//! `AppState` construction and optional-subsystem initialisers.

use std::path::PathBuf;

use crate::cli::Cli;

/// Build app state with DuckDB analytics (feature-gated).
#[cfg(feature = "analytics")]
pub fn build_state_with_analytics(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    server_config: &birdnet_web::server::ServerConfig,
) -> Result<birdnet_web::state::AppState, Box<dyn std::error::Error>> {
    let analytics_path = cli
        .analytics_db
        .clone()
        .or_else(|| config?.get("ANALYTICS_DB_PATH").map(PathBuf::from));

    if let Some(ref analytics_path) = analytics_path {
        tracing::info!(path = %analytics_path.display(), "enabling DuckDB analytics");
        birdnet_web::state::AppState::new_with_analytics(
            server_config.db_path.clone(),
            analytics_path,
        )
        .map_err(|e| format!("database error: {e}").into())
    } else {
        birdnet_web::state::AppState::new(server_config.db_path.clone())
            .map_err(|e| format!("database error: {e}").into())
    }
}

/// Initialize the species image cache.
pub fn init_image_cache(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let cache_dir = cli
        .image_cache_dir
        .clone()
        .or_else(|| config?.get("IMAGE_CACHE_DIR").map(PathBuf::from));

    let Some(ref cache_dir) = cache_dir else {
        return state;
    };

    match birdnet_integrations::species_images::ImageCache::with_wikipedia(cache_dir) {
        Ok(cache) => {
            tracing::info!(
                path = %cache_dir.display(),
                cached = cache.cached_count(),
                "species image cache enabled"
            );
            state.with_image_cache(cache)
        }
        Err(e) => {
            tracing::warn!(error = %e, "species image cache not available (non-fatal)");
            state
        }
    }
}

/// Initialize i18n language settings.
pub fn init_i18n(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let lang = if cli.lang == "en" {
        config
            .and_then(|c| c.get("DATABASE_LANG"))
            .map_or_else(|| "en".to_string(), std::string::ToString::to_string)
    } else {
        cli.lang.clone()
    };

    if lang == "en" {
        return state;
    }

    let labels_dir = cli
        .labels_dir
        .clone()
        .or_else(|| config?.get("LABELS_DIR").map(PathBuf::from));

    let Some(labels_dir) = labels_dir else {
        tracing::warn!(lang = %lang, "language set but no --labels-dir configured");
        return state;
    };

    let mut mgr = birdnet_core::i18n::I18nManager::new(&lang);
    match mgr.load_language(&lang, &labels_dir) {
        Ok(()) => {
            tracing::info!(lang = %lang, "i18n language loaded");
            state.with_i18n(mgr)
        }
        Err(e) => {
            tracing::warn!(lang = %lang, error = %e, "failed to load language pack");
            state
        }
    }
}

/// Initialize audio source for live streaming.
pub fn init_audio_source(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let source = cli
        .rtsp_url
        .clone()
        .or_else(|| cli.alsa_device.clone())
        .or_else(|| config?.get("RTSP_URL").map(String::from))
        .or_else(|| config?.get("ALSA_CARD").map(String::from));

    match source {
        Some(src) => {
            tracing::info!(source = %src, "live audio stream source configured");
            state.with_audio_source(src)
        }
        None => state,
    }
}

/// Initialize custom site name.
pub fn init_site_name(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let name = cli
        .site_name
        .clone()
        .or_else(|| config?.get("SITENAME").map(String::from));

    match name {
        Some(n) if !n.is_empty() => {
            tracing::info!(site_name = %n, "custom site name configured");
            state.with_site_name(n)
        }
        _ => state,
    }
}

/// Reinstall the behavioral `DuckDB` extension and exit (`--refresh-extension`).
///
/// Resolves the analytics database path the same way
/// [`build_state_with_analytics`] does, opens it, force-reinstalls the latest
/// community build, and loads it to verify. Any failure (no path configured,
/// offline, or a version mismatch) is returned so the process exits non-zero
/// with a clear message.
///
/// # Errors
///
/// Returns an error if no analytics path is configured, the database cannot be
/// opened, or the extension cannot be reinstalled and loaded.
#[cfg(feature = "analytics")]
pub fn run_refresh_extension(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    use birdnet_behavioral::connection::AnalyticsDb;

    let analytics_path = cli
        .analytics_db
        .clone()
        .or_else(|| config?.get("ANALYTICS_DB_PATH").map(PathBuf::from))
        .ok_or(
            "no analytics database configured — set --analytics-db or ANALYTICS_DB_PATH \
             before --refresh-extension",
        )?;

    tracing::info!(path = %analytics_path.display(), "refreshing behavioral DuckDB extension");
    let mut adb = AnalyticsDb::open(&analytics_path).map_err(|e| format!("DuckDB error: {e}"))?;
    adb.refresh_extension()
        .map_err(|e| format!("failed to refresh behavioral extension: {e}"))?;
    tracing::info!(
        duckdb = adb.duckdb_version().as_deref().unwrap_or("unknown"),
        extension = adb.extension_version().as_deref().unwrap_or("unknown"),
        "behavioral extension reinstalled and loaded"
    );
    Ok(())
}

/// Without the `analytics` feature there is no `DuckDB` extension to refresh.
///
/// # Errors
///
/// Always returns an error explaining that the `analytics` feature is required.
#[cfg(not(feature = "analytics"))]
pub fn run_refresh_extension(
    _cli: &Cli,
    _config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "--refresh-extension requires the `analytics` feature; rebuild with `--features analytics`"
            .into(),
    )
}

/// Resolve the auto-update check interval from an optional configured value.
///
/// Defaults to 24 hours; a positive integer count of hours overrides it.
/// Malformed, zero, or absent values fall back to the default. Split from the
/// env lookup so the parsing is unit-testable without touching process env.
#[cfg(feature = "analytics")]
fn resolve_update_interval(configured: Option<&str>) -> std::time::Duration {
    const DEFAULT_HOURS: u64 = 24;
    let hours = configured
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&h| h > 0)
        .unwrap_or(DEFAULT_HOURS);
    std::time::Duration::from_secs(hours.saturating_mul(3600))
}

/// Interval between behavioral-extension auto-update checks, from
/// `BIRDNET_ANALYTICS_UPDATE_INTERVAL_HOURS` (default 24).
#[cfg(feature = "analytics")]
fn analytics_update_interval() -> std::time::Duration {
    resolve_update_interval(
        std::env::var("BIRDNET_ANALYTICS_UPDATE_INTERVAL_HOURS")
            .ok()
            .as_deref(),
    )
}

/// Spawn the background task that keeps the behavioral extension up to date.
///
/// After a short startup delay it runs an update check, then repeats every
/// [`analytics_update_interval`]. The initial extension load already happened
/// during `AppState` construction; this only pulls newer community builds.
/// Failures (offline, or no matching build published yet) are logged at debug
/// and retried on the next tick, so a station picks up a freshly published
/// build without a manual reinstall.
#[cfg(feature = "analytics")]
pub fn spawn_extension_auto_update(state: birdnet_web::state::AppState) {
    use birdnet_behavioral::connection::{AnalyticsDb, ExtensionUpdate};

    let interval = analytics_update_interval();
    tokio::spawn(async move {
        // Brief delay so the first check doesn't compete with startup I/O.
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            let st = state.clone();
            // The update touches DuckDB and the network, so run it on the
            // blocking pool; map the error to a String inside the closure so
            // the result is `Send` regardless of the DuckDB error type.
            let result = tokio::task::spawn_blocking(move || {
                st.with_analytics_mut(|db: &mut AnalyticsDb| {
                    let outcome = db.update_extension().map_err(|e| e.to_string());
                    (outcome, db.extension_version())
                })
            })
            .await;
            match result {
                Ok(Some((Ok(outcome), version))) => {
                    let v = version.as_deref().unwrap_or("unknown");
                    match outcome {
                        ExtensionUpdate::Installed => {
                            tracing::info!(
                                version = v,
                                "behavioral extension installed by auto-update"
                            );
                        }
                        ExtensionUpdate::Checked => {
                            tracing::debug!(
                                version = v,
                                "checked behavioral extension for updates"
                            );
                        }
                    }
                }
                Ok(Some((Err(e), _))) => {
                    tracing::debug!(error = %e, "behavioral extension auto-update failed (non-fatal)");
                }
                // Analytics is not configured (or went away) — nothing to do.
                Ok(None) => break,
                Err(e) => {
                    tracing::debug!(error = %e, "behavioral extension auto-update task panicked");
                }
            }
            tokio::time::sleep(interval).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{init_audio_source, init_i18n, init_image_cache, init_site_name};
    use crate::helpers::test_support::{config_with, default_cli, test_state};

    // ── init_audio_source ──────────────────────────────────────────────

    #[test]
    fn audio_source_prefers_cli_rtsp_over_alsa() {
        // CLI sets both; rtsp_url wins by ordering in the helper.
        let mut cli = default_cli();
        cli.rtsp_url = Some("rtsp://camera.local/stream".to_owned());
        cli.alsa_device = Some("plughw:1,0".to_owned());
        let state = init_audio_source(test_state(), &cli, None);
        assert_eq!(state.audio_source(), Some("rtsp://camera.local/stream"));
    }

    #[test]
    fn audio_source_uses_cli_alsa_when_no_rtsp() {
        let mut cli = default_cli();
        cli.alsa_device = Some("plughw:1,0".to_owned());
        let state = init_audio_source(test_state(), &cli, None);
        assert_eq!(state.audio_source(), Some("plughw:1,0"));
    }

    #[test]
    fn audio_source_falls_back_to_config_rtsp() {
        let cli = default_cli();
        let cfg = config_with(&[("RTSP_URL", "rtsp://config.local/cam")]);
        let state = init_audio_source(test_state(), &cli, Some(&cfg));
        assert_eq!(state.audio_source(), Some("rtsp://config.local/cam"));
    }

    #[test]
    fn audio_source_falls_back_to_config_alsa() {
        let cli = default_cli();
        let cfg = config_with(&[("ALSA_CARD", "hw:1,0")]);
        let state = init_audio_source(test_state(), &cli, Some(&cfg));
        assert_eq!(state.audio_source(), Some("hw:1,0"));
    }

    #[test]
    fn audio_source_none_when_nothing_configured() {
        let cli = default_cli();
        let state = init_audio_source(test_state(), &cli, None);
        assert_eq!(state.audio_source(), None);
    }

    // ── init_site_name ─────────────────────────────────────────────────

    #[test]
    fn site_name_prefers_cli_over_config() {
        let mut cli = default_cli();
        cli.site_name = Some("CliStation".to_owned());
        let cfg = config_with(&[("SITENAME", "ConfigStation")]);
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "CliStation");
    }

    #[test]
    fn site_name_uses_config_when_cli_absent() {
        let cli = default_cli();
        let cfg = config_with(&[("SITENAME", "ConfigStation")]);
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "ConfigStation");
    }

    #[test]
    fn site_name_ignores_empty_string() {
        // An empty SITENAME is treated as not-configured; the default
        // "BirdNet-Behavior" remains in place.
        let cli = default_cli();
        let cfg = config_with(&[("SITENAME", "")]);
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "BirdNet-Behavior");
    }

    #[test]
    fn site_name_none_when_nothing_configured() {
        let cli = default_cli();
        let state = init_site_name(test_state(), &cli, None);
        // Default fallback exposed via `site_name()`.
        assert_eq!(state.site_name(), "BirdNet-Behavior");
    }

    // ── init_i18n ──────────────────────────────────────────────────────

    #[test]
    fn i18n_noop_when_lang_is_english() {
        // English is the default; no language pack needs loading.
        let cli = default_cli();
        let state = init_i18n(test_state(), &cli, None);
        // i18n manager not installed — i18n_ref returns None.
        assert!(state.with_i18n_ref(|_| ()).is_none());
    }

    #[test]
    fn i18n_warns_and_returns_state_when_no_labels_dir() {
        // lang != "en" but no labels_dir → helper logs a warning and
        // returns state unchanged. The unit test pins the unchanged
        // return; the warning emission lives in tracing and is verified
        // by hand.
        let mut cli = default_cli();
        cli.lang = "de".to_owned();
        let state = init_i18n(test_state(), &cli, None);
        assert!(state.with_i18n_ref(|_| ()).is_none());
    }

    #[test]
    fn i18n_uses_config_database_lang_when_cli_default() {
        // CLI defaults to "en"; config overrides to "de" but no labels
        // dir is provided, so the helper still logs and returns
        // unchanged state.
        let cli = default_cli();
        let cfg = config_with(&[("DATABASE_LANG", "de")]);
        let state = init_i18n(test_state(), &cli, Some(&cfg));
        assert!(state.with_i18n_ref(|_| ()).is_none());
    }

    // ── init_image_cache ───────────────────────────────────────────────

    #[test]
    fn image_cache_noop_when_neither_cli_nor_config_set() {
        let cli = default_cli();
        let state = init_image_cache(test_state(), &cli, None);
        assert!(state.image_cache().is_none());
    }

    #[test]
    fn image_cache_installed_when_cli_dir_set() {
        // The directory just needs to be writable; the cache uses
        // Wikipedia as the backing provider but only the directory
        // matters for the construction path that this unit covers.
        let tmp = tempfile::tempdir().unwrap();
        let mut cli = default_cli();
        cli.image_cache_dir = Some(tmp.path().to_path_buf());
        let state = init_image_cache(test_state(), &cli, None);
        assert!(state.image_cache().is_some());
    }

    #[test]
    fn image_cache_installed_when_config_dir_set() {
        let tmp = tempfile::tempdir().unwrap();
        let cli = default_cli();
        let cfg = config_with(&[(
            "IMAGE_CACHE_DIR",
            tmp.path().to_str().expect("tempdir path is utf8"),
        )]);
        let state = init_image_cache(test_state(), &cli, Some(&cfg));
        assert!(state.image_cache().is_some());
    }

    // ── resolve_update_interval ────────────────────────────────────────

    #[cfg(feature = "analytics")]
    #[test]
    fn update_interval_defaults_when_absent_or_invalid() {
        use std::time::Duration;
        let day = Duration::from_secs(24 * 3600);
        assert_eq!(super::resolve_update_interval(None), day);
        assert_eq!(super::resolve_update_interval(Some("")), day);
        assert_eq!(super::resolve_update_interval(Some("abc")), day);
        // Zero would mean "never check" — treat as invalid and use the default.
        assert_eq!(super::resolve_update_interval(Some("0")), day);
    }

    #[cfg(feature = "analytics")]
    #[test]
    fn update_interval_honours_positive_hours() {
        use std::time::Duration;
        assert_eq!(
            super::resolve_update_interval(Some("1")),
            Duration::from_secs(3600)
        );
        assert_eq!(
            super::resolve_update_interval(Some(" 12 ")),
            Duration::from_secs(12 * 3600)
        );
    }
}
