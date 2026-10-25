//! Configuration checks: file presence/parse, value validation, listen address.

use std::path::PathBuf;

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self as cfg_validate, Severity as ConfigSeverity};
use birdnet_core::detection::corroboration::REFERENCE_SPAN;
use birdnet_core::detection::pipeline::PipelineConfig;

use super::Check;
use crate::cli::Cli;

/// Warn when the station has no coordinates, because the effect is invisible
/// and severe.
///
/// `SpeciesFilter::filter_species` takes `Option<(lat, lon)>`. With `None` the
/// metadata model cannot run at all, so **occurrence filtering is skipped and
/// every one of the ~11 000 species stays a candidate**. The station keeps
/// working, keeps detecting, and reports species that have never occurred
/// within a thousand miles — which reads as a bad model rather than as a
/// missing setting.
///
/// Nothing said so. The existing config validation checks that a latitude is
/// *within range*, and warns when one of the pair is set without the other, but
/// is silent when both are absent. The onboarding wizard asks for location on
/// its second step and can auto-detect it, so a station onboarded through the
/// dashboard is fine; one installed non-interactively, or by an operator who
/// pressed Enter past the installer's prompt, is not — and never hears about it
/// again.
///
/// Resolution goes through [`crate::daemon::resolve_station_coords`],
/// the same function the detection daemon uses, rather than a third copy of the
/// precedence rule. That function's own doc records what a second copy cost
/// last time: the daemon read `cli.latitude` alone, so a normally-configured
/// station handed it `None` and the species filter was silently inert.
///
/// The settings table is consulted as a fallback because `--doctor` runs from
/// `ExecStartPre`, before the settings overlay has layered `/admin/settings`
/// onto the config — so a station configured entirely through the dashboard has
/// its coordinates only in SQLite at the moment this runs. Reading the config
/// alone would nag exactly the operators who did it the easy way.
pub(super) fn check_station_location(cli: &Cli, config: Option<&Config>) -> Check {
    let (lat, lon) = crate::daemon::resolve_station_coords(cli, config);
    if let (Some(lat), Some(lon)) = (lat, lon) {
        return Check::pass("Station location", format!("{lat:.4}, {lon:.4}"));
    }
    if let Some((lat, lon)) = location_from_settings(config) {
        return Check::pass(
            "Station location",
            format!("{lat:.4}, {lon:.4} (from the dashboard settings)"),
        );
    }
    Check::warn(
        "Station location",
        "no latitude/longitude set — every species in the model stays a candidate",
        "the occurrence filter cannot run without coordinates, so detections will \
         include birds that do not occur near you. Set it in the setup \
         wizard (its Location step has a button that looks it up) or under \
         Settings → Capture → Location & Recording Schedule, or put LATITUDE / \
         LONGITUDE in the config and restart",
    )
}

/// Warn when `BirdWeather` uploads are configured but cannot happen because
/// the station has no position to publish them at.
///
/// A station with a token and no usable `LATITUDE` / `LONGITUDE` used to build
/// its client at 0°, 0° and post every detection there, onto a public map in
/// the Gulf of Guinea. The client is now not built (see
/// [`crate::integrations::create_birdweather_client`], which warns once at
/// startup), so the station is quiet on `BirdWeather` instead of wrong — and
/// quiet is what a healthy station that is not configured for it also looks
/// like. This is where an operator who set a token finds out why nothing
/// arrives.
///
/// Said only for a station that has a token (from the flag, the config, or —
/// since `--doctor` runs before the settings overlay — the dashboard's
/// settings table); one that never set it up has nothing to be told. The
/// position is judged by [`crate::integrations::birdweather_location`], the
/// rule the client builder uses, with the same settings-table fallback
/// [`check_station_location`] has for a position typed on the dashboard.
pub(super) fn check_birdweather_location(cli: &Cli, config: Option<&Config>) -> Option<Check> {
    const NAME: &str = "BirdWeather location";

    let has_token = |t: &str| !t.trim().is_empty();
    let configured = cli.birdweather_token.as_deref().is_some_and(has_token)
        || config
            .and_then(|c| c.get("BIRDWEATHER_TOKEN"))
            .is_some_and(has_token)
        || setting_from_settings(config, "birdweather_token")
            .as_deref()
            .is_some_and(has_token);
    if !configured {
        return None;
    }

    let from_settings = || {
        location_from_settings(config).and_then(|(lat, lon)| {
            birdnet_integrations::birdweather::check_location(Some(lat), Some(lon)).ok()
        })
    };
    Some(
        match crate::integrations::birdweather_location(cli, config) {
            Ok((lat, lon)) => Check::pass(NAME, format!("uploads at {lat:.4}, {lon:.4}")),
            Err(why) => match from_settings() {
                Some((lat, lon)) => Check::pass(
                    NAME,
                    format!("uploads at {lat:.4}, {lon:.4} (from the dashboard settings)"),
                ),
                None => Check::warn(
                    NAME,
                    format!(
                        "a station token is set but uploads are off: the station's position is not \
                     usable — {why}"
                    ),
                    crate::integrations::LOCATION_FIX,
                ),
            },
        },
    )
}

/// One value from the dashboard's settings table, read directly from SQLite.
///
/// `None` on any failure, like [`location_from_settings`]: a diagnostic must
/// not turn a database problem into a finding of its own.
fn setting_from_settings(config: Option<&Config>, key: &str) -> Option<String> {
    let db_path = crate::helpers::db_path_from_config(config);
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    birdnet_db::settings::get(&conn, key).ok()
}

