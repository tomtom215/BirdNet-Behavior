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

/// The upload bar from its config value.
///
/// Unset or empty is "none" (`None`): a station that has not asked for a bar
/// uploads everything it records. A value that is not usable is an `Err`
/// naming the key. Out-of-range is refused rather than clamped: `75` typed for
/// 75 % clamps to 1 and would stop every upload. The caller then uploads with
/// **no** bar, which is the setting's own "off", and says so.
///
/// Independent of [`birdweather_accuracy`]: a bad value here must never decide
/// what happens to the privacy setting (it did, once: see that function).
///
/// # Errors
///
/// A sentence naming the key and what is wrong with its value.
fn birdweather_threshold(threshold: Option<&str>) -> Result<Option<f32>, String> {
    let Some(text) = threshold.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let v = birdnet_core::config::locale::parse_decimal(text)
        .map_err(|_| format!("BIRDWEATHER_THRESHOLD={text:?} is not a number between 0 and 1"))?;
    if !(0.0..=1.0).contains(&v) {
        return Err(format!(
            "BIRDWEATHER_THRESHOLD={text:?} is outside 0 to 1 (for 75 %, write 0.75)"
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(Some(v as f32).filter(|c| *c > 0.0))
}

/// The blur radius in metres from its config value; `0.0` for unset, blank or
/// `0` (no blur, which is the setting's own default).
///
/// A value that is set and not usable is an `Err`, and **the caller must not
/// build the client on it**. The setting is a privacy one: it keeps the
/// station's address off a public map, and a station that typed `5km` or
/// `60000` has said it wants that. Falling back to `0.0` ("exact") is the one
/// outcome that contradicts what was asked, and it is what an earlier version
/// did, because a bad *threshold* in the same call took the accuracy down with
/// it. Clamping to the largest radius was considered for the too-big case and
/// rejected: it is a guess about what was meant, and an unparseable value has
/// no such guess, so one rule (refuse, say so, upload nothing) covers both.
///
/// # Errors
///
/// A sentence naming the key and what is wrong with its value.
fn birdweather_accuracy(accuracy: Option<&str>) -> Result<f64, String> {
    let Some(text) = accuracy.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(0.0);
    };
    let v = birdnet_core::config::locale::parse_decimal(text)
        .map_err(|_| format!("BIRDWEATHER_LOCATION_ACCURACY={text:?} is not a number of metres"))?;
    let max = birdnet_integrations::birdweather::MAX_LOCATION_ACCURACY_M;
    if !(0.0..=max).contains(&v) {
        return Err(format!(
            "BIRDWEATHER_LOCATION_ACCURACY={text:?} is outside 0 to {max} metres"
        ));
    }
    Ok(v)
}

/// What the station keeps beside its database to remember its blur offset.
pub const BLUR_SEED_FILE: &str = "birdweather-blur-seed";

/// The station's blur seed: read from `dir`, or drawn and written there once.
///
/// The seed is what makes the posted position the same on every upload and on
/// every restart (see `birdnet_integrations::birdweather::fuzz_location` for
/// what that hides). Written atomically with mode 0600. A file that exists but
/// is not 64 hex characters is an error and is **not** regenerated: that would
/// silently move the station on the map and hand an observer a second sample.
///
/// # Errors
///
/// The file cannot be read or written, is malformed, or the system generator
/// failed.
pub fn load_or_create_blur_seed(
    dir: &std::path::Path,
) -> std::io::Result<[u8; birdnet_integrations::birdweather::BLUR_SEED_LEN]> {
    use birdnet_integrations::birdweather::BLUR_SEED_LEN;
    use std::io::{Error, ErrorKind, Write as _};
    let path = dir.join(BLUR_SEED_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let hex = text.trim();
            let decoded = (hex.len() == BLUR_SEED_LEN * 2 && hex.is_ascii())
                .then(|| {
                    (0..BLUR_SEED_LEN)
                        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
                        .collect::<Option<Vec<u8>>>()
                })
                .flatten()
                .and_then(|v| <[u8; BLUR_SEED_LEN]>::try_from(v).ok());
            return decoded.ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "{} is not a {}-character hex seed",
                        path.display(),
                        BLUR_SEED_LEN * 2
                    ),
                )
            });
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let seed = birdnet_integrations::birdweather::random_blur_seed()
        .ok_or_else(|| Error::other("the system random generator failed"))?;
    let hex = seed.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    });
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{BLUR_SEED_FILE}.{}.tmp", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(hex.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(seed)
}

