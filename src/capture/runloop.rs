//! The supervisor's background run loop and its per-tick schedule/clock gating.
//!
//! [`run_supervisor`] reconciles every capture source on a fixed cadence until
//! asked to stop, deciding each tick whether recording is allowed (the solar /
//! fixed-window schedule) and which sources are inside a quiet window — both
//! gated on the system clock looking trustworthy.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use birdnet_core::audio::capture::{CaptureManager, CaptureStatusHandle, LocalOffset};
use birdnet_scheduler::{DailySchedule, ScheduleClock, ScheduleConfig, SolarDay};
use birdnet_web::metrics::SharedMetrics;

use super::schedule;
use super::supervisor::{SolarMinutes, Supervisor};

/// How often the supervisor reconciles each source toward its desired state.
/// Short enough to notice a dead subprocess and resume after a scheduled
/// pause promptly; the per-source backoff timers (not this cadence) govern
/// restart spacing.
const SUPERVISE_TICK: Duration = Duration::from_secs(2);

/// The supervisor's background loop: reconcile every source on a fixed
/// cadence until asked to stop.
///
/// Each tick re-checks whether the system clock looks NTP-synced. A Raspberry
/// Pi has no battery-backed RTC, so at boot the clock can read the epoch (or a
/// stale value) until `systemd-timesyncd`/NTP catches up. While the clock is
/// untrustworthy we **fail open** — record continuously regardless of the
/// solar/fixed window — because trusting a bogus date could drop us into a
/// "night" window and silently lose a whole session. Normal scheduling
/// resumes automatically once the clock becomes plausible.
pub(super) fn run_supervisor(
    mut supervisor: Supervisor<CaptureManager>,
    schedule_config: &ScheduleConfig,
    metrics: &SharedMetrics,
    status: &CaptureStatusHandle,
    local_offset: &LocalOffset,
    stop: &AtomicBool,
) {
    tracing::info!("capture supervisor started");
    // Start from "synced" so an unsynced clock at boot trips the warning on
    // the very first tick.
    let mut clock_synced = true;
    while !stop.load(Ordering::Relaxed) {
        // Republish the station's UTC offset for the segment writer, which
        // stamps it into every filename. Doing it here — rather than once at
        // startup — is what makes a station keep naming files correctly across
        // a daylight-saving change it never restarts for. The lookup is cached
        // for a minute inside `birdnet_db::clock`, so this costs a relaxed
        // atomic load on all but one tick a minute.
        local_offset.set(birdnet_db::clock::local_utc_offset_secs());

        let secs = now_unix_secs();
        let synced = schedule::secs_look_synced(secs);
        if synced != clock_synced {
            log_clock_sync_change(synced);
            clock_synced = synced;
        }
        let now = Instant::now();
        let offset = local_offset.get();
        supervisor.tick(
            now,
            recording_allowed(schedule_config, secs, offset),
            quiet_minute_of_day(secs, offset),
            solar_minutes(schedule_config, secs, offset),
            metrics,
        );
        // Publish per-source health for the web layer's Station Health page,
        // using the same monotonic instant the tick reconciled against.
        supervisor.publish_status(now, secs, status);
        sleep_with_stop(SUPERVISE_TICK, stop);
    }
    tracing::info!("capture supervisor stopped");
}

/// Current Unix time in seconds (0 if the clock is somehow before the epoch).
fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether recording is allowed for `secs`, failing **open** while the clock
/// looks unsynced so a bogus boot-time date can't silence the station.
fn recording_allowed(config: &ScheduleConfig, secs: u64, offset_secs: i64) -> bool {
    !schedule::secs_look_synced(secs) || schedule_allows_at(config, secs, offset_secs)
}

/// Evaluate the recording schedule for a given (trusted) Unix timestamp.
///
/// The schedule is asked in the clock it means. A solar gate compares against
/// sunrise/sunset, which [`birdnet_scheduler::SolarDay`] reports in UTC; a fixed
/// window is an operator typing "06:00", which means six in the morning where
/// the station stands. Both were evaluated in UTC before, which is right for
/// solar and wrong for fixed by exactly the station's offset — `fixed:06:00-20:00`
/// on a UTC-8 station recorded 22:00-12:00 local, missing the dawn chorus it was
/// configured to capture.
fn schedule_allows_at(config: &ScheduleConfig, secs: u64, offset_secs: i64) -> bool {
    // The *date* stays UTC-derived: it selects which day's solar events to use,
    // and `SolarDay` is computed for a UTC day. Only the minute-of-day the gate
    // is compared against moves.
    let (year, month, day, utc_minute) = schedule::civil_from_unix_secs(secs);
    let schedule = DailySchedule::for_date(config, year, month, day);
    let minute = match schedule.clock() {
        ScheduleClock::Utc => utc_minute,
        ScheduleClock::Local => local_minute_of_day(secs, offset_secs),
    };
    schedule.is_allowed(minute)
}