/// Report whether species occurrence filtering will actually run.
///
/// Two things are needed and the diagnostic used to check only one. The
/// metadata ("geo") model takes `(latitude, longitude, week)`, so coordinates
/// are necessary — but the model file itself is the other half, and no install
/// ships one: `install.sh` fetches the classifier and its labels, and
/// `BIRDNET_METADATA_MODEL` sits in `.env.example` under "bring your own
/// model". A station that never sets it runs with the filter inert, keeps
/// every one of the classifier's species as a candidate regardless of where it
/// is, and reports birds from other continents — which reads as a bad model
/// rather than as a missing file.
///
/// `check_station_location` asserted the opposite ("species filtering by
/// occurrence is active") from coordinates alone, so the one check that could
/// have caught this said it was fine. It now reports only the coordinates; the
/// filter's state is this check's to report.
///
/// It loads the model the way the daemon does (ON-9) — `exists()` used to be
/// the whole check, and a truncated download or a model for another classifier
/// passed it — and then reports how the two vocabularies aligned. That last
/// part is the point: the two label files disagree about the genus of dozens
/// of species, every one of them dropped in silence before
/// `inference::vocabulary` existed, and the counts here are where an operator
/// can see how many and which rule saved them.
pub(super) fn check_occurrence_filter(cli: &Cli, config: Option<&Config>) -> Check {
    const NAME: &str = "Species occurrence filter";

    let model = cli
        .metadata_model
        .clone()
        .or_else(|| config.and_then(|c| c.get("METADATA_MODEL_PATH").map(PathBuf::from)));

    let Some(model) = model else {
        return Check::warn(
            NAME,
            "off — no metadata model configured, so every species the classifier knows stays a candidate wherever the station is",
            "set METADATA_MODEL_PATH in the config (or BIRDNET_METADATA_MODEL / --metadata-model) to a BirdNET metadata model, and METADATA_LABELS_PATH to the label file it shipped with. See the Species filtering section of the manual for which model matches the installed classifier",
        );
    };

    if !model.exists() {
        return Check::fail(
            NAME,
            format!("{} does not exist", model.display()),
            "occurrence filtering was asked for but the model is not on disk: fix METADATA_MODEL_PATH or download the model again",
        );
    }

    let labels = cli
        .metadata_labels
        .clone()
        .or_else(|| config.and_then(|c| c.get("METADATA_LABELS_PATH").map(PathBuf::from)));

    if let Some(labels) = labels.as_ref()
        && !labels.exists()
    {
        return Check::fail(
            NAME,
            format!("metadata label file {} does not exist", labels.display()),
            "the station refuses to start on a label file it cannot read: fix METADATA_LABELS_PATH or remove it if the model is indexed like the classifier",
        );
    }

    let (lat, lon) = crate::daemon::resolve_station_coords(cli, config);
    let has_coords = (lat.is_some() && lon.is_some()) || location_from_settings(config).is_some();

    if !has_coords {
        return Check::warn(
            NAME,
            format!(
                "{} is present but the station has no coordinates, so the model cannot run and filtering is off",
                model.display()
            ),
            "the model takes latitude and longitude as two of its three inputs: set them in the setup wizard (its Location step has a button that looks it up) or under Settings → Capture → Location & Recording Schedule, or put LATITUDE / LONGITUDE in the config and restart",
        );
    }

    // ON-9: `exists()` was the whole check. Load it the way the daemon does,
    // which also compares its declared width with the vocabulary it will be
    // read against; a truncated download or a model for another classifier
    // fails here and not on the first inference of a station nobody watches.
    let classifier_labels = cli
        .labels
        .clone()
        .or_else(|| config.and_then(|c| c.get("LABELS_PATH").map(PathBuf::from)))
        .filter(|p| p.exists())
        .and_then(|p| birdnet_core::inference::labels::LabelSet::load(&p).ok());
    let meta_labels = labels
        .as_ref()
        .and_then(|p| birdnet_core::inference::labels::LabelSet::load(p).ok());
    // The same alias file the daemon reads, so the counts below are the ones
    // the running station will have.
    let aliases = cli
        .species_aliases
        .clone()
        .or_else(|| config.and_then(|c| c.get("SPECIES_ALIASES_PATH").map(PathBuf::from)))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map_or_else(std::collections::HashMap::new, |text| {
            birdnet_core::inference::vocabulary::parse_alias_file(&text).0
        });

    let mut alignment = None;
    if let Some(classifier) = classifier_labels.as_ref() {
        match birdnet_core::inference::species_filter::SpeciesFilter::load_with_vocabulary(
            &model,
            meta_labels,
            classifier,
            &aliases,
            birdnet_core::inference::species_filter::SpeciesFilterConfig::default(),
        ) {
            Ok(filter) => alignment = filter.alignment_stats().cloned(),
            Err(e) => {
                return Check::fail(
                    NAME,
                    format!(
                        "{} is present but cannot be used as a metadata model ({e}); the daemon \
                         would fall back to admitting every species",
                        model.display()
                    ),
                    "download the metadata model that matches the installed classifier again, and \
                     the label file it shipped with",
                );
            }
        }
    }

    let how = labels.map_or_else(
        || " (indexed against the classifier's labels)".to_owned(),
        |l| format!(" (matched by name through {})", l.display()),
    );
    let loaded = if classifier_labels.is_some() {
        "; loads, and its width matches the vocabulary"
    } else {
        "; not loaded here because the classifier's labels file could not be read"
    };
    Check::pass(
        NAME,
        format!(
            "active — {}{how}{loaded}{}",
            model.display(),
            alignment
                .as_ref()
                .map_or_else(String::new, describe_alignment)
        ),
    )
}

/// Say which language species names will be shown in, and how much of the
/// installed classifier it covers.
///
/// The setting used to do nothing at all and report nothing, so a station set
/// to Korean showed English with no word of why. Now it works, and for a language
/// the bundled tables only partly cover it still shows English for most birds —
/// which reads as a bug unless something states the extent. This is that
/// statement, from the same count `/api/v2/languages` serves.
///
/// Resolution follows what the daemon will end up using. `--doctor` runs from
/// `ExecStartPre`, before the overlay has laid `/admin/settings` over the config,
/// so a station configured on the dashboard has its language only in SQLite at
/// this moment; the settings table is read first because once seeded it wins
/// over the file.
pub(super) fn check_species_names(cli: &Cli, config: Option<&Config>) -> Check {
    let configured = if cli.explicit.has("lang") {
        cli.lang.clone()
    } else {
        language_from_settings(config)
            .unwrap_or_else(|| crate::helpers::language_setting(cli, config))
    };
    let labels = cli
        .labels
        .clone()
        .or_else(|| config.and_then(|c| c.get("LABELS_PATH")).map(PathBuf::from))
        .and_then(|path| birdnet_core::inference::labels::LabelSet::load(&path).ok());
    species_names(&configured, labels.as_ref())
}

/// The verdict for a configured language and the classifier's label set.
fn species_names(
    configured: &str,
    labels: Option<&birdnet_core::inference::labels::LabelSet>,
) -> Check {
    use birdnet_core::i18n::{ENGLISH, LANGUAGES, LanguagePack, NameLocalizer, find_language};

    const NAME: &str = "Species names";
    let configured = configured.trim();
    if configured.is_empty() {
        return Check::pass(NAME, "shown as the classifier gives them (English)");
    }
    let Some(language) = find_language(configured) else {
        let codes: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
        return Check::warn(
            NAME,
            format!(
                "{configured:?} is not a language species names are available in, \
                 so they are shown as the classifier gives them"
            ),
            format!(
                "choose one on Settings, or set DATABASE_LANG to one of: {}",
                codes.join(", ")
            ),
        );
    };
    if language.code == ENGLISH {
        return Check::pass(NAME, "shown as the classifier gives them (English)");
    }
    let Ok(pack) = LanguagePack::bundled(language.code) else {
        return Check::fail(
            NAME,
            format!(
                "the built-in {} names could not be read",
                language.english_name
            ),
            "this is a defect in the build; please report it",
        );
    };
    let names = NameLocalizer::new(pack, labels);
    names.coverage().map_or_else(
        || {
            Check::pass(
                NAME,
                format!(
                    "{} ({}); no classifier label file is configured, so how many species \
                     have a name in it cannot be counted",
                    language.english_name, language.native_name
                ),
            )
        },
        |c| {
            Check::pass(
                NAME,
                format!(
                    "{} ({}): {} of the classifier's {} species have a name in it; \
                     the rest keep the classifier's own name",
                    language.english_name, language.native_name, c.named, c.of
                ),
            )
        },
    )
}

