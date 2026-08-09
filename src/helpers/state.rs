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
///
/// Image caching is on by default: when neither `--image-cache-dir` nor the
/// `IMAGE_CACHE_DIR` config key is set, the cache is placed in an `images/`
/// subdirectory beside the operational SQLite database (mirroring
/// [`default_analytics_path`]), so a stock install shows species photos out of
/// the box — matching BirdNET-Pi and the analytics default. Operators who want
/// it off (e.g. air-gapped deployments that must not reach Wikipedia on demand)
/// can pass an empty `--image-cache-dir ""` or set `IMAGE_CACHE_DIR=`, which we
/// honour as an opt-out.
pub fn init_image_cache(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    db_path: &std::path::Path,
) -> birdnet_web::state::AppState {
    // Offline mode is the master switch: no species-image downloads at all.
    // Checked before the per-feature opt-out so an operator who set `--offline`
    // does not also have to know about `--image-cache-dir ""`.
    if !super::egress::image_downloads_allowed(cli) {
        tracing::info!(
            "species image downloads disabled by offline mode; \
             already-cached images are still served"
        );
        return state;
    }

    // CLI wins over config; an explicitly empty value is an opt-out.
    let configured = cli
        .image_cache_dir
        .clone()
        .or_else(|| config.and_then(|c| c.get("IMAGE_CACHE_DIR").map(PathBuf::from)));

    if configured
        .as_ref()
        .is_some_and(|p| p.as_os_str().is_empty())
    {
        tracing::info!("species image cache disabled via empty image-cache-dir");
        return state;
    }

    let cache_dir = configured.unwrap_or_else(|| default_image_cache_dir(db_path));

    match birdnet_integrations::species_images::ImageCache::with_wikipedia(&cache_dir) {
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

/// Default species-image cache directory — an `images/` subdirectory beside the
/// operational SQLite database. Mirrors [`default_analytics_path`] so a stock
/// install shows species photos without extra configuration.
fn default_image_cache_dir(db_path: &std::path::Path) -> PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("images")
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

/// Verify the behavioral `DuckDB` extension loads, and exit (`--verify-extension`).
///
/// Opens a throwaway database in the system temp directory — never the
/// station's analytics store, so this is safe to run against a live install —
/// and loads the extension through the normal fallback chain.
///
/// Run with networking disabled to prove the *offline* guarantee: with no
/// network the cached and community-registry stages cannot succeed, so success
/// means the build-time embedded copy loaded. That is the property `docker.yml`
/// asserts on every image, and the one that was silently false for every Docker
/// image built before 2026-08-08 (the `Dockerfile` embedded an extension built
/// for DuckDB v1.5.3 into a v1.5.5 engine).
///
/// # Errors
///
/// Returns an error if the temporary database cannot be opened or the extension
/// cannot be loaded, so the process exits non-zero.
#[cfg(feature = "analytics")]
pub fn run_verify_extension(
    _cli: &Cli,
    _config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    use birdnet_behavioral::connection::AnalyticsDb;

    let dir = std::env::temp_dir().join("birdnet-verify-extension");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join("verify.duckdb");
    // A stale file from a previous run would still work, but starting clean
    // keeps the check honest about what a fresh station sees.
    let _ = std::fs::remove_file(&path);

    let mut adb = AnalyticsDb::open(&path).map_err(|e| format!("DuckDB error: {e}"))?;
    let engine = adb
        .duckdb_version()
        .unwrap_or_else(|| "unknown".to_string());

    // Reported whether or not the load succeeds: when a networked station masks
    // a bad embed by installing from the registry, this is the line that shows
    // the packaging is still wrong.
    tracing::info!(
        engine = %engine,
        embedded_extension = AnalyticsDb::embedded_extension_version().unwrap_or("<none embedded>"),
        embedded_for_duckdb =
            AnalyticsDb::embedded_extension_duckdb_version().unwrap_or("<none embedded>"),
        embedded_platform = AnalyticsDb::embedded_extension_platform().unwrap_or("<none embedded>"),
        "behavioral extension: build-time embedding"
    );

    if let Some(mismatch) = adb.embedded_extension_mismatch() {
        return Err(format!(
            "the embedded behavioral extension targets DuckDB {} but this binary links DuckDB {}; \
             it can never load. Rebuild with the extension published for {} \
             (community-extensions.duckdb.org/{}/<platform>/).",
            mismatch.embedded_for, mismatch.engine, mismatch.engine, mismatch.engine
        )
        .into());
    }

    adb.load_extension()
        .map_err(|e| format!("behavioral extension did not load: {e}"))?;

    tracing::info!(
        duckdb = %engine,
        extension = adb.extension_version().as_deref().unwrap_or("unknown"),
        path = %path.display(),
        "behavioral extension loaded"
    );
    let _ = std::fs::remove_file(&path);
    Ok(())
}

/// Without the `analytics` feature there is no `DuckDB` extension to verify.
///
/// # Errors
///
/// Always returns an error explaining that the `analytics` feature is required.
#[cfg(not(feature = "analytics"))]
pub fn run_verify_extension(
    _cli: &Cli,
    _config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "--verify-extension requires the `analytics` feature; rebuild with `--features analytics`"
            .into(),
    )
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
    use super::{init_i18n, init_image_cache, init_site_name};
    use crate::helpers::test_support::{config_with, default_cli, test_state};

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
    fn image_cache_defaults_on_beside_db_when_unconfigured() {
        // Neither CLI nor config set ⇒ the cache defaults to `<db_dir>/images`
        // so a stock install shows species photos out of the box.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("birds.db");
        let cli = default_cli();
        let state = init_image_cache(test_state(), &cli, None, &db_path);
        assert!(state.image_cache().is_some());
        assert!(
            tmp.path().join("images").is_dir(),
            "default cache dir should be created beside the database"
        );
    }

    #[test]
    fn image_cache_disabled_via_empty_cli_value() {
        // `--image-cache-dir ""` is an explicit opt-out (e.g. air-gapped).
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("birds.db");
        let mut cli = default_cli();
        cli.image_cache_dir = Some(std::path::PathBuf::new());
        let state = init_image_cache(test_state(), &cli, None, &db_path);
        assert!(state.image_cache().is_none());
        assert!(
            !tmp.path().join("images").exists(),
            "opt-out must not create the default cache dir"
        );
    }

    #[test]
    fn image_cache_disabled_via_empty_config_value() {
        // `IMAGE_CACHE_DIR=` in the config file is also an opt-out.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("birds.db");
        let cli = default_cli();
        let cfg = config_with(&[("IMAGE_CACHE_DIR", "")]);
        let state = init_image_cache(test_state(), &cli, Some(&cfg), &db_path);
        assert!(state.image_cache().is_none());
    }

    #[test]
    fn image_cache_installed_when_cli_dir_set() {
        // The directory just needs to be writable; the cache uses
        // Wikipedia as the backing provider but only the directory
        // matters for the construction path that this unit covers. The
        // db_path is unused when an explicit dir is configured.
        let tmp = tempfile::tempdir().unwrap();
        let mut cli = default_cli();
        cli.image_cache_dir = Some(tmp.path().to_path_buf());
        let state = init_image_cache(test_state(), &cli, None, std::path::Path::new(":memory:"));
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
        let state = init_image_cache(
            test_state(),
            &cli,
            Some(&cfg),
            std::path::Path::new(":memory:"),
        );
        assert!(state.image_cache().is_some());
    }
}
