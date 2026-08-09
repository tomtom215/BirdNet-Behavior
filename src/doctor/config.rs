//! Configuration checks: file presence/parse, value validation, listen address.

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self as cfg_validate, Severity as ConfigSeverity};

use super::Check;
use crate::cli::Cli;

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
/// Resolution deliberately mirrors `app.rs` exactly — config `CADDY_PWD`, then
/// the environment — so the two can never disagree. Like the runtime warning,
/// it therefore tracks the env/config knob: a password set only through the
/// accounts UI also protects the panel but is not visible here, which the
/// remediation text accounts for by naming `CADDY_PWD`.
pub(super) fn check_admin_exposure(cli: &Cli, config: Option<&Config>) -> Check {
    let password_configured = config
        .and_then(|c| c.get("CADDY_PWD").map(str::to_owned))
        .or_else(|| std::env::var("CADDY_PWD").ok())
        .is_some_and(|pwd| !pwd.is_empty());
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

    fn config_from(entries: &[(&str, &str)]) -> Config {
        let content = entries
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        Config::parse(&content).unwrap()
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
