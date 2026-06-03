//! System-clock and timezone sanity checks.
//!
//! BirdNet-Behavior relies on the OS clock for wall-clock time but makes that
//! reliance *visible* here rather than carrying an explicit timezone stack:
//!
//! * Detection timestamps come from the recording filenames the capture step
//!   writes using the system's local time, so a wrong OS clock silently
//!   corrupts every timestamp.
//! * The recording-window gate is evaluated in **UTC** (see
//!   `capture::schedule`). Solar schedules are timezone-independent (sunrise is
//!   an absolute instant), but a **fixed** window's hours are therefore
//!   interpreted as UTC, not local — a non-UTC station needs to know that.
//!
//! These checks surface both situations in plain language; they never change the
//! clock or timezone.

use std::time::{SystemTime, UNIX_EPOCH};

use birdnet_core::config::Config;

use super::Check;

/// Unix seconds at 2020-01-01 UTC. A clock reading earlier has almost certainly
/// not been set or NTP-synced yet. Mirrors the capture supervisor's
/// `CLOCK_SYNCED_FLOOR_SECS`, which fails recording *open* on an unsynced clock
/// so a bogus boot-time date can't silence the station.
const CLOCK_SYNCED_FLOOR_SECS: u64 = 1_577_836_800;

/// Run the clock + timezone checks.
pub(super) fn check_clock(config: Option<&Config>) -> Vec<Check> {
    let mut out = vec![clock_check_for(now_unix_secs())];
    if let Some(check) = config
        .and_then(|c| c.get("RECORDING_SCHEDULE"))
        .and_then(schedule_timezone_check)
    {
        out.push(check);
    }
    out
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Verdict for a clock reading of `now` Unix seconds (pure, for testing).
fn clock_check_for(now: u64) -> Check {
    if now < CLOCK_SYNCED_FLOOR_SECS {
        Check::warn(
            "System clock",
            "the clock reads before 2020 — it looks unset or not yet NTP-synced",
            "detection timestamps will be wrong and the station records continuously until the \
             clock syncs; check `timedatectl status` and, on a Pi without an RTC, ensure network \
             time is reachable (`sudo timedatectl set-ntp true`)",
        )
    } else {
        Check::pass("System clock", "set to a plausible current time")
    }
}

/// Surface that a FIXED recording window is evaluated in UTC; `None` for
/// solar / all-day schedules, which need no timezone (pure, for testing).
fn schedule_timezone_check(schedule: &str) -> Option<Check> {
    if schedule.trim().to_ascii_lowercase().starts_with("fixed:") {
        Some(Check::warn(
            "Recording schedule timezone",
            format!("the fixed window {schedule:?} is evaluated in UTC, not local time"),
            "express the hours in UTC, or use a timezone-independent solar schedule \
             (RECORDING_SCHEDULE=solar / sunrise-to-sunset)",
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;

    #[test]
    fn current_time_passes() {
        // ~2023-11-14; comfortably after the floor.
        assert_eq!(clock_check_for(1_700_000_000).status, Status::Pass);
    }

    #[test]
    fn unset_clock_warns() {
        assert_eq!(clock_check_for(0).status, Status::Warn);
        // Just before the 2020 floor.
        assert_eq!(
            clock_check_for(CLOCK_SYNCED_FLOOR_SECS - 1).status,
            Status::Warn
        );
    }

    #[test]
    fn fixed_schedule_warns_about_utc() {
        let check = schedule_timezone_check("fixed:06:00-20:00").expect("fixed should warn");
        assert_eq!(check.status, Status::Warn);
        assert!(check.message.contains("UTC"));
    }

    #[test]
    fn solar_and_all_day_have_no_timezone_caveat() {
        assert!(schedule_timezone_check("solar").is_none());
        assert!(schedule_timezone_check("all-day").is_none());
        assert!(schedule_timezone_check("sunrise-to-sunset").is_none());
    }

    #[test]
    fn check_clock_returns_clock_check_even_without_config() {
        let checks = check_clock(None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "System clock");
    }
}
