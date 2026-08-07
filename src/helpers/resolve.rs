//! Resolve a setting the operator can supply from more than one place.
//!
//! A station's configuration arrives by three routes that all have to coexist:
//! a CLI flag (or its `BIRDNET_*` environment variable), the file config at
//! `/etc/birdnet/birdnet.conf`, and the admin settings form. The rule the whole
//! binary follows is:
//!
//! > **explicit CLI flag / env → admin settings → config file → built-in default**
//!
//! The middle two collapse into one lookup because
//! [`crate::helpers::overlay_db_settings`] has already layered the settings
//! table on top of the parsed config by the time any subsystem is built — so
//! reading the config *is* reading the admin settings, with the file as the
//! fallback.
//!
//! What this module adds is the first step. `clap` fills in `default_value`
//! before anyone looks, so a bare `cli.segment_duration` is `15` whether the
//! operator typed `--segment-duration 15` or said nothing at all. Reading it
//! directly therefore hands the CLI an unconditional win and makes the admin
//! setting unreachable — which is exactly why `segment_duration`,
//! `freq_shift_hz`, `night_inhibit` and friends were editable in the web UI and
//! had no effect on the running station.
//!
//! [`Cli::explicit`](crate::cli::Cli::explicit) records what was really
//! supplied, and the helpers here use it to defer to the config only when the
//! flag was left alone.

use birdnet_core::config::Config;

use crate::cli::Cli;

/// Resolve a parseable setting.
///
/// Returns `cli_value` when the operator actually passed the flag (or set its
/// environment variable); otherwise the config value under `config_key` if it
/// is present and parses; otherwise `cli_value`, which at that point is clap's
/// documented default.
///
/// `arg_id` is the clap argument id — the struct field name, e.g.
/// `"segment_duration"`.
pub fn setting<T>(
    cli: &Cli,
    arg_id: &str,
    cli_value: T,
    config: Option<&Config>,
    config_key: &str,
) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if cli.explicit.has(arg_id) {
        return cli_value;
    }
    config
        .and_then(|c| c.get_parsed::<T>(config_key).ok())
        .unwrap_or(cli_value)
}

/// Resolve a string setting, treating a blank config value as absent.
///
/// Same precedence as [`setting`]. Blank is treated as unset so an empty line in
/// `birdnet.conf` (or a cleared field in the web form) falls through to the
/// default rather than blanking the value.
pub fn setting_str(
    cli: &Cli,
    arg_id: &str,
    cli_value: &str,
    config: Option<&Config>,
    config_key: &str,
) -> String {
    if cli.explicit.has(arg_id) {
        return cli_value.to_owned();
    }
    config
        .and_then(|c| c.get(config_key))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map_or_else(|| cli_value.to_owned(), ToOwned::to_owned)
}

/// Resolve a boolean setting.
///
/// Same precedence as [`setting`]. The config side accepts the spellings the
/// settings form and `birdnet.conf` both produce — `true`/`false`, `1`/`0`,
/// `yes`/`no`, `on`/`off` — case-insensitively; anything else is treated as
/// absent rather than silently reading as `false`.
///
/// A bare `--flag` is only ever *present*, so an explicit `false` cannot be
/// expressed on the command line; leaving the flag off and setting the config
/// key is how a station turns one of these off.
pub fn setting_bool(
    cli: &Cli,
    arg_id: &str,
    cli_value: bool,
    config: Option<&Config>,
    config_key: &str,
) -> bool {
    if cli.explicit.has(arg_id) {
        return cli_value;
    }
    config
        .and_then(|c| c.get(config_key))
        .and_then(|v| parse_bool(v.trim()))
        .unwrap_or(cli_value)
}

