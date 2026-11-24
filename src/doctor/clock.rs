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
    out.push(timezone_mismatch_check(
        system_timezone(),
        detected_timezone(config),
        zoneinfo_version().as_deref(),
        station_coords(cli, config),
        birdnet_db::clock::local_utc_offset_secs(),
    ));
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
///
/// Without a zone from the wizard — always the case for a station whose
/// location was typed at the installer prompt or into the config file — the
/// machine's current UTC offset (`offset_secs`, daylight saving included) is
/// compared with the station's longitude instead (OOB-06); see
/// [`location_offset_check`].
fn timezone_mismatch_check(
    system: Option<String>,
    detected: Option<String>,
    zoneinfo: Option<&str>,
    location: Option<(f64, f64)>,
    offset_secs: i64,
) -> Check {
    use birdnet_web::routes::pages::onboarding::plausible_timezone;
    let zoneinfo = zoneinfo.map_or_else(
        || "no zoneinfo version file".to_owned(),
        |v| format!("zoneinfo {v}"),
    );
    // No zone anywhere used to be silence (NT-6) — and it is exactly the
    // containerised case, where the answer is guaranteed wrong: the station
    // runs on UTC and files every detection under UTC hours.
    let Some(system) = system else {
        return Check::warn(
            "Timezone",
            format!(
                "no timezone is configured on this machine ({zoneinfo}): local time is UTC, \
                 so detections are filed under UTC hours"
            ),
            "in a container, set TZ=<Area/City> in .env (for example TZ=Europe/Berlin); on a \
             host, sudo timedatectl set-timezone <Area/City>. Then restart the station",
        );
    };
    // A zone glibc does not know is UTC with a configured-looking name — the
    // worse failure. `TZ=Nowhere/Fake` gets here.
    if !plausible_timezone(&system) {
        return Check::warn(
            "Timezone",
            format!(
                "this machine's timezone is set to {system}, which is not a known zone \
                 ({zoneinfo}): local time falls back to UTC, so detections are filed under \
                 UTC hours"
            ),
            "use an IANA name such as Europe/Berlin: TZ in .env for a container, \
             sudo timedatectl set-timezone on a host. Then restart the station",
        );
    }
    // The wizard's row is operator-writable and used to be trusted as it was
    // (ON-8): `timezone=Mars/Olympus` made the doctor say "set-timezone
    // Mars/Olympus". A row that is not a zone is reported as that, and never
    // compared against.
    let Some(detected) = detected else {
        if let Some(check) =
            location.and_then(|(lat, lon)| location_offset_check(&system, offset_secs, lat, lon))
        {
            return check;
        }
        if let Some((_, lon)) = location {
            return Check::pass(
                "Timezone",
                format!(
                    "{system} (this machine's clock, {zoneinfo}, now {now}) is within \
                     {SOLAR_OFFSET_TOLERANCE_HOURS} hours of the station's longitude \
                     ({lon:.1}); the setup wizard has not recorded the station's zone, so \
                     a wrong zone closer than that cannot be seen",
                    now = utc_offset_label(offset_secs),
                ),
            );
        }
        return Check::pass(
            "Timezone",
            format!(
                "{system} (this machine's clock, {zoneinfo}); the setup wizard has not \
                 recorded the station's zone and no coordinates are set, so there is nothing \
                 to compare it against"
            ),
        );
    };
    if !plausible_timezone(&detected) {
        return Check::warn(
            "Timezone",
            format!(
                "the station's recorded zone, {detected}, is not a known zone, so this \
                 machine's {system} cannot be checked against it"
            ),
            "re-run Detect on the setup wizard's location step, or set the timezone \
             setting to an IANA zone name such as Europe/London",
        );
    }
    if system == detected {
        return Check::pass(
            "Timezone",
            format!("{system} — matches the station's location ({zoneinfo})"),
        );
    }
    Check::warn(
        "Timezone",
        format!(
            "this machine's clock is set to {system}, but the time zone saved for the station \
             is {detected}"
        ),
        format!(
            "Detection times follow the machine's own time zone ({system}); the zone saved for \
             the station ({detected}) is a note, and nothing in the browser changes the \
             machine's clock. If {system} is where the station is, no action is needed (correct \
             the saved zone by looking up the location again in Settings). If the machine is on \
             the wrong zone, on the machine itself: sudo timedatectl set-timezone {detected}, \
             then sudo systemctl restart birdnet-behavior"
        ),
    )
}

