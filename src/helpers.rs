//! Helper functions for the main binary entry point.
//!
//! Extracted from `main.rs` for modularity. Contains database utilities,
//! startup initialization, and system integration helpers.

use std::path::PathBuf;

use crate::cli::Cli;

/// Resolve the database path from config, falling back to a default location.
pub fn db_path_from_config(config: Option<&birdnet_core::config::Config>) -> PathBuf {
    config.and_then(|c| c.get("DB_PATH")).map_or_else(
        || {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/pi".into());
            PathBuf::from(format!("{home}/BirdNet-Behavior/birds.db"))
        },
        PathBuf::from,
    )
}

/// Run a database integrity check and exit.
pub fn run_integrity_check(
    config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = db_path_from_config(config);
    tracing::info!(path = %db_path.display(), "running integrity check");
    match birdnet_db::resilience::full_integrity_check(&db_path) {
        Ok(true) => {
            tracing::info!("database integrity check PASSED");
            Ok(())
        }
        Ok(false) => {
            tracing::error!("database integrity check FAILED — corruption detected");
            std::process::exit(1);
        }
        Err(e) => Err(Box::new(e)),
    }
}

/// Create a database backup and exit.
pub fn run_backup(
    config: Option<&birdnet_core::config::Config>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = db_path_from_config(config);
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");
    tracing::info!(path = %db_path.display(), "creating database backup");
    let backup_path = birdnet_db::resilience::backup_database(&db_path, &backup_dir)?;
    tracing::info!(backup = %backup_path.display(), "backup created successfully");
    Ok(())
}

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

/// Start the disk manager as a background thread.
pub fn start_disk_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) -> Option<std::thread::JoinHandle<()>> {
    use birdnet_core::audio::capture::{DiskManager, DiskManagerConfig, FullDiskAction};

    let monitored_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))?;

    let max_files_per_species = if cli.max_files_per_species > 0 {
        cli.max_files_per_species
    } else {
        config
            .and_then(|c| c.get_parsed::<u32>("MAX_FILES_SPECIES").ok())
            .unwrap_or(0)
    };

    let purge_threshold = config
        .and_then(|c| c.get_parsed::<u8>("DISK_PURGE_THRESHOLD").ok())
        .unwrap_or(95);

    let locked_file_names =
        state.with_db(|conn| birdnet_db::sqlite::locked_file_names(conn).unwrap_or_default());

    let config_obj = DiskManagerConfig {
        monitored_dir: monitored_dir.clone(),
        purge_threshold,
        full_disk_action: FullDiskAction::Purge,
        max_files_per_species,
        check_interval_secs: 60,
        exclude_paths: cli.disk_exclude.clone(),
        locked_file_names,
    };

    tracing::info!(
        dir = %monitored_dir.display(),
        max_files_per_species,
        purge_threshold,
        excluded_paths = cli.disk_exclude.len(),
        "disk manager configured"
    );

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let manager = DiskManager::new(config_obj);

    let handle = std::thread::spawn(move || {
        manager.run(&stop_rx);
    });

    std::mem::forget(stop_tx);
    Some(handle)
}

