//! `AppState` construction and optional-subsystem initialisers.

use std::path::PathBuf;

use crate::cli::Cli;

/// Build app state with DuckDB analytics (feature-gated).
///
/// Analytics is on by default: when neither `--analytics-db` nor the
/// `ANALYTICS_DB_PATH` config key is set, the DuckDB file is placed alongside
/// the SQLite database with a `.duckdb` extension (e.g. `birds.db` →
/// `birds.duckdb`). Operators who want analytics disabled can either pass an
/// empty `--analytics-db ""` (which we honour as opt-out) or build with
/// `--no-default-features`.
#[cfg(feature = "analytics")]
pub fn build_state_with_analytics(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    server_config: &birdnet_web::server::ServerConfig,
) -> Result<birdnet_web::state::AppState, Box<dyn std::error::Error>> {
    let cli_explicit = cli.analytics_db.clone();
    let opt_out = cli_explicit
        .as_ref()
        .is_some_and(|p| p.as_os_str().is_empty());
    if opt_out {
        tracing::info!("DuckDB analytics disabled via empty --analytics-db");
        return birdnet_web::state::AppState::new(server_config.db_path.clone())
            .map_err(|e| format!("database error: {e}").into());
    }
    let analytics_path = cli_explicit
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| config.and_then(|c| c.get("ANALYTICS_DB_PATH").map(PathBuf::from)))
        .unwrap_or_else(|| default_analytics_path(&server_config.db_path));

    tracing::info!(path = %analytics_path.display(), "enabling DuckDB analytics");
    birdnet_web::state::AppState::new_with_analytics(server_config.db_path.clone(), &analytics_path)
        .map_err(|e| format!("database error: {e}").into())
}

/// Default analytics database path — same directory and stem as the operational
/// SQLite database, with the `.duckdb` extension. Picked so installs that never
/// explicitly enable analytics still get the full feature set out of the box.
#[cfg(feature = "analytics")]
fn default_analytics_path(db_path: &std::path::Path) -> PathBuf {
    let mut p = db_path.to_path_buf();
    p.set_extension("duckdb");
    p
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
}
