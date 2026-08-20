//! A solar recording schedule must allow a day's worth of recording *wherever
//! the station stands*, not only near Greenwich.
//!
//! # Why this file exists
//!
//! [`SolarDay`] reports sunrise and sunset as minutes since **UTC** midnight,
//! wrapped into `[0, 1440)`. For any station far enough east or west, the two
//! ends of one local day land on different UTC days, so the wrapped sunrise
//! minute is *larger* than the wrapped sunset minute — 19:33 UTC to 05:11 UTC
//! in Auckland, 09:24 UTC to 00:30 UTC in New York in June.
//!
//! [`NightInhibit`] compared them as a plain `from <= m < until` interval, which
//! is empty whenever `from > until`. The station therefore recorded **nothing at
//! all**, all day, every day, with `RECORDING_SCHEDULE=solar` — the mode the
//! manual recommends for anchoring capture to the dawn chorus.
//!
//! Every pre-existing test in this crate used London (51.5074, −0.1278), the
//! one place on Earth where the wrap never happens, which is exactly why a
//! green suite said nothing about it.

use birdnet_scheduler::{DailySchedule, Location, ScheduleConfig, SolarDay};

/// Minutes of the UTC day a solar schedule allows recording at `(lat, lon)`.
fn allowed_minutes(lat: f64, lon: f64, y: u32, m: u32, d: u32) -> usize {
    let cfg = ScheduleConfig {
        location: Some(Location::new(lat, lon).expect("valid coordinates")),
        pre_sunrise_offset_min: 0,
        post_sunset_offset_min: 0,
        night_inhibit: true,
        fixed_window: None,
    };
    let sched = DailySchedule::for_date(&cfg, y, m, d);
    (0..1440).filter(|&mm| sched.is_allowed(mm)).count()
}

/// The gate. Sixteen real stations spanning every inhabited longitude band, at
/// both solstices and both equinoxes. Each must get a plausible day's recording
/// — never zero, and never the whole 1440 minutes, which would mean the gate
/// had stopped gating rather than started working.
#[test]
fn every_inhabited_longitude_gets_a_days_recording() {
    // (name, lat, lon). Chosen to straddle the ±90° boundaries where the UTC
    // wrap begins, in both directions, and to include the far-eastern and
    // far-western extremes.
    const STATIONS: &[(&str, f64, f64)] = &[
        ("London, UK", 51.51, -0.13),
        ("Berlin, DE", 52.52, 13.40),
        ("Cape Town, ZA", -33.92, 18.42),
        ("Nairobi, KE", -1.29, 36.82),
        ("Mumbai, IN", 19.08, 72.88),
        ("Bangkok, TH", 13.76, 100.50),
        ("Beijing, CN", 39.90, 116.41),
        ("Tokyo, JP", 35.68, 139.69),
        ("Sydney, AU", -33.87, 151.21),
        ("Auckland, NZ", -36.85, 174.76),
        ("Sao Paulo, BR", -23.55, -46.63),
        ("New York, US", 40.71, -74.01),
        ("Chicago, US", 41.88, -87.63),
        ("Denver, US", 39.74, -104.99),
        ("Seattle, US", 47.61, -122.33),
        ("Honolulu, US", 21.31, -157.86),
    ];
    // Solstices and equinoxes: the days on which the wrap is at its widest and
    // its narrowest.
    const DAYS: &[(u32, u32)] = &[(3, 20), (6, 21), (9, 22), (12, 21)];

    let mut broken = Vec::new();
    for &(name, lat, lon) in STATIONS {
        for &(m, d) in DAYS {
            let mins = allowed_minutes(lat, lon, 2026, m, d);
            if mins == 0 || mins == 1440 {
                broken.push(format!("{name} on 2026-{m:02}-{d:02}: {mins} min"));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "a solar schedule must allow part of each day everywhere; these did not:\n  {}",
        broken.join("\n  ")
    );
}

/// The window has to be the *right* one, not merely non-empty: the allowed
/// minutes must be exactly the ones between sunrise and sunset.
///
/// Auckland's June window runs 19:33 UTC → 05:11 UTC the next day. A gate that
/// merely inverted the comparison would allow the complement — the night — and
/// still pass the "not zero" check above.
#[test]
fn the_allowed_minutes_are_the_daylight_ones_not_their_complement() {
    let loc = Location::new(-36.85, 174.76).expect("Auckland");
    let solar = SolarDay::for_date(loc, 2026, 6, 21).expect("solar day");
    let (rise, set) = (
        solar.sunrise_utc_min.expect("sunrise"),
        solar.sunset_utc_min.expect("sunset"),
    );
    assert!(rise > set, "this fixture only means anything if it wraps");

    let cfg = ScheduleConfig {
        location: Some(loc),
        pre_sunrise_offset_min: 0,
        post_sunset_offset_min: 0,
        night_inhibit: true,
        fixed_window: None,
    };
    let sched = DailySchedule::for_date(&cfg, 2026, 6, 21);

    assert!(sched.is_allowed(rise), "sunrise itself is daylight");
    assert!(
        sched.is_allowed((rise + 1) % 1440),
        "the minute after sunrise is daylight"
    );
    assert!(
        sched.is_allowed(set.saturating_sub(1)),
        "the minute before sunset is daylight"
    );
    assert!(!sched.is_allowed(set), "sunset is the exclusive end");
    assert!(
        !sched.is_allowed(rise.saturating_sub(1)),
        "the minute before sunrise is night"
    );
    // The midpoint of the *night* — halfway from sunset round to sunrise.
    let night_len = (rise + 1440 - set) % 1440;
    assert!(
        !sched.is_allowed((set + night_len / 2) % 1440),
        "the middle of the night is not daylight"
    );
}
