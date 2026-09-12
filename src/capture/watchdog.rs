//! Operator-tunable timings for the capture supervisor (`G-9`).
//!
//! Every number the supervisor decides with — how often it looks, how long a
//! process may go without writing a segment before it is called stalled, how
//! fast the restart backoff grows and how loudly a source that stays down is
//! reported — was a `const`. Defensible defaults, but a station whose storage
//! is slow, or whose camera legitimately pauses, had no way to say so short of
//! rebuilding.
//!
//! # What is *not* here, and why
//!
//! **A maximum retry count.** Upstream's watchdog has one, and this station
//! deliberately does not: a field sensor that has been unreachable for six
//! hours must still be reachable on hour seven, and a supervisor that has given
//! up is a station that is silently not recording. Making it configurable would
//! be offering an operator a way to break exactly the property the supervisor
//! exists to provide, so it is a divergence rather than a missing knob.
//!
//! **The deadman timer.** "This station has stopped *detecting*" is a different
//! question from "this source has stopped *delivering audio*", and it already
//! has its own setting (`BIRDNET_DEADMAN_HOURS`). A microphone in an arctic
//! winter writes segments through hours of silence; the stall threshold below
//! is about segments, not about birds, so no amount of quiet trips it.
//!
//! **The flapping threshold and window.** Those live in `birdnet-core`
//! (`FLAP_THRESHOLD`, `FLAP_WINDOW`) because the web layer renders against them
//! too, and a per-station value would have to reach both. Left fixed rather
//! than half-wired.

use std::time::Duration;

use birdnet_core::config::Config;

/// The supervisor's timings, and how each was arrived at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WatchdogConfig {
    /// How often the supervisor reconciles every source.
    pub check_interval: Duration,
    /// How many segment-durations of no output make a live process stalled.
    pub stall_segments: u64,
    /// Floor under the stall threshold, so a station writing 3-second segments
    /// does not get a hair-triggered verdict.
    pub stall_floor: Duration,
    /// First retry delay after a source is found dead.
    pub backoff_base: Duration,
    /// Ceiling on the retry delay. A persistently broken source is still
    /// retried this often, forever.
    pub backoff_cap: Duration,
    /// How long a source may be unexpectedly down before the loud warning.
    pub down_warn_after: Duration,
    /// Cadence for repeating that warning.
    pub down_warn_every: Duration,
}

impl Default for WatchdogConfig {
    /// The values this supervisor has always used.
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(2),
            stall_segments: 4,
            stall_floor: Duration::from_secs(120),
            backoff_base: Duration::from_secs(2),
            backoff_cap: Duration::from_secs(60),
            down_warn_after: Duration::from_secs(120),
            down_warn_every: Duration::from_secs(300),
        }
    }
}

impl WatchdogConfig {
    /// The silent-stall threshold for a source writing `segment_secs`-long
    /// segments.
    ///
    /// Several consecutive segments must be overdue before a running process is
    /// called stalled — one slow write must not bounce a healthy source — with
    /// the floor covering short segments.
    #[must_use]
    pub(super) fn stall_after(&self, segment_secs: u32) -> Duration {
        Duration::from_secs(u64::from(segment_secs).saturating_mul(self.stall_segments))
            .max(self.stall_floor)
    }
}

/// One knob's two names and the bounds it is held to.
struct Knob {
    /// The environment variable.
    env: &'static str,
    /// The `birdnet.conf` key.
    ///
    /// Written out rather than derived from `env` by stripping the prefix, so
    /// that `tests/every_config_key_is_known.rs` — which finds config keys by
    /// scanning the source for literals — can see it. A key assembled at
    /// runtime is invisible to that scan, and the station would then call a
    /// real setting a typo.
    config_key: &'static str,
    /// Smallest accepted value.
    min: u64,
    /// Largest accepted value.
    max: u64,
}

/// A value the operator supplied that could not be used as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Adjustment {
    /// The variable that was adjusted.
    pub key: &'static str,
    /// What the operator asked for, verbatim.
    pub asked: String,
    /// What is in effect instead.
    pub used: u64,
    /// Why.
    pub reason: &'static str,
}

