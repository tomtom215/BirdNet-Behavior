//! A multi-source site's listening is reported as bounds, sensor time apart
//! (decision 2 of 2026-11-25; N06-02, N26-06), and one listened-day rule is
//! named (RES-26, N26-05).

use birdnet_db::sqlite::EffortHour;
use birdnet_db::sqlite::queries::events::SiteMap;
use birdnet_db::sqlite::queries::site_effort::{
    LISTENED_DAY, ListenedDayRule, is_listened_day, local_hour_capacity, site_effort_hours,
    site_hours,
};

fn hour(date: &str, h: u8, source: &str, secs: f64) -> EffortHour {
    EffortHour {
        date: date.to_owned(),
        hour: h,
        source: source.to_owned(),
        seconds_analysed: secs,
        seconds_up: secs,
    }
}

fn yard() -> SiteMap {
    SiteMap::identity().with("A", "yard").with("B", "yard")
}

#[test]
fn two_sources_at_one_site_give_bounds_not_a_sum() {
    let hours = [
        hour("2026-05-01", 6, "A", 1800.0),
        hour("2026-05-01", 6, "B", 2400.0),
    ];
    let got = site_hours(&hours, &yard(), |_, _| Some(3600.0));
    assert_eq!(got.len(), 1);
    let h = &got[0];
    assert_eq!(h.site, "yard");
    assert!((h.lower_bound_secs - 2400.0).abs() < 1e-9, "{h:?}");
    assert!((h.upper_bound_secs - 3600.0).abs() < 1e-9, "{h:?}");
    assert!((h.sensor_secs - 4200.0).abs() < 1e-9, "{h:?}");
}

#[test]
fn the_upper_bound_is_the_sum_when_it_fits_in_the_hour() {
    let hours = [
        hour("2026-05-01", 6, "A", 600.0),
        hour("2026-05-01", 6, "B", 900.0),
    ];
    let h = &site_hours(&hours, &yard(), |_, _| Some(3600.0))[0];
    assert!((h.lower_bound_secs - 900.0).abs() < 1e-9);
    assert!((h.upper_bound_secs - 1500.0).abs() < 1e-9);
}

#[test]
fn a_repeated_hour_holds_two_hours_and_an_unknown_one_is_not_capped() {
    let hours = [
        hour("2026-10-25", 2, "A", 3000.0),
        hour("2026-10-25", 2, "B", 3000.0),
    ];
    let fold = &site_hours(&hours, &yard(), |_, _| Some(7200.0))[0];
    assert!((fold.upper_bound_secs - 6000.0).abs() < 1e-9, "{fold:?}");
    let unknown = &site_hours(&hours, &yard(), |_, _| None)[0];
    assert!((unknown.upper_bound_secs - 6000.0).abs() < 1e-9);
}

#[test]
fn a_source_credited_past_the_hour_keeps_upper_at_least_lower() {
    // The ledger credits a segment to the hour it starts in.
    let hours = [hour("2026-05-01", 6, "A", 3610.0)];
    let h = &site_hours(&hours, &yard(), |_, _| Some(3600.0))[0];
    assert!(h.upper_bound_secs >= h.lower_bound_secs, "{h:?}");
}

#[test]
fn by_default_each_source_is_its_own_site() {
    let hours = [
        hour("2026-05-01", 6, "A", 1800.0),
        hour("2026-05-01", 6, "B", 2400.0),
    ];
    let got = site_hours(&hours, &SiteMap::identity(), |_, _| Some(3600.0));
    let sites: Vec<&str> = got.iter().map(|h| h.site.as_str()).collect();
    assert_eq!(sites, ["A", "B"]);
    for h in &got {
        assert!((h.lower_bound_secs - h.upper_bound_secs).abs() < 1e-9);
        assert!((h.lower_bound_secs - h.sensor_secs).abs() < 1e-9);
    }
}

#[test]
fn the_query_reads_the_ledger_by_site() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("schema");
    for (source, secs) in [("A", 1800.0), ("B", 2400.0), ("C", 600.0)] {
        conn.execute(
            "INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up)
             VALUES ('2026-01-10', 6, ?1, ?2, 3600)",
            rusqlite::params![source, secs],
        )
        .expect("seed");
    }
    let got = site_effort_hours(&conn, "2026-01-10", "2026-01-10", &yard()).expect("read");
    let summary: Vec<(String, usize, f64, f64, f64)> = got
        .iter()
        .map(|h| {
            (
                h.site.clone(),
                h.sources.len(),
                h.lower_bound_secs,
                h.upper_bound_secs,
                h.sensor_secs,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("C".to_owned(), 1, 600.0, 600.0, 600.0),
            ("yard".to_owned(), 2, 2400.0, 3600.0, 4200.0),
        ]
    );
}

#[test]
fn every_local_hour_of_a_january_day_has_a_capacity() {
    // No zone in current use changes its offset on these dates, so whatever the
    // host's zone, each day is 24 hours of 3600 s.
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    let cap = local_hour_capacity(&conn, "2026-01-10", "2026-01-12").expect("read");
    assert_eq!(cap.len(), 72);
    assert!(cap.values().all(|s| (*s - 3600.0).abs() < 1e-9), "{cap:?}");
}

#[test]
fn the_listened_day_rule_is_half_the_typical_day() {
    assert_eq!(LISTENED_DAY, ListenedDayRule::HalfOfTypicalDay);
    assert_eq!(LISTENED_DAY.name(), "half-of-typical-day");
    assert!(is_listened_day(43_200.0, Some(86_400.0)));
    assert!(!is_listened_day(43_199.0, Some(86_400.0)));
    // A station on a two-hours-a-day schedule listened on each of its days.
    assert!(is_listened_day(7_200.0, Some(7_200.0)));
    // A minute before a power cut is not a listened day (the old 60 s rule).
    assert!(!is_listened_day(61.0, Some(86_400.0)));
    assert!(ListenedDayRule::AtLeastSeconds(60.0).is_listened(61.0, Some(86_400.0)));
    assert!(!is_listened_day(86_400.0, None));
    assert!(ListenedDayRule::AnyAudio.is_listened(1.0, None));
    assert!(!ListenedDayRule::AnyAudio.is_listened(0.0, None));
}