/// The language `/admin/settings` has stored, if the database can be read and
/// holds one. `None` on any failure: this check must not turn a database
/// problem into a language finding (`check_database` owns that).
fn language_from_settings(config: Option<&Config>) -> Option<String> {
    let db_path = crate::helpers::db_path_from_config(config);
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    birdnet_db::settings::get(&conn, "database_lang")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// The one-line summary of how the two vocabularies lined up.
///
/// Both directions are named because they fail differently and an operator
/// needs to tell them apart. A metadata species with no classifier counterpart
/// is nearly always right — the geomodel knows 12 012 species and the
/// classifier emits 11 560. A *classifier* species no metadata row resolves to
/// is one this station cannot report at all while the filter runs, and there
/// is nothing else that would ever say so.
fn describe_alignment(stats: &birdnet_core::inference::vocabulary::AlignmentStats) -> String {
    let recovered = if stats.by_common_name == 0 && stats.by_alias == 0 {
        String::new()
    } else {
        format!(
            " ({} by common name + epithet, {} by an operator alias, e.g. {})",
            stats.by_common_name,
            stats.by_alias,
            stats
                .recovered_sample
                .first()
                .map_or("—", std::string::String::as_str)
        )
    };
    format!(
        "; {} of the metadata model's {} species map onto the classifier{recovered}, {} do not, \
         and {} of the classifier's {} species are reached by no metadata row and so cannot pass \
         while it runs",
        stats.matched(),
        stats.metadata_species,
        stats.unmatched,
        stats.classifier_unreachable,
        stats.classifier_species,
    )
}

/// Is the repeat-confirmation filter set to something that can actually reject?
///
/// The one defect worth a check of its own: `CONFIRMATION_LEVEL=lenient` with
/// the default overlap of zero is accepted, logged as enabled, and rejects
/// nothing — a six-second neighbourhood holds two 3-second windows, and 20% of
/// two rounds up to one, which every detection already satisfies. The operator
/// sees no change and has no way to tell whether that is the filter working or
/// the filter idle.
///
/// The daemon warns about this at startup too, but the daemon is not running
/// when somebody is editing the config file, which is when this matters.
pub(super) fn check_confirmation_filter(cli: &Cli, config: Option<&Config>) -> Check {
    const NAME: &str = "Repeat-confirmation filter";

    let (level, warning) = crate::daemon::resolve_confirmation_level(
        &cli.confirmation_level,
        "off",
        config.and_then(|c| c.get("CONFIRMATION_LEVEL")),
    );

    if let Some(warning) = warning {
        return Check::fail(
            NAME,
            warning,
            "CONFIRMATION_LEVEL (or --confirmation-level) takes one of: off, lenient, moderate, balanced, strict",
        );
    }

    if !level.enabled() {
        return Check::pass(
            NAME,
            "off — every detection that passes the other filters is recorded",
        );
    }

    let overlap = crate::daemon::resolve_f32_with_default(
        cli.overlap,
        0.0,
        config.and_then(|c| c.get_parsed::<f32>("OVERLAP").ok()),
    );
    let chunk_secs = PipelineConfig::default().chunk_duration_secs;

    if level.is_effective_at(overlap, chunk_secs) {
        let required = level.required_confirmations_at(overlap, chunk_secs);
        return Check::pass(
            NAME,
            format!(
                "{} — a species must be heard in {required} of the windows within \
                 {REFERENCE_SPAN:.0}s to be recorded (overlap {overlap}s)",
                level.as_str()
            ),
        );
    }

    let fix = level.minimum_overlap(chunk_secs).map_or_else(
        || {
            "no overlap makes this level bite at the current window length; raise \
             CONFIRMATION_LEVEL instead"
                .to_owned()
        },
        |need| {
            format!(
                "set OVERLAP (or --overlap) to at least {need}s, or raise \
                 CONFIRMATION_LEVEL — `balanced` and `strict` reject at any overlap"
            )
        },
    );
    Check::warn(
        NAME,
        format!(
            "{} is set but rejects nothing at overlap {overlap}s — a single window is \
             already the whole neighbourhood it is asked to agree with",
            level.as_str()
        ),
        fix,
    )
}

/// Coordinates as stored by the dashboard, read directly from SQLite.
///
/// Returns `None` on any failure — absent database, missing table, unparseable
/// value. A diagnostic must not turn a database problem into a location
/// finding: `check_database` owns the database's health.
fn location_from_settings(config: Option<&Config>) -> Option<(f64, f64)> {
    let db_path = crate::helpers::db_path_from_config(config);
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let lat = birdnet_db::settings::get(&conn, "latitude")
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    let lon = birdnet_db::settings::get(&conn, "longitude")
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    Some((lat, lon))
}

/// How the station's database shows it was set up from the browser, or `None`
/// when it does not (a new database, an unreadable one, none yet).
///
/// Two signs, either of which is a deliberate act in the browser and neither of
/// which the settings table's own seeding can fake: the setup wizard was
/// finished (`onboarding_complete`), or the Capture tab holds a source.
fn set_up_in_the_browser(config: Option<&Config>) -> Option<&'static str> {
    set_up_in_the_browser_at(&crate::helpers::db_path_from_config(config))
}

/// [`set_up_in_the_browser`] for a database at `db_path`.
fn set_up_in_the_browser_at(db_path: &std::path::Path) -> Option<&'static str> {
    use birdnet_db::audio_sources::AudioSourceStore;

    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    if birdnet_db::settings::get(&conn, "onboarding_complete").is_ok_and(|v| v.trim() == "true") {
        return Some("the setup wizard was finished");
    }
    if conn.list().is_ok_and(|rows| !rows.is_empty()) {
        return Some("the Capture tab holds an audio source");
    }
    None
}

pub(super) fn check_config_file(cli: &Cli, config: Option<&Config>) -> Check {
    // Consulted only when the file is absent: reading a database for a
    // station whose file loaded would be work for an answer nobody asks for.
    let browser = if config.is_none() && !cli.config.exists() {
        set_up_in_the_browser(config)
    } else {
        None
    };
    config_file_verdict(cli, config, browser)
}

/// [`check_config_file`] given what the database says about how the station
/// was set up.
fn config_file_verdict(cli: &Cli, config: Option<&Config>, browser: Option<&str>) -> Check {
    if config.is_some() {
        return Check::pass(
            "Configuration file",
            format!("loaded from {}", cli.config.display()),
        );
    }
    if cli.config.exists() {
        Check::fail(
            "Configuration file",
            format!("{} exists but could not be parsed", cli.config.display()),
            "check the file for syntax errors (key=value, one per line; '#' for comments)",
        )
    } else if let Some(how) = browser {
        // An install with no file is a supported way to run: everything the
        // file would say is a setting in the browser, and the built-in defaults
        // fill in the rest. Reporting the absence as a problem sent the owner
        // of a browser-configured station looking for a file they never needed.
        Check::pass(
            "Configuration file",
            format!(
                "no file at {}, and none is needed: this station is set up from the browser \
                 ({how}), with the built-in defaults for the rest",
                cli.config.display()
            ),
        )
    } else {
        // The default config path (/etc/birdnet/birdnet.conf) needs sudo on
        // macOS, where the recommended home is the user-writable Application
        // Support directory the launchd LaunchAgent points at.
        let remediation: &str = if cfg!(target_os = "macos") {
            "create \"$HOME/Library/Application Support/birdnet-behavior/birdnet.conf\" with KEY=value \
             lines (file keys have no BIRDNET_ prefix, e.g. LATITUDE=51.48) and start with -c that \
             path (the /etc default needs sudo on macOS)"
        } else {
            "create /etc/birdnet/birdnet.conf with KEY=value lines — file keys have no BIRDNET_ \
             prefix (LATITUDE=51.48, not BIRDNET_LATITUDE) — or re-run the installer, which writes \
             a commented starter there. Copying .env.example gives BIRDNET_* keys the file ignores"
        };
        Check::warn(
            "Configuration file",
            format!(
                "{} not found — using built-in defaults",
                cli.config.display()
            ),
            remediation,
        )
    }
}