/// `BIRDNET_<env>` from the environment, else `<env without prefix>` from the
/// config file, clamped into the knob's range.
///
/// Out of range is clamped rather than refused: unlike a loudness target — where
/// a mistyped value would silently change every clip — every value here is a
/// timing, and the nearest usable one is what the operator was reaching for. A
/// value that is not a number at all falls back to the default, because there is
/// no nearest anything.
fn resolve(knob: &Knob, config: Option<&Config>, default: u64, out: &mut Vec<Adjustment>) -> u64 {
    let raw = std::env::var(knob.env)
        .ok()
        .or_else(|| config.and_then(|c| c.get(knob.config_key).map(str::to_owned)))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let Some(raw) = raw else {
        return default;
    };
    let Ok(value) = raw.parse::<u64>() else {
        out.push(Adjustment {
            key: knob.env,
            asked: raw,
            used: default,
            reason: "not a whole number of seconds",
        });
        return default;
    };
    if value < knob.min {
        out.push(Adjustment {
            key: knob.env,
            asked: raw,
            used: knob.min,
            reason: "below the smallest usable value",
        });
        return knob.min;
    }
    if value > knob.max {
        out.push(Adjustment {
            key: knob.env,
            asked: raw,
            used: knob.max,
            reason: "above the largest usable value",
        });
        return knob.max;
    }
    value
}

// The knobs. Each range is chosen so no setting inside it can stop the
// supervisor doing its job — the point of tuning is a station that behaves
// differently, not one that stops being supervised.

/// Reconcile cadence. Below a second the supervisor spends its time waking up;
/// above five minutes a dead source is dead for five minutes.
const CHECK: Knob = Knob {
    env: "BIRDNET_WATCHDOG_CHECK_SECS",
    config_key: "WATCHDOG_CHECK_SECS",
    min: 1,
    max: 300,
};

/// Segments of missing output before a live process is called stalled. One
/// would fire on a single slow write; sixty is most of an hour at 60 s
/// segments.
const STALL_SEGMENTS: Knob = Knob {
    env: "BIRDNET_WATCHDOG_STALL_SEGMENTS",
    config_key: "WATCHDOG_STALL_SEGMENTS",
    min: 2,
    max: 60,
};

/// Floor under the stall threshold.
const STALL_FLOOR: Knob = Knob {
    env: "BIRDNET_WATCHDOG_STALL_FLOOR_SECS",
    config_key: "WATCHDOG_STALL_FLOOR_SECS",
    min: 10,
    max: 3_600,
};

/// First retry delay.
const BACKOFF_BASE: Knob = Knob {
    env: "BIRDNET_WATCHDOG_BACKOFF_BASE_SECS",
    config_key: "WATCHDOG_BACKOFF_BASE_SECS",
    min: 1,
    max: 60,
};

/// Retry-delay ceiling. Never unbounded: an hour between attempts is already
/// an hour of lost audio after the source comes back.
const BACKOFF_CAP: Knob = Knob {
    env: "BIRDNET_WATCHDOG_BACKOFF_CAP_SECS",
    config_key: "WATCHDOG_BACKOFF_CAP_SECS",
    min: 2,
    max: 3_600,
};

/// How long down before the loud warning.
const DOWN_WARN_AFTER: Knob = Knob {
    env: "BIRDNET_WATCHDOG_DOWN_WARN_AFTER_SECS",
    config_key: "WATCHDOG_DOWN_WARN_AFTER_SECS",
    min: 10,
    max: 86_400,
};

/// How often to repeat it.
const DOWN_WARN_EVERY: Knob = Knob {
    env: "BIRDNET_WATCHDOG_DOWN_WARN_EVERY_SECS",
    config_key: "WATCHDOG_DOWN_WARN_EVERY_SECS",
    min: 30,
    max: 86_400,
};

// The two bounds that are about the *watchdog still working* rather than about
// a sensible range are asserted at compile time, so widening one is a build
// failure rather than a test failure: a reconcile cadence over five minutes
// means a dead source stays dead for that long, and an unbounded retry ceiling
// is a source that never comes back.
const _: () = assert!(CHECK.max <= 300);
const _: () = assert!(BACKOFF_CAP.max <= 3_600);

