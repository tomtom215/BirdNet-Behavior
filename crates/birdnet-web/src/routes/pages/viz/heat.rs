//! Grids of cells: the hour-by-weekday activity grid and the year calendar.
//!
//! Both colour a count through one of the sequential ramps (`--seq-1` … `--seq-7`, the
//! sun ramp for time of day; `--seq-moss-*`, the brand green, for the year), spaced by
//! the square root of the count so that one detection and ten are told apart on a day
//! whose busiest hour had two hundred. What a cell says is never colour alone:
//! every cell has a tooltip and a keyboard stop, and the ramp's scale is printed under
//! the chart with the numbers the steps start at.
//!
//! Three states are kept apart, because a reader will otherwise read them as one:
//!
//! * **a count**: a step on the ramp;
//! * **none heard** — the station listened and heard nothing: an empty cell (`--chart-track`);
//! * **not listening** — the ledger (`recording_effort`) has no audio for that day: a
//!   hatched cell that says so. Only drawn where the ledger holds something for the
//!   period at all; a station with no ledger claims nothing.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::kit;
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// The class that paints a cell for sequential step `step` (1-7) of `ramp`.
fn step_class(ramp: Ramp, step: usize) -> String {
    match ramp {
        Ramp::Sun => format!("ck-q{step}"),
        Ramp::Moss => format!("ck-m{step}"),
    }
}

/// Which sequential ramp.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ramp {
    /// Sand to brick (dark theme: ember to gold): how much, the default heat map.
    Sun,
    /// The brand green: a year of days.
    Moss,
}

