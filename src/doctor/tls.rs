//! TLS checks: is the dashboard encrypted, and can it actually start that way?
//!
//! Two distinct questions, and the second is the one that costs an operator a
//! callout. `--tls-mode manual` with a path typo, or a key that does not match
//! its certificate, fails at *startup* — after systemd has already restarted
//! the unit a few times. Everything here is what `birdnet_web::tls` would do at
//! boot, run early enough to be told about over coffee instead.

use birdnet_core::config::Config;
use birdnet_web::tls::{TlsMode, TlsSettings};

use super::Check;
use crate::cli::Cli;

/// Name shared by the checks so a report reads as one group.
const NAME: &str = "HTTPS";

pub(super) fn check_tls(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    let Ok(plain_addr) = cli.listen.parse::<std::net::SocketAddr>() else {
        // `check_listen_address` has already failed on this; don't say it twice.
        return vec![Check::skip(
            NAME,
            "listen address is not parseable — see the check above",
        )];
    };
    let db_path = crate::helpers::db_path_from_config(config);

    let plan = match crate::helpers::tls::plan(cli, config, plain_addr, &db_path) {
        Ok(p) => p,
        Err(e) => {
            return vec![Check::fail(
                NAME,
                e,
                "set --tls-mode to one of: off, self-signed, manual",
            )];
        }
    };

    let mut out = Vec::new();

    if !plan.settings.mode.enabled() {
        // Not a warning by default. A station on a trusted LAN behind a
        // reverse proxy is a perfectly good deployment, and nagging every
        // operator about it would train them to ignore this report. It only
        // becomes advice when the dashboard is on a routable address.
        let msg = format!("off — the dashboard serves plain HTTP on {plain_addr}");
        out.push(if plain_addr.ip().is_loopback() {
            Check::pass(NAME, format!("{msg} (loopback only)"))
        } else {
            Check::warn(
                NAME,
                msg,
                "on a network anyone else can reach, set --tls-mode self-signed (or put a \
                 TLS-terminating reverse proxy in front)",
            )
        });
        return out;
    }

    for warning in &plan.warnings {
        out.push(Check::warn(NAME, warning.clone(), "review the TLS flags"));
    }

    // The expensive, honest check: do exactly what startup does.
    match birdnet_web::tls::server_config(&plan.settings) {
        Ok(Some(_)) => out.push(Check::pass(
            NAME,
            describe_ready(&plan.settings, plan.https_addr),
        )),
        Ok(None) => out.push(Check::skip(NAME, "TLS is off")),
        Err(e) => out.push(Check::fail(
            NAME,
            format!(
                "mode {} is configured but unusable: {e}",
                plan.settings.mode
            ),
            remediation_for(&plan.settings),
        )),
    }

    if plan.settings.mode == TlsMode::SelfSigned {
        let ca = birdnet_web::tls::ca_certificate_path(&plan.settings.state_dir);
        out.push(if ca.exists() {
            Check::pass(
                NAME,
                format!("import {} to stop the browser warning", ca.display()),
            )
        } else {
            // Reachable when `server_config` above failed; the message there
            // is the actionable one, so this stays a skip rather than piling on.
            Check::skip(NAME, "no CA generated yet — see the failure above")
        });
    }

    out
}

/// One line describing a TLS setup that is ready to serve.
fn describe_ready(settings: &TlsSettings, addr: Option<std::net::SocketAddr>) -> String {
    let where_ = addr.map_or_else(String::new, |a| format!(" on {a}"));
    match settings.mode {
        TlsMode::SelfSigned => format!(
            "self-signed{where_}, covering {} (valid {} days)",
            settings.hostnames.join(", "),
            settings.validity_days
        ),
        TlsMode::Manual => format!(
            "manual{where_}, from {}",
            settings
                .cert
                .as_ref()
                .map_or_else(|| "?".into(), |p| p.display().to_string())
        ),
        TlsMode::Off => "off".to_string(),
    }
}

/// What to actually do about a mode that will not start.
fn remediation_for(settings: &TlsSettings) -> String {
    match settings.mode {
        TlsMode::Manual => format!(
            "check that --tls-cert ({}) and --tls-key ({}) exist, are readable by the service \
             user, are PEM, and are a matching pair",
            settings
                .cert
                .as_ref()
                .map_or_else(|| "unset".into(), |p| p.display().to_string()),
            settings
                .key
                .as_ref()
                .map_or_else(|| "unset".into(), |p| p.display().to_string()),
        ),
        TlsMode::SelfSigned => format!(
            "check that {} is writable by the service user",
            settings.state_dir.display()
        ),
        TlsMode::Off => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use crate::helpers::test_support::cli_from;

    fn statuses(checks: &[Check]) -> Vec<Status> {
        checks.iter().map(|c| c.status).collect()
    }

    #[test]
    fn plain_http_on_loopback_is_fine() {
        let cli = cli_from(&["birdnet-behavior", "--listen", "127.0.0.1:8502"]);
        let checks = check_tls(&cli, None);
        assert_eq!(statuses(&checks), vec![Status::Pass], "{checks:?}");
    }

    #[test]
    fn plain_http_on_a_routable_address_is_worth_saying() {
        let cli = cli_from(&["birdnet-behavior", "--listen", "0.0.0.0:8502"]);
        let checks = check_tls(&cli, None);
        assert_eq!(
            statuses(&checks),
            vec![Status::Warn],
            "a dashboard anyone on the network can reach, in the clear, should \
             not report clean: {checks:?}"
        );
    }

    #[test]
    fn an_unknown_mode_fails_the_diagnostic() {
        let cli = cli_from(&["birdnet-behavior", "--tls-mode", "sure"]);
        let checks = check_tls(&cli, None);
        assert_eq!(statuses(&checks), vec![Status::Fail], "{checks:?}");
    }

    #[test]
    fn manual_mode_with_missing_files_fails_here_rather_than_at_boot() {
        let cli = cli_from(&[
            "birdnet-behavior",
            "--tls-mode",
            "manual",
            "--tls-cert",
            "/nonexistent/bird.crt",
            "--tls-key",
            "/nonexistent/bird.key",
        ]);
        let checks = check_tls(&cli, None);
        assert!(
            checks.iter().any(|c| c.status == Status::Fail),
            "{checks:?}"
        );
        assert!(
            checks.iter().any(|c| c
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("--tls-cert"))),
            "the remediation has to name the flag that is wrong: {checks:?}"
        );
    }

    #[test]
    fn self_signed_passes_and_names_the_ca_to_import() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cli = cli_from(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-dir",
            dir.path().to_str().expect("utf-8 path"),
        ]);
        let checks = check_tls(&cli, None);
        assert!(
            checks.iter().all(|c| c.status == Status::Pass),
            "{checks:?}"
        );
        assert!(
            checks.iter().any(|c| c.message.contains("local-ca.crt")),
            "an operator has to be told which file to import: {checks:?}"
        );
    }
}
