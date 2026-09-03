//! System-clock and timezone sanity checks.
//!
//! BirdNet-Behavior relies on the OS clock for wall-clock time but makes that
//! reliance *visible* here rather than carrying an explicit timezone stack:
//!
//! * Detection timestamps come from the recording filenames the capture step
//!   writes using the system's local time, so a wrong OS clock silently
//!   corrupts every timestamp.
//! * The recording gate is asked in the clock it means
//!   (`birdnet_scheduler::DailySchedule::clock`). A **fixed** window is an
//!   operator typing "06:00", so it is evaluated in **local** time; a **solar**
//!   schedule compares against sunrise and sunset, which are absolute instants,
//!   so it is evaluated in **UTC**. (Earlier releases evaluated both in UTC,
//!   which was right for solar and wrong for fixed by exactly the station's
//!   offset; this module still says so, because an operator who compensated for
//!   the old behaviour needs telling.)
//!
//! These checks surface both situations in plain language; they never change the
//! clock or timezone.

use std::time::{SystemTime, UNIX_EPOCH};

use birdnet_core::config::Config;

use crate::cli::Cli;

use super::Check;

/// The floor below which a clock reading is not trusted.
///
/// This used to be its own constant at `2020-01-01`, under a comment saying it
/// *"mirrors the capture supervisor's"*. It did not: the supervisor's was
/// `2024-01-01`, 1 461 days later. For any reading in those four years this
/// check printed `[ PASS ] set to a plausible current time` while the
/// supervisor treated the same reading as untrustworthy and disabled the
/// recording schedule and every quiet window — the diagnostic telling an
/// operator the opposite of what the station was doing.
///
/// One constant now, in the module that owns the calendar arithmetic.
const CLOCK_SYNCED_FLOOR_SECS: u64 = birdnet_core::civil::CLOCK_PLAUSIBLE_FLOOR_SECS;

/// Run the clock + timezone checks.
pub(super) fn check_clock(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
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
        .and_then(|s| schedule_timezone_check(s, birdnet_db::clock::local_utc_offset_secs()))
    {
        out.push(check);
    }
    if let Some(check) = timezone_mismatch_check(system_timezone(), detected_timezone(config)) {
        out.push(check);
    }
    if let Some(check) = solar_window_check(cli, config) {
        out.push(check);
    }
    out
}

/// Report how much of the day a **solar** schedule actually allows.
///
/// # Why a diagnostic, and not just a test
///
/// A solar schedule is the only gate here whose inputs come from outside the
/// configuration — the station's coordinates and the date — so it is the only
/// one that can be correct in every test and wrong on one particular hillside.
/// It was: [`birdnet_scheduler::SolarDay`] reports sunrise and sunset wrapped
/// into the UTC day, and for any station far enough east or west the two land
/// on different UTC days. `NightInhibit` read that as an empty window and the
/// station recorded **nothing**, all day, everywhere east of about 90° E or
/// west of about 75° W — Tokyo, Sydney, Auckland, Bangkok, Seattle and Honolulu
/// year-round; New York and Toronto every June.
///
/// The wrap is fixed (`birdnet-scheduler`'s `solar_window_worldwide` gate), but
/// the *shape* of that failure is what this check exists for: it was silent.
/// Capture simply never started, `--doctor` said every setting was valid, and
/// the only signal was the detection deadman hours later, reporting the wrong
/// cause. So the resolved window is now printed on every run, and a schedule
/// that allows no minutes — or every minute, which means the gate has stopped
/// gating — is an error with the answer in it.
fn solar_window_check(cli: &Cli, config: Option<&Config>) -> Option<Check> {
    let schedule = config
        .and_then(|c| c.get("RECORDING_SCHEDULE"))
        .map(ToOwned::to_owned)
        .or_else(|| setting_from_db(config, "recording_schedule"))?;
    if !matches!(
        schedule.trim().to_ascii_lowercase().as_str(),
        "solar" | "sunrise-to-sunset"
    ) {
        return None;
    }
    let (lat, lon) = crate::daemon::resolve_station_coords(cli, config);
    let (Some(lat), Some(lon)) = (lat, lon) else {
        // `check_station_location` already warns about this, and a solar
        // schedule without coordinates degrades to all-day rather than to
        // silence. One finding per cause.
        return None;
    };
    let location = birdnet_scheduler::Location::new(lat, lon).ok()?;
    let offset_secs = birdnet_db::clock::local_utc_offset_secs();
    let (y, m, d) = today_utc_ymd();
    let sched = birdnet_scheduler::DailySchedule::for_date(
        &birdnet_scheduler::ScheduleConfig {
            location: Some(location),
            pre_sunrise_offset_min: twilight_offset(cli, config, "PRE_SUNRISE_OFFSET"),
            post_sunset_offset_min: twilight_offset(cli, config, "POST_SUNSET_OFFSET"),
            night_inhibit: true,
            fixed_window: None,
        },
        y,
        m,
        d,
    );
    let minutes = (0..1440).filter(|&mm| sched.is_allowed(mm)).count();
    Some(solar_window_verdict(
        minutes,
        sched.solar.as_ref(),
        offset_secs,
    ))
}