/// Every watchdog environment variable.
///
/// Read by `helpers::env_keys`, whose scan finds a station's readable variables
/// by looking for `env::var("…")` literals and so cannot see a name that
/// arrives from this table — and by the gate that checks each is documented in
/// `.env.example`.
pub const WATCHDOG_ENV_KEYS: &[&str] = &[
    CHECK.env,
    STALL_SEGMENTS.env,
    STALL_FLOOR.env,
    BACKOFF_BASE.env,
    BACKOFF_CAP.env,
    DOWN_WARN_AFTER.env,
    DOWN_WARN_EVERY.env,
];

/// Read the watchdog configuration, reporting anything that had to be adjusted.
pub(super) fn from_config(config: Option<&Config>) -> (WatchdogConfig, Vec<Adjustment>) {
    let d = WatchdogConfig::default();
    let mut adjusted = Vec::new();
    let secs = |n: u64| Duration::from_secs(n);
    let cfg = WatchdogConfig {
        check_interval: secs(resolve(
            &CHECK,
            config,
            d.check_interval.as_secs(),
            &mut adjusted,
        )),
        stall_segments: resolve(&STALL_SEGMENTS, config, d.stall_segments, &mut adjusted),
        stall_floor: secs(resolve(
            &STALL_FLOOR,
            config,
            d.stall_floor.as_secs(),
            &mut adjusted,
        )),
        backoff_base: secs(resolve(
            &BACKOFF_BASE,
            config,
            d.backoff_base.as_secs(),
            &mut adjusted,
        )),
        backoff_cap: secs(resolve(
            &BACKOFF_CAP,
            config,
            d.backoff_cap.as_secs(),
            &mut adjusted,
        )),
        down_warn_after: secs(resolve(
            &DOWN_WARN_AFTER,
            config,
            d.down_warn_after.as_secs(),
            &mut adjusted,
        )),
        down_warn_every: secs(resolve(
            &DOWN_WARN_EVERY,
            config,
            d.down_warn_every.as_secs(),
            &mut adjusted,
        )),
    };

    // A cap below the base would make the "doubling" shrink the delay, which is
    // not a configuration anybody means. Raise the cap rather than lower the
    // base: the operator who set a long base wanted a slow retry.
    let cfg = if cfg.backoff_cap < cfg.backoff_base {
        adjusted.push(Adjustment {
            key: BACKOFF_CAP.env,
            asked: cfg.backoff_cap.as_secs().to_string(),
            used: cfg.backoff_base.as_secs(),
            reason: "a retry ceiling below the first delay would make the backoff shrink",
        });
        WatchdogConfig {
            backoff_cap: cfg.backoff_base,
            ..cfg
        }
    } else {
        cfg
    };

    (cfg, adjusted)
}

#[cfg(test)]
mod tests {
    use super::{
        Adjustment, BACKOFF_BASE, BACKOFF_CAP, CHECK, Knob, STALL_FLOOR, STALL_SEGMENTS,
        WATCHDOG_ENV_KEYS, WatchdogConfig, from_config, resolve,
    };
    use birdnet_core::config::Config;
    use std::time::Duration;

    fn cfg_of(pairs: &[(&str, &str)]) -> Config {
        let mut c = Config::empty();
        for (k, v) in pairs {
            c.set(*k, *v);
        }
        c
    }

    /// A station that configures nothing gets exactly the timings it had
    /// before this existed.
    ///
    /// The whole feature is worthless if it changes the default behaviour, and
    /// this is the assertion that says it does not.
    #[test]
    fn an_unconfigured_station_keeps_the_timings_it_always_had() {
        let (cfg, adjusted) = from_config(None);
        assert_eq!(cfg, WatchdogConfig::default());
        assert!(adjusted.is_empty(), "{adjusted:?}");

        assert_eq!(cfg.check_interval, Duration::from_secs(2));
        assert_eq!(cfg.stall_segments, 4);
        assert_eq!(cfg.stall_floor, Duration::from_secs(120));
        assert_eq!(cfg.backoff_base, Duration::from_secs(2));
        assert_eq!(cfg.backoff_cap, Duration::from_secs(60));
        assert_eq!(cfg.down_warn_after, Duration::from_secs(120));
        assert_eq!(cfg.down_warn_every, Duration::from_secs(300));
    }