pub(super) fn check_config_values(cli: &Cli, config: &Config) -> Vec<Check> {
    let findings = cfg_validate::validate(config);
    if findings.is_empty() {
        return vec![Check::pass(
            "Configuration values",
            "all settings are within valid ranges",
        )];
    }
    // An error used to be a Fail, which through ExecStartPre's exit-2 gate
    // kept the service from starting at all: a typo over SSH became a restart
    // loop with no web UI (LC-6). The start now recovers on its own — it runs
    // on the last-good copy when one exists, web-only on the file otherwise —
    // so the doctor reports the same decision the start will make, as a
    // warning, and the gate lets the start happen.
    let recovery = crate::helpers::startup_config::recovery_for(&cli.config);
    let has_errors = findings.iter().any(|f| f.severity == ConfigSeverity::Error);
    let mut out: Vec<Check> = findings
        .into_iter()
        .map(|f| {
            let name = format!("Config: {}", f.key);
            match f.severity {
                ConfigSeverity::Error => Check::warn(
                    name,
                    format!(
                        "{} — an error: the station will not run on this value",
                        f.message
                    ),
                    f.remediation,
                ),
                ConfigSeverity::Warning => Check::warn(name, f.message, f.remediation),
            }
        })
        .collect();
    if has_errors {
        out.push(recovery.map_or_else(
            || {
                Check::warn(
                    "Config: fallback",
                    "the file has errors and there is no last good copy; the station will start \
                     web-only, recording nothing, so that this report is reachable at /station",
                    "fix the file and restart, or `--apply-config <corrected file>`; the boot \
                     journal reports `config_rejected` until then",
                )
            },
            |last_good| {
                Check::warn(
                    "Config: fallback",
                    format!(
                        "the file has errors; the station will start on the last good \
                         configuration, {}",
                        last_good.display()
                    ),
                    "fix the file and restart, or `--apply-config <corrected file>`; the boot \
                     journal reports `config_reverted` until then",
                )
            },
        ));
    }
    out
}

pub(super) fn check_listen_address(cli: &Cli) -> Check {
    match cli.listen.parse::<std::net::SocketAddr>() {
        Ok(addr) => Check::pass(
            "Web listen address",
            format!("{addr} parses as a valid socket address"),
        ),
        Err(e) => Check::fail(
            "Web listen address",
            format!("{:?} is not a valid socket address: {e}", cli.listen),
            "use the form HOST:PORT, e.g. 127.0.0.1:8502 or 0.0.0.0:8502",
        ),
    }
}

/// Whether `/admin` is reachable from the network with no password.
///
/// Split from [`check_admin_exposure`] so the decision is unit-testable:
/// resolving `CADDY_PWD` reads the process environment, and `std::env::set_var`
/// is `unsafe` in edition 2024 while this crate forbids `unsafe_code`.
fn admin_exposure(listen: &str, password_configured: bool) -> Check {
    const NAME: &str = "Admin authentication";

    // An unparseable address is already a FAIL from `check_listen_address`;
    // don't editorialise about exposure we cannot determine.
    let Ok(addr) = listen.parse::<std::net::SocketAddr>() else {
        return Check::skip(
            NAME,
            "listen address is not parseable — see the check above",
        );
    };

    if password_configured {
        return Check::pass(NAME, format!("admin password is set ({addr})"));
    }
    if addr.ip().is_loopback() {
        return Check::pass(
            NAME,
            format!(
                "no admin password, but {addr} is loopback-only — /admin is not reachable from the network"
            ),
        );
    }
    Check::warn(
        NAME,
        format!(
            "{addr} is reachable from the network and NO admin password is set — anyone who can \
             reach it can change settings, trigger backups and update the software"
        ),
        "set CADDY_PWD in the config (the bare-metal installer generates one; Docker does not), \
         or bind --listen to 127.0.0.1 and reach the panel over an SSH tunnel",
    )
}

/// What the mutating `/api/v2` surface is doing (`O-1`).
///
/// Three answers, and the middle one is why this exists. A token that is
/// *configured but refused* — too short — leaves the write API off, and the
/// only sign of it is a `warn!` at startup that an operator who set the value
/// and moved on will never read. `--doctor` is the tool the docs point them at.
///
/// Split from [`check_api_surface`] so the decision is unit-testable without a
/// process environment, and resolution is shared with
/// [`crate::helpers::resolve_api_token`] for the same reason
/// [`check_admin_exposure`] shares `resolve_admin_password`: two copies of a
/// credential-resolution rule is exactly how this project once shipped an open
/// `/admin` that its own diagnostic called protected.
fn api_surface(token: Option<&str>) -> Check {
    const NAME: &str = "API write surface";
    let Some(token) = token else {
        return Check::pass(
            NAME,
            "off — the /api/v2 endpoints are read-only, which is the default",
        );
    };
    match birdnet_web::api_token::ApiToken::new(token) {
        Ok(_) => Check::pass(
            NAME,
            "on — the four /api/v2 write endpoints accept this station's bearer token",
        ),
        Err(e) => Check::warn(
            NAME,
            format!("{e}, so the /api/v2 write endpoints are OFF despite being configured"),
            "generate a longer token with `openssl rand -base64 48` and set it as \
             BNB_API_TOKEN in the config or the environment",
        ),
    }
}

/// Report what the mutating `/api/v2` surface is doing.
pub(super) fn check_api_surface(config: Option<&Config>) -> Check {
    let token = crate::helpers::resolve_api_token(
        config,
        std::env::var(birdnet_web::api_token::API_TOKEN_KEY).ok(),
    );
    api_surface(token.as_deref())
}

/// Report whether the admin panel is exposed without authentication.
///
/// The station already logs this at startup (`src/app.rs`), but `--doctor` is
/// the tool the docs point operators at and it said nothing — it checked only
/// that the listen address *parses*. It is the one security-relevant property
/// of a default install: `--listen` defaults to `0.0.0.0:8502`, and with no
/// admin password the cookie middleware synthesises the seed admin and serves
/// `/admin` to anyone (verified: `/admin/settings` returns 200 unauthenticated).
///
/// Resolution is *shared* with the auth bootstrap
/// (`helpers::resolve_admin_password`) — config `CADDY_PWD`, then the
/// environment — so the two cannot disagree. They previously each had their own
/// copy of the rule and a comment asserting they matched: this check read the
/// config while the bootstrap read only the environment, so every bare-metal
/// install (installer writes the password to the config; the unit sets no
/// `EnvironmentFile`) served `/admin` unauthenticated while this reported it
/// protected. A password set through the setup wizard or the accounts page
/// counts too — see [`password_configured`].
pub(super) fn check_admin_exposure(cli: &Cli, config: Option<&Config>) -> Check {
    // Delegate to the resolver the auth bootstrap itself uses, so the
    // diagnostic and the runtime cannot drift apart. They previously held two
    // copies of this rule and a comment claiming they agreed — the copies
    // differed (config-then-env here, env-only there), and the result was a
    // station serving /admin to the network while this check reported it safe.
    admin_exposure(&cli.listen, password_configured(config))
}