/// The furthest, in hours, the machine's UTC offset may sit from the station's
/// solar offset (`longitude / 15`) before [`location_offset_check`] warns.
///
/// The audit proposed "more than 2 h" against `round(lon / 15)`. That flags
/// real stations on their correct zone, because some legal zones sit well away
/// from the sun, and daylight saving adds an hour on top (the comparison uses
/// the offset in force *now*, so it has to allow for summer time):
///
/// * western Alaska: Nome on `America/Anchorage` in summer (UTC−8 at 165.4° W,
///   solar −11.0) is 3.0 h out; Adak on `America/Adak` in summer 2.8 h;
/// * China on one zone: Kashgar on `Asia/Shanghai` (UTC+8 at 76° E, solar
///   +5.1) is 2.9 h, the country's western edge 3.1 h; Urumqi 2.2 h;
/// * Spain on Central European time: Madrid in summer 2.2 h, Galicia 2.6 h;
/// * Argentina on UTC−3: Mendoza 1.6 h; Chile's Easter Island in summer 2.3 h;
///   Nuuk in summer 2.5 h.
///
/// So the margin is 3.5 h against the unrounded solar offset: half an hour
/// clear of the furthest legal case found (3.1 h), and it needs no special
/// case for half-hour and 45-minute zones (India, Nepal, Chatham), which a
/// rounded comparison would misjudge by up to half an hour. What it still
/// catches is the mistake it is for: a machine on another continent's zone —
/// Raspberry Pi OS's default `Europe/London` (or UTC) in the Americas, Asia
/// or Australia is five hours or more out. What it cannot catch is a wrong
/// zone within about three hours of the right one (UTC on a station in
/// Germany or Moscow); only the wizard's looked-up zone can say that, and the
/// comparison above makes it when there is one.
const SOLAR_OFFSET_TOLERANCE_HOURS: f64 = 3.5;

/// How far, in hours, a UTC offset of `offset_secs` is from the solar offset
/// at `longitude`, wrapped into −12..=12 so that a zone across the date line
/// from its longitude (Kiribati's UTC+14 at 157° W, Samoa's UTC+13 at 172° W)
/// is measured the short way round, which is the true difference in local
/// time of day. Positive: the clock is ahead of the sun.
fn solar_offset_gap_hours(offset_secs: i64, longitude: f64) -> f64 {
    // Real offsets are within ±14 h, so this conversion is exact; anything
    // outside i32 is not an offset and is treated as UTC.
    let clock = f64::from(i32::try_from(offset_secs).unwrap_or(0)) / 3600.0;
    let gap = (clock - longitude / 15.0).rem_euclid(24.0);
    if gap > 12.0 { gap - 24.0 } else { gap }
}

/// `UTC+5`, `UTC-3:30`, `UTC+0` — an offset in whole or part hours.
fn utc_offset_label(offset_secs: i64) -> String {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let (h, m) = (offset_secs.abs() / 3600, (offset_secs.abs() % 3600) / 60);
    if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

/// Compare the machine's UTC offset with the station's longitude (pure, for
/// testing): `None` when they agree within [`SOLAR_OFFSET_TOLERANCE_HOURS`] or
/// the coordinates are not a position, else the warning (OOB-06).
///
/// `offset_secs` is the machine's offset at the moment of the check, so the
/// caller's clock is the "now" this is judged at; tests pass the offset a zone
/// has on a chosen date.
fn location_offset_check(
    system: &str,
    offset_secs: i64,
    latitude: f64,
    longitude: f64,
) -> Option<Check> {
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }
    let gap = solar_offset_gap_hours(offset_secs, longitude);
    if gap.abs() <= SOLAR_OFFSET_TOLERANCE_HOURS {
        return None;
    }
    // Rounded to the nearest whole hour for the reader; the margin above makes
    // this at least 4.
    #[allow(clippy::cast_possible_truncation)]
    let hours = gap.abs().round() as i64;
    #[allow(clippy::cast_possible_truncation)]
    let solar = (longitude / 15.0).round() as i64 * 3600;
    let direction = if gap > 0.0 { "ahead of" } else { "behind" };
    Some(Check::warn(
        "Timezone",
        format!(
            "this machine's clock is set to {system} ({now}), but the station's location \
             ({latitude:.1}, {longitude:.1}) is near {solar}: the clock runs about {hours} \
             hours {direction} the station's day, so every detection is filed at the wrong \
             hour",
            now = utc_offset_label(offset_secs),
            solar = utc_offset_label(solar),
        ),
        "Detection times follow this machine's time zone. If the coordinates are right, set \
         the machine to the station's zone (timedatectl list-timezones lists them): on the \
         machine itself, sudo timedatectl set-timezone <Area/City>, then sudo systemctl \
         restart birdnet-behavior; in a container, TZ=<Area/City> in .env, then restart the \
         container. If the coordinates are wrong, correct them under Settings. A station kept \
         on UTC on purpose can ignore this",
    ))
}

