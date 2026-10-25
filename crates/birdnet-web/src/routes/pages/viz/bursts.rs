//! Bursts of singing as a timeline: one lane per day, the hours of the day along it, one bar
//! for every burst.
//!
//! The data is the behavioural engine's `sessionize`: runs of one species' detections with no
//! gap longer than the session gap (30 minutes), each with its start, its end and how many
//! detections it held. A burst is drawn exactly where it began and exactly as long as it
//! lasted, so a bar is a fact about the day and nothing is invented between them. Bursts of
//! different species can overlap in time, so within a day they are packed into as few
//! sub-lanes as it takes ([`pack_lanes`]), the way a Gantt chart is, and a burst is never
//! drawn over another. The colour is the species, in the order of how many bursts it had in
//! what is shown, and the legend brings one species forward.
//!
//! A bar under four minutes long would be a sliver a finger cannot find, so it is drawn at
//! four and its reading says how long it really lasted.
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#![allow(clippy::items_after_statements, clippy::many_single_char_names)]

use std::fmt::Write as _;

use super::kit::{bar_path, slot_class, tip};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// One burst: a species singing, on one day, from `start_secs` to `end_secs` after midnight.
pub struct Burst {
    /// The species as the station displays it.
    pub species: String,
    /// Its colour slot in this chart.
    pub slot: usize,
    /// The local date, `YYYY-MM-DD`.
    pub date: String,
    /// Seconds after midnight the burst began.
    pub start_secs: u32,
    /// Seconds after midnight it ended (on the same day: one that ran past midnight is cut there).
    pub end_secs: u32,
    /// Detections in it.
    pub detections: u32,
}

/// The sub-lane each span goes in, so that no two spans in one sub-lane overlap or come
/// closer than `gap`. Greedy by start time (which is optimal for intervals: it uses as few
/// sub-lanes as the busiest instant needs). Spans are `(start, end)` and may arrive in any
/// order; the answer is in the order they arrived.
pub fn pack_lanes(spans: &[(f64, f64)], gap: f64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| spans[a].0.total_cmp(&spans[b].0).then(a.cmp(&b)));
    let mut lane_end: Vec<f64> = Vec::new();
    let mut out = vec![0; spans.len()];
    for i in order {
        let (s, e) = spans[i];
        let lane = lane_end.iter().position(|&end| end + gap <= s);
        let lane = lane.unwrap_or_else(|| {
            lane_end.push(f64::MIN);
            lane_end.len() - 1
        });
        lane_end[lane] = e;
        out[i] = lane;
    }
    out
}

/// `1h 05m`, `15m 19s`, `45s`: how long a burst lasted.
pub fn duration_text(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// `06:23` for seconds after midnight.
fn clock(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600 % 24, secs % 3600 / 60)
}

/// The weekday and day of `date` (`Wed 21 Oct`), or the date as given if it does not parse.
fn day_name(date: &str) -> String {
    let parse = |r: std::ops::Range<usize>| date.get(r).and_then(|s| s.parse::<u32>().ok());
    let (Some(m), Some(d)) = (parse(5..7), parse(8..10)) else {
        return date.to_owned();
    };
    let days = crate::routes::pages::date_to_epoch_days(date);
    let weekday = ((days + 4) % 7) as usize; // 1970-01-01 was a Thursday; WEEKDAYS starts Sunday
    match (
        super::kit::WEEKDAYS.get(weekday),
        super::kit::MONTHS.get(m as usize - 1),
    ) {
        (Some(w), Some(mo)) => format!("{w} {d} {mo}"),
        _ => date.to_owned(),
    }
}

/// `21 Oct` for `date`: the lane's label (the weekday is in a burst's reading).
fn short_day(date: &str) -> String {
    let full = day_name(date);
    full.split_once(' ')
        .map_or_else(|| full.clone(), |(_, rest)| rest.to_owned())
}