/// Whether the station has an admin password, counted the way the runtime
/// counts it: `CADDY_PWD` (config, then environment — the auth bootstrap's
/// own resolver), or a real hash on the seed admin row, which is where the
/// setup wizard and the accounts page put a password. Reading only the first
/// reported a wizard-protected private station as locked out.
///
/// The database read is best-effort and read-only: a missing or unreadable
/// database counts as "no password here", and `check_database` owns its
/// health.
fn password_configured(config: Option<&Config>) -> bool {
    use birdnet_db::accounts::UserStore as _;
    if crate::helpers::resolve_admin_password(config, std::env::var("CADDY_PWD").ok()).is_some() {
        return true;
    }
    let db_path = crate::helpers::db_path_from_config(config);
    if !db_path.exists() {
        return false;
    }
    rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()
    .and_then(|conn| conn.find_user_by_name("admin").ok())
    .is_some_and(|admin| !birdnet_db::accounts::is_legacy_password_hash(&admin.pwd_argon2))
}

/// Private mode (`O-4`): on, off, or on with nothing to sign in with.
///
/// The third is the one that matters. A private station with no admin
/// password answers `503` to everything but the sign-in and the probe — the
/// gate fails closed rather than falling back to the open station the
/// operator asked not to have — and the only other sign of it is an `error!`
/// at startup. The password resolves through the same rule the auth
/// bootstrap uses, for the reason [`check_admin_exposure`] gives.
pub(super) fn check_private_mode(cli: &Cli, config: Option<&Config>) -> Check {
    let setting = crate::helpers::resolve_private_mode(cli, config);
    private_mode(&setting, password_configured(config))
}

