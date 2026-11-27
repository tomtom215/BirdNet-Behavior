//! MSC-02, RES-18, N26-06, UIC-02: an analytic rate counts events (not rows)
//! over the site's listening (bounds, not summed sensor time).

use birdnet_db::sqlite::EffortHour;
use birdnet_db::sqlite::queries::events::{EventRow, SiteMap, group_events, utc_date};
use birdnet_db::sqlite::queries::rates::{RateBin, RatePoint, rates};
use birdnet_db::sqlite::queries::site_effort::site_hours;

const DAY1: i64 = 20_574 * 86_400; // 2026-05-01 00:00 UTC

fn row(id: &str, t: i64, source: &str) -> EventRow {
    EventRow {
        row_id: id.to_owned(),
        instant: t,
        sci_name: "Turdus merula".to_owned(),
        com_name: "Blackbird".to_owned(),
        source: Some(source.to_owned()),
        confidence: 0.9,
    }
}

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

/// Local time is UTC in these tests.
fn utc(t: i64) -> Option<(String, u8)> {
    Some((utc_date(t), u8::try_from(t.rem_euclid(86_400) / 3600).ok()?))
}

fn rate_of(
    rows: &[EventRow],
    hours: &[EffortHour],
    sites: &SiteMap,
    bin: RateBin,
) -> Vec<RatePoint> {
    let events = group_events(rows, 4.0, sites);
    let h = site_hours(hours, sites, |_, _| Some(3600.0));
    rates(&events, &h, bin, utc)
}

/// Ten calls in the 06:00 hour of 1 May, a minute apart.
fn calls(source: &str, every: i64) -> Vec<EventRow> {
    (0_i64..10)
        .filter(|i| i % every == 0)
        .map(|i| row(&format!("{source}{i}"), DAY1 + 6 * 3600 + 60 * i, source))
        .collect()
}

#[test]
fn a_quieter_microphone_added_to_a_site_does_not_lower_its_rate() {
    // A alone: listened the whole hour, heard ten calls.
    let alone = rate_of(
        &calls("A", 1),
        &[hour("2026-05-01", 6, "A", 3600.0)],
        &yard(),
        RateBin::Day,
    );
    assert_eq!(alone.len(), 1);
    assert!(
        (alone[0].rate_low.unwrap() - 10.0).abs() < 1e-9,
        "{alone:?}"
    );
    // B beside it: listened ten minutes, heard every fifth of the same calls.
    let mut rows = calls("A", 1);
    rows.extend(calls("B", 5));
    let both = rate_of(
        &rows,
        &[
            hour("2026-05-01", 6, "A", 3600.0),
            hour("2026-05-01", 6, "B", 600.0),
        ],
        &yard(),
        RateBin::Day,
    );
    let p = &both[0];
    assert_eq!((p.events, p.rows), (10, 12), "{p:?}");
    assert!((p.sensor_secs - 4200.0).abs() < 1e-9);
    for r in [p.rate_low, p.rate_high] {
        let r = r.expect("a rate");
        assert!(
            (r - 10.0).abs() < 1e-9,
            "adding a quieter source changed the site's rate from 10 to {r}: {p:?}"
        );
    }
}

#[test]
fn two_sources_hearing_every_call_count_each_call_once() {
    let mut rows = calls("A", 1);
    rows.extend(calls("B", 1));
    let p = &rate_of(
        &rows,
        &[
            hour("2026-05-01", 6, "A", 3600.0),
            hour("2026-05-01", 6, "B", 3600.0),
        ],
        &yard(),
        RateBin::Day,
    )[0];
    assert_eq!((p.events, p.rows), (10, 20), "{p:?}");
    assert!((p.rate_high.unwrap() - 10.0).abs() < 1e-9, "{p:?}");
    // Counterpart: as two sites (the default) they are two lines of ten.
    let apart = rate_of(
        &rows,
        &[
            hour("2026-05-01", 6, "A", 3600.0),
            hour("2026-05-01", 6, "B", 3600.0),
        ],
        &SiteMap::identity(),
        RateBin::Day,
    );
    assert_eq!(apart.len(), 2);
    assert!(apart.iter().all(|p| p.events == 10));
}

#[test]
fn half_the_listening_and_the_same_events_is_twice_the_rate() {
    let mut rows = calls("A", 1);
    rows.extend((0..10).map(|i| row(&format!("d2-{i}"), DAY1 + 86_400 + 6 * 3600 + 60 * i, "A")));
    let pts = rate_of(
        &rows,
        &[
            hour("2026-05-01", 6, "A", 3600.0),
            hour("2026-05-02", 6, "A", 1800.0),
        ],
        &SiteMap::identity(),
        RateBin::Day,
    );
    assert_eq!(pts.len(), 2, "{pts:?}");
    let (d1, d2) = (pts[0].rate_low.unwrap(), pts[1].rate_low.unwrap());
    assert!((d2 / d1 - 2.0).abs() < 1e-9, "{d1} then {d2}");
    assert!(pts[1].ci_lower.unwrap() < d2 && d2 < pts[1].ci_upper.unwrap());
}

#[test]
fn events_in_an_unlistened_hour_are_not_a_rate() {
    let pts = rate_of(
        &calls("A", 1),
        &[hour("2026-05-01", 9, "A", 3600.0)],
        &SiteMap::identity(),
        RateBin::Day,
    );
    let p = &pts[0];
    assert_eq!((p.events, p.events_outside_listening), (0, 10), "{p:?}");
    assert_eq!(p.status, "measured");
    assert!(
        p.rate_low.unwrap().abs() < 1e-12,
        "listened 09:00 and heard none"
    );
    // With no ledger at all there is no rate, not a rate of zero.
    let none = rate_of(
        &calls("A", 1),
        &[],
        &SiteMap::identity(),
        RateBin::HourOfDay,
    );
    assert_eq!(none[0].status, "no_effort");
    assert_eq!(none[0].rate_low, None);
}