/// Parse the boolean spellings a config file or the settings form can carry.
fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::{cli_with_explicit, config_with, default_cli};

    #[test]
    fn config_wins_when_the_flag_was_left_at_its_default() {
        // The case that was broken: clap materialises 15, the operator set 30
        // in the web UI, the overlay put 30 into the config — and the runtime
        // must use 30.
        let cfg = config_with(&[("SEGMENT_DURATION", "30")]);
        let cli = default_cli();
        assert_eq!(
            setting::<u32>(&cli, "segment_duration", 15, Some(&cfg), "SEGMENT_DURATION"),
            30
        );
    }

    #[test]
    fn explicit_flag_beats_the_config() {
        // An operator who really passed the flag keeps it: a value in
        // `birdnet.conf` or the admin form must not override an explicit
        // command line, which is what the systemd unit and Docker rely on.
        let cfg = config_with(&[("SEGMENT_DURATION", "30")]);
        let cli = cli_with_explicit(&["segment_duration"]);
        assert_eq!(
            setting::<u32>(&cli, "segment_duration", 20, Some(&cfg), "SEGMENT_DURATION"),
            20
        );
    }

    #[test]
    fn falls_back_to_the_default_without_a_config() {
        let cli = default_cli();
        assert_eq!(
            setting::<u32>(&cli, "segment_duration", 15, None, "SEGMENT_DURATION"),
            15
        );
    }

    #[test]
    fn unparseable_config_value_falls_back_rather_than_failing() {
        // A typo in the config must not take the station down or silently
        // resolve to zero; it keeps the default.
        let cfg = config_with(&[("SEGMENT_DURATION", "fifteen")]);
        let cli = default_cli();
        assert_eq!(
            setting::<u32>(&cli, "segment_duration", 15, Some(&cfg), "SEGMENT_DURATION"),
            15
        );
    }

    #[test]
    fn string_setting_follows_the_same_precedence() {
        let cfg = config_with(&[("WEEKLY_REPORT_SCHEDULE", "friday")]);
        let cli = default_cli();
        assert_eq!(
            setting_str(
                &cli,
                "weekly_report_schedule",
                "monday",
                Some(&cfg),
                "WEEKLY_REPORT_SCHEDULE"
            ),
            "friday"
        );

        let cli = cli_with_explicit(&["weekly_report_schedule"]);
        assert_eq!(
            setting_str(
                &cli,
                "weekly_report_schedule",
                "sunday",
                Some(&cfg),
                "WEEKLY_REPORT_SCHEDULE"
            ),
            "sunday"
        );
    }

    #[test]
    fn blank_config_string_is_treated_as_unset() {
        let cfg = config_with(&[("WEEKLY_REPORT_SCHEDULE", "   ")]);
        let cli = default_cli();
        assert_eq!(
            setting_str(
                &cli,
                "weekly_report_schedule",
                "monday",
                Some(&cfg),
                "WEEKLY_REPORT_SCHEDULE"
            ),
            "monday"
        );
    }

    #[test]
    fn bool_setting_reads_the_spellings_the_form_and_config_produce() {
        let cli = default_cli();
        for truthy in ["true", "TRUE", "1", "yes", "on"] {
            let cfg = config_with(&[("NIGHT_INHIBIT", truthy)]);
            assert!(
                setting_bool(&cli, "night_inhibit", false, Some(&cfg), "NIGHT_INHIBIT"),
                "{truthy} should read as true"
            );
        }
        for falsy in ["false", "0", "no", "off"] {
            let cfg = config_with(&[("NIGHT_INHIBIT", falsy)]);
            assert!(
                !setting_bool(&cli, "night_inhibit", true, Some(&cfg), "NIGHT_INHIBIT"),
                "{falsy} should read as false"
            );
        }
    }

    #[test]
    fn unrecognised_bool_keeps_the_cli_value() {
        let cfg = config_with(&[("NIGHT_INHIBIT", "maybe")]);
        let cli = default_cli();
        // Must not silently read as `false` and switch night inhibit off.
        assert!(setting_bool(
            &cli,
            "night_inhibit",
            true,
            Some(&cfg),
            "NIGHT_INHIBIT"
        ));
    }

    #[test]
    fn explicit_bool_flag_beats_the_config() {
        let cfg = config_with(&[("NIGHT_INHIBIT", "false")]);
        let cli = cli_with_explicit(&["night_inhibit"]);
        assert!(setting_bool(
            &cli,
            "night_inhibit",
            true,
            Some(&cfg),
            "NIGHT_INHIBIT"
        ));
    }
}
