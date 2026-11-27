//! When a recording started, from what the operator said, and the station's
//! name for each instant after it.
//!
//! A recording's start is never read from the host clock or from a file's
//! modification time: it is given (`--ingest-start`, an instant with its UTC
//! offset written out) or read from the recording's name under a rule the
//! operator chose, with the offset that name was written in
//! (`--ingest-name-rule` and `--ingest-utc-offset`). With neither, the
//! recording is refused; nothing is guessed.
//!
//! The station names a segment by its **local** wall-clock time
//! (`YYYY-MM-DD-birdnet-<source>-HH:MM:SS.wav`) and the detection daemon turns
//! that back into an instant through the host's zone rules for that date
//! (`daemon::local_offset`). So a piece is named by converting its instant to
//! local time through the same rules (SQLite's `localtime`, as `birdnet_db`'s
//! clock and the daemon use), and then checked by converting back: in the
//! repeated hour of an autumn change two instants share one local name, and
//! the daemon will read it as one of them. Such a piece is reported as
//! ambiguous, an hour's uncertainty in its detections' times, rather than
//! named as if it were not.

use birdnet_core::civil;

/// The furthest from UTC any zone is, in seconds.
const MAX_OFFSET_SECS: i64 = 14 * 3600;

/// Parse a UTC offset: `Z`, `UTC`, or `+HH:MM` / `-HH:MM`.
///
/// # Errors
///
/// Anything else, or an offset past ±14:00.
pub fn parse_utc_offset(text: &str) -> Result<i64, String> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("z") || t.eq_ignore_ascii_case("utc") {
        return Ok(0);
    }
    let bad = || format!("{text:?} is not a UTC offset: write Z, or +HH:MM / -HH:MM");
    let (sign, rest) = match t.as_bytes().first() {
        Some(b'+') => (1, &t[1..]),
        Some(b'-') => (-1, &t[1..]),
        _ => return Err(bad()),
    };
    let (h, m) = rest.split_once(':').ok_or_else(bad)?;
    if h.len() != 2 || m.len() != 2 {
        return Err(bad());
    }
    let h: i64 = h.parse().map_err(|_| bad())?;
    let m: i64 = m.parse().map_err(|_| bad())?;
    let secs = sign * (h * 3600 + m * 60);
    if m > 59 || secs.abs() > MAX_OFFSET_SECS {
        return Err(bad());
    }
    Ok(secs)
}

/// `date` and `time` (local at `offset_secs` east of UTC) as an instant.
fn instant(date: &str, time: &str, offset_secs: i64) -> Option<i64> {
    let c = civil::parse_civil(date, time)?;
    if c.day > civil::days_in_month(c.year, c.month) {
        return None;
    }
    Some(civil::unix_secs_from_civil(&c) - offset_secs)
}

/// Parse an instant written with its offset: `YYYY-MM-DDTHH:MM:SS` followed by
/// `Z` or `±HH:MM` (a space for the `T` is accepted).
///
/// # Errors
///
/// No offset (a local time would be read in whichever zone this host is in,
/// which is the guess this refuses), or anything that is not such an instant.
pub fn parse_start(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let bad = || {
        format!(
            "{text:?} is not a start time: write it with its UTC offset, as \
             2026-05-19T04:30:00Z or 2026-05-19T06:30:00+02:00"
        )
    };
    if t.len() < 20 || !t.is_char_boundary(19) {
        return Err(bad());
    }
    let (stamp, offset) = t.split_at(19);
    let (date, time) = stamp.split_once(['T', ' ']).ok_or_else(bad)?;
    let offset = parse_utc_offset(offset).map_err(|_| bad())?;
    instant(date, time, offset).ok_or_else(bad)
}

/// The rules a recording's start can be read from its name by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRule {
    /// The last `YYYYMMDD_HHMMSS` in the name, as `AudioMoth` (`20260519_043000`)
    /// and Song Meter (`SMA01234_20260519_043000`) write it, in the offset the
    /// operator gives: `AudioMoth` writes UTC unless set otherwise, Song Meters
    /// write the recorder's local time, and the name says neither.
    YyyymmddHhmmss,
}

impl NameRule {
    /// Parse the `--ingest-name-rule` value.
    ///
    /// # Errors
    ///
    /// An unknown rule.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "yyyymmdd_hhmmss" | "audiomoth" | "songmeter" => Ok(Self::YyyymmddHhmmss),
            other => Err(format!(
                "{other:?} is not a name rule: the one there is is yyyymmdd_hhmmss \
                 (AudioMoth and Song Meter names)"
            )),
        }
    }

    /// The start `stem` names, at `offset_secs` east of UTC.
    #[must_use]
    pub fn start(self, stem: &str, offset_secs: i64) -> Option<i64> {
        let Self::YyyymmddHhmmss = self;
        let b = stem.as_bytes();
        (0..b.len().saturating_sub(14)).rev().find_map(|i| {
            let w = &b[i..i + 15];
            let digits = |r: std::ops::Range<usize>| w[r].iter().all(u8::is_ascii_digit);
            if !(digits(0..8) && w[8] == b'_' && digits(9..15)) {
                return None;
            }
            // Not a run of digits that merely contains the shape.
            if i > 0 && b[i - 1].is_ascii_digit() || b.get(i + 15).is_some_and(u8::is_ascii_digit) {
                return None;
            }
            let s = std::str::from_utf8(w).ok()?;
            let date = format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8]);
            let time = format!("{}:{}:{}", &s[9..11], &s[11..13], &s[13..15]);
            instant(&date, &time, offset_secs)
        })
    }
}

