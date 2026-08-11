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
    // The settings-table fallback matters for the same reason it does for the
    // station location: `--doctor` runs from `ExecStartPre`, before the
    // settings overlay merges `/admin/settings` onto the config. Reading the
    // file alone would stay silent for exactly the operators who set their
    // recording window the easy way — and the window is now settable there.
    if let Some(check) = config
        .and_then(|c| c.get("RECORDING_SCHEDULE"))
        .map(ToOwned::to_owned)
        .or_else(|| setting_from_db(config, "recording_schedule"))
        .as_deref()
        .and_then(schedule_timezone_check)
    {
        out.push(check);
    }
    if let Some(check) = timezone_mismatch_check(system_timezone(), detected_timezone(config)) {
        out.push(check);
    }
    out
}

/// Compare the host's timezone with the one the setup wizard detected.
///
/// The wizard's location step looks up the station's timezone along with its
/// coordinates and stores it — but nothing in this process can *apply* it: the
/// timezone is a system setting and the service does not run as root. Stored
/// and never mentioned again, it was a dead setting; this is what makes it
/// worth having.
///
/// It matters because the clock is not cosmetic here. Capture names each
/// recording from the system's local time, and those filenames become the
/// `Date` and `Time` of every detection parsed out of them — so a Pi left on
/// UTC in a UTC+2 country files its dawn chorus two hours early, its "today"
/// rolls over at the wrong moment, and retention deletes by the wrong day.
/// Raspberry Pi OS images default to UTC unless the imager set otherwise,
/// which makes this a common state rather than an exotic one.
///
/// A warning, never an error: the station works, its timestamps are just
/// shifted, and only the operator can say which is right.
fn timezone_mismatch_check(system: Option<String>, detected: Option<String>) -> Option<Check> {
    let (system, detected) = (system?, detected?);
    if system == detected {
        return Some(Check::pass(
            "Timezone",
            format!("{system} — matches the station's location"),
        ));
    }
    Some(Check::warn(
        "Timezone",
        format!(
            "this machine's clock is set to {system}, but the station's location is in {detected}"
        ),
        format!(
            "detection times come from the system clock, so they will be recorded in {system}. \
             Fix with:  sudo timedatectl set-timezone {detected}   \
             (then restart: sudo systemctl restart birdnet-behavior)"
        ),
    ))
}

/// The host's configured timezone name, e.g. `Europe/Berlin`.
///
/// `/etc/timezone` is the plain-text form Debian and Raspberry Pi OS keep;
/// `/etc/localtime` is a symlink into the zoneinfo tree on systemd hosts. Try
/// both, since neither is universal. `None` when the host uses neither
/// convention — this check then stays silent rather than guessing.
fn system_timezone() -> Option<String> {
    if let Ok(raw) = std::fs::read_to_string("/etc/timezone") {
        let name = raw.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let s = target.to_str()?;
    // ".../zoneinfo/Europe/Berlin" → "Europe/Berlin"
    let (_, zone) = s.split_once("/zoneinfo/")?;
    (!zone.is_empty()).then(|| zone.to_string())
}

/// The timezone the onboarding wizard detected, from the settings table.
///
/// Read read-only and best-effort: a missing database or table simply means no
/// comparison to make. `check_database` owns the database's health.
fn detected_timezone(config: Option<&Config>) -> Option<String> {
    setting_from_db(config, "timezone")
}

/// One non-empty value from the `settings` table, read-only and best-effort.
///
/// A missing database or table simply means there is nothing to compare
/// against; `check_database` owns the database's health, and a diagnostic must
/// not turn a storage problem into a finding about something else.
fn setting_from_db(config: Option<&Config>, key: &str) -> Option<String> {
    let db_path = crate::helpers::db_path_from_config(config);
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let value = birdnet_db::settings::get(&conn, key).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
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
        assert_eq!(checks[0].name, "System clock");
    }

    // ── timezone mismatch ───────────────────────────────────────────────
    //
    // Raspberry Pi OS images default to UTC unless the imager set otherwise,
    // so "station in Europe/Berlin, clock on UTC" is a common state, not an
    // exotic one — and it shifts every detection's timestamp by the offset.

    #[test]
    fn mismatched_timezone_warns_with_the_exact_fix() {
        let check = timezone_mismatch_check(Some("UTC".into()), Some("Europe/Berlin".into()))
            .expect("a mismatch must be reported");
        assert_eq!(check.status, Status::Warn);
        assert!(check.message.contains("UTC"), "{}", check.message);
        assert!(check.message.contains("Europe/Berlin"));
        let fix = check.remediation.expect("a warning must carry a fix");
        assert!(
            fix.contains("timedatectl set-timezone Europe/Berlin"),
            "the operator needs the command, not a description: {fix}"
        );
    }

    #[test]
    fn matching_timezone_passes() {
        let check =
            timezone_mismatch_check(Some("Europe/Berlin".into()), Some("Europe/Berlin".into()))
                .expect("a match is still worth reporting");
        assert_eq!(check.status, Status::Pass);
    }

    #[test]
    fn timezone_check_is_silent_when_either_side_is_unknown() {
        // Nothing to compare: never guess, and never nag a station that simply
        // has not been through the wizard.
        assert!(timezone_mismatch_check(None, Some("Europe/Berlin".into())).is_none());
        assert!(timezone_mismatch_check(Some("UTC".into()), None).is_none());
        assert!(timezone_mismatch_check(None, None).is_none());
    }

    #[test]
    fn system_timezone_reads_a_real_host_or_says_it_cannot() {
        // Not asserting a value — this runs on hosts with either convention,
        // or neither. Asserting it never panics and never returns a blank.
        if let Some(tz) = system_timezone() {
            assert!(!tz.trim().is_empty(), "a blank timezone is not an answer");
            assert!(!tz.contains("/zoneinfo/"), "path not stripped: {tz}");
        }
    }
}