/// The timeline of `bursts` over the `max_days` most recent days that have one, newest on top,
/// with a legend of the species (`legend` holds `(name, slot)`, strongest first).
#[must_use]
pub fn burst_timeline(bursts: &[Burst], legend: &[(String, usize)], max_days: usize) -> String {
    if bursts.is_empty() {
        return EMPTY.to_string();
    }
    let mut days: Vec<&str> = bursts.iter().map(|b| b.date.as_str()).collect();
    days.sort_unstable();
    days.dedup();
    days.reverse();
    days.truncate(max_days);

    const W: f64 = 640.0;
    const PAD_L: f64 = 56.0;
    const PAD_R: f64 = 10.0;
    const TOP: f64 = 22.0;
    const ROW: f64 = 14.0; // a sub-lane
    const BAR: f64 = 10.0;
    const MIN_W: f64 = 4.0;
    let inner = W - PAD_L - PAD_R;
    let x = |secs: f64| PAD_L + secs / 86_400.0 * inner;

    // Pack each day's bursts, then stack the days.
    struct Lane<'a> {
        date: &'a str,
        top: f64,
        height: f64,
        placed: Vec<(usize, usize)>, // (burst index, sub-lane)
    }
    let mut lanes: Vec<Lane<'_>> = Vec::new();
    let mut y = TOP;
    for date in &days {
        let in_day: Vec<usize> = bursts
            .iter()
            .enumerate()
            .filter(|(_, b)| b.date == *date)
            .map(|(i, _)| i)
            .collect();
        let spans: Vec<(f64, f64)> = in_day
            .iter()
            .map(|&i| {
                let b = &bursts[i];
                let (s, e) = (x(f64::from(b.start_secs)), x(f64::from(b.end_secs)));
                (s, e.max(s + MIN_W))
            })
            .collect();
        let sub = pack_lanes(&spans, 2.0);
        let rows = sub.iter().copied().max().map_or(1, |m| m + 1);
        let height = rows as f64 * ROW + 10.0;
        lanes.push(Lane {
            date,
            top: y,
            height,
            placed: in_day.into_iter().zip(sub).collect(),
        });
        y += height;
    }
    let h = y + 6.0;

    let mut s = format!(
        r#"<div class="ck bt-fig"><svg viewBox="0 0 {W} {h:.0}" width="100%" role="img" class="bt-svg ck-in" data-ck="marks" data-ck-dim="series" data-ck-id="bursts" data-ck-label="Left and right arrows step through the bursts, newest first">"#,
    );
    s.push_str(&svg_a11y(
        "Bursts of singing by day",
        &format!(
            "{} bursts over {} days. Each row is a day with the hours of the day along it; each bar is one burst of a species, from the moment it began to the moment it ended.",
            bursts.iter().filter(|b| days.contains(&b.date.as_str())).count(),
            days.len(),
        ),
    ));
    // Hours.
    for (hr, label) in [(0, "12a"), (6, "6a"), (12, "12p"), (18, "6p"), (24, "12a")] {
        let xx = x(f64::from(hr) * 3600.0);
        let anchor = match hr {
            0 => "start",
            24 => "end",
            _ => "middle",
        };
        let _ = write!(
            s,
            r#"<line class="ck-grid" x1="{xx:.1}" y1="{TOP}" x2="{xx:.1}" y2="{bot:.1}"/><text class="ck-lbl bt-hr mono" x="{xx:.1}" y="12" text-anchor="{anchor}">{label}</text>"#,
            bot = h - 6.0,
        );
    }
    let mut index = 0usize;
    for lane in &lanes {
        let _ = write!(
            s,
            r#"<text class="bt-day" x="0" y="{ty:.1}">{day}</text><line class="ck-grid" x1="{PAD_L}" y1="{ly:.1}" x2="{xr:.1}" y2="{ly:.1}"/>"#,
            ty = lane.top + 14.0,
            day = escape_html(&short_day(lane.date)),
            ly = lane.top + lane.height,
            xr = W - PAD_R,
        );
        let mut mine = lane.placed.clone();
        mine.sort_by_key(|&(i, _)| bursts[i].start_secs);
        for (i, sub) in mine {
            let b = &bursts[i];
            let x0 = x(f64::from(b.start_secs));
            let w = (x(f64::from(b.end_secs)) - x0).max(MIN_W);
            let by = lane.top + 5.0 + sub as f64 * ROW;
            let data_tip = tip(
                &b.species,
                &[
                    (
                        "Began",
                        &clock(b.start_secs),
                        Some(&crate::routes::pages::atoms::series_color(b.slot, 0)),
                    ),
                    (
                        "Lasted",
                        &duration_text(b.end_secs.saturating_sub(b.start_secs)),
                        None,
                    ),
                    ("Detections", &b.detections.to_string(), None),
                ],
                Some(&day_name(&b.date)),
            );
            let _ = write!(
                s,
                r#"<path class="bt-bar {slot}" d="{d}" data-tip="{data_tip}" data-ck-i="{index}" data-ck-s="{slot_n}" data-ck-f="{slot_n}"/>"#,
                slot = slot_class(b.slot),
                slot_n = b.slot,
                d = bar_path(x0, by, w, BAR, 3.0),
            );
            index += 1;
        }
    }
    s.push_str("</svg>");
    s.push_str(r#"<div class="viz-legend ck-legend bt-key">"#);
    for (name, slot) in legend {
        let _ = write!(
            s,
            r#"<button type="button" class="ck-leg bnb-meta viz-legend-item" data-ck-s="{slot}" aria-pressed="false"><span class="viz-swatch ck-sw {class}"></span>{name}</button>"#,
            class = slot_class(*slot),
            name = escape_html(name),
        );
    }
    s.push_str("</div></div>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(species: &str, slot: usize, date: &str, start: u32, end: u32) -> Burst {
        Burst {
            species: species.to_owned(),
            slot,
            date: date.to_owned(),
            start_secs: start,
            end_secs: end,
            detections: 3,
        }
    }

    /// Overlapping bursts go in different sub-lanes and never share one: with the old table
    /// two species singing at 06:20 were two rows far apart; on a timeline they are two
    /// bars, and one drawn over the other would hide a fact.
    #[test]
    fn overlapping_bursts_are_stacked_and_never_overlap() {
        let spans = [
            (10.0, 50.0),
            (20.0, 40.0),
            (45.0, 90.0),
            (60.0, 70.0),
            (100.0, 120.0),
        ];
        let lanes = pack_lanes(&spans, 2.0);
        for i in 0..spans.len() {
            for j in i + 1..spans.len() {
                if lanes[i] == lanes[j] {
                    let (a, b) = (spans[i], spans[j]);
                    assert!(
                        a.1 + 2.0 <= b.0 || b.1 + 2.0 <= a.0,
                        "{a:?} and {b:?} share lane {}",
                        lanes[i]
                    );
                }
            }
        }
        // As few lanes as the busiest instant needs: at x = 48 three spans (10-50, 20-40
        // is over by then, 45-90) hold the line, so two lanes; with 20-40 inside 10-50 at x=30.
        assert_eq!(lanes.iter().copied().max(), Some(1));
        // Nothing to pack, nothing packed.
        assert!(pack_lanes(&[], 2.0).is_empty());
        // Spans that merely touch with no gap do not fit side by side.
        assert_eq!(pack_lanes(&[(0.0, 10.0), (10.0, 20.0)], 2.0), vec![0, 1]);
        assert_eq!(pack_lanes(&[(0.0, 10.0), (12.0, 20.0)], 2.0), vec![0, 0]);
    }

    #[test]
    fn nothing_to_show_is_the_placeholder() {
        assert_eq!(burst_timeline(&[], &[], 7), EMPTY);
    }

    /// A bar starts where the burst began and is as long as it lasted: 06:00 to 07:00 on a
    /// 24-hour axis is a quarter of the way along and a twenty-fourth of the width.
    #[test]
    fn a_bar_is_where_and_as_long_as_the_burst() {
        let b = [burst("American Robin", 0, "2026-10-21", 6 * 3600, 7 * 3600)];
        let svg = burst_timeline(&b, &[("American Robin".to_owned(), 0)], 7);
        let d = svg
            .split("<path class=\"bt-bar ck-c1\" d=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("a bar");
        let nums: Vec<f64> = d
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse().ok())
            .collect();
        let (lo, hi) = nums
            .iter()
            .step_by(2)
            .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        let inner = 640.0 - 56.0 - 10.0;
        assert!((lo - (56.0 + inner / 4.0)).abs() < 0.6, "starts at {lo}");
        assert!(
            ((hi - lo) - inner / 24.0).abs() < 0.6,
            "is {} wide",
            hi - lo
        );
        assert!(
            svg.contains("Wed 21 Oct"),
            "the weekday is in the reading: {svg:.200}"
        );
        assert!(
            svg.contains(">21 Oct<"),
            "the lane says the day: {svg:.200}"
        );
        assert!(
            svg.contains("1h 00m"),
            "the reading says how long it lasted"
        );
    }

    /// A burst shorter than four minutes is still four units wide to point at, and its reading
    /// still says how long it really lasted.
    #[test]
    fn a_very_short_burst_is_still_a_target_and_says_how_long_it_was() {
        let b = [burst("Blue Jay", 1, "2026-10-21", 8 * 3600, 8 * 3600 + 20)];
        let svg = burst_timeline(&b, &[("Blue Jay".to_owned(), 1)], 7);
        assert!(svg.contains("20s"), "{svg:.300}");
    }

    /// Only the newest days that have a burst are shown, newest first.
    #[test]
    fn the_newest_days_are_shown_newest_first() {
        let b = [
            burst("A", 0, "2026-10-18", 3600, 4000),
            burst("A", 0, "2026-10-21", 3600, 4000),
            burst("A", 0, "2026-10-20", 3600, 4000),
            burst("A", 0, "2026-10-19", 3600, 4000),
        ];
        let svg = burst_timeline(&b, &[("A".to_owned(), 0)], 3);
        let at = |d: &str| svg.find(d).unwrap_or(usize::MAX);
        assert!(at("21 Oct") < at("20 Oct") && at("20 Oct") < at("19 Oct"));
        assert!(!svg.contains("18 Oct"), "only three days");
        assert_eq!(svg.matches("<path class=\"bt-bar").count(), 3);
    }

    #[test]
    fn durations_and_days_read_naturally() {
        assert_eq!(duration_text(45), "45s");
        assert_eq!(duration_text(15 * 60 + 19), "15m 19s");
        assert_eq!(duration_text(3600 + 5 * 60), "1h 05m");
        assert_eq!(day_name("2026-10-21"), "Wed 21 Oct");
        assert_eq!(day_name("2026-01-01"), "Thu 1 Jan");
        assert_eq!(day_name("oops"), "oops");
    }
}