/// The station's coordinates: the flag or config file, else the dashboard's
/// settings table (`--doctor` runs before the overlay merges it).
fn station_coords(cli: &Cli, config: Option<&Config>) -> Option<(f64, f64)> {
    match crate::daemon::resolve_station_coords(cli, config) {
        (Some(lat), Some(lon)) => Some((lat, lon)),
        _ => super::config::location_from_settings(config),
    }
}

/// The version of the zoneinfo the machine keeps local time with, from the
/// first line of `tzdata.zi` (`# version 2025b`); `None` without the file.
/// Reported, not judged (NT-6): there is nothing offline to compare it to.
fn zoneinfo_version() -> Option<String> {
    zoneinfo_version_in(std::path::Path::new(
        birdnet_web::routes::pages::onboarding::ZONEINFO_DIR,
    ))
}

/// [`zoneinfo_version`] against another zoneinfo root, for tests.
fn zoneinfo_version_in(root: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("tzdata.zi")).ok()?;
    let first = text.lines().next()?;
    let version = first.strip_prefix("# version ")?.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

/// The zone this machine keeps local time in, e.g. `Europe/Berlin`.
fn system_timezone() -> Option<String> {
    let localtime = std::fs::read_link("/etc/localtime").ok();
    system_timezone_from(
        std::env::var("TZ").ok().as_deref(),
        std::fs::read_to_string("/etc/timezone").ok().as_deref(),
        localtime.as_deref().and_then(std::path::Path::to_str),
    )
}