/// Minute-of-day in the station's local time.
///
/// Saturating rather than wrapping: a garbage offset should not be able to move
/// the answer to a different day. `LocalOffset` already clamps to +/-14 h, so the
/// shifted timestamp stays in range for any real time zone.
fn local_minute_of_day(secs: u64, offset_secs: i64) -> u32 {
    let shifted = i64::try_from(secs)
        .unwrap_or(i64::MAX)
        .saturating_add(offset_secs)
        .max(0);
    schedule::civil_from_unix_secs(u64::try_from(shifted).unwrap_or(0)).3
}

/// The current **local** minute-of-day used to evaluate per-source quiet
/// windows, or `None` while the clock looks unsynced.
///
/// Local, not UTC: a quiet window is an operator saying "don't record between
/// 22:00 and 06:00", and they mean their own night. Quiet windows fail **open**
/// just like the schedule — a bogus boot-time date must not pause a source — so
/// the supervisor only enforces them once the clock is trustworthy.
fn quiet_minute_of_day(secs: u64, offset_secs: i64) -> Option<u32> {
    schedule::secs_look_synced(secs).then(|| local_minute_of_day(secs, offset_secs))
}

/// Today's sunrise and sunset, in the same local minute-of-day frame the quiet
/// windows are compared against.
///
/// Empty when the station has no coordinates or the clock is untrusted, which
/// makes any solar quiet window inactive — the same fail-open stance the global
/// schedule takes, and for the same reason: a guessed solar time pauses capture
/// in a way nobody can see from the outside.
fn solar_minutes(config: &ScheduleConfig, secs: u64, offset_secs: i64) -> SolarMinutes {
    if !schedule::secs_look_synced(secs) {
        return SolarMinutes::default();
    }
    let Some(location) = config.location else {
        return SolarMinutes::default();
    };
    // The *date* is UTC-derived because `SolarDay` is computed for a UTC day;
    // only the resulting minute-of-day is shifted into local time, exactly as
    // `schedule_allows_at` does for the global gate.
    let (year, month, day, _) = schedule::civil_from_unix_secs(secs);
    let Ok(solar) = SolarDay::for_date(location, year, month, day) else {
        return SolarMinutes::default();
    };
    let shift = |utc_min: u32| {
        let day = i64::from(24 * 60);
        let minutes = i64::from(utc_min) + offset_secs / 60;
        u32::try_from(minutes.rem_euclid(day)).ok()
    };
    SolarMinutes {
        sunrise_min: solar.sunrise_utc_min.and_then(shift),
        sunset_min: solar.sunset_utc_min.and_then(shift),
    }
}

/// Log a one-line notice when the clock's apparent sync state changes.
fn log_clock_sync_change(synced: bool) {
    if synced {
        tracing::info!("system clock looks NTP-synced; honouring the recording schedule");
    } else {
        tracing::warn!(
            "system clock looks UNSYNCED (no RTC, NTP not ready) — recording continuously so no \
             session is missed; detection timestamps may be wrong until time syncs"
        );
    }
}