/// Today's UTC calendar date, for picking which day's solar events to resolve.
fn today_utc_ymd() -> (u32, u32, u32) {
    let civil = birdnet_core::civil::civil_from_unix_secs(
        i64::try_from(now_unix_secs()).unwrap_or(i64::MAX),
    );
    (civil.year, civil.month, civil.day)
}

/// One twilight offset in minutes, from the CLI or the config, defaulting to 0.
fn twilight_offset(cli: &Cli, config: Option<&Config>, key: &str) -> u32 {
    match key {
        "PRE_SUNRISE_OFFSET" => cli.pre_sunrise_offset,
        _ => cli.post_sunset_offset,
    }
    .or_else(|| config.and_then(|c| c.get_parsed::<u32>(key).ok()))
    .unwrap_or(0)
}

/// The verdict for a resolved solar window (pure, for testing).
///
/// `minutes` is how many minutes of the UTC day the gate allows.
fn solar_window_verdict(
    minutes: usize,
    solar: Option<&birdnet_scheduler::SolarDay>,
    offset_secs: i64,
) -> Check {
    let window = solar
        .and_then(|s| Some((s.sunrise_utc_min?, s.sunset_utc_min?)))
        .map_or_else(
            || "the sun neither rises nor sets here today".to_owned(),
            |(rise, set)| {
                let local = |utc_min: u32| {
                    let m = (i64::from(utc_min) + offset_secs / 60).rem_euclid(1440);
                    format!("{:02}:{:02}", m / 60, m % 60)
                };
                let utc = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
                format!(
                    "sunrise {} UTC ({} local) to sunset {} UTC ({} local)",
                    utc(rise),
                    local(rise),
                    utc(set),
                    local(set)
                )
            },
        );
    let (h, m) = (minutes / 60, minutes % 60);
    match minutes {
        0 => Check::fail(
            "Recording schedule (solar)",
            format!("the solar window allows NO recording today — {window}"),
            "this station would record nothing at all. Set RECORDING_SCHEDULE=all-day to keep              capturing while this is investigated, and report the station's latitude/longitude              with this message",
        ),
        1440 => Check::warn(
            "Recording schedule (solar)",
            format!("the solar window allows the whole day — {window}"),
            "a solar schedule that never inhibits is either a polar summer or a gate that has              stopped gating. If this station is not inside a polar circle, set              RECORDING_SCHEDULE=all-day so the intent is explicit and report this message",
        ),
        _ => Check::pass(
            "Recording schedule (solar)",
            format!("recording {h}h {m:02}m today — {window}"),
        ),
    }
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
            "the clock reads before 2024 — it looks unset or not yet NTP-synced",
            "detection timestamps will be wrong and the station records continuously until the \
             clock syncs; check `timedatectl status` and, on a Pi without an RTC, ensure network \
             time is reachable (`sudo timedatectl set-ntp true`)",
        )
    } else {
        Check::pass("System clock", "set to a plausible current time")
    }
}

