//! Turn the TLS flags, config keys and defaults into one decided plan.
//!
//! Kept apart from `app.rs` because the interesting part is not the wiring but
//! the arithmetic between three settings that can contradict each other —
//! `--listen`, `--tls-listen` and `--tls-redirect` — and that arithmetic is
//! worth testing without standing up a server.
//!
//! The rule, in one place:
//!
//! | `--tls-mode` | `--tls-listen` | Result |
//! |---|---|---|
//! | `off` | anything | Plain HTTP on `--listen`. Nothing else runs. |
//! | on | unset | HTTPS on `--listen`'s host, port 8503; plain HTTP stays on `--listen`. |
//! | on | equal to `--listen` | HTTPS on that one socket. There is no plain port. |
//! | on | a different address | HTTPS there, plain HTTP on `--listen`. |
//!
//! `--tls-redirect` turns the plain listener into a redirector. It is only
//! meaningful in the rows that still have one.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use birdnet_core::config::Config;
use birdnet_web::tls::{TlsMode, TlsSettings};

use crate::cli::Cli;
use crate::helpers::resolve;

/// Port HTTPS uses when the operator names no address of its own.
///
/// One above the dashboard's 8502, so the pair reads as a pair. Not 443:
/// binding a privileged port would mean either running as root or handing the
/// service `CAP_NET_BIND_SERVICE`, and this project's systemd unit
/// deliberately does neither.
pub const DEFAULT_HTTPS_PORT: u16 = 8503;

/// What the binary should actually do about TLS.
#[derive(Debug, Clone)]
pub struct TlsPlan {
    /// Certificate settings, ready for `birdnet_web::tls::server_config`.
    pub settings: TlsSettings,
    /// Where HTTPS binds. `None` when TLS is off.
    pub https_addr: Option<SocketAddr>,
    /// Whether HTTPS and plain HTTP would be the same socket — in which case
    /// only HTTPS is bound.
    pub shares_socket: bool,
    /// Whether the plain listener should serve redirects rather than the app.
    pub redirect: bool,
    /// Anything the operator should be told about a setting that was adjusted
    /// or ignored, ready to log.
    pub warnings: Vec<String>,
}

impl TlsPlan {
    /// Whether a plain-HTTP listener should be bound at all.
    #[must_use]
    pub const fn wants_plain_listener(&self) -> bool {
        !self.shares_socket
    }
}

/// Decide the TLS plan from the CLI, the config file, and the database path.
///
/// `plain_addr` is the already-parsed `--listen`. `db_path` only supplies the
/// default location for generated certificates, which live beside the database
/// so a station's whole mutable state stays in one directory.
///
/// # Errors
///
/// Returns a human-readable message if `--tls-mode` or `--tls-listen` cannot be
/// understood. Both are startup-fatal: silently falling back to plain HTTP
/// after being asked for TLS is the one outcome an operator must never get.
pub fn plan(
    cli: &Cli,
    config: Option<&Config>,
    plain_addr: SocketAddr,
    db_path: &Path,
) -> Result<TlsPlan, String> {
    let mut warnings = Vec::new();

    let mode_raw = resolve::setting_str(cli, "tls_mode", &cli.tls_mode, config, "TLS_MODE");
    let mode: TlsMode = mode_raw
        .parse()
        .map_err(|e| format!("{e} (from --tls-mode / TLS_MODE)"))?;

    if !mode.enabled() {
        return Ok(TlsPlan {
            settings: TlsSettings {
                mode,
                ..TlsSettings::default()
            },
            https_addr: None,
            shares_socket: false,
            redirect: false,
            warnings,
        });
    }

    // Where HTTPS binds.
    let https_addr = match resolve_opt_str(
        cli,
        "tls_listen",
        cli.tls_listen.as_deref(),
        config,
        "TLS_LISTEN",
    ) {
        Some(raw) => raw
            .parse::<SocketAddr>()
            .map_err(|e| format!("--tls-listen {raw:?} is not a socket address: {e}"))?,
        None => SocketAddr::new(plain_addr.ip(), DEFAULT_HTTPS_PORT),
    };

    let shares_socket = https_addr == plain_addr;

    let mut redirect = resolve::setting_bool(
        cli,
        "tls_redirect",
        cli.tls_redirect,
        config,
        "TLS_REDIRECT",
    );
    if redirect && shares_socket {
        // Not an error: an operator who set both has expressed a coherent
        // intent ("only serve HTTPS"), and the redirect is simply unreachable.
        warnings.push(format!(
            "--tls-redirect is ignored because HTTPS and HTTP share {plain_addr}; there is no \
             plain-HTTP port to redirect from"
        ));
        redirect = false;
    }

    let state_dir = cli
        .tls_dir
        .clone()
        .or_else(|| config_path(config, "TLS_DIR"))
        .unwrap_or_else(|| default_state_dir(db_path));

    let hostnames = resolve_hostnames(cli, config, https_addr);

    let cert = cli
        .tls_cert
        .clone()
        .or_else(|| config_path(config, "TLS_CERT"));
    let key = cli
        .tls_key
        .clone()
        .or_else(|| config_path(config, "TLS_KEY"));

    let validity_days = resolve::setting(cli, "tls_days", cli.tls_days, config, "TLS_DAYS");
    let validity_days = if validity_days == 0 {
        warnings.push("--tls-days 0 is not a certificate; using 397".to_string());
        397
    } else {
        validity_days
    };

    Ok(TlsPlan {
        settings: TlsSettings {
            mode,
            cert,
            key,
            state_dir,
            hostnames,
            validity_days,
        },
        https_addr: Some(https_addr),
        shares_socket,
        redirect,
        warnings,
    })
}