/// Sleep up to `total`, waking early when `stop` is set so shutdown stays
/// responsive — without a busy loop.
fn sleep_with_stop(total: Duration, stop: &AtomicBool) {
    const STEP: Duration = Duration::from_millis(200);
    let mut elapsed = Duration::ZERO;
    while elapsed < total {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(STEP);
        elapsed += STEP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    fn fixed_window_config(spec: &str) -> ScheduleConfig {
        let mut c = cli();
        c.recording_schedule = spec.to_string();
        schedule::parse_schedule_config(&c, None)
    }

    #[test]
    fn recording_allowed_fails_open_when_clock_unsynced() {
        // A window that excludes 00:00. At the epoch (an unset-RTC reading) the
        // clock is untrustworthy, so we record anyway rather than believe the
        // bogus 1970 date and stay silent. Without the fail-open guard the
        // 06:00–07:00 window would reject 00:00 and this would be false.
        let config = fixed_window_config("fixed:06:00-07:00");
        assert!(recording_allowed(&config, 0, 0));
    }

    #[test]
    fn recording_allowed_honours_schedule_once_clock_synced() {
        let config = fixed_window_config("fixed:06:00-07:00");
        let midnight_2024 = 1_704_067_200; // 2024-01-01 00:00:00 UTC — clock looks synced
        // 06:30 local (offset 0, so also UTC) is inside the window.
        assert!(recording_allowed(
            &config,
            midnight_2024 + 6 * 3600 + 30 * 60,
            0
        ));
        // 12:00 is outside — and the clock is trusted, so we honour it.
        assert!(!recording_allowed(&config, midnight_2024 + 12 * 3600, 0));
    }

    #[test]
    fn sleep_with_stop_returns_promptly_when_already_stopped() {
        let stop = AtomicBool::new(true);
        let start = Instant::now();
        sleep_with_stop(Duration::from_secs(10), &stop);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must wake immediately when the stop flag is already set"
        );
    }

    #[test]
    fn now_unix_secs_is_after_2020() {
        // 2020-01-01 UTC; a sanity floor that catches a broken clock helper.
        assert!(now_unix_secs() > 1_577_836_800);
    }

    #[test]
    fn quiet_minute_of_day_none_when_clock_unsynced() {
        // At the epoch the clock is untrusted → quiet windows fail open (None).
        assert_eq!(quiet_minute_of_day(0, 0), None);
    }

    #[test]
    fn quiet_minute_of_day_is_local_minute_when_synced() {
        // 2024-01-01 06:30:00 UTC. In UTC that is minute 390; on a UTC-8
        // station it is 22:30 the previous evening, which is the minute an
        // operator's "quiet from 22:00" has to be compared against.
        let secs = 1_704_067_200 + 6 * 3600 + 30 * 60;
        assert_eq!(quiet_minute_of_day(secs, 0), Some(6 * 60 + 30));
        assert_eq!(
            quiet_minute_of_day(secs, -8 * 3600),
            Some(22 * 60 + 30),
            "a quiet window is the operator's night, not Greenwich's"
        );
        assert_eq!(quiet_minute_of_day(secs, 2 * 3600), Some(8 * 60 + 30));
    }

    /// A fixed window is what the operator typed, in their own time.
    ///
    /// This is the gate that reproduces the original defect: with the schedule
    /// evaluated in UTC, `fixed:06:00-20:00` on a UTC-8 station was really
    /// 22:00-12:00 local — it recorded through the night and stopped at midday,
    /// missing the dawn chorus entirely. `--doctor` warned about it; nothing
    /// fixed it.
    #[test]
    fn a_fixed_window_is_evaluated_in_local_time() {
        let config = fixed_window_config("fixed:06:00-20:00");
        // 2024-01-01 14:00 UTC = 06:00 local on a UTC-8 station: inside.
        let inside_local = 1_704_067_200 + 14 * 3600;
        assert!(
            recording_allowed(&config, inside_local, -8 * 3600),
            "06:00 local must be inside a 06:00-20:00 window"
        );
        // 2024-01-01 06:00 UTC = 22:00 local the previous day: outside.
        let outside_local = 1_704_067_200 + 6 * 3600;
        assert!(
            !recording_allowed(&config, outside_local, -8 * 3600),
            "22:00 local must be outside a 06:00-20:00 window"
        );
    }

    /// The counterpart, so the change above cannot be "the window now always
    /// matches": on a UTC station local and UTC agree, and the same window
    /// still admits 06:00 and excludes 22:00.
    #[test]
    fn a_fixed_window_on_a_utc_station_is_unchanged() {
        let config = fixed_window_config("fixed:06:00-20:00");
        assert!(recording_allowed(&config, 1_704_067_200 + 6 * 3600, 0));
        assert!(!recording_allowed(&config, 1_704_067_200 + 22 * 3600, 0));
    }

    /// Solar must stay on UTC. `SolarDay` reports sunrise and sunset as UTC
    /// minutes, so handing the gate a local minute would move the whole
    /// schedule by the offset — the same bug, in the other direction.
    #[test]
    fn a_solar_schedule_asks_for_utc() {
        let mut c = cli();
        c.recording_schedule = "solar".to_string();
        c.latitude = Some(51.5074);
        c.longitude = Some(-0.1278);
        let config = schedule::parse_schedule_config(&c, None);
        let schedule = DailySchedule::for_date(&config, 2024, 6, 21);
        assert_eq!(
            schedule.clock(),
            ScheduleClock::Utc,
            "solar events are absolute instants"
        );
        // And the offset must therefore make no difference to the answer.
        let midday_utc = 1_718_928_000; // 2024-06-21 00:00 UTC + 12 h below
        for offset in [-8 * 3600, 0, 2 * 3600] {
            assert_eq!(
                recording_allowed(&config, midday_utc + 12 * 3600, offset),
                recording_allowed(&config, midday_utc + 12 * 3600, 0),
                "a solar schedule must not shift with the station's offset"
            );
        }
    }

    #[test]
    fn a_fixed_window_reports_the_local_clock() {
        let config = fixed_window_config("fixed:06:00-20:00");
        assert_eq!(
            DailySchedule::for_date(&config, 2024, 1, 1).clock(),
            ScheduleClock::Local
        );
    }
}
