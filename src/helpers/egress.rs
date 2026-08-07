//! Which outbound connections the station may make on its own.
//!
//! A station reaches the network for two kinds of reason, and they deserve
//! different defaults:
//!
//! * **Because the operator asked it to** — Apprise, `BirdWeather`, MQTT, SMTP,
//!   the heartbeat ping, the weather poll. Each is off until it is configured,
//!   and configuring one *is* the consent. Nothing here touches them.
//! * **On its own initiative** — the daily release check against
//!   `api.github.com` and the Wikipedia species-image downloads. These are on by
//!   default, and until now the update check had no off switch at all: it was
//!   the station's one unconditional, undocumented outbound connection, which
//!   matters on a metered or cellular link, in an air-gapped deployment, and
//!   anywhere the answer to "what does this contact?" has to be given to
//!   somebody else.
//!
//! The decisions live here rather than at each call site so the policy is one
//! readable thing, `--offline` cannot drift out of step with the switch it
//! implies, and both are testable without opening a socket.
//!
//! The complete egress inventory is documented for operators in
//! `docs/book/getting-started/configuration.md`; keep the two in step.

use crate::cli::Cli;

/// Whether the daily release check may run.
///
/// Off when either `--no-update-check` or `--offline` is set.
#[must_use]
pub fn update_check_allowed(cli: &Cli) -> bool {
    !cli.no_update_check && !cli.offline
}

/// Whether species images may be downloaded from Wikipedia.
///
/// Off when `--offline` is set. The pre-existing per-feature opt-out — an
/// explicitly empty `--image-cache-dir` — is handled where the cache is built;
/// this is the master switch on top of it.
#[must_use]
pub fn image_downloads_allowed(cli: &Cli) -> bool {
    !cli.offline
}

/// One line naming what `--offline` turned off, for the startup log.
///
/// Returns `None` when the station is not in offline mode, so the caller logs
/// nothing on the ordinary path.
#[must_use]
pub fn offline_notice(cli: &Cli) -> Option<&'static str> {
    cli.offline.then_some(
        "offline mode: no update check and no species-image downloads. \
         Integrations you configured explicitly (Apprise, BirdWeather, MQTT, SMTP, \
         heartbeat, weather) are unaffected",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::default_cli;

    #[test]
    fn everything_is_allowed_by_default() {
        // The shipped default stays as it was: this slice adds switches, it
        // does not quietly change what a station does out of the box.
        let cli = default_cli();
        assert!(update_check_allowed(&cli));
        assert!(image_downloads_allowed(&cli));
        assert!(offline_notice(&cli).is_none());
    }

    #[test]
    fn no_update_check_stops_only_the_update_check() {
        let mut cli = default_cli();
        cli.no_update_check = true;
        assert!(!update_check_allowed(&cli));
        assert!(
            image_downloads_allowed(&cli),
            "the narrow switch must not also disable images"
        );
    }

    #[test]
    fn offline_implies_the_narrow_switch() {
        // The whole point of the master switch: an operator who sets `--offline`
        // must not have to also know about `--no-update-check`.
        let mut cli = default_cli();
        cli.offline = true;
        assert!(!update_check_allowed(&cli));
        assert!(!image_downloads_allowed(&cli));
        assert!(offline_notice(&cli).is_some());
    }

    #[test]
    fn both_switches_together_are_consistent() {
        let mut cli = default_cli();
        cli.offline = true;
        cli.no_update_check = true;
        assert!(!update_check_allowed(&cli));
        assert!(!image_downloads_allowed(&cli));
    }

    #[test]
    fn offline_does_not_touch_operator_configured_integrations() {
        // Configuring an integration is the consent; offline mode is about
        // connections the station makes on its own initiative. Pinned because
        // silently muting a configured BirdWeather upload or alert channel
        // would be a far worse surprise than an unwanted GitHub request.
        let mut cli = default_cli();
        cli.offline = true;
        cli.apprise_url = Some("http://localhost:8000".to_owned());
        cli.birdweather_token = Some("token".to_owned());
        cli.mqtt_host = Some("mqtt.local".to_owned());
        cli.heartbeat_url = Some("https://hc-ping.com/abc".to_owned());

        assert!(crate::integrations::create_apprise_client(&cli, None).is_some());
        assert!(crate::integrations::create_birdweather_client(&cli, None).is_some());
        assert!(crate::integrations::create_heartbeat_client(&cli, None).is_some());
        assert!(crate::integrations::create_mqtt_client(&cli, None).is_some());
    }
}
