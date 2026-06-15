//! The supervisor's background run loop and its per-tick schedule/clock gating.
//!
//! [`run_supervisor`] reconciles every capture source on a fixed cadence until
//! asked to stop, deciding each tick whether recording is allowed (the solar /
//! fixed-window schedule) and which sources are inside a quiet window — both
//! gated on the system clock looking trustworthy.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use birdnet_core::audio::capture::{CaptureManager, CaptureStatusHandle};
use birdnet_scheduler::{DailySchedule, ScheduleConfig};
use birdnet_web::metrics::SharedMetrics;

use super::schedule;
use super::supervisor::Supervisor;

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
    stop: &AtomicBool,
) {
    tracing::info!("capture supervisor started");
    // Start from "synced" so an unsynced clock at boot trips the warning on
    // the very first tick.
    let mut clock_synced = true;
    while !stop.load(Ordering::Relaxed) {
        let secs = now_unix_secs();
        let synced = schedule::secs_look_synced(secs);
        if synced != clock_synced {
            log_clock_sync_change(synced);
            clock_synced = synced;
        }
        let now = Instant::now();
        supervisor.tick(
            now,
            recording_allowed(schedule_config, secs),
            quiet_minute_of_day(secs),
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
fn recording_allowed(config: &ScheduleConfig, secs: u64) -> bool {
    !schedule::secs_look_synced(secs) || schedule_allows_at(config, secs)
}

/// Evaluate the recording schedule for a given (trusted) Unix timestamp.
fn schedule_allows_at(config: &ScheduleConfig, secs: u64) -> bool {
    let (year, month, day, minutes_now) = schedule::civil_from_unix_secs(secs);
    DailySchedule::for_date(config, year, month, day).is_allowed(minutes_now)
}

/// The current minute-of-day (UTC) used to evaluate per-source quiet windows,
/// or `None` while the clock looks unsynced. Quiet windows fail **open** just
/// like the schedule — a bogus boot-time date must not pause a source — so the
/// supervisor only enforces them once the clock is trustworthy.
fn quiet_minute_of_day(secs: u64) -> Option<u32> {
    schedule::secs_look_synced(secs).then(|| schedule::civil_from_unix_secs(secs).3)
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
        assert!(recording_allowed(&config, 0));
    }

    #[test]
    fn recording_allowed_honours_schedule_once_clock_synced() {
        let config = fixed_window_config("fixed:06:00-07:00");
        let midnight_2024 = 1_704_067_200; // 2024-01-01 00:00:00 UTC — clock looks synced
        // 06:30 UTC is inside the window.
        assert!(recording_allowed(
            &config,
            midnight_2024 + 6 * 3600 + 30 * 60
        ));
        // 12:00 UTC is outside — and the clock is trusted, so we honour it.
        assert!(!recording_allowed(&config, midnight_2024 + 12 * 3600));
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
        assert_eq!(quiet_minute_of_day(0), None);
    }

    #[test]
    fn quiet_minute_of_day_is_utc_minute_when_synced() {
        // 2024-01-01 06:30:00 UTC → minute-of-day 390, and the clock looks synced.
        let secs = 1_704_067_200 + 6 * 3600 + 30 * 60;
        assert_eq!(quiet_minute_of_day(secs), Some(6 * 60 + 30));
    }
}