/// Generated certificates live beside the database, so a station's mutable
/// state stays in one directory to back up, move, or wipe.
fn default_state_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("tls")
}

/// A config value read as a path, treating blank as absent.
fn config_path(config: Option<&Config>, key: &str) -> Option<PathBuf> {
    config
        .and_then(|c| c.get(key))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Resolve an optional string flag through the same precedence as
/// [`resolve::setting_str`], without inventing a default for it.
fn resolve_opt_str(
    cli: &Cli,
    arg_id: &str,
    cli_value: Option<&str>,
    config: Option<&Config>,
    config_key: &str,
) -> Option<String> {
    if cli.explicit.has(arg_id) {
        return cli_value.map(ToOwned::to_owned);
    }
    config
        .and_then(|c| c.get(config_key))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .or_else(|| cli_value.map(ToOwned::to_owned))
}

/// The SAN list for a generated certificate.
///
/// An explicit list replaces the derived one outright rather than adding to it:
/// an operator who names the one hostname they use should not also be issued a
/// certificate advertising their LAN address.
fn resolve_hostnames(cli: &Cli, config: Option<&Config>, bind: SocketAddr) -> Vec<String> {
    let configured = resolve_opt_str(
        cli,
        "tls_hostnames",
        cli.tls_hostnames.as_deref(),
        config,
        "TLS_HOSTNAMES",
    );

    match configured {
        Some(raw) if !raw.trim().is_empty() => {
            let mut names: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            names.dedup();
            if names.is_empty() {
                birdnet_web::tls::default_hostnames(
                    bind,
                    birdnet_web::tls::system_hostname().as_deref(),
                )
            } else {
                names
            }
        }
        _ => birdnet_web::tls::default_hostnames(
            bind,
            birdnet_web::tls::system_hostname().as_deref(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::cli_from;

    fn addr(s: &str) -> SocketAddr {
        s.parse().expect("test address")
    }

    fn plan_for(args: &[&str]) -> TlsPlan {
        let cli = cli_from(args);
        let listen = addr(&cli.listen);
        plan(&cli, None, listen, Path::new("/var/lib/birdnet/birds.db")).expect("plan")
    }

    #[test]
    fn off_by_default_binds_no_https() {
        let p = plan_for(&["birdnet-behavior"]);
        assert_eq!(p.settings.mode, TlsMode::Off);
        assert!(p.https_addr.is_none());
        assert!(p.wants_plain_listener());
        assert!(!p.redirect);
    }

    #[test]
    fn https_defaults_to_a_port_of_its_own_beside_the_plain_one() {
        // Deliberately not the default `--listen`: with 0.0.0.0 this could not
        // tell "derived from --listen" from "hardcoded 0.0.0.0", which is what
        // `https_follows_a_loopback_listen` below is for. What this one pins is
        // the *port* — that HTTPS gets its own, and does not land on the plain
        // port and displace it.
        let p = plan_for(&[
            "birdnet-behavior",
            "--listen",
            "10.1.2.3:8502",
            "--tls-mode",
            "self-signed",
        ]);
        assert_eq!(p.https_addr, Some(addr("10.1.2.3:8503")));
        assert!(
            p.wants_plain_listener(),
            "plain HTTP must keep answering on --listen unless the operator asked otherwise"
        );
    }

    #[test]
    fn https_follows_a_loopback_listen() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--listen",
            "127.0.0.1:9000",
            "--tls-mode",
            "self-signed",
        ]);
        assert_eq!(
            p.https_addr,
            Some(addr("127.0.0.1:8503")),
            "a station bound to loopback must not have HTTPS silently exposed on 0.0.0.0"
        );
    }

    #[test]
    fn one_socket_when_tls_listen_matches_listen() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--listen",
            "0.0.0.0:8502",
            "--tls-mode",
            "self-signed",
            "--tls-listen",
            "0.0.0.0:8502",
        ]);
        assert_eq!(p.https_addr, Some(addr("0.0.0.0:8502")));
        assert!(p.shares_socket);
        assert!(
            !p.wants_plain_listener(),
            "binding the same address twice would fail with EADDRINUSE"
        );
    }

    #[test]
    fn redirect_is_dropped_when_there_is_no_plain_port() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-listen",
            "0.0.0.0:8502",
            "--tls-redirect",
        ]);
        assert!(!p.redirect);
        assert!(
            p.warnings
                .iter()
                .any(|w| w.contains("--tls-redirect is ignored")),
            "dropping a setting silently is how an operator ends up believing \
             plain HTTP is closed when it never was: {:?}",
            p.warnings
        );
    }

    #[test]
    fn redirect_survives_when_the_ports_differ() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-redirect",
        ]);
        assert!(p.redirect);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    }

    #[test]
    fn an_unknown_mode_is_fatal_rather_than_a_silent_downgrade() {
        let cli = cli_from(&["birdnet-behavior", "--tls-mode", "yes-please"]);
        let err = plan(&cli, None, addr("0.0.0.0:8502"), Path::new("/tmp/x.db"))
            .expect_err("an unparseable mode must not start the server on plain HTTP");
        assert!(err.contains("--tls-mode"), "{err}");
    }

    #[test]
    fn an_unparseable_tls_listen_is_fatal() {
        let cli = cli_from(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-listen",
            "8503",
        ]);
        let err = plan(&cli, None, addr("0.0.0.0:8502"), Path::new("/tmp/x.db"))
            .expect_err("a bare port is not a socket address");
        assert!(err.contains("--tls-listen"), "{err}");
    }

    #[test]
    fn certificates_live_beside_the_database() {
        let cli = cli_from(&["birdnet-behavior", "--tls-mode", "self-signed"]);
        let p = plan(
            &cli,
            None,
            addr("0.0.0.0:8502"),
            Path::new("/var/lib/birdnet/birds.db"),
        )
        .expect("plan");
        assert_eq!(p.settings.state_dir, PathBuf::from("/var/lib/birdnet/tls"));
    }

    #[test]
    fn an_explicit_tls_dir_wins() {
        let cli = cli_from(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-dir",
            "/srv/certs",
        ]);
        let p = plan(
            &cli,
            None,
            addr("0.0.0.0:8502"),
            Path::new("/var/lib/birdnet/birds.db"),
        )
        .expect("plan");
        assert_eq!(p.settings.state_dir, PathBuf::from("/srv/certs"));
    }

    #[test]
    fn explicit_hostnames_replace_the_derived_list() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-hostnames",
            "birds.example.com, 10.0.0.5",
        ]);
        assert_eq!(
            p.settings.hostnames,
            vec!["birds.example.com".to_string(), "10.0.0.5".to_string()],
            "an operator who names their hostname should not also be issued a \
             certificate advertising the machine's LAN address"
        );
    }

    #[test]
    fn derived_hostnames_include_localhost_and_the_bound_address() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--listen",
            "192.168.1.40:8502",
            "--tls-mode",
            "self-signed",
        ]);
        assert!(p.settings.hostnames.iter().any(|h| h == "localhost"));
        assert!(p.settings.hostnames.iter().any(|h| h == "192.168.1.40"));
    }

    #[test]
    fn manual_mode_carries_the_paths_through() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--tls-mode",
            "manual",
            "--tls-cert",
            "/etc/ssl/bird.crt",
            "--tls-key",
            "/etc/ssl/bird.key",
        ]);
        assert_eq!(p.settings.mode, TlsMode::Manual);
        assert_eq!(p.settings.cert, Some(PathBuf::from("/etc/ssl/bird.crt")));
        assert_eq!(p.settings.key, Some(PathBuf::from("/etc/ssl/bird.key")));
    }

    #[test]
    fn zero_validity_days_is_corrected_rather_than_minted() {
        let p = plan_for(&[
            "birdnet-behavior",
            "--tls-mode",
            "self-signed",
            "--tls-days",
            "0",
        ]);
        assert_eq!(p.settings.validity_days, 397);
        assert!(!p.warnings.is_empty());
    }
}
