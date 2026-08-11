//! Recording-schedule parsing and the hand-rolled UTC clock.
//!
//! Turns the `--recording-schedule` CLI flag (`all-day` / `solar` /
//! `fixed:HH:MM-HH:MM`) into a [`ScheduleConfig`], converts Unix time to a
//! civil date ([`civil_from_unix_secs`]) for evaluating the schedule gate, and
//! reports whether the clock looks NTP-synced ([`secs_look_synced`]). Acting on
//! the gate (starting/stopping capture) is the supervisor's job; this module is
//! pure parsing + clock.

use birdnet_scheduler::{Location, RecordingWindow, ScheduleConfig};

use crate::cli::Cli;
use crate::helpers::resolve;

/// Parse a schedule string from CLI into a `ScheduleConfig`.
///
/// Supported formats:
/// - `"all-day"` — no restriction
/// - `"solar"` — sunrise-to-sunset (requires lat/lon and night-inhibit)
/// - `"fixed:HH:MM-HH:MM"` — fixed daily window
pub(super) fn parse_schedule_config(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> ScheduleConfig {
    let location = resolve_location(cli, config);
    let (pre_sunrise_offset_min, post_sunset_offset_min) = resolve_twilight_offsets(cli, config);

    // Resolved from the config, not read off the CLI. `--recording-schedule`
    // carries a clap `default_value` of "all-day", so reading `cli` directly
    // meant the default always won and **`RECORDING_SCHEDULE` in
    // `birdnet.conf` was ignored outright**: a station configured for `solar`
    // recorded around the clock, and a `fixed:HH:MM-HH:MM` window never
    // applied. Nothing contradicted it — `birdnet_core::config::validate`
    // validates the key, and `--doctor`'s clock check reads it from the config
    // to warn that a fixed window is evaluated in UTC, so the diagnostic
    // reported on a schedule the runtime never used.
    //
    // Its sibling `resolve_twilight_offsets` below has always gone through
    // `resolve::setting`; this line was the one that did not.
    let schedule_str = resolve::setting_str(
        cli,
        "recording_schedule",
        &cli.recording_schedule,
        config,
        "RECORDING_SCHEDULE",
    )
    .trim()
    .to_lowercase();

    if schedule_str == "solar" {
        return ScheduleConfig {
            location,
            pre_sunrise_offset_min,
            post_sunset_offset_min,
            night_inhibit: true,
            fixed_window: None,
        };
    }

    if let Some(fixed_spec) = schedule_str.strip_prefix("fixed:") {
        if let Some(window) = parse_fixed_window(fixed_spec) {
            return ScheduleConfig {
                location: None,
                pre_sunrise_offset_min: 0,
                post_sunset_offset_min: 0,
                night_inhibit: false,
                fixed_window: Some(window),
            };
        }
        tracing::warn!(spec = %fixed_spec, "invalid fixed schedule, falling back to all-day");
    }

    // "all-day" or unrecognized — but respect the night-inhibit setting.
    ScheduleConfig {
        location,
        pre_sunrise_offset_min,
        post_sunset_offset_min,
        night_inhibit: resolve::setting_bool(
            cli,
            "night_inhibit",
            cli.night_inhibit,
            config,
            "NIGHT_INHIBIT",
        ),
        fixed_window: None,
    }
}

/// Resolve the pre-sunrise and post-sunset offsets, in minutes.
///
/// `ScheduleConfig` has always carried the two ends separately, and the settings
/// page has always offered two fields — but the runtime only had one
/// `--twilight-offset` and wrote it to both, so the two could never differ no
/// matter which surface the operator used. Each end now resolves on its own:
///
/// 1. its own flag (`--pre-sunrise-offset` / `--post-sunset-offset`), then
/// 2. its own config key — which is where `/admin/settings` lands via the
///    settings overlay — then
/// 3. `--twilight-offset`, which keeps the existing symmetric behaviour for
///    every station that never sets either end explicitly.
fn resolve_twilight_offsets(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> (u32, u32) {
    let both = resolve::setting::<u32>(
        cli,
        "twilight_offset",
        cli.twilight_offset,
        config,
        "TWILIGHT_OFFSET",
    );
    // These flags have no clap default, so `Some` already means the operator
    // supplied one — no explicit-source check needed.
    let end = |flag: Option<u32>, config_key: &str| -> u32 {
        flag.or_else(|| config.and_then(|c| c.get_parsed::<u32>(config_key).ok()))
            .unwrap_or(both)
    };
    (
        end(cli.pre_sunrise_offset, "PRE_SUNRISE_OFFSET"),
        end(cli.post_sunset_offset, "POST_SUNSET_OFFSET"),
    )
}

/// Parse `"HH:MM-HH:MM"` into a `RecordingWindow`.
fn parse_fixed_window(spec: &str) -> Option<RecordingWindow> {
    let parts: Vec<&str> = spec.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let start = parse_hhmm(parts[0])?;
    let end = parse_hhmm(parts[1])?;
    RecordingWindow::fixed(start, end).ok()
}

/// Parse `"HH:MM"` into minutes since midnight.
///
/// `pub(super)` so the per-source quiet-window parsing in [`super`] can reuse
/// the same validated `HH:MM` → minutes conversion the schedule uses.
pub(super) fn parse_hhmm(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    if h >= 24 || m >= 60 {
        return None;
    }
    Some(h * 60 + m)
}

/// Resolve latitude/longitude from CLI flags or config.
fn resolve_location(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> Option<Location> {
    let lat = cli
        .latitude
        .or_else(|| config?.get_parsed::<f64>("LATITUDE").ok())?;
    let lon = cli
        .longitude
        .or_else(|| config?.get_parsed::<f64>("LONGITUDE").ok())?;
    Location::new(lat, lon).ok()
}

/// Unix-time floor below which the system clock is treated as unsynced.
///
/// A Raspberry Pi has no battery-backed RTC, so before NTP syncs it commonly
/// reports the epoch or a stale build-time value. `2024-01-01T00:00:00Z` is
/// safely before this project's deployment era yet far above any unset-clock
/// reading, so a value below it means "time isn't trustworthy yet".
const CLOCK_SYNCED_FLOOR_SECS: u64 = 1_704_067_200;

/// Whether the system clock (as a Unix timestamp in seconds) looks NTP-synced.
///
/// Pure so the boundary is unit-testable. A `false` result tells callers to
/// fail *open* — keep recording rather than trust a bogus date for solar
/// scheduling — until the clock becomes plausible.
pub(super) const fn secs_look_synced(secs: u64) -> bool {
    secs >= CLOCK_SYNCED_FLOOR_SECS
}

/// Convert a Unix timestamp (seconds since 1970-01-01 UTC) into
/// `(year, month, day, minutes_since_midnight)`.
///
/// Pure (no clock access) so the date arithmetic can be property-tested
/// directly. Implements Howard Hinnant's `civil_from_days`:
/// <http://howardhinnant.github.io/date_algorithms.html>.
pub(super) fn civil_from_unix_secs(secs: u64) -> (u32, u32, u32, u32) {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    #[allow(clippy::cast_possible_truncation)]
    let minutes = (time_of_day / 60) as u32;

    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let z = days as i64 + 719_468;
    // `z` is always >= 0 here — a u64 timestamp cannot predate the epoch — so
    // Hinnant's proleptic negative-era branch is unreachable; divide directly.
    let era = z / 146_097;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (y as u32, m, d, minutes)
}

#[cfg(test)]
mod tests {
    use super::{
        CLOCK_SYNCED_FLOOR_SECS, civil_from_unix_secs, parse_fixed_window, parse_hhmm,
        parse_schedule_config, resolve_location, secs_look_synced,
    };
    use crate::cli::Cli;
    use crate::helpers::test_support::{cli_with_explicit, config_with, default_cli};
    use birdnet_scheduler::traits::RecordingGate;
    use clap::Parser;
    use proptest::prelude::*;

    // ── the two twilight ends resolve independently ─────────────────────
    //
    // `ScheduleConfig` has always had separate pre/post fields and the settings
    // page has always shown two inputs, but the runtime wrote one
    // `--twilight-offset` into both, so no surface could make dawn and dusk
    // differ. These pin that they now can.

    #[test]
    fn twilight_offsets_can_differ_via_settings() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = config_with(&[("PRE_SUNRISE_OFFSET", "60"), ("POST_SUNSET_OFFSET", "15")]);
        let sched = parse_schedule_config(&cli, Some(&cfg));
        assert_eq!(sched.pre_sunrise_offset_min, 60);
        assert_eq!(sched.post_sunset_offset_min, 15);
    }

    #[test]
    fn twilight_offsets_can_differ_via_flags() {
        let cli = Cli::parse_from([
            "birdnet-behavior",
            "--pre-sunrise-offset",
            "45",
            "--post-sunset-offset",
            "10",
        ]);
        let sched = parse_schedule_config(&cli, None);
        assert_eq!(sched.pre_sunrise_offset_min, 45);
        assert_eq!(sched.post_sunset_offset_min, 10);
    }

    #[test]
    fn one_end_set_leaves_the_other_on_the_shared_twilight_offset() {
        let cli = Cli::parse_from(["birdnet-behavior", "--pre-sunrise-offset", "90"]);
        let sched = parse_schedule_config(&cli, None);
        assert_eq!(sched.pre_sunrise_offset_min, 90);
        // Unset end keeps the documented --twilight-offset default.
        assert_eq!(sched.post_sunset_offset_min, 30);
    }

    #[test]
    fn a_specific_end_flag_beats_the_settings_for_that_end_only() {
        let cli = Cli::parse_from(["birdnet-behavior", "--pre-sunrise-offset", "5"]);
        let cfg = config_with(&[("PRE_SUNRISE_OFFSET", "60"), ("POST_SUNSET_OFFSET", "15")]);
        let sched = parse_schedule_config(&cli, Some(&cfg));
        assert_eq!(sched.pre_sunrise_offset_min, 5, "flag wins for its own end");
        assert_eq!(
            sched.post_sunset_offset_min, 15,
            "the untouched end still takes the settings value"
        );
    }

    #[test]
    fn solar_schedule_also_honours_the_split_offsets() {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.recording_schedule = "solar".to_owned();
        let cfg = config_with(&[
            ("LATITUDE", "51.5"),
            ("LONGITUDE", "-0.13"),
            ("PRE_SUNRISE_OFFSET", "75"),
            ("POST_SUNSET_OFFSET", "20"),
        ]);
        let sched = parse_schedule_config(&cli, Some(&cfg));
        assert_eq!(sched.pre_sunrise_offset_min, 75);
        assert_eq!(sched.post_sunset_offset_min, 20);
        assert!(sched.night_inhibit, "solar always inhibits at night");
    }

    // ── night inhibit reaches the schedule ──────────────────────────────

    #[test]
    fn night_inhibit_from_settings_reaches_the_schedule() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        let cfg = config_with(&[
            ("LATITUDE", "51.5"),
            ("LONGITUDE", "-0.13"),
            ("NIGHT_INHIBIT", "true"),
        ]);
        assert!(parse_schedule_config(&cli, Some(&cfg)).night_inhibit);
    }

    #[test]
    fn night_inhibit_defaults_off_without_a_setting() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        assert!(!parse_schedule_config(&cli, None).night_inhibit);
    }

    #[test]
    fn explicit_night_inhibit_flag_beats_a_false_setting() {
        let mut cli = cli_with_explicit(&["night_inhibit"]);
        cli.night_inhibit = true;
        let cfg = config_with(&[("NIGHT_INHIBIT", "false")]);
        assert!(parse_schedule_config(&cli, Some(&cfg)).night_inhibit);
    }

    /// A default `Cli` with the recording schedule overridden — the only
    /// field these schedule-parsing tests vary.
    fn cli_with_schedule(schedule: &str) -> Cli {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.recording_schedule = schedule.to_string();
        cli
    }

    // ---- parse_hhmm --------------------------------------------------------

    #[test]
    fn parse_hhmm_valid() {
        assert_eq!(parse_hhmm("06:00"), Some(360));
        assert_eq!(parse_hhmm("20:30"), Some(1230));
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
    }

    #[test]
    fn parse_hhmm_invalid() {
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
        assert_eq!(parse_hhmm("abc"), None);
        assert_eq!(parse_hhmm(""), None);
    }

    #[test]
    fn parse_hhmm_rejects_wrong_part_count() {
        assert_eq!(parse_hhmm("1:2:3"), None);
        assert_eq!(parse_hhmm("12"), None);
    }

    #[test]
    fn parse_hhmm_boundary_values() {
        // Pins the `>= 24` / `>= 60` guards at their exact boundaries.
        assert_eq!(parse_hhmm("23:00"), Some(1380));
        assert_eq!(parse_hhmm("00:59"), Some(59));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("00:60"), None);
    }

    // ---- parse_fixed_window ------------------------------------------------

    #[test]
    fn parse_fixed_window_valid() {
        let w = parse_fixed_window("06:00-20:00").unwrap();
        assert!(w.is_allowed(720)); // noon
        assert!(!w.is_allowed(300)); // 05:00
    }

    #[test]
    fn parse_fixed_window_invalid() {
        assert!(parse_fixed_window("06:00").is_none());
        assert!(parse_fixed_window("20:00-06:00").is_none());
        assert!(parse_fixed_window("").is_none());
    }

    // ---- parse_schedule_config ---------------------------------------------

    #[test]
    fn parse_schedule_all_day() {
        let cli = cli_with_schedule("all-day");
        let config = parse_schedule_config(&cli, None);
        assert!(config.fixed_window.is_none());
        assert!(!config.night_inhibit);
    }

    #[test]
    fn parse_schedule_solar() {
        let cli = cli_with_schedule("solar");
        let config = parse_schedule_config(&cli, None);
        assert!(config.night_inhibit);
        assert_eq!(config.pre_sunrise_offset_min, 30);
        assert_eq!(config.post_sunset_offset_min, 30);
        assert!(config.fixed_window.is_none());
    }

    #[test]
    fn parse_schedule_fixed() {
        let cli = cli_with_schedule("fixed:08:00-18:00");
        let config = parse_schedule_config(&cli, None);
        assert!(config.fixed_window.is_some());
        // The fixed branch must not also assert a solar night-inhibit.
        assert!(!config.night_inhibit);
    }

    #[test]
    fn parse_schedule_invalid_fixed_falls_back_to_all_day() {
        let cli = cli_with_schedule("fixed:99:99");
        let config = parse_schedule_config(&cli, None);
        assert!(config.fixed_window.is_none());
        assert!(!config.night_inhibit);
    }

    #[test]
    fn parse_schedule_respects_night_inhibit_flag() {
        let mut cli = cli_with_schedule("all-day");
        cli.night_inhibit = true;
        let config = parse_schedule_config(&cli, None);
        assert!(config.night_inhibit);
        assert!(config.fixed_window.is_none());
    }

    #[test]
    fn parse_schedule_carries_location() {
        let mut cli = cli_with_schedule("solar");
        cli.latitude = Some(40.0);
        cli.longitude = Some(-105.0);
        let config = parse_schedule_config(&cli, None);
        assert!(config.location.is_some());
    }

    // ---- resolve_location --------------------------------------------------

    #[test]
    fn resolve_location_from_cli() {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.latitude = Some(40.0);
        cli.longitude = Some(-105.0);
        assert!(resolve_location(&cli, None).is_some());
    }

    #[test]
    fn resolve_location_requires_both_coords() {
        let mut cli = Cli::parse_from(["birdnet-behavior"]);
        cli.latitude = Some(40.0); // longitude missing
        assert!(resolve_location(&cli, None).is_none());
    }

    #[test]
    fn resolve_location_none_without_coords() {
        let cli = Cli::parse_from(["birdnet-behavior"]);
        assert!(resolve_location(&cli, None).is_none());
    }

    // ---- secs_look_synced / civil_from_unix_secs ---------------------------

    #[test]
    fn clock_below_floor_is_unsynced() {
        assert!(!secs_look_synced(0)); // epoch — the classic unset-RTC reading
        assert!(!secs_look_synced(CLOCK_SYNCED_FLOOR_SECS - 1));
    }

    #[test]
    fn clock_at_or_above_floor_is_synced() {
        assert!(secs_look_synced(CLOCK_SYNCED_FLOOR_SECS));
        assert!(secs_look_synced(1_900_000_000)); // year 2030
    }

    #[test]
    fn civil_epoch_is_1970() {
        assert_eq!(civil_from_unix_secs(0), (1970, 1, 1, 0));
    }

    #[test]
    fn civil_end_of_first_day() {
        // 1970-01-01 23:59 UTC.
        assert_eq!(
            civil_from_unix_secs(23 * 3600 + 59 * 60),
            (1970, 1, 1, 23 * 60 + 59)
        );
    }

    #[test]
    fn civil_known_timestamp() {
        // 1_700_000_000 == 2023-11-14T22:13:20Z (independently verifiable).
        assert_eq!(
            civil_from_unix_secs(1_700_000_000),
            (2023, 11, 14, 22 * 60 + 13)
        );
    }

    // ---- oracle: Hinnant's inverse, used to cross-check ---------------------

    fn is_leap(y: i64) -> bool {
        (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
    }

    fn days_in_month(y: i64, m: u32) -> u32 {
        match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap(y) => 29,
            2 => 28,
            other => panic!("invalid month {other}"),
        }
    }

    /// Days since 1970-01-01 for civil `(y, m, d)` — Hinnant's
    /// `days_from_civil`, the exact inverse of `civil_from_unix_secs`.
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = (if y >= 0 { y } else { y - 399 }) / 400;
        let yoe = y - era * 400; // [0, 399]
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
        era * 146_097 + doe - 719_468
    }

    #[test]
    fn civil_matches_oracle_every_day_for_centuries() {
        // Walk every calendar day from 1970 through 2200 and assert the
        // forward conversion matches an independently-advanced civil date.
        // This exercises every month length, every leap day (Feb 29), and
        // the century rule — 2100 is divisible by 4 but is NOT a leap year.
        let mut y = 1970i64;
        let mut m = 1u32;
        let mut d = 1u32;
        let end = days_from_civil(2201, 1, 1);
        let mut day = 0i64;
        while day < end {
            let secs = u64::try_from(day).expect("non-negative day") * 86_400 + 12 * 3600;
            let expected = (u32::try_from(y).expect("year fits u32"), m, d, 720u32);
            assert_eq!(civil_from_unix_secs(secs), expected, "at day offset {day}");

            day += 1;
            d += 1;
            if d > days_in_month(y, m) {
                d = 1;
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
            }
        }
    }

    // ── RECORDING_SCHEDULE reaches the runtime ──────────────────────────
    //
    // The station recorded around the clock whatever `birdnet.conf` said,
    // because this resolver read `cli.recording_schedule` — which carries a
    // clap default of "all-day", so the default always won. Every existing
    // test set `cli.recording_schedule` by hand, exercising only the CLI path,
    // which is why nothing caught it.

    #[test]
    fn config_schedule_solar_is_honoured() {
        let cli = default_cli();
        let cfg = config_with(&[
            ("RECORDING_SCHEDULE", "solar"),
            ("LATITUDE", "52.5"),
            ("LONGITUDE", "13.4"),
        ]);
        let sc = parse_schedule_config(&cli, Some(&cfg));
        assert!(
            sc.night_inhibit,
            "a solar schedule in the config must inhibit overnight recording"
        );
        assert!(sc.location.is_some(), "solar needs the station coordinates");
    }

    #[test]
    fn config_schedule_fixed_window_is_honoured() {
        let cli = default_cli();
        let cfg = config_with(&[("RECORDING_SCHEDULE", "fixed:06:00-20:00")]);
        let sc = parse_schedule_config(&cli, Some(&cfg));
        assert!(
            sc.fixed_window.is_some(),
            "a fixed window in the config must be applied, not silently dropped"
        );
    }

    /// Counter-test: the old behaviour, pinned. Reading the CLI field directly
    /// yields "all-day" for both configs above — the exact silent 24/7
    /// recording this fix removes.
    #[test]
    fn reading_the_cli_field_directly_would_ignore_the_config() {
        let cli = default_cli();
        assert_eq!(
            cli.recording_schedule, "all-day",
            "the clap default is what used to win over any configured schedule"
        );
    }

    #[test]
    fn an_explicit_cli_flag_still_beats_the_config() {
        let mut cli = cli_with_explicit(&["recording_schedule"]);
        cli.recording_schedule = "all-day".to_owned();
        let cfg = config_with(&[
            ("RECORDING_SCHEDULE", "solar"),
            ("LATITUDE", "52.5"),
            ("LONGITUDE", "13.4"),
        ]);
        let sc = parse_schedule_config(&cli, Some(&cfg));
        assert!(
            !sc.night_inhibit,
            "an operator passing --recording-schedule must override the config"
        );
    }

    #[test]
    fn a_blank_config_schedule_falls_through_to_the_default() {
        let cli = default_cli();
        let cfg = config_with(&[("RECORDING_SCHEDULE", "   ")]);
        let sc = parse_schedule_config(&cli, Some(&cfg));
        assert!(sc.fixed_window.is_none());
        assert!(!sc.night_inhibit, "blank must mean unset, not malformed");
    }

    proptest! {
        /// Round-trip: build a timestamp from a civil date via the oracle,
        /// then assert the forward conversion recovers it.
        #[test]
        fn civil_round_trips(
            y in 1970i64..=4000,
            m in 1u32..=12,
            d in 1u32..=28,
            secs_of_day in 0u64..86_400,
        ) {
            let base = u64::try_from(days_from_civil(y, i64::from(m), i64::from(d)))
                .expect("non-negative") * 86_400;
            let (ry, rm, rd, rmin) = civil_from_unix_secs(base + secs_of_day);
            prop_assert_eq!(ry, u32::try_from(y).expect("year fits u32"));
            prop_assert_eq!(rm, m);
            prop_assert_eq!(rd, d);
            prop_assert_eq!(rmin, u32::try_from(secs_of_day / 60).expect("minutes fit u32"));
        }
    }
}