    /// A value in the config file reaches the supervisor.
    #[test]
    fn a_configured_value_is_used() {
        let config = cfg_of(&[
            ("WATCHDOG_STALL_SEGMENTS", "8"),
            ("WATCHDOG_DOWN_WARN_EVERY_SECS", "1800"),
        ]);
        let (cfg, adjusted) = from_config(Some(&config));
        assert_eq!(cfg.stall_segments, 8);
        assert_eq!(cfg.down_warn_every, Duration::from_secs(1800));
        assert!(adjusted.is_empty(), "{adjusted:?}");

        // And the untouched knobs keep their defaults, so configuring one thing
        // does not quietly reset the rest.
        assert_eq!(cfg.check_interval, WatchdogConfig::default().check_interval);
    }

    /// Out of range is clamped, and the operator is told.
    ///
    /// Silently accepting `0` for the reconcile cadence would spin the
    /// supervisor; silently ignoring it would leave a station running on
    /// timings its config file does not describe. Clamp, and say so.
    #[test]
    fn an_out_of_range_value_is_clamped_and_reported() {
        let config = cfg_of(&[("WATCHDOG_CHECK_SECS", "0")]);
        let (cfg, adjusted) = from_config(Some(&config));
        assert_eq!(cfg.check_interval, Duration::from_secs(CHECK.min));
        assert_eq!(
            adjusted,
            vec![Adjustment {
                key: CHECK.env,
                asked: "0".to_owned(),
                used: CHECK.min,
                reason: "below the smallest usable value",
            }]
        );

        let config = cfg_of(&[("WATCHDOG_CHECK_SECS", "99999")]);
        let (cfg, adjusted) = from_config(Some(&config));
        assert_eq!(cfg.check_interval, Duration::from_secs(CHECK.max));
        assert_eq!(adjusted.len(), 1);
        assert_eq!(adjusted[0].used, CHECK.max);
    }

    /// A value that is not a number falls back to the default, reported.
    ///
    /// There is no nearest usable value for `"soon"`, so clamping has nothing
    /// to clamp to.
    #[test]
    fn a_value_that_is_not_a_number_falls_back_and_is_reported() {
        let config = cfg_of(&[("WATCHDOG_BACKOFF_BASE_SECS", "soon")]);
        let (cfg, adjusted) = from_config(Some(&config));
        assert_eq!(cfg.backoff_base, WatchdogConfig::default().backoff_base);
        assert_eq!(adjusted.len(), 1);
        assert_eq!(adjusted[0].reason, "not a whole number of seconds");
    }

    /// Blank is "not configured", not zero.
    ///
    /// `docker compose` interpolates `${BIRDNET_WATCHDOG_CHECK_SECS:-}` whether
    /// or not the operator set it, so a blank must not be read as a value —
    /// and must not be reported as an adjustment either, or every container
    /// station would log seven warnings at every start.
    #[test]
    fn a_blank_value_is_not_configured() {
        let config = cfg_of(&[("WATCHDOG_CHECK_SECS", "  ")]);
        let (cfg, adjusted) = from_config(Some(&config));
        assert_eq!(cfg, WatchdogConfig::default());
        assert!(adjusted.is_empty(), "{adjusted:?}");
    }

    /// A retry ceiling below the first delay would make the backoff shrink as
    /// a source kept failing, which is nobody's intent.
    #[test]
    fn a_ceiling_below_the_base_is_raised_to_it() {
        let config = cfg_of(&[
            ("WATCHDOG_BACKOFF_BASE_SECS", "30"),
            ("WATCHDOG_BACKOFF_CAP_SECS", "10"),
        ]);
        let (cfg, adjusted) = from_config(Some(&config));
        assert_eq!(cfg.backoff_base, Duration::from_secs(30));
        assert_eq!(
            cfg.backoff_cap,
            Duration::from_secs(30),
            "the ceiling must be raised to the base, not the base lowered: an operator \
             who set a long first delay wanted a slow retry"
        );
        assert!(
            adjusted.iter().any(|a| a.key == BACKOFF_CAP.env),
            "{adjusted:?}"
        );
    }

