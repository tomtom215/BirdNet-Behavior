//! The station's UTC offset **on a given date**, rather than today.
//!
//! [`birdnet_db::clock::local_utc_offset_secs`] answers "what is the offset
//! now". That is the right question for a detection written live, and the
//! wrong one for anything dated on the other side of a daylight-saving change:
//! the night filter was handed the startup offset once and applied it to every
//! later day, so after the autumn change it placed sunrise an hour late in the
//! station's clock and quarantined the dawn chorus as an implausible hour until
//! the service restarted; and a backlog analysed after the change was stamped
//! with an instant an hour out.
//!
//! The lookup asks SQLite, for the same reason `birdnet_db::clock` does: its
//! `'utc'` modifier converts a local timestamp through the host's zone rules
//! **for the date given** (migration 32's backfill relies on the same thing),
//! so the answer agrees with how the rest of the station converts stored
//! wall-clock times.

/// Seconds a date-derived instant may lie in the future before it is treated
/// as impossible. A detection is analysed after its segment closed, so its
/// instant is never really ahead of the clock; the slack only absorbs
/// rounding, not a real lead.
const FUTURE_SLACK_SECS: i64 = 300;

/// The UTC offset, east-positive seconds, in force at local `date` `time`
/// under the host's zone rules. `None` when the timestamp does not name a
/// civil time or SQLite cannot answer.
///
/// In the repeated autumn hour the local time names two instants and SQLite
/// picks one; callers that know better (a live detection) keep their own
/// offset — see [`detection_instant`].
#[must_use]
pub(super) fn utc_offset_at(date: &str, time: &str) -> Option<i64> {
    let c = birdnet_core::civil::parse_civil(date, time)?;
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    );
    let conn = rusqlite::Connection::open_in_memory().ok()?;
    conn.query_row(
        "SELECT CAST(strftime('%s', ?1) AS INTEGER) - CAST(strftime('%s', ?1, 'utc') AS INTEGER)",
        [&stamp],
        |row| row.get::<_, Option<i64>>(0),
    )
    .ok()
    .flatten()
    .filter(|o| (-14 * 3600..=14 * 3600).contains(o))
}

/// The UTC offset in force at local noon on `date` — the offset a day's
/// sunrise and sunset are read in. Daylight-saving changes happen in the small
/// hours, before sunrise, in every zone that observes them.
#[must_use]
pub(super) fn utc_offset_on_day(date: &str) -> Option<i64> {
    utc_offset_at(date, "12:00:00")
}

/// The instant a detection's local `date`/`time` names.
///
/// Two readings are available: `offset_now`, the offset in force as it is
/// processed, and `offset_on_date`, the zone rules' offset for its own date.
/// For a live detection they agree, except in the repeated autumn hour, where
/// only the live offset can tell the two passes apart — the date-derived
/// reading then lies an hour in the future, which is how it is recognised.
/// For a backlog recorded on the other side of a change, the date-derived
/// reading is right and the live one is an hour out.
///
/// So: the date-derived reading, unless it is in the future.
#[must_use]
pub(super) fn detection_instant(
    date: &str,
    time: &str,
    now_secs: i64,
    offset_now: i64,
    offset_on_date: Option<i64>,
) -> Option<i64> {
    let live = birdnet_core::civil::unix_secs_from_local(date, time, offset_now)?;
    let Some(offset) = offset_on_date else {
        return Some(live);
    };
    let dated = birdnet_core::civil::unix_secs_from_local(date, time, offset)?;
    if dated > now_secs.saturating_add(FUTURE_SLACK_SECS) {
        Some(live)
    } else {
        Some(dated)
    }
}

#[cfg(test)]
mod tests {
    use super::{detection_instant, utc_offset_at, utc_offset_on_day};

    /// Env var marking the re-executed child, which runs with `TZ` pinned.
    const CHILD_MARKER: &str = "BNB_LOCAL_OFFSET_PROBE";

    /// Berlin's rules as a POSIX `TZ` string, so the test does not depend on
    /// tzdata being installed: CET (+1), CEST (+2) from the last Sunday of
    /// March to the last Sunday of October at 03:00 local.
    const BERLIN: &str = "CET-1CEST,M3.5.0,M10.5.0/3";

    /// Re-run `test_name` in a child with `TZ` set. Returns `true` in the
    /// child (carry on and assert), `false` in the parent after the child
    /// passed. `std::env::set_var` is `unsafe` in edition 2024 and `unsafe`
    /// is forbidden here, so the zone is set on a fresh process.
    fn in_berlin(test_name: &str) -> bool {
        if std::env::var_os(CHILD_MARKER).is_some() {
            return true;
        }
        let exe = std::env::current_exe().expect("test binary path");
        let status = std::process::Command::new(exe)
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env("TZ", BERLIN)
            .env(CHILD_MARKER, "1")
            .status()
            .expect("re-exec the test binary");
        assert!(status.success(), "{test_name} failed under TZ={BERLIN}");
        false
    }

    /// The offset follows the date, not the day the process started.
    #[test]
    fn the_offset_is_the_one_in_force_on_the_date_asked_about() {
        if !in_berlin(
            "daemon::local_offset::tests::the_offset_is_the_one_in_force_on_the_date_asked_about",
        ) {
            return;
        }
        assert_eq!(utc_offset_on_day("2026-10-24"), Some(7200), "CEST");
        assert_eq!(utc_offset_on_day("2026-10-26"), Some(3600), "CET");
        assert_eq!(utc_offset_on_day("2026-01-15"), Some(3600));
        assert_eq!(utc_offset_on_day("2026-07-15"), Some(7200));
        assert_eq!(utc_offset_at("2026-10-25", "04:00:00"), Some(3600));
        assert_eq!(utc_offset_at("not a date", "12:00:00"), None);
    }

    // Berlin, 2026-10-25: +2 -> +1 at 01:00Z. Local 02:30 is 00:30Z (CEST
    // pass) and 01:30Z (CET pass).
    const CEST_PASS: i64 = 1_792_888_200;
    const CET_PASS: i64 = 1_792_891_800;

    /// A backlog recorded under summer time and analysed after the change is
    /// converted with the offset of its own date, not today's.
    #[test]
    fn a_backlog_from_before_the_change_gets_its_own_dates_offset() {
        // 2026-10-24 12:00 CEST is 10:00Z; analysed two days later under CET.
        let now = 1_793_010_000; // 2026-10-26, well after
        let instant = detection_instant("2026-10-24", "12:00:00", now, 3600, Some(7200));
        assert_eq!(instant, Some(1_792_836_000));
        // Without a date-derived offset, the live one is all there is.
        assert_eq!(
            detection_instant("2026-10-24", "12:00:00", now, 3600, None),
            Some(1_792_839_600)
        );
    }

    /// The repeated hour, processed live: the zone rules name the later (CET)
    /// instant for both passes, which for the first pass is an hour in the
    /// future. The live offset is kept there, so the two passes still land an
    /// hour apart.
    #[test]
    fn the_first_pass_of_the_repeated_hour_keeps_the_live_offset() {
        let first = detection_instant("2026-10-25", "02:30:00", CEST_PASS + 20, 7200, Some(3600));
        assert_eq!(first, Some(CEST_PASS));
        let second = detection_instant("2026-10-25", "02:30:00", CET_PASS + 20, 3600, Some(3600));
        assert_eq!(second, Some(CET_PASS));
    }
}
