//! Configuration checks: file presence/parse, value validation, listen address.

use std::path::PathBuf;

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self as cfg_validate, Severity as ConfigSeverity};

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
         include birds that do not occur near you. Set it on the dashboard \
         (Settings → Location has a detect button), or put LATITUDE / LONGITUDE \
         in the config and restart",
    )
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
/// Deliberately no ONNX session is opened. `--doctor` runs from
/// `ExecStartPre` on every start, and a diagnostic that loads a model can fail
/// for reasons that have nothing to do with the thing being diagnosed. The
/// vocabulary alignment a session would prove is checked by
/// `SpeciesFilter::load_with_vocabulary` at startup, which refuses a
/// mismatched model outright; this check owns the configuration half.
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
            "the model takes latitude and longitude as two of its three inputs: set them on the dashboard (Settings → Location has a detect button) or put LATITUDE / LONGITUDE in the config and restart",
        );
    }

    let how = labels.map_or_else(
        || " (indexed against the classifier's labels)".to_owned(),
        |l| format!(" (matched by name through {})", l.display()),
    );
    Check::pass(NAME, format!("active — {}{how}", model.display()))
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

pub(super) fn check_config_file(cli: &Cli, config: Option<&Config>) -> Check {
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
    } else {
        // The default config path (/etc/birdnet/birdnet.conf) needs sudo on
        // macOS, where the recommended home is the user-writable Application
        // Support directory the launchd LaunchAgent points at.
        let remediation: &str = if cfg!(target_os = "macos") {
            "copy .env.example to \"$HOME/Library/Application Support/birdnet-behavior/birdnet.conf\" \
             and start with -c that path (the /etc default needs sudo on macOS)"
        } else {
            "copy .env.example to /etc/birdnet/birdnet.conf and edit before going to production"
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

pub(super) fn check_config_values(config: &Config) -> Vec<Check> {
    let findings = cfg_validate::validate(config);
    if findings.is_empty() {
        return vec![Check::pass(
            "Configuration values",
            "all settings are within valid ranges",
        )];
    }
    findings
        .into_iter()
        .map(|f| {
            let name = format!("Config: {}", f.key);
            match f.severity {
                ConfigSeverity::Error => Check::fail(name, f.message, f.remediation),
                ConfigSeverity::Warning => Check::warn(name, f.message, f.remediation),
            }
        })
        .collect()
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
/// protected. Like the runtime warning it tracks the env/config knob: a
/// password set only through the accounts UI also protects the panel but is not
/// visible here, which the remediation text accounts for by naming
/// `CADDY_PWD`.
pub(super) fn check_admin_exposure(cli: &Cli, config: Option<&Config>) -> Check {
    // Delegate to the resolver the auth bootstrap itself uses, so the
    // diagnostic and the runtime cannot drift apart. They previously held two
    // copies of this rule and a comment claiming they agreed — the copies
    // differed (config-then-env here, env-only there), and the result was a
    // station serving /admin to the network while this check reported it safe.
    let password_configured =
        crate::helpers::resolve_admin_password(config, std::env::var("CADDY_PWD").ok()).is_some();
    admin_exposure(&cli.listen, password_configured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    // ── admin exposure ─────────────────────────────────────────────────

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

    #[test]
    fn config_values_pass_when_all_valid() {
        // ALSA_CARD set so the audio-source check stays quiet; no invalid values.
        let cfg = config_from(&[("ALSA_CARD", "hw:1")]);
        let checks = check_config_values(&cfg);
        assert!(!checks.is_empty());
        assert!(checks.iter().all(|c| c.status == Status::Pass));
    }

    #[test]
    fn config_values_flags_out_of_range_error() {
        let cfg = config_from(&[("ALSA_CARD", "hw:1"), ("CONFIDENCE", "5.0")]);
        let checks = check_config_values(&cfg);
        assert!(
            checks
                .iter()
                .any(|c| c.status == Status::Fail && c.name.contains("CONFIDENCE"))
        );
    }

    #[test]
    fn config_values_flags_warning() {
        // LATITUDE set without LONGITUDE → a warning keyed to one of the pair.
        let cfg = config_from(&[("ALSA_CARD", "hw:1"), ("LATITUDE", "10.0")]);
        let checks = check_config_values(&cfg);
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
}