    /// The stall threshold is several segments, with a floor.
    #[test]
    fn the_stall_threshold_scales_with_the_segment_and_has_a_floor() {
        let cfg = WatchdogConfig::default();
        // 60 s segments: four of them is well past the floor.
        assert_eq!(cfg.stall_after(60), Duration::from_secs(240));
        // 3 s segments: four of them is 12 s, so the floor decides.
        assert_eq!(cfg.stall_after(3), cfg.stall_floor);

        // And the operator's segment count is what scales it.
        let patient = WatchdogConfig {
            stall_segments: 10,
            ..cfg
        };
        assert_eq!(patient.stall_after(60), Duration::from_secs(600));
    }

    /// Every knob is documented in `.env.example`.
    ///
    /// A tuning knob nobody can discover is a constant with extra steps, and
    /// `helpers::env_keys` only checks variables read through `env::var` with a
    /// literal name — these are read through a table, so that scan cannot see
    /// them and this is what covers them instead.
    #[test]
    fn every_watchdog_knob_is_documented() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env.example");
        let text = std::fs::read_to_string(path).expect("read .env.example");
        for key in WATCHDOG_ENV_KEYS {
            assert!(
                text.contains(&format!("{key}=")),
                "{key} is a watchdog setting with no line in .env.example"
            );
        }
    }

    /// No knob's range can hold a value that stops the supervisor supervising.
    ///
    /// The point of tuning is a station that behaves differently, not one that
    /// has turned its watchdog off. Written as a property over the table so a
    /// knob added later is covered without anyone remembering to.
    #[test]
    fn no_knobs_range_can_disable_the_watchdog() {
        for knob in [
            &CHECK,
            &STALL_SEGMENTS,
            &STALL_FLOOR,
            &BACKOFF_BASE,
            &BACKOFF_CAP,
        ] {
            assert!(knob.min >= 1, "{} can be set to zero", knob.env);
            assert!(knob.min <= knob.max, "{} has an empty range", knob.env);
        }
    }

    /// The resolver reads a knob's value out of the config file under the
    /// name without the `BIRDNET_` prefix.
    ///
    /// The *other* half of the precedence — the environment winning over the
    /// file — is not exercised here and deliberately so: setting an environment
    /// variable needs `std::env::set_var`, which is `unsafe`, and this
    /// workspace forbids `unsafe` outright. The precedence is one `or_else` in
    /// `resolve` and is the same shape every other input in this binary uses.
    #[test]
    fn a_knob_is_read_from_the_config_file_without_the_prefix() {
        let knob = Knob {
            env: "BIRDNET_WATCHDOG_CHECK_SECS",
            config_key: "WATCHDOG_CHECK_SECS",
            min: 1,
            max: 100,
        };
        let mut adjusted = Vec::new();
        let config = cfg_of(&[("WATCHDOG_CHECK_SECS", "7")]);
        assert_eq!(resolve(&knob, Some(&config), 1, &mut adjusted), 7);

        // The prefixed name is the environment's, not the file's: a config file
        // written with `BIRDNET_` on the front is a mistake worth not honouring
        // silently, because it would look configured and do nothing.
        let prefixed = cfg_of(&[("BIRDNET_WATCHDOG_CHECK_SECS", "7")]);
        assert_eq!(
            resolve(&knob, Some(&prefixed), 1, &mut adjusted),
            1,
            "the file key is the unprefixed one"
        );

        // And the table's two names really are the same knob written twice —
        // a mismatched pair would give an operator a variable and a config key
        // that configure different things.
        for k in [
            &CHECK,
            &STALL_SEGMENTS,
            &STALL_FLOOR,
            &BACKOFF_BASE,
            &BACKOFF_CAP,
        ] {
            assert_eq!(
                k.env.strip_prefix("BIRDNET_"),
                Some(k.config_key),
                "{} and {} are not the same knob",
                k.env,
                k.config_key
            );
        }
    }
}