/// Give a blurring client the station's persistent seed from `dir`.
///
/// A client without a blur radius is returned unchanged. When the seed file
/// cannot be used the client keeps the random seed it drew for this process: the
/// position is still blurred and still the same on every post until a restart,
/// and the log says the offset will change then.
#[must_use]
pub fn with_persisted_blur_seed(
    client: birdnet_integrations::birdweather::Client,
    dir: Option<&std::path::Path>,
) -> birdnet_integrations::birdweather::Client {
    if client.location_accuracy() <= 0.0 {
        return client;
    }
    let Some(dir) = dir else {
        return client;
    };
    match load_or_create_blur_seed(dir) {
        Ok(seed) => client.with_blur_seed(seed),
        Err(e) => {
            tracing::warn!(
                error = %e,
                dir = %dir.display(),
                "BirdWeather: could not keep the position-blur seed; the blurred position stays \
                 the same until the next restart and then changes"
            );
            client
        }
    }
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

    // The two BirdWeather-only settings (P-2), resolved independently. Read as
    // literal `config.get`s, for `tests/every_config_key_is_known.rs`.
    //
    // A bad blur radius refuses the client: it is a privacy setting, and the
    // only other outcomes are "exact" (what it was written to prevent) or a
    // guess at what was meant. A bad threshold only loses the threshold.
    let accuracy_m =
        match birdweather_accuracy(config.and_then(|c| c.get("BIRDWEATHER_LOCATION_ACCURACY"))) {
            Ok(metres) => metres,
            Err(why) => {
                tracing::error!(
                    "BirdWeather uploads are OFF: {why}. Every upload would otherwise carry the \
                 station's exact position onto a public map. Set the blur to a number of \
                 metres from 0 to {} (0 or empty for no blur) and restart",
                    birdnet_integrations::birdweather::MAX_LOCATION_ACCURACY_M
                );
                return None;
            }
        };
    let min_confidence =
        match birdweather_threshold(config.and_then(|c| c.get("BIRDWEATHER_THRESHOLD"))) {
            Ok(floor) => floor,
            Err(why) => {
                tracing::warn!(
                    "{why}; BirdWeather uploads carry no extra confidence bar until it is fixed"
                );
                None
            }
        };

    match birdnet_integrations::birdweather::Client::new(&token, lat, lon) {
        Ok(client) => {
            let client = match min_confidence {
                Some(c) => client.with_min_confidence(c),
                None => client,
            };
            let client = client.with_location_accuracy(accuracy_m);
            if let Some(floor) = client.min_confidence() {
                tracing::info!(
                    floor,
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
        use super::{birdweather_accuracy as acc, birdweather_threshold as thr};
        assert_eq!(thr(None), Ok(None));
        assert_eq!(acc(None), Ok(0.0));
        assert_eq!(thr(Some("")), Ok(None));
        assert_eq!(acc(Some("  ")), Ok(0.0));
        assert_eq!(thr(Some("0")), Ok(None), "0 is none");
        assert_eq!(acc(Some("0")), Ok(0.0), "0 is none");
        assert_eq!(thr(Some("0,85")), Ok(Some(0.85)));
        assert_eq!(acc(Some("250")), Ok(250.0));
        assert!(thr(Some("75")).unwrap_err().contains("0.75"));
        assert!(thr(Some("-0.1")).is_err());
        assert!(thr(Some("high")).is_err());
        assert!(acc(Some("-5")).is_err());
        assert!(acc(Some("50001")).is_err());
        assert!(acc(Some("1.5km")).is_err());
        assert!(acc(Some("50000")).is_ok(), "the cap itself is allowed");
    }

    /// The validator (in `birdnet-core`, which cannot see the integrations
    /// crate) and the client must agree on the widest blur.
    #[test]
    fn the_validator_and_the_client_agree_on_the_widest_blur() {
        assert!(
            (birdnet_core::config::validate::BIRDWEATHER_MAX_ACCURACY_M
                - birdnet_integrations::birdweather::MAX_LOCATION_ACCURACY_M)
                .abs()
                < f64::EPSILON
        );
    }

    fn client_with(extra: &[(&str, &str)]) -> Option<birdnet_integrations::birdweather::Client> {
        let mut entries = vec![
            ("BIRDWEATHER_TOKEN", "tok"),
            ("LATITUDE", "51.48"),
            ("LONGITUDE", "-0.13"),
        ];
        entries.extend_from_slice(extra);
        create_birdweather_client(&default_cli(), Some(&config_with(&entries)))
    }

    /// H1. A bad `BIRDWEATHER_THRESHOLD` used to discard a valid
    /// `BIRDWEATHER_LOCATION_ACCURACY` in the same call, and the station posted
    /// its exact position. Each setting is resolved on its own now.
    #[test]
    fn a_bad_threshold_does_not_take_a_good_blur_with_it() {
        let c = client_with(&[
            ("BIRDWEATHER_THRESHOLD", "85"),
            ("BIRDWEATHER_LOCATION_ACCURACY", "500"),
        ])
        .expect("a bad threshold alone does not stop uploads");
        assert!((c.location_accuracy() - 500.0).abs() < f64::EPSILON);
        assert_eq!(c.min_confidence(), None, "the bad bar is dropped");
        let p = c.post_location().expect("seeded");
        assert_ne!(p, (51.48, -0.13), "the exact position was posted");
        // The counterpart: a good threshold with a good blur keeps both.
        let both = client_with(&[
            ("BIRDWEATHER_THRESHOLD", "0.85"),
            ("BIRDWEATHER_LOCATION_ACCURACY", "500"),
        ])
        .unwrap();
        assert_eq!(both.min_confidence(), Some(0.85));
        assert!((both.location_accuracy() - 500.0).abs() < f64::EPSILON);
    }

    /// H1. A radius that is set and unusable never becomes "exact": no client,
    /// so nothing is posted at all.
    #[test]
    fn a_bad_blur_radius_posts_nothing_rather_than_the_exact_position() {
        for bad in ["60000", "5km", "-5", "nan", "1e9"] {
            assert!(
                client_with(&[("BIRDWEATHER_LOCATION_ACCURACY", bad)]).is_none(),
                "{bad:?} built a client"
            );
            assert!(
                client_with(&[
                    ("BIRDWEATHER_THRESHOLD", "0.9"),
                    ("BIRDWEATHER_LOCATION_ACCURACY", bad)
                ])
                .is_none(),
                "{bad:?} with a good threshold built a client"
            );
        }
        // Counterparts: no blur at all is a valid choice, and so is 0.
        let exact = client_with(&[]).expect("no blur set");
        assert_eq!(exact.post_location(), Some((51.48, -0.13)));
        assert!(client_with(&[("BIRDWEATHER_LOCATION_ACCURACY", "0")]).is_some());
        assert!(client_with(&[("BIRDWEATHER_LOCATION_ACCURACY", "50000")]).is_some());
    }

    #[test]
    fn the_blur_seed_is_kept_beside_the_database_and_is_not_replaced() {
        use super::{BLUR_SEED_FILE, load_or_create_blur_seed};
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create_blur_seed(dir.path()).unwrap();
        assert_eq!(load_or_create_blur_seed(dir.path()).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(BLUR_SEED_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A damaged file is an error, not a quiet new seed that would move the
        // station on the map.
        std::fs::write(dir.path().join(BLUR_SEED_FILE), "not hex").unwrap();
        assert!(load_or_create_blur_seed(dir.path()).is_err());
    }

    #[test]
    fn a_restarted_station_posts_the_same_blurred_position() {
        use super::with_persisted_blur_seed;
        let dir = tempfile::tempdir().unwrap();
        let post = || {
            with_persisted_blur_seed(
                client_with(&[("BIRDWEATHER_LOCATION_ACCURACY", "800")]).unwrap(),
                Some(dir.path()),
            )
            .post_location()
            .unwrap()
        };
        let before = post();
        assert_eq!(before, post(), "the offset changed across a restart");
        // The counterpart: without a kept seed, two starts disagree.
        let unkept = || {
            client_with(&[("BIRDWEATHER_LOCATION_ACCURACY", "800")])
                .unwrap()
                .post_location()
                .unwrap()
        };
        assert_ne!(unkept(), unkept());
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
