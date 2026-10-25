//! MQTT over TLS: do the certificate files named in the configuration exist,
//! parse, and fit together?
//!
//! The station publishes from a thread per detection, so a bad `MQTT_CA_FILE`
//! or a client key that does not belong to its certificate shows up as one
//! warning per detection in the journal and a silent Home Assistant. This runs
//! the publisher's own TLS set-up (`birdnet_integrations::mqtt::check_tls`)
//! without connecting, so it is told about over coffee: the broker can be down
//! and the answer is still right.

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;

const NAME: &str = "MQTT TLS";

/// `None` when MQTT is not configured, or is configured without TLS: there is
/// nothing to check, and an operator who never set it up is told nothing.
pub(super) fn check_mqtt_tls(cli: &Cli, config: Option<&Config>) -> Option<Check> {
    let (host, tls) = crate::integrations::broker_plan(cli, config)?;
    let tls = tls?;
    let name = tls.server_name.clone().unwrap_or_else(|| host.clone());
    Some(match birdnet_integrations::mqtt::check_tls(&tls, &name) {
        Ok(()) => {
            let mut parts = vec![format!("broker {host}")];
            if let Some(ca) = &tls.ca_file {
                parts.push(format!("extra trust anchors from {}", ca.display()));
            }
            if let Some(cert) = &tls.client_cert {
                parts.push(format!("client certificate {}", cert.display()));
            }
            Check::pass(NAME, parts.join("; "))
        }
        Err(e) => Check::fail(
            NAME,
            e.to_string(),
            "fix MQTT_CA_FILE / MQTT_CLIENT_CERT / MQTT_CLIENT_KEY (BIRDNET_MQTT_*): each must be a \
             readable PEM file, and the key must belong to the certificate",
        ),
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::check_mqtt_tls;
    use crate::cli::Cli;
    use crate::doctor::Status;

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["birdnet-behavior"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    #[test]
    fn a_station_without_mqtt_or_without_tls_is_told_nothing() {
        assert!(check_mqtt_tls(&cli(&[]), None).is_none());
        assert!(check_mqtt_tls(&cli(&["--mqtt-host", "broker.lan"]), None).is_none());
    }

    #[test]
    fn a_missing_certificate_file_fails_and_names_it() {
        let check = check_mqtt_tls(
            &cli(&[
                "--mqtt-host",
                "broker.lan",
                "--mqtt-client-cert",
                "/nonexistent/client.pem",
                "--mqtt-client-key",
                "/nonexistent/client-key.pem",
            ]),
            None,
        )
        .expect("TLS is on: a client certificate implies it");
        assert_eq!(check.status, Status::Fail, "{check:?}");
        assert!(
            check.message.contains("/nonexistent/client.pem"),
            "{check:?}"
        );
    }

    #[test]
    fn a_certificate_with_no_key_fails() {
        let check = check_mqtt_tls(
            &cli(&[
                "--mqtt-host",
                "broker.lan",
                "--mqtt-client-cert",
                "/nonexistent/client.pem",
            ]),
            None,
        )
        .expect("TLS is on");
        assert_eq!(check.status, Status::Fail, "{check:?}");
        assert!(check.message.contains("private key"), "{check:?}");
    }
}