/// Report which clock a FIXED recording window is evaluated in; `None` for
/// solar / all-day schedules, which need no timezone thought (pure, for
/// testing). Fixed windows are **local** — see the module docs; this exists to
/// tell an operator who set UTC hours to compensate for the old behaviour that
/// they should set them back.
fn schedule_timezone_check(schedule: &str, offset_secs: i64) -> Option<Check> {
    if !schedule.trim().to_ascii_lowercase().starts_with("fixed:") {
        return None;
    }
    if offset_secs == 0 {
        return Some(Check::pass(
            "Recording schedule timezone",
            format!(
                "the fixed window {schedule:?} is evaluated in local time (this station is on UTC, so the hours are unchanged)"
            ),
        ));
    }
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let (h, m) = (offset_secs.abs() / 3600, (offset_secs.abs() % 3600) / 60);
    Some(Check::pass(
        "Recording schedule timezone",
        format!(
            "the fixed window {schedule:?} is evaluated in local time (UTC{sign}{h:02}:{m:02}). \
             Earlier releases evaluated it in UTC — if these hours were chosen to compensate \
             for that, set them to the local hours you actually want"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;

    #[test]
    fn current_time_passes() {
        // 2026-05-03; comfortably after the floor.
        //
        // This used to read `1_700_000_000` — 2023-11-14 — described as
        // "comfortably after the floor". It was, for *this* check's own floor
        // of 2020-01-01, and it was four months *before* the capture
        // supervisor's floor of 2024-01-01. A station whose clock read that
        // was told by the diagnostic that its clock was fine while the
        // supervisor disabled its recording schedule. The two now share one
        // constant, so the fixture has to move with it.
        assert_eq!(clock_check_for(1_777_000_000).status, Status::Pass);
    }

    /// The diagnostic and the capture supervisor must answer the same question
    /// the same way, everywhere, not merely at one point each.
    ///
    /// This is the gate the previous arrangement did not have. Both sides had
    /// tests; each tested its own constant, so neither could see that they were
    /// 1 461 days apart. Sweeping a range that spans both old floors is what
    /// makes a divergence impossible to reintroduce quietly.
    #[test]
    fn the_doctor_and_the_capture_supervisor_agree_about_every_clock_reading() {
        use crate::capture::schedule::secs_look_synced;

        // 2018-01-01 to 2030-01-01, weekly. Spans the old doctor floor
        // (2020-01-01) and the old supervisor floor (2024-01-01), so any
        // reintroduced gap between them lands inside this sweep.
        let mut secs: u64 = 1_514_764_800;
        let end: u64 = 1_893_456_000;
        let mut passes = 0_u32;
        let mut warns = 0_u32;
        while secs < end {
            let doctor_ok = clock_check_for(secs).status == Status::Pass;
            let supervisor_ok = secs_look_synced(secs);
            assert_eq!(
                doctor_ok, supervisor_ok,
                "at {secs}: --doctor says pass={doctor_ok} while the capture \
                 supervisor says synced={supervisor_ok}. An operator reading \
                 the diagnostic would be told the opposite of what the station \
                 is doing."
            );
            if doctor_ok {
                passes += 1;
            } else {
                warns += 1;
            }
            secs += 86_400 * 7;
        }

        // The discrimination: two predicates that both answered the same
        // constant everywhere would agree vacuously. The sweep has to contain
        // both answers.
        assert!(
            passes > 100,
            "the sweep saw only {passes} plausible readings"
        );
        assert!(
            warns > 100,
            "the sweep saw only {warns} implausible readings"
        );
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

    /// The window is local time now, and the report has to say which hours that
    /// actually means — an operator who set UTC hours to compensate for the old
    /// behaviour would otherwise be silently shifted.
    #[test]
    fn fixed_schedule_reports_the_local_interpretation() {
        let check =
            schedule_timezone_check("fixed:06:00-20:00", 2 * 3600).expect("fixed should report");
        assert_eq!(check.status, Status::Pass);
        assert!(
            check.message.contains("local time"),
            "got {:?}",
            check.message
        );
        assert!(
            check.message.contains("UTC+02:00"),
            "the offset must be named so the operator can check it: {:?}",
            check.message
        );
        assert!(
            check.message.contains("compensate"),
            "an upgrading station needs to be told the interpretation changed: {:?}",
            check.message
        );
    }

    /// A negative offset must read as a negative offset, not as a stray minus.
    #[test]
    fn fixed_schedule_reports_a_western_offset() {
        let check = schedule_timezone_check("fixed:06:00-20:00", -8 * 3600).expect("reports");
        assert!(check.message.contains("UTC-08:00"), "{:?}", check.message);
    }

    /// On a UTC station nothing changed, and saying "compensate" there would be
    /// advice to act on a difference that does not exist.
    #[test]
    fn fixed_schedule_on_a_utc_station_says_nothing_changed() {
        let check = schedule_timezone_check("fixed:06:00-20:00", 0).expect("reports");
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("unchanged"), "{:?}", check.message);
        assert!(!check.message.contains("compensate"));
    }

    // ── solar window ───────────────────────────────────────────────────

    /// A station that would record nothing must be told so, in terms it can
    /// act on. This is the verdict that fires for the pre-fix `NightInhibit`
    /// at every longitude east of ~90° E or west of ~75° W.
    #[test]
    fn a_solar_window_that_allows_no_minutes_is_an_error() {
        let check = solar_window_verdict(0, None, 0);
        assert_eq!(check.status, Status::Fail);
        assert!(
            check.message.contains("NO recording"),
            "must say what is wrong: {}",
            check.message
        );
        assert!(
            check
                .remediation
                .as_deref()
                .is_some_and(|r| r.contains("all-day")),
            "must say what to do about it"
        );
    }

    /// The counterpart. A gate that has stopped gating looks like success from
    /// the inside, so "the whole day is allowed" cannot be a pass either.
    #[test]
    fn a_solar_window_that_allows_the_whole_day_warns() {
        assert_eq!(solar_window_verdict(1440, None, 0).status, Status::Warn);
    }

    /// And a real window is a plain pass that reports both clocks, so an
    /// operator can check it against their own sunrise without arithmetic.
    #[test]
    fn a_real_solar_window_passes_and_names_both_clocks() {
        let solar = birdnet_scheduler::SolarDay::for_date(
            birdnet_scheduler::Location::new(-36.85, 174.76).expect("Auckland"),
            2026,
            6,
            21,
        )
        .expect("solar day");
        // NZST is UTC+12.
        let check = solar_window_verdict(577, Some(&solar), 12 * 3600);
        assert_eq!(check.status, Status::Pass);
        assert!(check.message.contains("9h 37m"), "{}", check.message);
        // Sunrise 19:33 UTC is 07:33 local; a check that forgot the offset
        // would print 19:33 twice.
        assert!(
            check.message.contains("19:33 UTC (07:33 local)"),
            "both clocks, correctly converted: {}",
            check.message
        );
    }

    /// The whole point of the check is that it fires for a *solar* schedule.
    /// A fixed or all-day station has nothing to report here.
    #[test]
    fn the_solar_check_is_silent_for_other_schedules() {
        let cli: Cli = clap::Parser::parse_from(["birdnet-behavior"]);
        for schedule in ["all-day", "fixed:06:00-20:00"] {
            let cfg = Config::parse(&format!(
                "RECORDING_SCHEDULE={schedule}\nLATITUDE=-36.85\nLONGITUDE=174.76\nDB_PATH=/nonexistent/bnb-clock-test.db"
            ))
            .expect("parse");
            assert!(
                solar_window_check(&cli, Some(&cfg)).is_none(),
                "{schedule} should not produce a solar-window finding"
            );
        }
    }

    /// End to end against the real solver: an Auckland station on a solar
    /// schedule gets a pass with a plausible day in it. Before the
    /// `NightInhibit` wrap fix this was `Status::Fail` with "NO recording".
    #[test]
    fn an_auckland_station_on_solar_reports_a_real_days_recording() {
        let cli: Cli = clap::Parser::parse_from(["birdnet-behavior"]);
        let cfg = Config::parse(
            "RECORDING_SCHEDULE=solar\nLATITUDE=-36.85\nLONGITUDE=174.76\nDB_PATH=/nonexistent/bnb-clock-test.db",
        )
        .expect("parse");
        let check = solar_window_check(&cli, Some(&cfg)).expect("a solar station reports");
        assert_eq!(check.status, Status::Pass, "{}", check.message);
    }

    #[test]
    fn solar_and_all_day_have_no_timezone_caveat() {
        // The offset must make no difference either: only a fixed window has a
        // timezone question to answer.
        for offset in [-8 * 3600, 0, 2 * 3600] {
            assert!(schedule_timezone_check("solar", offset).is_none());
            assert!(schedule_timezone_check("all-day", offset).is_none());
            assert!(schedule_timezone_check("sunrise-to-sunset", offset).is_none());
        }
    }

    #[test]
    fn check_clock_returns_clock_check_even_without_config() {
        let checks = check_clock(&clap::Parser::parse_from(["birdnet-behavior"]), None);
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