/// Generate an Avahi mDNS service file for local network discovery.
pub fn maybe_install_avahi_service(port: u16, site_name: &str) {
    let avahi_dir = std::path::Path::new("/etc/avahi/services");
    if !avahi_dir.exists() {
        return;
    }

    let service_file = avahi_dir.join("birdnet-behavior.service");
    if service_file.exists() {
        return;
    }

    let name = if site_name.is_empty() || site_name == "BirdNet-Behavior" {
        "BirdNet-Behavior".to_string()
    } else {
        site_name.to_string()
    };

    let xml = format!(
        r#"<?xml version="1.0" standalone='no'?>
<!DOCTYPE service-group SYSTEM "avahi-service.dtd">
<service-group>
  <name replace-wildcards="yes">{name} on %h</name>
  <service>
    <type>_http._tcp</type>
    <port>{port}</port>
    <txt-record>path=/</txt-record>
    <txt-record>software=BirdNet-Behavior</txt-record>
  </service>
</service-group>
"#
    );

    match std::fs::write(&service_file, xml) {
        Ok(()) => tracing::info!(
            path = %service_file.display(),
            "Avahi mDNS service file written — station discoverable as birdnet.local"
        ),
        Err(e) => tracing::debug!(
            error = %e,
            "Could not write Avahi service file (non-fatal, run as root to enable mDNS)"
        ),
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the small config-and-state helpers used by `main.rs`.
    //!
    //! These exist because the carryover from PR #49 noted that `src/helpers.rs`
    //! was at 0 % unit coverage. The functions here are all pure
    //! config-to-state mappings; the test surface is whether the right
    //! precedence rules fire (CLI flag wins; otherwise config value wins;
    //! otherwise built-in default) and whether the no-op return path is
    //! taken when neither source supplies the value.
    //!
    //! Tests that need a config build one through `Config::parse` rather
    //! than reading a file from disk. Tests that need an `AppState` use
    //! `AppState::from_connection` against an in-memory `SQLite`
    //! connection — the same pattern the rest of the workspace uses for
    //! state-shaped unit tests.
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn default_cli() -> Cli {
        // `Cli::parse_from` with just the binary name materialises every
        // `default_value` and leaves Options at None / Vecs empty —
        // exactly the "user passed no flags" baseline a config has to
        // override.
        Cli::parse_from(["birdnet-behavior"])
    }

    fn config_with(entries: &[(&str, &str)]) -> birdnet_core::config::Config {
        let content = entries
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        birdnet_core::config::Config::parse(&content).unwrap()
    }

    fn test_state() -> birdnet_web::state::AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        birdnet_web::state::AppState::from_connection(conn, PathBuf::from(":memory:"))
    }

    // ── db_path_from_config ────────────────────────────────────────────

    #[test]
    fn db_path_uses_config_value_when_present() {
        let cfg = config_with(&[("DB_PATH", "/srv/birds.db")]);
        assert_eq!(
            db_path_from_config(Some(&cfg)),
            PathBuf::from("/srv/birds.db")
        );
    }

    #[test]
    fn db_path_falls_back_to_home_when_config_absent() {
        // No DB_PATH in the config — the helper should construct
        // $HOME/BirdNet-Behavior/birds.db. We assert the suffix to
        // avoid coupling the test to the current HOME value.
        let cfg = config_with(&[("SOMETHING_ELSE", "irrelevant")]);
        let path = db_path_from_config(Some(&cfg));
        assert!(
            path.ends_with("BirdNet-Behavior/birds.db"),
            "expected default to end with BirdNet-Behavior/birds.db; got {}",
            path.display()
        );
    }

    #[test]
    fn db_path_falls_back_when_config_is_none() {
        // No config at all: same default-construction path.
        let path = db_path_from_config(None);
        assert!(path.ends_with("BirdNet-Behavior/birds.db"));
    }

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

    // ── maybe_install_avahi_service ────────────────────────────────────

    #[test]
    fn avahi_is_noop_when_target_dir_absent() {
        // /etc/avahi/services rarely exists inside CI containers and
        // this function returns early without trying to write — the
        // test pins that "did not panic, did not write" contract.
        maybe_install_avahi_service(8502, "TestStation");
        // No assertion needed beyond "did not panic"; the early-return
        // path is the entire surface for an unprivileged caller.
    }

    // ── start_disk_manager ─────────────────────────────────────────────

    #[test]
    fn disk_manager_returns_none_when_no_watch_dir() {
        // No --watch-dir flag and no RECS_DIR in config; the helper
        // returns None instead of starting an unconfigured manager.
        let cli = default_cli();
        let state = test_state();
        let handle = start_disk_manager(&cli, None, &state);
        assert!(handle.is_none());
    }

    #[test]
    fn disk_manager_starts_when_watch_dir_present() {
        // With a watch dir configured the helper spawns the manager
        // thread; we get back a JoinHandle. The thread itself runs
        // forever (the stop channel is leaked by design) so we don't
        // join it — but having a handle means the spawn happened.
        let tmp = tempfile::tempdir().unwrap();
        let mut cli = default_cli();
        cli.watch_dir = Some(tmp.path().to_path_buf());
        let state = test_state();
        let handle = start_disk_manager(&cli, None, &state);
        assert!(handle.is_some());
    }
}
