//! The station's local UTC offset.
//!
//! # Why this lives in the database crate
//!
//! The workspace carries no `chrono`/`time` dependency and forbids `unsafe`, so
//! neither `localtime_r` nor a tz-database parser is reachable. SQLite's
//! `localtime` modifier consults the same zoneinfo everything else on the box
//! does — including the `date('now','localtime')` comparisons the detection
//! queries already make — so asking SQLite is both the cheapest way to learn the
//! offset and the only way to be sure the answer *agrees* with how detections
//! are stored. This crate owns SQLite, so this is where that rule lives.
//!
//! # Who needs it
//!
//! Two subsystems, which is why it is no longer a private helper inside the web
//! crate's page module:
//!
//! * the **web** pages, which plot local-hour bars and a local "now" marker;
//! * **capture**, whose segment filenames carry local civil time (`arecord
//!   --use-strftime` called `strftime` on `localtime()`, and the in-process
//!   segment writer that replaced it has to produce the same names).
//!
//! A second copy of this rule would let those two disagree, and the way that
//! failure presents — timestamps quietly two hours out on a CEST station — is
//! exactly the bug class this project keeps paying for.

use std::sync::atomic::{AtomicI64, Ordering};

/// How long a computed offset is reused before being re-read.
///
/// The value only moves at a daylight-saving boundary, so opening a connection
/// more often than this would be absurd; the resulting staleness is at most a
/// minute, twice a year.
const CACHE_SECS: i64 = 60;

/// Last computed offset. Seeded to `0` (UTC), which is also the fallback when
/// SQLite cannot answer.
static OFFSET_SECS: AtomicI64 = AtomicI64::new(0);

/// When [`OFFSET_SECS`] was computed. `i64::MIN` forces a first read.
static COMPUTED_AT: AtomicI64 = AtomicI64::new(i64::MIN);

/// Seconds since the Unix epoch, saturating to `0` before it.
fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The system's UTC offset in seconds, east-positive (CEST → `7200`).
///
/// Cached for a minute. Falls back to the last known value
/// — `0`/UTC if there has never been one — when SQLite cannot answer, rather
/// than failing: a station that cannot read its own time zone should keep
/// recording with slightly wrong filenames, not stop.
#[must_use]
pub fn local_utc_offset_secs() -> i64 {
    let now = unix_secs();
    if now.saturating_sub(COMPUTED_AT.load(Ordering::Relaxed)) < CACHE_SECS {
        return OFFSET_SECS.load(Ordering::Relaxed);
    }
    let offset =
        query_local_utc_offset_secs().unwrap_or_else(|| OFFSET_SECS.load(Ordering::Relaxed));
    OFFSET_SECS.store(offset, Ordering::Relaxed);
    COMPUTED_AT.store(now, Ordering::Relaxed);
    offset
}

/// Ask SQLite for the current UTC offset. `None` if the query fails, so the
/// caller can fall back rather than pretend the station is in UTC.
fn query_local_utc_offset_secs() -> Option<i64> {
    let conn = rusqlite::Connection::open_in_memory().ok()?;
    conn.query_row(
        "SELECT CAST(ROUND((julianday('now','localtime') - julianday('now')) * 86400.0) AS INTEGER)",
        [],
        |row| row.get::<_, i64>(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the runner's time zone is, the answer has to be a real one: a
    /// whole number of minutes inside the range zones actually span. A broken
    /// query would surface here as a wild value rather than a plausible-looking
    /// wrong one.
    #[test]
    fn offset_is_a_real_time_zone() {
        let offset = local_utc_offset_secs();
        assert!(
            (-12 * 3600..=14 * 3600).contains(&offset),
            "offset {offset}s is outside the range of real time zones"
        );
        assert_eq!(offset % 60, 0, "no time zone has a sub-minute offset");
    }

    #[test]
    fn repeated_reads_agree() {
        // The cache must not change the answer, only how often it is computed.
        assert_eq!(local_utc_offset_secs(), local_utc_offset_secs());
    }

    /// The uncached query must agree with the cached accessor — this is what
    /// pins the cache to the SQLite answer rather than to a stale seed.
    #[test]
    fn query_matches_the_cached_value() {
        let direct = query_local_utc_offset_secs().expect("SQLite answers");
        assert_eq!(direct, local_utc_offset_secs());
    }
}
