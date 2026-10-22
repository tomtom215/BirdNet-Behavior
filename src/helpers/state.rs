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

    // Whether this machine can carry the analytics engine at all (`G-33`).
    // DuckDB treats its memory limit as permission to use that much, and the
    // flat 256 MiB default is half of physical RAM on a 512 MB board — the
    // standing invitation to be OOM-killed mid-query at three in the morning
    // that this check exists to withdraw.
    let budget = birdnet_behavioral::memory::decide(
        std::env::var("BIRDNET_DUCKDB_MEMORY_LIMIT").ok().as_deref(),
        birdnet_behavioral::memory::detect_ceiling(),
    );
    if matches!(budget, birdnet_behavioral::memory::Budget::TooSmall { .. }) {
        // Refused, not crippled: a station that cannot run analytics is still a
        // station that records and classifies birds, and taking the whole
        // process down over an optional subsystem would be the worse failure.
        tracing::warn!("{}", budget.explain());
        return birdnet_web::state::AppState::new(server_config.db_path.clone())
            .map_err(|e| format!("database error: {e}").into());
    }
    tracing::info!(path = %analytics_path.display(), "enabling DuckDB analytics");
    tracing::info!("{}", budget.explain());
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

    let get = |key: &str| config.and_then(|c| c.get(key));
    let provider = get("IMAGE_PROVIDER").unwrap_or("wikipedia");

    match birdnet_integrations::species_images::ImageCache::from_settings(
        &cache_dir,
        provider,
        get("FLICKR_API_KEY"),
        get("FLICKR_FILTER_EMAIL"),
        birdnet_integrations::species_images::wikipedia::DEFAULT_THUMB_WIDTH,
    ) {
        Ok(cache) => {
            tracing::info!(
                path = %cache_dir.display(),
                cached = cache.cached_count(),
                provider,
                "species image cache enabled"
            );
            state.with_image_cache(cache)
        }
        Err(e) => {
            // A misconfigured Flickr key lands here. Falling back to Wikipedia
            // rather than to *nothing*: the operator asked for photographs, and
            // the setting they got wrong is which source, not whether. The
            // error is logged at warn so it is still visible and fixable.
            tracing::warn!(
                error = %e,
                provider,
                "configured image provider unavailable; falling back to Wikipedia"
            );
            match birdnet_integrations::species_images::ImageCache::with_wikipedia(&cache_dir) {
                Ok(cache) => state.with_image_cache(cache),
                Err(e) => {
                    tracing::warn!(error = %e, "species image cache not available (non-fatal)");
                    state
                }
            }
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

/// Choose the language species common names are shown in.
///
/// Always installs a [`NameLocalizer`](birdnet_core::i18n::NameLocalizer), even
/// for English, so that the Settings page can show what each other language
/// would cover of this station's classifier. The names stored with detections
/// are never changed; see [`birdnet_core::i18n`].
///
/// A value that names no language we have is reported and treated as English,
/// not as an error: the station keeps recording either way, and
/// `/admin/settings` refuses to save one.
///
/// `observed` is `(scientific name, common name)` for every species the
/// database already holds (see [`observed_names`]). It is passed in rather than
/// read here because this runs from `serve`, an async function, and a database
/// read on a runtime worker is what `database_work_stays_off_the_async_runtime`
/// exists to refuse.
pub fn init_i18n(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    observed: &[(String, String)],
) -> birdnet_web::state::AppState {
    use birdnet_core::i18n::{ENGLISH, LanguagePack, NameLocalizer, find_language};

    let configured = language_setting(cli, config);
    let configured = configured.trim();
    let code = if configured.is_empty() {
        ENGLISH
    } else if let Some(language) = find_language(configured) {
        language.code
    } else {
        tracing::warn!(
            lang = configured,
            "DATABASE_LANG is not a language species names are available in; \
             names are shown as the classifier gives them (see /api/v2/languages)"
        );
        ENGLISH
    };

    let labels_dir = cli
        .labels_dir
        .clone()
        .or_else(|| config?.get("LABELS_DIR").map(PathBuf::from));
    let pack = LanguagePack::bundled_with_overrides(code, labels_dir.as_deref(), |e| {
        tracing::warn!(
            lang = code,
            error = %e,
            "the labels file in LABELS_DIR could not be used; the bundled names are shown"
        );
    })
    .unwrap_or_else(|e| {
        tracing::warn!(lang = code, error = %e, "species names are unavailable; showing them as stored");
        LanguagePack::bundled(ENGLISH).expect("English is always bundled")
    });

    let labels = classifier_label_path(cli, config)
        .and_then(|path| birdnet_core::inference::labels::LabelSet::load(&path).ok());
    // The names actually stored — an import's, an earlier model's — as well as the
    // label file's, so a page that has only the stored name can still translate it.
    let names = NameLocalizer::new(pack, labels.as_ref()).with_observed_names(
        observed
            .iter()
            .map(|(sci, com)| (sci.as_str(), com.as_str())),
    );
    match names.coverage() {
        Some(c) => tracing::info!(
            lang = names.language().code,
            named = c.named,
            of = c.of,
            "species common names shown in {}; the rest keep the classifier's name",
            names.language().english_name
        ),
        None if names.is_identity() => {
            tracing::debug!("species common names shown as the classifier gives them");
        }
        None => tracing::info!(
            lang = names.language().code,
            "species common names shown in {} (no classifier label file to count coverage against)",
            names.language().english_name
        ),
    }
    state.with_names(names)
}

/// `(scientific name, common name)` for every species the database holds, for
/// [`init_i18n`]. Blocks on the database: call it through
/// `AppState::run_blocking` from async code.
///
/// A database that cannot be read gives an empty list and a warning: the names
/// are then those of the classifier's label file, which is what a station with
/// no history has, and the station still starts.
pub fn observed_names(state: &birdnet_web::state::AppState) -> Vec<(String, String)> {
    match state.with_db(|conn| birdnet_db::sqlite::top_species(conn, 100_000)) {
        Ok(species) => species
            .into_iter()
            .map(|s| (s.sci_name, s.com_name))
            .collect(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not read the species already stored; only the classifier's \
                 labels will be translated"
            );
            Vec::new()
        }
    }
}

/// The configured common-name language, before it is checked against the
/// languages there are: `--lang` when it was given, else `DATABASE_LANG`
/// (which `/admin/settings` writes through the overlay), else `"en"`.
///
/// `--lang en` given explicitly wins over a `DATABASE_LANG=de` in the file, as
/// every other flag does; the old test for "is the flag still its default"
/// could not tell that from not passing it.
pub fn language_setting(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> String {
    super::resolve::setting_str(cli, "lang", &cli.lang, config, "DATABASE_LANG")
}

/// Where the classifier's label file is, resolved as the daemon resolves it
/// (`--labels`, else `LABELS_PATH`).
fn classifier_label_path(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<PathBuf> {
    cli.labels
        .clone()
        .or_else(|| config?.get("LABELS_PATH").map(PathBuf::from))
}

/// Initialize custom site name.
///
/// The name shown on the dashboard, the audio-player and share pages is, in
/// order: `--site-name`, `SITENAME` (the *Site Name* setting), then
/// `STATION_NAME` (the *Station Name* setting, the one the setup wizard asks
/// for). The last step is why a name typed in the wizard shows up: the two
/// settings are different keys with different readers (`STATION_NAME` also
/// names the Home Assistant device and the eBird export's location), and
/// before this a station with only the wizard's name set showed
/// "BirdNet-Behavior" everywhere the site name is printed. An empty value is
/// the same as none.
pub fn init_site_name(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let named = |key: &str| {
        config?
            .get(key)
            .filter(|n| !n.trim().is_empty())
            .map(String::from)
    };
    let name = cli
        .site_name
        .clone()
        .or_else(|| named("SITENAME"))
        .or_else(|| named("STATION_NAME"));

    match name {
        Some(n) if !n.is_empty() => {
            tracing::info!(site_name = %n, "custom site name configured");
            state.with_site_name(n)
        }
        _ => state,
    }
}

/// The species-information site the station links to.
///
/// `/admin/settings` lands in `INFO_SITE` via the overlay; an explicit
/// `--info-site` still wins, as for every other bridged setting.
pub fn info_site(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> String {
    super::resolve::setting_str(cli, "info_site", &cli.info_site, config, "INFO_SITE")
}

/// Load the eBird species codes for the species page's "View on eBird" link
/// (NP-1), from the metadata model's label file when one is configured.
///
/// The path is resolved exactly as the daemon resolves it
/// (`--metadata-labels`, else `METADATA_LABELS_PATH`). A file that does not
/// parse is reported and skipped here: the daemon refuses to start on it with
/// its own message, and the web layer's only stake is the link.
pub fn init_species_codes(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let path = cli
        .metadata_labels
        .clone()
        .or_else(|| config?.get("METADATA_LABELS_PATH").map(PathBuf::from));
    let Some(path) = path else {
        tracing::info!(
            "no metadata label file configured: species pages will not link to eBird (the geomodel's label file supplies the species codes)"
        );
        return state;
    };
    match birdnet_core::inference::labels::LabelSet::load(&path) {
        Ok(labels) => {
            let codes: Vec<(String, String)> = labels
                .species_codes()
                .map(|(sci, code)| (sci.to_string(), code.to_string()))
                .collect();
            tracing::info!(
                path = %path.display(),
                species_with_code = codes.len(),
                "eBird species codes loaded for the species page link"
            );
            state.with_species_codes(codes)
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "metadata label file could not be read: species pages will not link to eBird"
            );
            state
        }
    }
}

/// Load the taxonomy the species pages browse by (`G-15`), from the
/// *classifier's* label file.
///
/// A different file from [`init_species_codes`]'s, and for a different reason.
/// The geomodel's label file carries the eBird species code; the classifier's
/// carries the `class` and `order` columns, and it is the classifier's names
/// the stored detections are keyed on. Resolved exactly as the daemon resolves
/// it (`--labels`, else `LABELS_PATH`).
///
/// A file that will not parse is reported and skipped: the daemon refuses to
/// start on it with its own message, and the web layer's only stake is a
/// browsing control.
pub fn init_taxonomy(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_web::state::AppState {
    let Some(path) = classifier_label_path(cli, config) else {
        tracing::info!(
            "no classifier label file configured: the species pages will not offer browsing by order or genus"
        );
        return state;
    };
    match birdnet_core::inference::labels::LabelSet::load(&path) {
        Ok(labels) => {
            let taxa: Vec<(String, birdnet_web::state::Taxon)> = labels
                .iter()
                .map(|l| {
                    (
                        l.scientific_name.clone(),
                        birdnet_web::state::Taxon {
                            class: l.class.clone(),
                            order: l.order.clone(),
                            genus: l.genus().map(ToOwned::to_owned),
                        },
                    )
                })
                .filter(|(_, t)| !t.is_empty())
                .collect();
            tracing::info!(
                path = %path.display(),
                species_with_taxonomy = taxa.len(),
                "taxonomy loaded for the species pages"
            );
            state.with_taxonomy(taxa)
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "classifier label file could not be read: the species pages will not offer browsing by order or genus"
            );
            state
        }
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
        embedded_abi = AnalyticsDb::embedded_extension_abi().unwrap_or("<none embedded>"),
        embedded_platform = AnalyticsDb::embedded_extension_platform().unwrap_or("<none embedded>"),
        "behavioral extension: build-time embedding"
    );

    if let Some(mismatch) = adb.embedded_extension_mismatch() {
        return Err(format!(
            "the embedded behavioral extension ({} ABI, version target {}, platform {}) cannot \
             load into this binary's DuckDB {} ({}): the {} disagrees. Rebuild with the \
             extension published for it (community-extensions.duckdb.org/{}/{}/).",
            mismatch.embedded_abi.unwrap_or("unknown"),
            mismatch.embedded_for,
            mismatch.embedded_platform.unwrap_or("unknown"),
            mismatch.engine,
            mismatch.engine_platform.as_deref().unwrap_or("unknown"),
            mismatch.kind,
            mismatch.engine,
            mismatch.engine_platform.as_deref().unwrap_or("<platform>"),
        )
        .into());
    }

    adb.load_extension()
        .map_err(|e| format!("behavioral extension did not load: {e}"))?;

    // Loading proves a library is present, not that it answers the way this
    // build's queries assume. Run every function on fixed events and compare.
    adb.verify_behavioral_functions()
        .map_err(|e| format!("behavioral extension loaded but failed its self-test: {e}"))?;

    tracing::info!(
        duckdb = %engine,
        extension = adb.extension_version().as_deref().unwrap_or("unknown"),
        path = %path.display(),
        "behavioral extension loaded and passed its functional self-test"
    );

    // ICU is the other half of an offline-capable station, and the half that
    // actually failed in the field: `CURRENT_DATE` lives in it, every dashboard
    // opens with a date window, and DuckDB's own answer is to autoinstall into
    // `$HOME/.duckdb` — which the shipped unit mounts read-only. Verifying the
    // query shape rather than the extension's `loaded` flag is deliberate: it
    // is the thing the dashboards actually need, and it cannot pass for the
    // wrong reason.
    tracing::info!(
        embedded_icu_for_duckdb =
            AnalyticsDb::embedded_icu_duckdb_version().unwrap_or("<none embedded>"),
        embedded_icu_platform = AnalyticsDb::embedded_icu_platform().unwrap_or("<none embedded>"),
        "ICU extension: build-time embedding"
    );
    adb.verify_date_window().map_err(|e| {
        format!(
            "the date-window query every dashboard opens with did not bind: {e}\n\
             CURRENT_DATE lives in DuckDB's ICU extension. With no ICU embedded at build time, \
             DuckDB tries to autoinstall it into $HOME/.duckdb — which fails under the shipped \
             unit's ProtectHome=read-only, and leaves every analytics page empty."
        )
    })?;

    tracing::info!("date-window query bound; ICU is available");
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
    use super::{init_i18n, init_image_cache, init_site_name, init_species_codes, observed_names};
    use crate::helpers::test_support::{config_with, default_cli, test_state};

    /// `/admin/settings` writes `info_site`, which the overlay turns into
    /// `INFO_SITE`. The station read only `--info-site`, so the setting — and
    /// the same line in `birdnet.conf` — changed nothing it linked to.
    #[test]
    fn the_info_site_setting_is_the_one_the_station_links_to() {
        let config = config_with(&[("INFO_SITE", "allaboutbirds")]);
        assert_eq!(
            super::info_site(&default_cli(), Some(&config)),
            "allaboutbirds"
        );
        // Counterparts: an explicit flag still wins, and nothing set is eBird.
        let cli = crate::helpers::test_support::cli_with_explicit(&["info_site"]);
        assert_eq!(super::info_site(&cli, Some(&config)), "ebird");
        assert_eq!(super::info_site(&default_cli(), None), "ebird");
    }

    // ── init_species_codes (NP-1) ─────────────────────────────────────

    #[test]
    fn species_codes_come_from_the_metadata_label_file() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("geomodel_labels.txt");
        std::fs::write(
            &labels,
            "1032549\tPetaurista albiventer\tWhite-bellied Giant Flying Squirrel\nzothaw\tButeo albonotatus\tZone-tailed Hawk\n",
        )
        .unwrap();

        // From the CLI flag.
        let mut cli = default_cli();
        cli.metadata_labels = Some(labels.clone());
        let state = init_species_codes(test_state(), &cli, None);
        assert_eq!(
            state.ebird_species_code("Buteo albonotatus"),
            Some("zothaw")
        );
        assert_eq!(
            state.ebird_species_code("buteo ALBONOTATUS"),
            Some("zothaw"),
            "the lookup is case-insensitive, like the label set's own"
        );
        assert_eq!(state.ebird_species_code("Parus major"), None);

        // From the config key, as the daemon resolves it.
        let cfg = config_with(&[("METADATA_LABELS_PATH", labels.to_str().unwrap())]);
        let state = init_species_codes(test_state(), &default_cli(), Some(&cfg));
        assert_eq!(
            state.ebird_species_code("Buteo albonotatus"),
            Some("zothaw")
        );

        // Nothing configured, or a file that is not there: no codes, no panic.
        let state = init_species_codes(test_state(), &default_cli(), None);
        assert_eq!(state.ebird_species_code("Buteo albonotatus"), None);
        let mut cli = default_cli();
        cli.metadata_labels = Some(dir.path().join("missing.txt"));
        let state = init_species_codes(test_state(), &cli, None);
        assert_eq!(state.ebird_species_code("Buteo albonotatus"), None);
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

    /// The setup wizard asks for the *Station Name*, and the dashboard prints
    /// the site name. Seen failing with the `STATION_NAME` step removed: the
    /// dashboard then said "BirdNet-Behavior" for a station named in the wizard.
    #[test]
    fn site_name_falls_back_to_the_station_name_the_wizard_asked_for() {
        let cli = default_cli();
        let cfg = config_with(&[("STATION_NAME", "Garden Station")]);
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "Garden Station");
        // An empty Site Name does not hide it.
        let cfg = config_with(&[("SITENAME", " "), ("STATION_NAME", "Garden Station")]);
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "Garden Station");
        // Counterpart: a Site Name that is set wins over it, and so does the flag.
        let cfg = config_with(&[
            ("SITENAME", "Front Yard"),
            ("STATION_NAME", "Garden Station"),
        ]);
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "Front Yard");
        let mut cli = default_cli();
        cli.site_name = Some("CliStation".to_owned());
        let state = init_site_name(test_state(), &cli, Some(&cfg));
        assert_eq!(state.site_name(), "CliStation");
    }

    #[test]
    fn site_name_none_when_nothing_configured() {
        let cli = default_cli();
        let state = init_site_name(test_state(), &cli, None);
        // Default fallback exposed via `site_name()`.
        assert_eq!(state.site_name(), "BirdNet-Behavior");
    }

    // ── init_i18n ──────────────────────────────────────────────────────

    /// Two species and the names a German station would show for them, in the
    /// classifier's V2.4 text format.
    fn label_file(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("labels.txt");
        std::fs::write(
            &path,
            "Turdus merula_Common Blackbird\nAcanthisitta chloris_Rifleman\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn english_shows_every_name_as_stored() {
        let state = init_i18n(test_state(), &default_cli(), None, &[]);
        assert!(state.names().is_identity());
        assert_eq!(state.names().language().code, "en");
        assert_eq!(
            state.names().name("Turdus merula", "Common Blackbird"),
            "Common Blackbird"
        );
    }

    #[test]
    fn a_language_on_the_command_line_translates_with_no_label_file_and_no_labels_dir() {
        // The setting used to need a `--labels-dir` nobody had, and without one
        // logged a warning and did nothing at all.
        let mut cli = default_cli();
        cli.lang = "de".to_owned();
        let state = init_i18n(test_state(), &cli, None, &[]);
        assert_eq!(state.names().language().code, "de");
        assert_eq!(
            state.names().name("Turdus merula", "Common Blackbird"),
            "Amsel"
        );
    }

    #[test]
    fn the_language_setting_in_the_config_is_the_one_used_when_the_flag_is_default() {
        let cfg = config_with(&[("DATABASE_LANG", "de")]);
        let state = init_i18n(test_state(), &default_cli(), Some(&cfg), &[]);
        assert_eq!(
            state.names().name("Turdus merula", "Common Blackbird"),
            "Amsel"
        );
        // And an explicit flag wins over it, `--lang en` included.
        let mut cli = crate::helpers::test_support::cli_with_explicit(&["lang"]);
        cli.lang = "fr".to_owned();
        let state = init_i18n(test_state(), &cli, Some(&cfg), &[]);
        assert_eq!(
            state.names().name("Turdus merula", "Common Blackbird"),
            "Merle noir"
        );
        cli.lang = "en".to_owned();
        let state = init_i18n(test_state(), &cli, Some(&cfg), &[]);
        assert!(
            state.names().is_identity(),
            "an explicit --lang en must beat DATABASE_LANG=de"
        );
    }

    #[test]
    fn a_regional_or_oddly_cased_code_still_picks_the_language() {
        for raw in ["DE", " de ", "de_AT", "de-DE"] {
            let cfg = config_with(&[("DATABASE_LANG", raw)]);
            let state = init_i18n(test_state(), &default_cli(), Some(&cfg), &[]);
            assert_eq!(state.names().language().code, "de", "{raw:?}");
        }
        let cfg = config_with(&[("DATABASE_LANG", "zh-cn")]);
        let state = init_i18n(test_state(), &default_cli(), Some(&cfg), &[]);
        assert_eq!(state.names().language().code, "zh_CN");
    }

    #[test]
    fn a_value_that_is_not_a_language_is_english_not_a_failure() {
        for raw in ["xx", "eng", "zh", "", "   "] {
            let cfg = config_with(&[("DATABASE_LANG", raw)]);
            let state = init_i18n(test_state(), &default_cli(), Some(&cfg), &[]);
            assert!(state.names().is_identity(), "{raw:?}");
            assert_eq!(state.names().language().code, "en", "{raw:?}");
        }
    }

    #[test]
    fn the_classifier_label_file_lets_every_language_be_counted_even_in_english() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cli = default_cli();
        cli.labels = Some(label_file(tmp.path()));
        let state = init_i18n(test_state(), &cli, None, &[]);
        // The station is in English, yet can say what German or Korean would cover.
        assert!(state.names().is_identity());
        let de = state.names().coverage_of("de").expect("counted");
        let ko = state.names().coverage_of("ko").expect("counted");
        assert_eq!((de.named, de.of), (2, 2));
        assert_eq!((ko.named, ko.of), (1, 2), "Korean has no Rifleman");
    }

    #[test]
    fn names_already_stored_in_the_database_are_translated_too() {
        // An import from another station: rows named by *its* labels, which the
        // station's own label file does not contain.
        let state = test_state();
        state.with_db(|conn| {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs)
                 VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
                         0.9, 0.7, 18, 1.25, 0.0, 'a.wav', 0)",
                [],
            )
            .expect("insert");
        });
        let mut cli = default_cli();
        cli.lang = "de".to_owned();
        let observed = observed_names(&state);
        assert_eq!(observed.len(), 1, "the row just inserted");
        let state = init_i18n(state, &cli, None, &observed);
        assert_eq!(state.names().name_of_stored("Eurasian Blackbird"), "Amsel");
        // Counterpart: a name nothing has stored is not guessed.
        assert_eq!(state.names().name_of_stored("Great Tit"), "Great Tit");
    }

    #[test]
    fn a_labels_dir_file_overrides_the_bundled_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("de_labels.txt"),
            "Turdus merula_Schwarzdrossel\n",
        )
        .unwrap();
        let mut cli = default_cli();
        cli.lang = "de".to_owned();
        cli.labels_dir = Some(tmp.path().to_owned());
        let state = init_i18n(test_state(), &cli, None, &[]);
        assert_eq!(
            state.names().name("Turdus merula", "Common Blackbird"),
            "Schwarzdrossel"
        );
        assert_eq!(
            state.names().name("Parus major", "Great Tit"),
            "Kohlmeise",
            "the rest is still the bundle"
        );
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