/// An instant's name in the station's local time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStamp {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM:SS`.
    pub time: String,
    /// The daemon will read this name back as a different instant (the
    /// repeated hour of an autumn change).
    pub ambiguous: bool,
}

/// `unix_secs` in the host's local time, as the daemon will read it back.
#[must_use]
pub fn station_local(unix_secs: i64) -> Option<LocalStamp> {
    let conn = rusqlite::Connection::open_in_memory().ok()?;
    let (date, time): (String, String) = conn
        .query_row(
            "SELECT strftime('%Y-%m-%d', ?1, 'unixepoch', 'localtime'), \
                    strftime('%H:%M:%S', ?1, 'unixepoch', 'localtime')",
            [unix_secs],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let back: Option<i64> = conn
        .query_row(
            "SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER)",
            [format!("{date} {time}")],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    Some(LocalStamp {
        ambiguous: back != Some(unix_secs),
        date,
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offset_is_written_out_or_refused() {
        assert_eq!(parse_utc_offset("Z"), Ok(0));
        assert_eq!(parse_utc_offset("utc"), Ok(0));
        assert_eq!(parse_utc_offset("+02:00"), Ok(7200));
        assert_eq!(parse_utc_offset("-05:30"), Ok(-19_800));
        for bad in ["", "2", "+2:00", "+0200", "+15:00", "+01:60", "CET"] {
            assert!(parse_utc_offset(bad).is_err(), "{bad:?}");
        }
    }

    /// A start with no offset would be read in whatever zone this host is
    /// in: refused, not guessed.
    #[test]
    fn a_start_needs_its_offset() {
        assert_eq!(parse_start("2026-05-19T04:30:00Z"), Ok(1_779_165_000));
        assert_eq!(parse_start("2026-05-19T06:30:00+02:00"), Ok(1_779_165_000));
        assert_eq!(parse_start("2026-05-19 06:30:00+02:00"), Ok(1_779_165_000));
        for bad in [
            "2026-05-19T04:30:00",
            "2026-05-19",
            "2026-02-30T04:30:00Z",
            "2026-05-19T25:30:00Z",
            "yesterday",
        ] {
            let e = parse_start(bad).expect_err(bad);
            assert!(e.contains("UTC offset"), "{e}");
        }
    }

    #[test]
    fn audiomoth_and_song_meter_names_give_their_start() {
        let r = NameRule::YyyymmddHhmmss;
        assert_eq!(r.start("20260519_043000", 0), Some(1_779_165_000));
        assert_eq!(
            r.start("SMA01234_20260519_063000", 7200),
            Some(1_779_165_000)
        );
        // The last such stamp wins; a longer run of digits is not one.
        assert_eq!(
            r.start("20250101_000000_20260519_043000", 0),
            Some(1_779_165_000)
        );
        assert_eq!(r.start("120260519_043000", 0), None);
        assert_eq!(r.start("20260519_0430001", 0), None);
        assert_eq!(r.start("recording", 0), None);
        assert_eq!(r.start("20261319_043000", 0), None, "month 13");
        assert!(NameRule::parse("audiomoth").is_ok());
        assert!(NameRule::parse("birdnet").is_err());
    }

    /// Env var marking the re-executed child, which runs with `TZ` pinned.
    const CHILD: &str = "BNB_INGEST_TZ_PROBE";

    /// Run `test` again in a child with `TZ=tz`; `true` in the child.
    fn in_zone(tz: &str, test: &str) -> bool {
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env("TZ", tz)
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success(), "{test} failed under TZ={tz}");
        false
    }

    /// A piece is named in the station's local time by the date's own rules,
    /// and a name in Berlin's repeated autumn hour, which the daemon will
    /// read as only one of its two instants, is reported ambiguous.
    #[test]
    fn a_piece_is_named_in_local_time_and_the_repeated_hour_is_flagged() {
        if !in_zone(
            "CET-1CEST,M3.5.0,M10.5.0/3",
            "ingest::start::tests::a_piece_is_named_in_local_time_and_the_repeated_hour_is_flagged",
        ) {
            return;
        }
        let summer = station_local(parse_start("2026-05-19T04:30:00Z").unwrap()).unwrap();
        assert_eq!(
            (summer.date.as_str(), summer.time.as_str()),
            ("2026-05-19", "06:30:00")
        );
        assert!(!summer.ambiguous);
        let winter = station_local(parse_start("2026-01-19T04:30:00Z").unwrap()).unwrap();
        assert_eq!(winter.time, "05:30:00");
        assert!(!winter.ambiguous);
        // 00:30Z is 02:30 CEST and 01:30Z is 02:30 CET on 2026-10-25.
        let first = station_local(parse_start("2026-10-25T00:30:00Z").unwrap()).unwrap();
        let second = station_local(parse_start("2026-10-25T01:30:00Z").unwrap()).unwrap();
        assert_eq!(first.time, "02:30:00");
        assert_eq!(second.time, "02:30:00");
        assert!(
            first.ambiguous ^ second.ambiguous,
            "exactly one of the two is read back as the other: {first:?} {second:?}"
        );
    }
}