/// The scale printed under a heat map, as HTML (so it wraps and never competes with the
/// chart's own text for room): the step swatches, what they stand for in counts, and
/// whatever else the chart wants to say beside it (`extra` is escaped here).
fn scale_legend(ramp: Ramp, max: f64, what: &str, extra: &[String], not_listening: bool) -> String {
    let mut s = String::from(r#"<div class="ck-scale"><span class="bnb-meta">Fewer</span><span class="ck-steps" aria-hidden="true">"#);
    for k in 1..=7_usize {
        let _ = write!(s, r#"<i class="ck-sw {}"></i>"#, step_class(ramp, k));
    }
    let _ = write!(
        s,
        r#"</span><span class="bnb-meta">More</span><span class="bnb-meta">{what}: 1 to {m}, in steps by square root</span>"#,
        what = escape_html(what),
        m = kit::compact((max * 10.0).round() / 10.0),
    );
    if not_listening {
        s.push_str(r#"<span class="bnb-meta"><i class="ck-off" aria-hidden="true"></i> Not listening</span>"#);
    }
    for e in extra {
        let _ = write!(s, r#"<span class="bnb-meta ck-scale-note">{}</span>"#, escape_html(e));
    }
    s.push_str("</div>");
    s
}

// ── hour × weekday ──────────────────────────────────────────────────────────────

/// What the weekday grid draws.
pub(crate) struct HourByWeekday<'a> {
    /// `data-ck-id` and the prefix of this chart's pattern id.
    pub id: &'a str,
    /// Total count per `[weekday][hour]` over the window (Sunday = 0).
    pub counts: [[i64; 24]; 7],
    /// How many days of each weekday the window held *and the station listened on*.
    /// Zero means "no data for that weekday", not "none heard".
    pub occurrences: [u32; 7],
    /// The ledger had rows for the period (so a weekday with none really was off).
    pub coverage_known: bool,
    /// Today's weekday and the hour it is now: the hours of today that have not happened yet
    /// are "not yet", not "none heard", and are not counted among the days they would average over.
    pub now: Option<(usize, usize)>,
    /// The window in days, for the words.
    pub window_days: u32,
}

/// The activity grid: hour of day across, weekday down, a rounded cell each, with the
/// totals for each weekday (right) and each hour (below) as small bars, the busiest cell
/// ringed and named, and the colour scale printed under it.
///
/// The colour is the *average per day* in that slot (the total divided by how many such
/// days were listened to), so a 90-day window, which holds thirteen of one weekday and
/// twelve of another, does not make the first look busier for having more days in it.
#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) fn hour_by_weekday(o: &HourByWeekday<'_>) -> String {
    let any: i64 = o.counts.iter().flatten().sum();
    if any <= 0 {
        return EMPTY.to_string();
    }
    // How many days this slot has really happened on: the weekday's days, less today when
    // today's hour has not come round yet.
    let occ_of = |d: usize, h: usize| -> u32 {
        let later_today = o.now.is_some_and(|(nd, nh)| nd == d && h > nh);
        o.occurrences[d].saturating_sub(u32::from(later_today && o.occurrences[d] > 0))
    };
    let not_yet = |d: usize, h: usize| o.now.is_some_and(|(nd, nh)| nd == d && h > nh) && occ_of(d, h) == 0 && o.occurrences[d] > 0;
    let avg = |d: usize, h: usize| -> f64 { o.counts[d][h] as f64 / f64::from(occ_of(d, h).max(1)) };
    let max_avg = (0..7).flat_map(|d| (0..24).map(move |h| (d, h))).map(|(d, h)| avg(d, h)).fold(0.0_f64, f64::max);
    let (peak_d, peak_h) = (0..7)
        .flat_map(|d| (0..24).map(move |h| (d, h)))
        .max_by(|a, b| avg(a.0, a.1).total_cmp(&avg(b.0, b.1)).then(b.cmp(a)))
        .unwrap_or((0, 0));

    let row_tot: Vec<i64> = (0..7).map(|d| o.counts[d].iter().sum()).collect();
    let col_tot: Vec<i64> = (0..24).map(|h| (0..7).map(|d| o.counts[d][h]).sum()).collect();
    let row_max = row_tot.iter().copied().max().unwrap_or(1).max(1);
    let col_max = col_tot.iter().copied().max().unwrap_or(1).max(1);

    let (label_w, cw, ch) = (46.0_f64, 31.0_f64, 24.0_f64);
    let marg_x = label_w + 24.0 * cw + 10.0;
    let w = marg_x + 74.0;
    let top = 20.0;
    let grid_h = 7.0 * ch;
    let col_y = top + grid_h + 8.0;
    let col_h = 26.0;
    let h = col_y + col_h + 6.0;

    let g = kit::id_safe(&format!("g-{}", o.id));
    let mut svg = String::with_capacity(32 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><div class="hm-scroll" tabindex="0" role="group" aria-label="Hour by day-of-week activity grid, scrolls sideways"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="hm-svg ck-in" data-ck="marks" data-ck-id="{id}" data-ck-grid="24" data-ck-start="{start}" data-ck-label="Left and right move between hours, up and down between days.">"#,
        id = escape_html(o.id),
        start = peak_d * 24 + peak_h,
    );
    svg.push_str(&svg_a11y(
        "Activity by hour and day of the week",
        "A grid of the average detections in each hour of each day of the week, with the total for each day and each hour beside it.",
    ));
    if o.occurrences.iter().any(|&n| n == 0) {
        let _ = write!(svg, "<defs>{}</defs>", kit::nodata_pattern(&g));
    }

    for hh in (0..24).step_by(3) {
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{x:.1}" y="12" text-anchor="middle" font-size="10">{hh:02}</text>"#,
            x = label_w + hh as f64 * cw + cw / 2.0,
        );
    }
    for d in 0..7_usize {
        let y0 = top + d as f64 * ch;
        let _ = write!(
            svg,
            r#"<text class="ck-lbl" x="{x:.1}" y="{y:.1}" text-anchor="end" font-size="10">{day}</text>"#,
            x = label_w - 6.0,
            y = y0 + ch / 2.0 + 3.5,
            day = kit::WEEKDAYS[d],
        );
        for hh in 0..24_usize {
            let x0 = label_w + hh as f64 * cw;
            let n = o.counts[d][hh];
            let a = avg(d, hh);
            let heading = format!("{} {:02}:00\u{2013}{:02}:00", kit::WEEKDAYS[d], hh, (hh + 1) % 24);
            let idx = d * 24 + hh;
            let occ = occ_of(d, hh);
            let count_s = kit::thousands(n);
            let (cls_attr, fill_attr, tip) = if not_yet(d, hh) {
                (r#" class="ck-future""#.to_string(), String::new(), kit::tip(&heading, &[], Some("Not yet")))
            } else if o.occurrences[d] == 0 && o.coverage_known {
                (
                    String::new(),
                    format!(r#" fill="url(#{g}-nd)""#),
                    kit::tip(&heading, &[], Some(&format!("The station was not listening on any {} in this period", kit::WEEKDAYS[d]))),
                )
            } else if n == 0 {
                (
                    r#" class="ck-track""#.to_string(),
                    String::new(),
                    kit::tip(&heading, &[], Some("None heard")),
                )
            } else {
                let step = kit::seq_step(a / max_avg);
                let per = format!("{a:.1}");
                let rows: Vec<(&str, &str, Option<&str>)> = if occ > 1 {
                    vec![("Average a day", &per, None), ("In all", &count_s, None)]
                } else {
                    vec![("Detections", &count_s, None)]
                };
                let over = format!("over {occ} {}s", kit::WEEKDAYS[d]);
                (
                    format!(r#" class="ck-fill {}""#, step_class(Ramp::Sun, step)),
                    String::new(),
                    kit::tip(&heading, &rows, (occ > 1).then_some(over.as_str())),
                )
            };
            let _ = write!(
                svg,
                r#"<rect{cls_attr}{fill_attr} x="{x:.1}" y="{y:.1}" width="{cwid:.1}" height="{chei:.1}" rx="4" data-ck-i="{idx}" data-tip="{tip}"/>"#,
                x = x0 + 1.0,
                y = y0 + 1.0,
                cwid = cw - 2.0,
                chei = ch - 2.0,
            );
        }
        // The weekday's total, as a bar and a number.
        let bw = row_tot[d] as f64 / row_max as f64 * 34.0;
        if row_tot[d] > 0 {
            let _ = write!(
                svg,
                r#"<path class="ck-fill ck-cother" d="{p}"/>"#,
                p = kit::hbar_path(marg_x, y0 + 5.0, bw.max(1.5), ch - 10.0, 3.0),
            );
        }
        let _ = write!(
            svg,
            r#"<text class="mono ck-annot" x="{x:.1}" y="{y:.1}" font-size="10">{t}</text>"#,
            x = marg_x + bw.max(1.5) + 5.0,
            y = y0 + ch / 2.0 + 3.5,
            t = if o.occurrences[d] == 0 && o.coverage_known { "off".to_string() } else { kit::compact(row_tot[d] as f64) },
        );
    }
    // The peak.
    let _ = write!(
        svg,
        r#"<rect class="ck-peak" x="{x:.1}" y="{y:.1}" width="{cwid:.1}" height="{chei:.1}" rx="4"/>"#,
        x = label_w + peak_h as f64 * cw + 0.5,
        y = top + peak_d as f64 * ch + 0.5,
        cwid = cw - 1.0,
        chei = ch - 1.0,
    );
    // The hour totals.
    let _ = write!(
        svg,
        r#"<text class="ck-lbl" x="{x:.1}" y="{y:.1}" text-anchor="end" font-size="10">Hour</text>"#,
        x = label_w - 6.0,
        y = col_y + col_h - 4.0,
    );
    for (hh, &t) in col_tot.iter().enumerate() {
        if t <= 0 {
            continue;
        }
        let bh = (t as f64 / col_max as f64 * col_h).max(1.5);
        let _ = write!(
            svg,
            r#"<path class="ck-fill ck-cother" d="{p}"/>"#,
            p = kit::bar_path(label_w + hh as f64 * cw + 5.0, col_y + col_h - bh, cw - 10.0, bh, 3.0),
        );
    }
    let words = format!("Busiest: {} {:02}:00, about {:.1} a day (ringed)", kit::WEEKDAYS[peak_d], peak_h, avg(peak_d, peak_h));
    svg.push_str("</svg></div>");
    svg.push_str(&scale_legend(
        Ramp::Sun,
        max_avg,
        "Average a day",
        &[words.clone()],
        o.occurrences.iter().any(|&n| n == 0) && o.coverage_known,
    ));
    let _ = write!(
        svg,
        r#"<p class="sr-only viz-readout">{}</p>"#,
        escape_html(&format!("{} over the last {} days. {words}.", kit::thousands(any), o.window_days)),
    );
    svg.push_str("</div>");
    svg
}

// ── calendar ────────────────────────────────────────────────────────────────────

/// What the year calendar draws.
pub(crate) struct Calendar<'a> {
    /// `data-ck-id` and the prefix of this chart's pattern id.
    pub id: &'a str,
    /// First and last day drawn, ISO dates, inclusive.
    pub from: &'a str,
    /// See `from`.
    pub to: &'a str,
    /// Count per ISO date; a date not in the map had none heard.
    pub counts: &'a BTreeMap<String, i64>,
    /// Seconds recorded per ISO date, when the station has a ledger for the period.
    pub effort: Option<&'a BTreeMap<String, f64>>,
    /// What is counted, plural.
    pub unit: &'a str,
    /// The chart's accessible name.
    pub name: &'a str,
}

/// A calendar of days, a column a week and a row a weekday (Sunday at the top), the way
/// people have learned to read a year of activity.
///
/// A day is a step on the moss ramp by the square root of its count; a day the station
/// listened to and heard nothing is an empty cell; a day with no audio in the ledger is
/// hatched. The busiest day is ringed and named. Months are labelled where they begin.
#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) fn calendar(o: &Calendar<'_>) -> String {
    let (Some(start), Some(end)) = (kit::epoch_day(o.from), kit::epoch_day(o.to)) else {
        return EMPTY.to_string();
    };
    if end < start || o.counts.is_empty() {
        return EMPTY.to_string();
    }
    let total: i64 = o.counts.values().sum();
    let (peak_date, peak_n) = o
        .counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map_or((String::new(), 0), |(d, n)| (d.clone(), *n));
    let max = peak_n.max(1) as f64;

    // Sunday-first weeks: the column of a day is its week since the first Sunday on or
    // before `start`; the row is its weekday.
    let dow = |day: i64| ((day + 4).rem_euclid(7)) as usize; // 1970-01-01 was a Thursday
    let first_col_day = start - dow(start) as i64;
    let weeks = ((end - first_col_day) / 7 + 1) as usize;
    let (label_w, cell, gap) = (38.0_f64, 12.0_f64, 3.0_f64);
    let pitch = cell + gap;
    let w = label_w + weeks as f64 * pitch + 4.0;
    let top = 18.0;
    let grid_h = 7.0 * pitch;
    let h = top + grid_h + 2.0;

    let g = kit::id_safe(&format!("g-{}", o.id));
    let date_of = |day: i64| -> String {
        let c = birdnet_core::civil::civil_from_days(day);
        format!("{:04}-{:02}-{:02}", c.0, c.1, c.2)
    };
    let off_known = o.effort.is_some_and(|e| !e.is_empty());
    let listened = |date: &str| -> bool {
        // Today is never "off": the recorder credits on a timer.
        o.effort.is_none_or(|e| e.get(date).is_some_and(|s| *s >= 60.0) || date == o.to)
    };

    let mut svg = String::with_capacity(48 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><div class="hm-scroll" tabindex="0" role="group" aria-label="{name}, scrolls sideways"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="cal-svg ck-in" data-ck="marks" data-ck-id="{id}" data-ck-grid="7" data-ck-major="col" data-ck-start="{startidx}" data-ck-label="Left and right move between weeks, up and down between days.">"#,
        name = escape_html(o.name),
        id = escape_html(o.id),
        startidx = {
            let pd = kit::epoch_day(&peak_date).unwrap_or(start);
            ((pd - first_col_day) / 7) * 7 + dow(pd) as i64
        },
    );
    svg.push_str(&svg_a11y(
        o.name,
        &format!("A calendar with a cell for each day from {} to {}; the colour is how many {} were heard that day.", o.from, o.to, o.unit),
    ));
    if off_known {
        let _ = write!(svg, "<defs>{}</defs>", kit::nodata_pattern(&g));
    }

    // Weekday labels on alternate rows, like the calendars people know.
    for (r, name) in [(1_usize, "Mon"), (3, "Wed"), (5, "Fri")] {
        let _ = write!(
            svg,
            r#"<text class="ck-lbl" x="{x:.1}" y="{y:.1}" text-anchor="end" font-size="10">{name}</text>"#,
            x = label_w - 6.0,
            y = top + r as f64 * pitch + cell - 1.5,
        );
    }
    // Month labels where the month begins, never closer than a label's width.
    let mut last_label_x = f64::NEG_INFINITY;
    let mut peak_cell: Option<(f64, f64)> = None;
    for day in start..=end {
        let col = ((day - first_col_day) / 7) as usize;
        let row = dow(day);
        let date = date_of(day);
        let x = label_w + col as f64 * pitch;
        let y = top + row as f64 * pitch;
        let c = birdnet_core::civil::civil_from_days(day);
        if c.2 == 1 || (day == start && c.2 <= 20) {
            let name = kit::MONTHS[(c.1 as usize).saturating_sub(1).min(11)];
            if x - last_label_x >= 30.0 {
                let _ = write!(
                    svg,
                    r#"<text class="mono ck-lbl" x="{x:.1}" y="11" font-size="10">{name}</text>"#
                );
                last_label_x = x;
            }
        }
        let n = o.counts.get(&date).copied().unwrap_or(0);
        let long = kit::civil_label(&date, 0).map_or_else(|| date.clone(), |(_, l)| format!("{l} {}", &date[..4]));
        let idx = col * 7 + row;
        let count_s = kit::thousands(n);
        let (cls, fill, tip) = if n > 0 {
            let step = kit::seq_step(n as f64 / max);
            (
                format!(r#" class="ck-fill {}""#, step_class(Ramp::Moss, step)),
                String::new(),
                kit::tip(&long, &[(&capitalise(o.unit), &count_s, None)], None),
            )
        } else if off_known && !listened(&date) {
            (
                String::new(),
                format!(r#" fill="url(#{g}-nd)""#),
                kit::tip(&long, &[], Some("The station was not listening")),
            )
        } else {
            (
                r#" class="ck-track""#.to_string(),
                String::new(),
                kit::tip(&long, &[], Some(if off_known { "Listened all day and heard none" } else { "None heard" })),
            )
        };
        let _ = write!(
            svg,
            r#"<rect{cls}{fill} x="{x:.1}" y="{y:.1}" width="{cell}" height="{cell}" rx="3" data-ck-i="{idx}" data-tip="{tip}"/>"#
        );
        if date == peak_date {
            peak_cell = Some((x, y));
        }
    }
    if let Some((x, y)) = peak_cell {
        let _ = write!(
            svg,
            r#"<rect class="ck-peak" x="{a:.1}" y="{b:.1}" width="{s}" height="{s}" rx="4"/>"#,
            a = x - 1.0,
            b = y - 1.0,
            s = cell + 2.0,
        );
    }
    let busiest = kit::civil_label(&peak_date, 0).map_or(peak_date.clone(), |(_, l)| l);
    let words = format!("Busiest day: {busiest}, {} (ringed)", kit::thousands(peak_n));
    svg.push_str("</svg></div>");
    svg.push_str(&scale_legend(Ramp::Moss, max, &capitalise(o.unit), &[words.clone()], off_known));
    let _ = write!(
        svg,
        r#"<p class="sr-only viz-readout">{}</p></div>"#,
        escape_html(&format!("{} {} from {} to {}. {words}.", kit::thousands(total), o.unit, o.from, o.to)),
    );
    svg
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> HourByWeekday<'static> {
        let mut counts = [[0_i64; 24]; 7];
        counts[1][6] = 23;
        counts[1][7] = 9;
        counts[3][12] = 2;
        counts[6][18] = 1;
        HourByWeekday { id: "t", counts, occurrences: [1; 7], coverage_known: true, window_days: 7, now: None }
    }

    #[test]
    fn an_empty_grid_is_not_a_chart() {
        let mut g = grid();
        g.counts = [[0; 24]; 7];
        assert!(hour_by_weekday(&g).contains("Not enough data"));
    }

    /// Every one of the 168 slots has a reading, in row-major order for the keyboard.
    #[test]
    fn every_slot_has_a_cell_a_reading_and_an_index() {
        let svg = hour_by_weekday(&grid());
        assert_eq!(svg.matches("data-tip=").count(), 168, "{svg}");
        for i in [0, 24 * 1 + 6, 167] {
            assert!(svg.contains(&format!(r#"data-ck-i="{i}""#)), "{i}");
        }
        assert!(svg.contains(r#"data-ck-grid="24""#) && svg.contains(r#"data-ck-start="30""#), "{svg}");
        assert!(svg.contains("Mon 06:00\u{2013}07:00&#10;Detections&#9;23"), "{svg}");
    }

    /// Three states stay three: a count is a ramp step, a quiet hour is an empty cell, and a weekday
    /// the station was off for is hatched and says so.
    #[test]
    fn a_count_a_quiet_hour_and_an_unrecorded_day_look_different() {
        let mut g = grid();
        g.occurrences[2] = 0; // Tuesday: not listening
        let svg = hour_by_weekday(&g);
        assert!(svg.contains(r#"class="ck-fill ck-q7""#), "the peak is the top step: {svg}");
        assert!(svg.contains(r#"class="ck-track""#), "{svg}");
        assert_eq!(svg.matches("-nd)").count(), 24, "all 24 Tuesday cells: {svg}");
        assert!(svg.contains("The station was not listening on any Tue in this period"), "{svg}");
        assert!(svg.contains(">off<"), "{svg}");
        // No ledger: Tuesday is just a quiet day, and nothing is hatched.
        g.coverage_known = false;
        assert!(!hour_by_weekday(&g).contains("-nd)"));
    }

    /// The colour is the average per day, not the sum, so a weekday with more days in the
    /// window is not painted busier for that alone.
    #[test]
    fn the_colour_is_per_day_not_per_window() {
        let mut g = grid();
        g.counts = [[0; 24]; 7];
        g.counts[0][6] = 40; // 4 Sundays
        g.counts[1][6] = 30; // 1 Monday
        g.occurrences = [4, 1, 1, 1, 1, 1, 1];
        let svg = hour_by_weekday(&g);
        // 10 a day on Sunday, 30 a day on Monday: Monday is the peak and Sunday is lower.
        assert!(svg.contains("Busiest: Mon 06:00, about 30.0 a day"), "{svg}");
        assert!(svg.contains("Sun 06:00\u{2013}07:00&#10;Average a day&#9;10.0&#10;In all&#9;40&#10;over 4 Suns"), "{svg}");
    }

    /// Today is the one weekday that is only partly lived: the hours of it still to come are
    /// "Not yet" (an outline, not a quiet cell), and they are not among the days a slot averages
    /// over, so a Wednesday of fourteen days divides this Wednesday's morning by one fewer.
    #[test]
    fn the_hours_of_today_still_to_come_are_not_yet_and_not_quiet() {
        let mut g = grid();
        g.now = Some((3, 10)); // Wednesday, ten o'clock
        let svg = hour_by_weekday(&g);
        // Wed 11:00 onward: 13 slots of "Not yet", and the 10:00 slot is today's so far.
        assert_eq!(svg.matches("&#10;Not yet").count(), 13, "11:00 to 23:00 are still to come: {svg}");
        assert!(svg.contains("Wed 12:00\u{2013}13:00&#10;Not yet"), "{svg}");
        assert!(svg.contains(r#"class="ck-future""#), "{svg}");
        assert!(!svg.contains("Wed 09:00\u{2013}10:00&#10;Not yet"), "09:00 has happened: {svg}");
        // Two Wednesdays in the window: the later hours average over the one that happened.
        let mut g = grid();
        g.occurrences[3] = 2;
        g.counts[3][12] = 8;
        g.now = Some((3, 10));
        let svg = hour_by_weekday(&g);
        assert!(svg.contains("Wed 12:00\u{2013}13:00&#10;Detections&#9;8"), "one day that hour has happened on: no average to give: {svg}");
        // No clock, no claim.
        assert!(!hour_by_weekday(&grid()).contains("Not yet"));
    }

    fn cal(from: &'static str, to: &'static str, counts: &'static BTreeMap<String, i64>, effort: Option<&'static BTreeMap<String, f64>>) -> String {
        calendar(&Calendar { id: "cal", from, to, counts, effort, unit: "detections", name: "The year in activity" })
    }

    fn counts_of(rows: &[(&str, i64)]) -> BTreeMap<String, i64> {
        rows.iter().map(|(d, n)| ((*d).to_string(), *n)).collect()
    }

    /// One cell per day from the first to the last, none outside it, and the columns are Sunday-first weeks.
    #[test]
    fn a_calendar_has_a_cell_for_each_day_and_only_those() {
        let c: &'static _ = Box::leak(Box::new(counts_of(&[("2026-10-14", 5), ("2026-10-20", 40)])));
        let svg = cal("2026-10-01", "2026-10-21", c, None);
        assert_eq!(svg.matches("data-tip=").count(), 21, "{svg}");
        // 1 Oct 2026 is a Thursday (row 4) in the first column; 21 Oct a Wednesday (row 3) in the fourth.
        assert!(svg.contains(r#"data-ck-i="4""#) && svg.contains(r#"data-ck-i="24""#) && !svg.contains(r#"data-ck-i="25""#), "{svg}");
        assert!(svg.contains("Wed 14 Oct 2026&#10;Detections&#9;5"), "{svg}");
        assert!(svg.contains("Busiest day: Tue 20 Oct, 40"), "{svg}");
        assert!(svg.contains(r#"data-ck-major="col""#), "{svg}");
        assert!(svg.contains(">Oct<"), "{svg}");
    }

    /// Quiet and off are different cells, and today is never off.
    #[test]
    fn a_quiet_day_and_an_unrecorded_day_are_told_apart() {
        let c: &'static _ = Box::leak(Box::new(counts_of(&[("2026-10-14", 5)])));
        let e: &'static _ = Box::leak(Box::new(
            [("2026-10-14", 86_400.0), ("2026-10-15", 86_400.0), ("2026-10-17", 10.0)]
                .iter()
                .map(|(d, s)| ((*d).to_string(), *s))
                .collect::<BTreeMap<_, _>>(),
        ));
        let svg = cal("2026-10-14", "2026-10-18", c, Some(e));
        assert!(svg.contains("Thu 15 Oct 2026&#10;Listened all day and heard none"), "{svg}");
        assert!(svg.contains("Fri 16 Oct 2026&#10;The station was not listening"), "{svg}");
        assert!(svg.contains("Sat 17 Oct 2026&#10;The station was not listening"), "ten seconds is not a day: {svg}");
        assert!(svg.contains("Sun 18 Oct 2026&#10;Listened all day"), "today is never off: {svg}");
        let no_ledger = cal("2026-10-14", "2026-10-18", c, None);
        assert!(!no_ledger.contains("not listening"), "{no_ledger}");
    }

    #[test]
    fn nothing_counted_or_a_bad_date_is_not_a_chart() {
        let empty: &'static BTreeMap<String, i64> = Box::leak(Box::default());
        assert!(cal("2026-10-01", "2026-10-21", empty, None).contains("Not enough data"));
        let c: &'static _ = Box::leak(Box::new(counts_of(&[("2026-10-14", 5)])));
        assert!(cal("nope", "2026-10-21", c, None).contains("Not enough data"));
        assert!(cal("2026-10-21", "2026-10-01", c, None).contains("Not enough data"));
    }

    /// A whole year, 365 days, in 53 columns, is one SVG whose text can still be read.
    #[test]
    fn a_year_is_fifty_three_weeks() {
        let rows: Vec<(String, i64)> = (0..365_i64).map(|i| (kit::shift_date("2025-10-22", i).map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}")).unwrap(), i % 37)).collect();
        let c: &'static BTreeMap<String, i64> = Box::leak(Box::new(rows.into_iter().collect()));
        let svg = cal("2025-10-22", "2026-10-21", c, None);
        assert_eq!(svg.matches("data-tip=").count(), 365);
        let months = svg.matches(r#"class="mono ck-lbl""#).count();
        assert!((11..=13).contains(&months), "a label a month: {months}");
    }
}