fn private_mode(setting: &crate::helpers::PrivateModeSetting, password_configured: bool) -> Check {
    const NAME: &str = "Private mode";
    let public = birdnet_web::private_mode::PublicAccess::describe(&setting.public);
    if !setting.rejected.is_empty() {
        return Check::fail(
            NAME,
            format!(
                "PUBLIC_ACCESS names {} — not a carve-out; the station skips it",
                setting.rejected.join(", ")
            ),
            "use a comma-separated list of live_audio, share and metrics",
        );
    }
    if !setting.enabled {
        return Check::pass(
            NAME,
            "off — viewing is open, the admin panel and every change need a sign-in",
        );
    }
    if !password_configured {
        return Check::fail(
            NAME,
            "on, but no admin password is set — the station answers 503 to everything but \
             the sign-in and the health probe, and nobody can sign in",
            "set CADDY_PWD in the config or the environment, or unset PRIVATE_MODE",
        );
    }
    Check::pass(NAME, format!("on — public access: {public}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    // ── species names ──────────────────────────────────────────────────

    fn two_species() -> birdnet_core::inference::labels::LabelSet {
        birdnet_core::inference::labels::LabelSet::load_from_str(
            "Turdus merula_Common Blackbird\nAcanthisitta chloris_Rifleman\n",
        )
        .expect("labels")
    }

    #[test]
    fn species_names_says_how_much_of_the_classifier_a_language_covers() {
        let set = two_species();
        let de = species_names("de", Some(&set));
        assert_eq!(de.status, Status::Pass);
        assert!(
            de.message.contains("2 of the classifier's 2 species"),
            "{}",
            de.message
        );
        // Korean has no name for the Rifleman: the same station reads differently.
        let ko = species_names("ko", Some(&set));
        assert!(
            ko.message.contains("1 of the classifier's 2 species"),
            "{}",
            ko.message
        );
    }

    #[test]
    fn species_names_without_a_label_file_says_it_cannot_count() {
        let c = species_names("de", None);
        assert_eq!(c.status, Status::Pass);
        assert!(c.message.contains("cannot be counted"), "{}", c.message);
    }

    #[test]
    fn species_names_for_english_or_nothing_is_the_classifiers_own() {
        for raw in ["en", "", "  ", "EN"] {
            let c = species_names(raw, Some(&two_species()));
            assert_eq!(c.status, Status::Pass, "{raw:?}");
            assert!(
                c.message.contains("as the classifier gives them"),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn species_names_warns_about_a_value_that_is_not_a_language() {
        let c = species_names("eng", None);
        assert_eq!(c.status, Status::Warn);
        assert!(
            c.remediation
                .as_deref()
                .is_some_and(|r| r.contains("zh_CN")),
            "the fix must list the choices: {:?}",
            c.remediation
        );
    }

    #[test]
    fn the_species_names_check_reads_the_flag_over_the_file() {
        let mut cli = cli();
        cli.explicit = crate::cli::ExplicitArgs::from_ids(["lang"]);
        cli.lang = "fr".to_owned();
        let cfg = crate::helpers::test_support::config_with(&[("DATABASE_LANG", "de")]);
        let c = check_species_names(&cli, Some(&cfg));
        assert!(c.message.contains("French"), "{}", c.message);
        // Without the flag the file's value is the one in force.
        let c = check_species_names(&self::cli(), Some(&cfg));
        assert!(c.message.contains("German"), "{}", c.message);
    }

    // ── private mode (O-4) ─────────────────────────────────────────────

    fn private_setting(enabled: bool, rejected: &[&str]) -> crate::helpers::PrivateModeSetting {
        crate::helpers::PrivateModeSetting {
            enabled,
            public: std::iter::once(birdnet_web::private_mode::PublicAccess::Share).collect(),
            rejected: rejected.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn private_mode_without_a_password_is_a_failure_naming_the_503() {
        let check = private_mode(&private_setting(true, &[]), false);
        assert_eq!(check.status, Status::Fail);
        assert!(check.message.contains("503"), "{}", check.message);
        assert!(
            check
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("CADDY_PWD")),
            "{:?}",
            check.remediation
        );
    }

    #[test]
    fn private_mode_with_a_password_passes_and_names_the_carve_outs() {
        let check = private_mode(&private_setting(true, &[]), true);
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("share"), "{}", check.message);
    }

    #[test]
    fn private_mode_off_passes_whatever_the_password() {
        assert_eq!(
            private_mode(&private_setting(false, &[]), false).status,
            Status::Pass
        );
        assert_eq!(
            private_mode(&private_setting(false, &[]), true).status,
            Status::Pass
        );
    }

    /// The runtime counts a password held only on the seed admin row — the
    /// setup wizard and the accounts page put it there — and the doctor read
    /// only `CADDY_PWD`. A private station whose password was set in the
    /// wizard was reported locked out: a Fail, which `ExecStartPre` turns into
    /// a refusal to start.
    #[test]
    fn a_password_set_in_the_wizard_counts() {
        use birdnet_db::accounts::UserStore as _;
        assert!(
            std::env::var("CADDY_PWD").is_err(),
            "precondition: CADDY_PWD set in the environment decides this test on its own"
        );
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("birds.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let config =
            Config::parse(&format!("DB_PATH={}\nPRIVATE_MODE=true", db.display())).unwrap();

        // Counterpart first: the seed admin with no password is still none.
        assert_eq!(
            check_private_mode(&cli(), Some(&config)).status,
            Status::Fail
        );

        let admin = conn.find_user_by_name("admin").unwrap();
        // Built at run time: the `CodeQL` query on hard-coded cryptographic
        // values follows a literal into a parameter called `password`.
        let chosen = format!("wizard-chosen-password-{}", std::process::id());
        let hash = birdnet_db::accounts::hash_password(&chosen).unwrap();
        conn.set_password(admin.id, &hash).unwrap();
        drop(conn);

        assert_eq!(
            check_private_mode(&cli(), Some(&config)).status,
            Status::Pass
        );
        let exposure = check_admin_exposure(&cli(), Some(&config));
        assert_eq!(exposure.status, Status::Pass, "{}", exposure.message);
    }

    #[test]
    fn a_rejected_carve_out_is_a_failure_even_with_private_mode_off() {
        let check = private_mode(&private_setting(false, &["live_audo"]), true);
        assert_eq!(check.status, Status::Fail);
        assert!(check.message.contains("live_audo"), "{}", check.message);
    }

    // ── admin exposure ─────────────────────────────────────────────────

    // ── the API write surface ──────────────────────────────────────────

    #[test]
    fn no_token_is_a_pass_because_off_is_the_default() {
        let check = api_surface(None);
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("read-only"), "{}", check.message);
    }

    #[test]
    fn a_usable_token_is_reported_as_on() {
        let check = api_surface(Some(&"a".repeat(64)));
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("on"), "{}", check.message);
    }

    #[test]
    fn a_token_that_was_refused_is_a_warning_and_not_a_pass() {
        // The case this check exists for: the operator set the knob, the
        // station refused it, and the only other sign is one startup log line.
        // Reporting "off — which is the default" here would be true and
        // useless, because they did not choose off.
        let check = api_surface(Some("too-short"));
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.message.contains("despite being configured"),
            "the message must distinguish refused from unset: {}",
            check.message
        );
        assert!(
            check
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("BNB_API_TOKEN")),
            "remediation should name the knob that fixes it"
        );
    }

    #[test]
    fn admin_exposure_warns_on_open_network_bind() {
        // The default install: 0.0.0.0 and no password.
        let check = admin_exposure("0.0.0.0:8502", false);
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.message.contains("NO admin password"),
            "detail should name the problem: {}",
            check.message
        );
        assert!(
            check
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("CADDY_PWD")),
            "remediation should name the knob that fixes it"
        );
    }

    #[test]
    fn admin_exposure_passes_when_a_password_is_set() {
        assert_eq!(admin_exposure("0.0.0.0:8502", true).status, Status::Pass);
    }

    #[test]
    fn admin_exposure_passes_on_loopback_without_a_password() {
        // Not exposed: unreachable from the network, so no password is fine.
        for addr in ["127.0.0.1:8502", "[::1]:8502"] {
            assert_eq!(
                admin_exposure(addr, false).status,
                Status::Pass,
                "{addr} is loopback and should not warn"
            );
        }
    }

    #[test]
    fn admin_exposure_skips_when_the_address_is_unparseable() {
        // check_listen_address already FAILs on this; two errors for one cause
        // is noise.
        assert_eq!(admin_exposure("not-an-address", false).status, Status::Skip);
    }

    /// Build a `Config` for a test, pinned to a database that does not exist.
    ///
    /// `check_station_location` falls back to the `settings` table of
    /// [`crate::helpers::db_path_from_config`], which with no `DB_PATH` key
    /// resolves to `$HOME/BirdNet-Behavior/birds.db`. On CI that file is
    /// absent and the fallback is inert; on any machine that has ever run a
    /// station — a developer's laptop, an operator running `cargo test` on the
    /// Pi — it is the **live database**, and these tests then assert against
    /// whatever coordinates that station happens to hold.
    /// `location_warns_when_unset_because_the_filter_goes_inert` was observed
    /// failing exactly that way, having passed minutes earlier in the same
    /// working tree, because a station had been started in between.
    ///
    /// So every config a test builds names a `DB_PATH` under the process's
    /// temp dir that is never created. Callers that want the fallback to find
    /// something pass their own `DB_PATH`, which wins (last key set wins in
    /// `Config::parse`).
    fn config_from(entries: &[(&str, &str)]) -> Config {
        let absent = std::env::temp_dir().join("bnb-doctor-config-tests-no-such.db");
        let content = std::iter::once(format!("DB_PATH={}", absent.display()))
            .chain(entries.iter().map(|(k, v)| format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join("\n");
        Config::parse(&content).unwrap()
    }

    #[test]
    fn location_warns_when_unset_because_the_filter_goes_inert() {
        let cfg = config_from(&[("ALSA_CARD", "hw:1")]);
        let check = check_station_location(&cli(), Some(&cfg));
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.message.contains("candidate"),
            "must name the consequence, not just the missing key: {}",
            check.message
        );
    }

    #[test]
    fn location_passes_from_the_config_file() {
        let cfg = config_from(&[
            ("ALSA_CARD", "hw:1"),
            ("LATITUDE", "42.3601"),
            ("LONGITUDE", "-71.0589"),
        ]);
        let check = check_station_location(&cli(), Some(&cfg));
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("42.3601"));
    }

    /// A station configured entirely through the dashboard keeps its
    /// coordinates in the settings table, and `--doctor` runs from
    /// `ExecStartPre` before the overlay merges them into the config. Reading
    /// the config alone would warn at exactly the operators who used the
    /// easiest, most-documented path.
    #[test]
    fn location_passes_from_the_dashboard_settings_table() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("birds.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            birdnet_db::settings::ensure_settings_table(&conn).unwrap();
            birdnet_db::settings::set(
                &conn,
                "latitude",
                "51.5074",
                birdnet_db::settings::SettingsCategory::Location,
            )
            .unwrap();
            birdnet_db::settings::set(
                &conn,
                "longitude",
                "-0.1278",
                birdnet_db::settings::SettingsCategory::Location,
            )
            .unwrap();
        }
        let cfg = config_from(&[
            ("ALSA_CARD", "hw:1"),
            ("DB_PATH", db_path.to_str().unwrap()),
        ]);
        let check = check_station_location(&cli(), Some(&cfg));
        assert_eq!(
            check.status,
            Status::Pass,
            "dashboard-configured station must not be warned at: {}",
            check.message
        );
        assert!(check.message.contains("51.5074"));
    }

    // ── BirdWeather needs a position to publish at ──────────────────────

    #[test]
    fn birdweather_without_a_position_warns_that_uploads_are_off() {
        let cfg = config_from(&[("BIRDWEATHER_TOKEN", "tok")]);
        let check = check_birdweather_location(&cli(), Some(&cfg)).expect("a token is set");
        assert_eq!(check.status, Status::Warn);
        assert!(
            check.message.contains("uploads are off") && check.message.contains("not set"),
            "must say what stopped and why: {}",
            check.message
        );
        assert!(
            check
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("LATITUDE") && r.contains("LONGITUDE")),
            "must name the keys: {:?}",
            check.remediation
        );

        // The Null Island default is its own finding, not "unset".
        let zero = config_from(&[
            ("BIRDWEATHER_TOKEN", "tok"),
            ("LATITUDE", "0"),
            ("LONGITUDE", "0"),
        ]);
        let check = check_birdweather_location(&cli(), Some(&zero)).expect("a token is set");
        assert_eq!(check.status, Status::Warn);
        assert!(check.message.contains("both 0"), "{}", check.message);
    }

    #[test]
    fn birdweather_with_a_real_position_passes() {
        let cfg = config_from(&[
            ("BIRDWEATHER_TOKEN", "tok"),
            ("LATITUDE", "42.3601"),
            ("LONGITUDE", "-71.0589"),
        ]);
        let check = check_birdweather_location(&cli(), Some(&cfg)).expect("a token is set");
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("42.3601"), "{}", check.message);
    }

    #[test]
    fn a_station_that_never_set_birdweather_up_is_told_nothing() {
        // Counterpart: no token, no finding, whatever the position is. A
        // warning on every station that does not use BirdWeather would be the
        // noise this diagnostic exists to avoid.
        let cfg = config_from(&[("ALSA_CARD", "hw:1")]);
        assert!(check_birdweather_location(&cli(), Some(&cfg)).is_none());
        assert!(check_birdweather_location(&cli(), None).is_none());
        let blank = config_from(&[("BIRDWEATHER_TOKEN", "   ")]);
        assert!(check_birdweather_location(&cli(), Some(&blank)).is_none());
    }

    /// A token and a position typed on the dashboard live only in the settings
    /// table when `--doctor` runs from `ExecStartPre`.
    #[test]
    fn birdweather_configured_on_the_dashboard_is_judged_from_the_settings_table() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("birds.db");
        let write = |lat: &str, lon: &str| {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            birdnet_db::migration::migrate(&conn).unwrap();
            birdnet_db::settings::ensure_settings_table(&conn).unwrap();
            for (key, value, cat) in [
                (
                    "birdweather_token",
                    "tok",
                    birdnet_db::settings::SettingsCategory::Notifications,
                ),
                (
                    "latitude",
                    lat,
                    birdnet_db::settings::SettingsCategory::Location,
                ),
                (
                    "longitude",
                    lon,
                    birdnet_db::settings::SettingsCategory::Location,
                ),
            ] {
                birdnet_db::settings::set(&conn, key, value, cat).unwrap();
            }
        };
        let cfg = config_from(&[("DB_PATH", db_path.to_str().unwrap())]);

        write("51.5074", "-0.1278");
        let check = check_birdweather_location(&cli(), Some(&cfg)).expect("token is in settings");
        assert_eq!(check.status, Status::Pass, "{}", check.message);
        assert!(check.message.contains("51.5074"), "{}", check.message);

        write("0", "0");
        let check = check_birdweather_location(&cli(), Some(&cfg)).expect("token is in settings");
        assert_eq!(check.status, Status::Warn, "{}", check.message);
    }

    #[test]
    fn location_absent_database_is_not_a_finding_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_from(&[
            ("ALSA_CARD", "hw:1"),
            ("DB_PATH", dir.path().join("absent.db").to_str().unwrap()),
        ]);
        // Still a location warning, but from the missing coordinates — never a
        // database error. check_database owns the database's health.
        let check = check_station_location(&cli(), Some(&cfg));
        assert_eq!(check.status, Status::Warn);
        assert!(check.message.contains("latitude"));
    }

    #[test]
    fn config_file_pass_when_loaded() {
        let cfg = config_from(&[("ALSA_CARD", "hw:1")]);
        let check = check_config_file(&cli(), Some(&cfg));
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("loaded from"));
    }

    #[test]
    fn config_file_warn_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.config = dir.path().join("nope.conf");
        let check = check_config_file(&cli, None);
        assert_eq!(check.status, Status::Warn);
        assert!(check.message.contains("not found"));
    }

    #[test]
    fn config_file_fail_when_present_but_unparsed() {
        // File exists on disk but we pass `None` to model a parse failure.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut cli = cli();
        cli.config = tmp.path().to_path_buf();
        let check = check_config_file(&cli, None);
        assert_eq!(check.status, Status::Fail);
        assert!(check.message.contains("could not be parsed"));
        assert!(check.remediation.is_some());
    }

    /// A station with no `birdnet.conf` that was set up from the browser is not
    /// misconfigured: the file is optional and the page said it could not be
    /// read.
    #[test]
    fn a_missing_file_on_a_station_set_up_in_the_browser_is_information() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.config = dir.path().join("nope.conf");
        let check = config_file_verdict(&cli, None, Some("the setup wizard was finished"));
        assert_eq!(check.status, Status::Pass, "{check:?}");
        assert!(
            check.message.contains("none is needed") && check.message.contains("browser"),
            "{}",
            check.message
        );
        assert!(check.remediation.is_none());
    }

    /// The counterparts: a missing file where nothing else set the station up
    /// stays a warning, and a file that exists but cannot be used is a failure
    /// whatever the database says.
    #[test]
    fn a_missing_file_with_nothing_else_configuring_the_station_still_warns() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.config = dir.path().join("nope.conf");
        let check = config_file_verdict(&cli, None, None);
        assert_eq!(check.status, Status::Warn, "{check:?}");
    }

    #[test]
    fn a_file_that_exists_but_cannot_be_used_is_not_excused_by_the_browser() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut cli = cli();
        cli.config = tmp.path().to_path_buf();
        let check = config_file_verdict(&cli, None, Some("the setup wizard was finished"));
        assert_eq!(check.status, Status::Fail, "{check:?}");
    }

    /// The database signs that count, read from real files.
    #[test]
    fn the_browser_signs_are_a_finished_wizard_or_a_source_and_nothing_else() {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
        let dir = tempfile::tempdir().unwrap();
        // Nothing yet: no file at all.
        assert_eq!(set_up_in_the_browser_at(&dir.path().join("none.db")), None);
        // A migrated database that nobody has touched.
        let fresh = dir.path().join("fresh.db");
        let conn = rusqlite::Connection::open(&fresh).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        assert_eq!(set_up_in_the_browser_at(&fresh), None);
        // A finished wizard.
        birdnet_db::settings::set(
            &conn,
            "onboarding_complete",
            "true",
            birdnet_db::settings::SettingsCategory::System,
        )
        .unwrap();
        assert!(set_up_in_the_browser_at(&fresh).is_some());
        // A source on the Capture tab, with the wizard never finished.
        let sourced = dir.path().join("sourced.db");
        let conn = rusqlite::Connection::open(&sourced).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.insert(&NewAudioSource::defaults(
            "src_1".to_string(),
            SourceKind::UsbAlsa,
            "plughw:1,0".to_string(),
        ))
        .unwrap();
        assert!(set_up_in_the_browser_at(&sourced).is_some());
    }

    #[test]
    fn config_values_pass_when_all_valid() {
        // ALSA_CARD set so the audio-source check stays quiet; no invalid values.
        let cfg = config_from(&[("ALSA_CARD", "hw:1")]);
        let checks = check_config_values(&Cli::parse_from(["birdnet-behavior"]), &cfg);
        assert!(!checks.is_empty());
        assert!(checks.iter().all(|c| c.status == Status::Pass));
    }

    #[test]
    fn config_values_flags_out_of_range_error() {
        // An out-of-range value is an error the station will not run on. It
        // is reported as a warning that says so (LC-6: a Fail through the
        // ExecStartPre gate was a restart loop), alongside the fallback line.
        let cfg = config_from(&[("ALSA_CARD", "hw:1"), ("CONFIDENCE", "5.0")]);
        let checks = check_config_values(&Cli::parse_from(["birdnet-behavior"]), &cfg);
        assert!(checks.iter().any(|c| {
            c.status == Status::Warn
                && c.name.contains("CONFIDENCE")
                && c.message.contains("an error")
        }));
        assert!(checks.iter().any(|c| c.name == "Config: fallback"));
        assert!(checks.iter().all(|c| c.status != Status::Fail));
    }

    #[test]
    fn config_values_flags_warning() {
        // LATITUDE set without LONGITUDE → a warning keyed to one of the pair.
        let cfg = config_from(&[("ALSA_CARD", "hw:1"), ("LATITUDE", "10.0")]);
        let checks = check_config_values(&Cli::parse_from(["birdnet-behavior"]), &cfg);
        assert!(checks.iter().any(|c| {
            c.status == Status::Warn
                && (c.name.contains("LATITUDE") || c.name.contains("LONGITUDE"))
        }));
    }

    #[test]
    fn listen_address_pass_on_valid_default() {
        let check = check_listen_address(&cli());
        assert_eq!(check.status, Status::Pass);
    }

    #[test]
    fn listen_address_fail_on_invalid() {
        let mut cli = cli();
        cli.listen = "not-an-address".to_string();
        let check = check_listen_address(&cli);
        assert_eq!(check.status, Status::Fail);
    }
}