/// [`system_timezone`] from its three inputs, in the order glibc consults
/// them (ON-7): `TZ` first — it is what SQLite's `localtime`, the one source
/// of the station's offset, ends up using, and it is how a container is told
/// its zone; then `/etc/timezone`, the plain-text form Debian and Raspberry
/// Pi OS keep; then the `/etc/localtime` symlink into the zoneinfo tree on
/// systemd hosts. A leading `:` on `TZ` is glibc's "this is a file" marker,
/// not part of the name. `None` when none of the three says anything, which
/// the check reports as UTC rather than staying silent.
fn system_timezone_from(
    tz_env: Option<&str>,
    etc_timezone: Option<&str>,
    localtime_target: Option<&str>,
) -> Option<String> {
    if let Some(tz) = tz_env {
        let tz = tz.trim().trim_start_matches(':');
        if !tz.is_empty() {
            return Some(tz.to_string());
        }
    }
    if let Some(raw) = etc_timezone {
        let name = raw.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    // ".../zoneinfo/Europe/Berlin" → "Europe/Berlin"
    let (_, zone) = localtime_target?.split_once("/zoneinfo/")?;
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
        let check = timezone_mismatch_check(
            Some("UTC".into()),
            Some("Europe/Berlin".into()),
            None,
            None,
            0,
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.message.contains("UTC"), "{}", check.message);
        assert!(check.message.contains("Europe/Berlin"));
        let fix = check.remediation.expect("a warning must carry a fix");
        assert!(
            fix.contains("timedatectl set-timezone Europe/Berlin"),
            "the operator needs the command, not a description: {fix}"
        );
        // The command is for whoever can run it and does not lead: the first
        // thing said is what the saved zone is and what follows the machine.
        let command_at = fix.find("sudo").expect("the command is still there");
        let words_before = &fix[..command_at];
        assert!(
            words_before.contains("machine's own time zone")
                && words_before.contains("a note")
                && words_before.contains("on the machine itself"),
            "plain words must come before the command: {fix}"
        );
    }

    #[test]
    fn matching_timezone_passes() {
        let check = timezone_mismatch_check(
            Some("Europe/Berlin".into()),
            Some("Europe/Berlin".into()),
            Some("2025b"),
            None,
            0,
        );
        assert_eq!(check.status, Status::Pass);
        assert!(
            check.message.contains("zoneinfo 2025b"),
            "{}",
            check.message
        );
    }

    /// NT-6 / ON-7. No zone on the machine used to be silence, and it is
    /// exactly the containerised case, where the station is on UTC. It is a
    /// warning now, naming both fixes; a zone glibc does not know is the same
    /// warning, since glibc falls back to UTC for it just as quietly.
    #[test]
    fn no_timezone_on_the_machine_is_a_warning_not_silence() {
        for detected in [None, Some("Europe/Berlin".to_owned())] {
            let check = timezone_mismatch_check(None, detected, None, None, 0);
            assert_eq!(check.status, Status::Warn, "{check:?}");
            assert!(check.message.contains("UTC hours"), "{}", check.message);
            let fix = check.remediation.expect("a fix");
            assert!(fix.contains("TZ=") && fix.contains("timedatectl"), "{fix}");
        }
        let check =
            timezone_mismatch_check(Some("Nowhere/Fake".into()), None, Some("2025b"), None, 0);
        assert_eq!(check.status, Status::Warn, "{check:?}");
        assert!(
            check.message.contains("not a known zone"),
            "{}",
            check.message
        );
        assert!(
            check.message.contains("zoneinfo 2025b"),
            "{}",
            check.message
        );
        // A known host zone with no recorded station zone is reported as a
        // pass naming the zone (ON-8), which is information, not a nag.
        assert_eq!(
            timezone_mismatch_check(Some("UTC".into()), None, None, None, 0).status,
            Status::Pass
        );
    }

    /// ON-7: `TZ` is what a container is told its zone with, and what glibc
    /// (so SQLite's `localtime`) consults first, so it outranks the two files;
    /// glibc's leading `:` is not part of the name.
    #[test]
    fn tz_outranks_the_files_and_the_files_still_work() {
        assert_eq!(
            system_timezone_from(
                Some("Europe/Berlin"),
                Some("Etc/UTC\n"),
                Some("/usr/share/zoneinfo/Etc/UTC")
            )
            .as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            system_timezone_from(Some(":Asia/Tokyo"), None, None).as_deref(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            system_timezone_from(Some(""), Some("Europe/London\n"), None).as_deref(),
            Some("Europe/London"),
            "a blank TZ is unset, not a zone"
        );
        assert_eq!(
            system_timezone_from(None, None, Some("/usr/share/zoneinfo/America/New_York"))
                .as_deref(),
            Some("America/New_York")
        );
        assert_eq!(system_timezone_from(None, None, None), None);
        assert_eq!(
            system_timezone_from(None, None, Some("/etc/localtime.bak")),
            None
        );
    }

    #[test]
    fn the_zoneinfo_version_is_read_from_tzdata_zi() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(zoneinfo_version_in(root.path()), None);
        std::fs::write(root.path().join("tzdata.zi"), "# version 2025b\n# ...\n").unwrap();
        assert_eq!(zoneinfo_version_in(root.path()).as_deref(), Some("2025b"));
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

    /// ON-8: a stored zone that is not a zone is named as such and never
    /// becomes a `set-timezone` instruction; a missing one is reported, not
    /// silently skipped.
    #[test]
    fn a_bogus_stored_zone_is_reported_not_compared_against() {
        let check = timezone_mismatch_check(
            Some("Europe/Berlin".into()),
            Some("Mars/Olympus".into()),
            None,
            None,
            0,
        );
        assert_eq!(check.status, crate::doctor::Status::Warn, "{check:?}");
        assert!(check.message.contains("not a known zone"), "{check:?}");
        assert!(
            !check
                .remediation
                .as_deref()
                .unwrap_or_default()
                .contains("set-timezone Mars/Olympus"),
            "the bogus zone must not become the fix: {check:?}"
        );

        let check = timezone_mismatch_check(Some("Europe/Berlin".into()), None, None, None, 0);
        assert_eq!(check.status, crate::doctor::Status::Pass, "{check:?}");
        assert!(check.message.contains("Europe/Berlin"), "{check:?}");
        assert!(check.message.contains("not recorded"), "{check:?}");
    }

    // ── time zone against the station's longitude (OOB-06) ──────────────
    //
    // Offsets are the ones each zone has on the date named, from tzdata; the
    // check is handed the offset in force "now", so a test picks its "now" by
    // picking the offset.

    const OHIO: (f64, f64) = (40.0, -83.0);
    const HOUR: i64 = 3600;

    /// The gate. A Pi left on Raspberry Pi OS's `Europe/London` in Ohio, with
    /// coordinates typed at the installer prompt (so no wizard zone): this was
    /// a plain pass, "nothing to compare it against", while every detection
    /// was filed five hours late.
    #[test]
    fn a_london_clock_on_an_ohio_station_warns_without_a_wizard_zone() {
        // 2026-01-15 (GMT, UTC+0) and 2026-07-15 (BST, UTC+1).
        for offset in [0, HOUR] {
            let check = timezone_mismatch_check(
                Some("Europe/London".into()),
                None,
                Some("2025b"),
                Some(OHIO),
                offset,
            );
            assert_eq!(check.status, Status::Warn, "offset {offset}: {check:?}");
            assert!(
                check.message.contains("Europe/London") && check.message.contains("UTC-6"),
                "the reader needs the zone it is on and the offset the location implies: {}",
                check.message
            );
            assert!(check.message.contains("wrong hour"), "{}", check.message);
            let fix = check.remediation.expect("a warning carries a fix");
            assert!(
                fix.contains("sudo timedatectl set-timezone")
                    && fix.contains("TZ=")
                    && fix.contains(".env"),
                "both fixes, host and container: {fix}"
            );
        }
    }

    /// Counterpart: the right zone for the same coordinates passes, in winter
    /// (EST, UTC-5) and in summer (EDT, UTC-4), and says what it compared.
    #[test]
    fn a_new_york_clock_on_an_ohio_station_passes() {
        for offset in [-5 * HOUR, -4 * HOUR] {
            let check = timezone_mismatch_check(
                Some("America/New_York".into()),
                None,
                None,
                Some(OHIO),
                offset,
            );
            assert_eq!(check.status, Status::Pass, "offset {offset}: {check:?}");
            assert!(check.message.contains("longitude"), "{}", check.message);
        }
    }

    /// The zones that sit far from the sun on purpose must not be told they
    /// are wrong. Each is a case named in [`SOLAR_OFFSET_TOLERANCE_HOURS`]'s
    /// doc, at its summer offset where it has one.
    #[test]
    fn zones_legally_far_from_their_solar_offset_do_not_warn() {
        let cases: &[(&str, f64, f64, i64)] = &[
            ("Asia/Shanghai", 43.8, 87.6, 8 * HOUR),   // Urumqi, 2.2 h
            ("Asia/Shanghai", 39.47, 75.99, 8 * HOUR), // Kashgar, 2.9 h
            ("Europe/Madrid", 40.42, -3.70, 2 * HOUR), // Madrid in CEST, 2.2 h
            ("Europe/Madrid", 42.88, -8.54, 2 * HOUR), // Santiago de Compostela, 2.6 h
            ("America/Argentina/Mendoza", -32.89, -68.83, -3 * HOUR),
            ("America/Anchorage", 64.50, -165.41, -8 * HOUR), // Nome in AKDT, 3.0 h
            ("America/Adak", 51.88, -176.66, -9 * HOUR),
            ("Pacific/Kiritimati", 1.87, -157.43, 14 * HOUR), // across the date line
            ("Pacific/Apia", -13.83, -171.76, 13 * HOUR),
            ("Asia/Kolkata", 23.02, 72.57, 5 * HOUR + 1800), // half-hour zone
            ("Asia/Kathmandu", 27.72, 85.32, 5 * HOUR + 2700),
        ];
        for &(zone, lat, lon, offset) in cases {
            assert!(
                location_offset_check(zone, offset, lat, lon).is_none(),
                "{zone} at ({lat}, {lon}) is a correct zone and must not warn"
            );
        }
    }

    /// The common wrong states the check is for, and the wrap the right way
    /// round: Kiribati's own zone passes above, a London clock there does not.
    #[test]
    fn another_continents_zone_warns() {
        let cases: &[(&str, f64, f64, i64)] = &[
            ("UTC", -33.87, 151.21, 0),              // Sydney on UTC
            ("Europe/London", 28.61, 77.21, 0),      // Delhi on GMT
            ("Europe/London", 47.61, -122.33, HOUR), // Seattle on BST
            ("America/New_York", 52.52, 13.40, -5 * HOUR),
            ("Europe/London", 1.87, -157.43, 0),
        ];
        for &(zone, lat, lon, offset) in cases {
            assert!(
                location_offset_check(zone, offset, lat, lon)
                    .is_some_and(|c| c.status == Status::Warn),
                "{zone} at ({lat}, {lon}) is another continent's zone and must warn"
            );
        }
    }

    /// The wizard's looked-up zone, when there is one, is the better witness
    /// and still decides; coordinates that are not a position decide nothing.
    #[test]
    fn a_wizard_zone_outranks_the_longitude_and_nonsense_coordinates_are_ignored() {
        let check = timezone_mismatch_check(
            Some("America/New_York".into()),
            Some("America/New_York".into()),
            None,
            Some((40.0, 13.4)),
            -5 * HOUR,
        );
        assert_eq!(check.status, Status::Pass, "{check:?}");
        assert!(location_offset_check("UTC", 0, 40.0, -500.0).is_none());
        assert!(location_offset_check("UTC", 0, 95.0, -83.0).is_none());
    }
}
