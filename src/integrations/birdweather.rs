//! `BirdWeather` upload client construction.

use crate::cli::Cli;

/// What to do about a station position `BirdWeather` cannot be given: the
/// closing sentence of the startup warning and of `--doctor`'s finding.
pub const LOCATION_FIX: &str = "Set LATITUDE and LONGITUDE (decimal degrees, e.g. LATITUDE=51.48 \
     and LONGITUDE=-0.13) in the config, or under Settings → Capture → Location & Recording \
     Schedule, and restart";

/// The coordinates `BirdWeather` may be given for this station, or a sentence
/// saying what is wrong with them.
///
/// Resolved through [`crate::daemon::resolve_station_coords`] — the rule the
/// detection daemon and `--doctor` already share — and then held to
/// [`birdnet_integrations::birdweather::check_location`]: set, parseable, on
/// the globe, and not exactly 0°, 0°.
///
/// Every upload carries the station's position onto a public map. This used to
/// be `unwrap_or(0.0)` for each, so a station with a token and no location
/// posted everything at "Null Island" in the Gulf of Guinea. There is no land
/// at 0°, 0°, so no station is lost by refusing it.
///
/// # Errors
///
/// A sentence saying what is wrong, for the log and the diagnostic to print as
/// it is; [`LOCATION_FIX`] says what to do about it.
pub fn birdweather_location(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Result<(f64, f64), String> {
    let (lat, lon) = crate::daemon::resolve_station_coords(cli, config);
    birdnet_integrations::birdweather::check_location(lat, lon).map_err(|problem| {
        // An unparseable value resolves to `None` like an absent one; say
        // which it was, because "not set" is a lie to the operator who set it.
        let typed_but_unreadable = [("LATITUDE", lat), ("LONGITUDE", lon)]
            .into_iter()
            .find_map(|(key, parsed)| {
                let text = config?.get(key)?.trim();
                (parsed.is_none() && !text.is_empty())
                    .then(|| format!("{key}={text:?} is not a number"))
            });
        typed_but_unreadable.unwrap_or_else(|| problem.to_string())
    })
}

/// The upload bar and the blur radius from their config values.
///
/// Unset or empty is "none" (`None`, `0.0`): a station that has not asked for
/// either uploads and positions exactly as before. A value that is not usable is
/// an `Err` naming the key, and the caller falls back to "none" for **that**
/// setting only, with a warning. Out-of-range is refused rather than clamped:
/// `75` typed for 75 % clamps to 1 and would stop every upload.
///
/// # Errors
///
/// A sentence naming the key and what is wrong with its value.
pub(crate) fn birdweather_options(
    threshold: Option<&str>,
    accuracy: Option<&str>,
) -> Result<(Option<f32>, f64), String> {
    let blank = |v: Option<&str>| {
        v.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let bar = match blank(threshold) {
        None => None,
        Some(text) => {
            let v = birdnet_core::config::locale::parse_decimal(&text).map_err(|_| {
                format!("BIRDWEATHER_THRESHOLD={text:?} is not a number between 0 and 1")
            })?;
            if !(0.0..=1.0).contains(&v) {
                return Err(format!(
                    "BIRDWEATHER_THRESHOLD={text:?} is outside 0 to 1 (for 75 %, write 0.75)"
                ));
            }
            #[allow(clippy::cast_possible_truncation)]
            Some(v as f32).filter(|c| *c > 0.0)
        }
    };
    let metres = match blank(accuracy) {
        None => 0.0,
        Some(text) => {
            let v = birdnet_core::config::locale::parse_decimal(&text).map_err(|_| {
                format!("BIRDWEATHER_LOCATION_ACCURACY={text:?} is not a number of metres")
            })?;
            let max = birdnet_integrations::birdweather::MAX_LOCATION_ACCURACY_M;
            if !(0.0..=max).contains(&v) {
                return Err(format!(
                    "BIRDWEATHER_LOCATION_ACCURACY={text:?} is outside 0 to {max} metres"
                ));
            }
            v
        }
    };
    Ok((bar, metres))
}

/// Create a `BirdWeather` client from CLI flags and/or config file values.
///
/// Returns `None` if no station token is configured, and `None` — with one
/// warning that names the keys to set — if the station has no usable
/// coordinates (see [`birdweather_location`]): detections are not uploaded at
/// a position the station never had.
pub fn create_birdweather_client(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<birdnet_integrations::birdweather::Client> {
    let token = cli
        .birdweather_token
        .clone()
        .or_else(|| config?.get("BIRDWEATHER_TOKEN").map(String::from))?;

    let (lat, lon) = match birdweather_location(cli, config) {
        Ok(at) => at,
        Err(why) => {
            tracing::warn!(
                "BirdWeather uploads are OFF: a station token is set but the station's \
                 position is not usable — {why}. {LOCATION_FIX}"
            );
            return None;
        }
    };

    // Optional API-base override: `BIRDNET_BIRDWEATHER_URL` env, then the
    // `BIRDWEATHER_URL` config key. For research programmes that route
    // observations to a self-hosted ingest (sensitive-species data kept
    // under the programme's own governance) and for the end-to-end test
    // suite. Logged whenever active so a misdirected station is visible in
    // the first journal lines, not after a season of silent uploads.
    let base_override = std::env::var("BIRDNET_BIRDWEATHER_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            config?
                .get("BIRDWEATHER_URL")
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(String::from)
        });

    // The two BirdWeather-only settings (P-2). Read as literal `config.get`s,
    // for `tests/every_config_key_is_known.rs`.
    let (min_confidence, accuracy_m) = match birdweather_options(
        config.and_then(|c| c.get("BIRDWEATHER_THRESHOLD")),
        config.and_then(|c| c.get("BIRDWEATHER_LOCATION_ACCURACY")),
    ) {
        Ok(options) => options,
        Err(why) => {
            tracing::warn!("{why}; BirdWeather uploads are not affected by that setting");
            (None, 0.0)
        }
    };

    match birdnet_integrations::birdweather::Client::new(&token, lat, lon) {
        Ok(client) => {
            let client = match min_confidence {
                Some(c) => client.with_min_confidence(c),
                None => client,
            };
            let client = client.with_location_accuracy(accuracy_m);
            if let Some(bar) = client.min_confidence() {
                tracing::info!(
                    bar,
                    "BirdWeather: only detections at or above this confidence are uploaded"
                );
            }
            if client.location_accuracy() > 0.0 {
                tracing::info!(
                    metres = client.location_accuracy(),
                    "BirdWeather: the posted position is blurred by up to this far"
                );
            }
            let client = if let Some(url) = base_override {
                let client = client.with_base_url(&url);
                tracing::info!(
                    endpoint = %crate::support::redact_value(client.base_url()),
                    "BirdWeather uploads enabled (custom endpoint)"
                );
                client
            } else {
                tracing::info!(lat, lon, "BirdWeather uploads enabled");
                client
            };
            Some(client)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to create BirdWeather client");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_birdweather_options_are_off_unless_set_and_refused_when_out_of_range() {
        use super::birdweather_options as opts;
        assert_eq!(opts(None, None), Ok((None, 0.0)));
        assert_eq!(opts(Some(""), Some("  ")), Ok((None, 0.0)));
        assert_eq!(opts(Some("0"), Some("0")), Ok((None, 0.0)), "0 is none");
        assert_eq!(opts(Some("0,85"), Some("250")), Ok((Some(0.85), 250.0)));
        assert!(opts(Some("75"), None).unwrap_err().contains("0.75"));
        assert!(opts(Some("-0.1"), None).is_err());
        assert!(opts(Some("high"), None).is_err());
        assert!(opts(None, Some("-5")).is_err());
        assert!(opts(None, Some("50001")).is_err());
        assert!(opts(None, Some("1.5km")).is_err());
        assert!(
            opts(None, Some("50000")).is_ok(),
            "the cap itself is allowed"
        );
    }

    use super::{birdweather_location, create_birdweather_client};
    use crate::integrations::test_support::{config_with, default_cli};

    #[test]
    fn birdweather_is_none_without_token() {
        let cli = default_cli();
        assert!(create_birdweather_client(&cli, None).is_none());
    }

    #[test]
    fn birdweather_built_from_cli_token() {
        let mut cli = default_cli();
        cli.birdweather_token = Some("station-abc".to_owned());
        cli.latitude = Some(42.36);
        cli.longitude = Some(-71.06);
        let client = create_birdweather_client(&cli, None).expect("client built");
        assert_eq!(client.coordinates(), (42.36, -71.06));
    }

    #[test]
    fn birdweather_built_from_config_token() {
        let cli = default_cli();
        let cfg = config_with(&[
            ("BIRDWEATHER_TOKEN", "config-station"),
            ("LATITUDE", "40.0"),
            ("LONGITUDE", "-74.0"),
        ]);
        let client = create_birdweather_client(&cli, Some(&cfg)).expect("client built");
        assert_eq!(client.coordinates(), (40.0, -74.0));
    }

    /// The defect: a token and no coordinates built a client at 0, 0 and every
    /// detection was uploaded to a public map in the Gulf of Guinea. The old
    /// version of this test asserted exactly that (`(0.0, 0.0)`), on the
    /// strength of a comment saying BirdWeather "treats that as an
    /// intentionally-anonymous station" that nothing in the repository backs.
    #[test]
    fn a_station_with_a_token_and_no_coordinates_is_not_uploaded_at_null_island() {
        let mut cli = default_cli();
        cli.birdweather_token = Some("anonymous".to_owned());
        assert!(
            create_birdweather_client(&cli, None).is_none(),
            "a client was built with no station position"
        );
    }

    /// Every way of having no usable position builds no client — unset,
    /// half-set, unparseable, off the globe, and exactly 0, 0 — and each is
    /// refused for its own reason. The counterpart: the same token with a real
    /// position builds one.
    #[test]
    fn only_a_usable_position_builds_a_client() {
        let cli = default_cli();
        let built = |lat: Option<&str>, lon: Option<&str>| {
            let mut entries = vec![("BIRDWEATHER_TOKEN", "tok")];
            if let Some(v) = lat {
                entries.push(("LATITUDE", v));
            }
            if let Some(v) = lon {
                entries.push(("LONGITUDE", v));
            }
            create_birdweather_client(&cli, Some(&config_with(&entries))).is_some()
        };
        assert!(!built(None, None), "unset");
        assert!(!built(Some("51.48"), None), "longitude missing");
        assert!(!built(None, Some("-0.13")), "latitude missing");
        assert!(!built(Some("north"), Some("-0.13")), "unparseable");
        assert!(
            !built(Some("951.48"), Some("-0.13")),
            "latitude off the globe"
        );
        assert!(
            !built(Some("51.48"), Some("-190")),
            "longitude off the globe"
        );
        assert!(!built(Some("0"), Some("0.0")), "exactly 0, 0");
        assert!(built(Some("51.48"), Some("-0.13")), "a real position");
        assert!(built(Some("0"), Some("25.5")), "the equator is a place");
    }

    #[test]
    fn the_refusal_names_the_keys_to_set_and_says_what_was_typed() {
        let cli = default_cli();
        let unset = birdweather_location(&cli, None).unwrap_err();
        assert!(unset.contains("not set"), "{unset}");
        // ...and the remedy printed after it names the two keys.
        assert!(
            super::LOCATION_FIX.contains("LATITUDE") && super::LOCATION_FIX.contains("LONGITUDE")
        );

        let cfg = config_with(&[("LATITUDE", "north"), ("LONGITUDE", "-0.13")]);
        let typed = birdweather_location(&cli, Some(&cfg)).unwrap_err();
        assert!(
            typed.contains("LATITUDE=\"north\" is not a number"),
            "an unparseable value was reported as unset: {typed}"
        );

        let cfg = config_with(&[("LATITUDE", "0"), ("LONGITUDE", "0")]);
        let zero = birdweather_location(&cli, Some(&cfg)).unwrap_err();
        assert!(zero.contains("both 0"), "{zero}");
        assert!(!zero.contains("not a number"), "{zero}");
    }

    #[test]
    fn a_command_line_position_beats_the_config_as_the_daemon_does() {
        let mut cli = default_cli();
        cli.latitude = Some(10.0);
        cli.longitude = Some(20.0);
        let cfg = config_with(&[("LATITUDE", "40.0"), ("LONGITUDE", "-74.0")]);
        assert_eq!(birdweather_location(&cli, Some(&cfg)), Ok((10.0, 20.0)));
    }
}