// ── the occurrence filter reports its real state ───────────────────────
//
// These gates were written against the pre-fix diagnostic and observed
// failing. `check_station_location` used to return
//
//     [ PASS ] Station location: 52.5200, 13.4050 — species filtering by
//              occurrence is active
//
// from coordinates alone. Coordinates are necessary but not sufficient: the
// filter also needs a metadata model, which no install ships and the
// diagnostic never looked for. So the one check able to catch a station
// running with no occurrence filtering at all asserted the opposite.
#[cfg(test)]
mod occurrence_filter_gates {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli_from(args: &[&str]) -> Cli {
        let mut v = vec!["birdnet-behavior"];
        v.extend_from_slice(args);
        Cli::parse_from(v)
    }

    /// The location check may report only what a coordinate pair proves.
    ///
    /// Fails on the old code, whose message contained "filtering by
    /// occurrence is active".
    #[test]
    fn station_location_does_not_claim_the_filter_is_active() {
        let cfg = Config::parse("LATITUDE=52.5200\nLONGITUDE=13.4050").unwrap();
        let check = check_station_location(&cli_from(&[]), Some(&cfg));

        assert_eq!(check.status, Status::Pass, "coordinates are set");
        assert!(
            !check.message.contains("is active"),
            "the location check cannot know whether occurrence filtering runs — \
             that needs a metadata model it never looks at: {}",
            check.message
        );
    }

    /// The station with no metadata model — every install today — must be
    /// told that occurrence filtering is off.
    ///
    /// Fails on the old code: no such check existed.
    #[test]
    fn no_metadata_model_warns_that_filtering_is_off() {
        let cfg = Config::parse("LATITUDE=52.5200\nLONGITUDE=13.4050").unwrap();
        let check = check_occurrence_filter(&cli_from(&[]), Some(&cfg));

        assert_eq!(
            check.status,
            Status::Warn,
            "a station with coordinates but no metadata model is running unfiltered"
        );
        assert!(
            check
                .remediation
                .is_some_and(|r| r.contains("METADATA_MODEL_PATH")),
            "the remediation must name the setting that turns it on"
        );
    }

    /// The counterpart: a configured, present model with coordinates is the
    /// one state that may report Pass. Without this the check above would be
    /// satisfied by a diagnostic that always warns.
    #[test]
    fn a_present_model_with_coordinates_passes() {
        let dir = tempfile::tempdir().unwrap();
        let mdata = dir.path().join("geomodel.onnx");
        std::fs::write(&mdata, vec![0u8; 16]).unwrap();
        let cfg = Config::parse(&format!(
            "LATITUDE=52.5200\nLONGITUDE=13.4050\nMETADATA_MODEL_PATH={}",
            mdata.display()
        ))
        .unwrap();

        let check = check_occurrence_filter(&cli_from(&[]), Some(&cfg));
        assert_eq!(check.status, Status::Pass, "{}", check.message);
    }

    /// A configured model that is not on disk is an error, not a warning:
    /// the operator asked for occurrence filtering and is not getting it.
    #[test]
    fn a_configured_but_missing_model_fails() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::parse(&format!(
            "LATITUDE=52.5200\nLONGITUDE=13.4050\nMETADATA_MODEL_PATH={}",
            dir.path().join("absent.onnx").display()
        ))
        .unwrap();

        assert_eq!(
            check_occurrence_filter(&cli_from(&[]), Some(&cfg)).status,
            Status::Fail
        );
    }

    /// A model without coordinates cannot run: the model takes latitude and
    /// longitude as two of its three inputs.
    #[test]
    fn a_model_without_coordinates_warns() {
        let dir = tempfile::tempdir().unwrap();
        let mdata = dir.path().join("geomodel.onnx");
        std::fs::write(&mdata, vec![0u8; 16]).unwrap();
        let cfg = Config::parse(&format!("METADATA_MODEL_PATH={}", mdata.display())).unwrap();

        let check = check_occurrence_filter(&cli_from(&[]), Some(&cfg));
        assert_eq!(check.status, Status::Warn, "{}", check.message);
        assert!(
            check.message.to_lowercase().contains("coordinate")
                || check.message.to_lowercase().contains("location"),
            "the message must say which half is missing: {}",
            check.message
        );
    }

    /// A metadata label file that is configured but absent is the same class
    /// of error as an absent model: the daemon refuses to start on it.
    #[test]
    fn a_configured_but_missing_label_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mdata = dir.path().join("geomodel.onnx");
        std::fs::write(&mdata, vec![0u8; 16]).unwrap();
        let cfg = Config::parse(&format!(
            "LATITUDE=52.5200\nLONGITUDE=13.4050\nMETADATA_MODEL_PATH={}\nMETADATA_LABELS_PATH={}",
            mdata.display(),
            dir.path().join("absent-labels.txt").display()
        ))
        .unwrap();

        assert_eq!(
            check_occurrence_filter(&cli_from(&[]), Some(&cfg)).status,
            Status::Fail
        );
    }

    /// LC-6: a configuration error is reported as a warning naming what the
    /// start will do, so `ExecStartPre`'s exit-2 gate lets the start happen and
    /// the station recovers on its own instead of looping.
    #[test]
    fn a_config_error_is_a_warning_that_names_the_fallback_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("birdnet.conf");
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.config = path.clone();
        let bad = Config::parse("LATITUDE=abc\nLONGITUDE=1\n").unwrap();

        let checks = check_config_values(&cli, &bad);
        assert!(
            checks
                .iter()
                .all(|c| c.status != crate::doctor::Status::Fail),
            "no Fail: the exit-2 gate must let the start happen: {checks:?}"
        );
        let fallback = checks
            .iter()
            .find(|c| c.name == "Config: fallback")
            .expect("the decision the start will make is reported");
        assert!(fallback.message.contains("web-only"), "{fallback:?}");

        std::fs::write(&path, "LATITUDE=1\nLONGITUDE=1\n").unwrap();
        crate::helpers::startup_config::record_last_good(&path).unwrap();
        let checks = check_config_values(&cli, &bad);
        let fallback = checks
            .iter()
            .find(|c| c.name == "Config: fallback")
            .expect("reported");
        assert!(
            fallback.message.contains("last good configuration"),
            "{fallback:?}"
        );

        let good = Config::parse("LATITUDE=1\nLONGITUDE=1\n").unwrap();
        assert!(
            check_config_values(&cli, &good)
                .iter()
                .all(|c| c.name != "Config: fallback"),
            "a good file needs no fallback line"
        );
    }
}
