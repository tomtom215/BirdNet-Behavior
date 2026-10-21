//! The morning running order: who opens the dawn, in what order, and when.
//!
//! One lane per voice of the station's dawn, in the order the chorus opens. Along a lane is
//! the clock from 04:00 to 09:00 and each dot is a morning: the moment that voice was first
//! heard that morning. A thin line joins one morning's dots from lane to lane, so a run
//! that goes in the expected order is a line that slants steadily to the right as it goes
//! down, and a morning that broke the order is a line that doubles back, which is exactly
//! what a table of "mornings reaching each step" cannot show. A tick marks the typical time
//! (the median over every morning the voice was part of a run, not only the ones drawn), the
//! newest complete morning is drawn large and its times are written out, and each lane says
//! how many mornings got that far.
//!
//! The counts and the times are the behavioural engine's: `window_funnel` for how many
//! mornings reached each step, `sequence_match_events` for when each step fired. Nothing here
//! is estimated.
//!
//! The picture is drawn narrow (560 units) with the names above their lanes and not beside
//! them, so it reads at a phone's width without the text-fitting script having to grow it.
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

use super::kit::{slot_class, tip};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// How many of the newest mornings are joined by a line from lane to lane.
const LINES: usize = 14;

/// One voice of the dawn.
pub struct Voice {
    /// The species as the station displays it.
    pub name: String,
    /// Mornings that reached this step of the run.
    pub reached: u64,
    /// The typical time it is first heard, seconds after midnight (median over every
    /// morning it was part of a run).
    pub typical_secs: Option<u32>,
}

/// One morning: its date and the time each step of the run fired, seconds after midnight,
/// in the order of the voices. A morning that stopped at the second voice has two.
pub struct Morning {
    pub date: String,
    pub times: Vec<u32>,
}

/// `06:23` for seconds after midnight.
fn clock(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600 % 24, secs % 3600 / 60)
}

/// A steady, repeatable offset in `-1.0..=1.0` for the `i`th morning, so a morning's dots sit
/// at the same height in every lane (which is what makes its line between lanes a slope and
/// not a scribble) and the same morning draws in the same place on every request.
fn spread(i: usize) -> f64 {
    let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
    (h % 2001) as f64 / 1000.0 - 1.0
}

/// The median of `v` (the lower middle of an even count), or `None` if empty.
pub fn median(v: &mut [u32]) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[(v.len() - 1) / 2])
}

/// The running order as a picture. `mornings` are drawn newest first; the first one that
/// completed every step is the highlighted one. `hours` is the window the run was looked for
/// in, first and last hour inclusive (`(4, 8)` is 04:00 to 08:59).
#[must_use]
pub fn morning_order(voices: &[Voice], mornings: &[Morning], hours: (u32, u32)) -> String {
    let first = voices.first().map_or(0, |v| v.reached);
    if voices.len() < 2 || first == 0 {
        return EMPTY.to_string();
    }
    const W: f64 = 560.0;
    const PAD_X: f64 = 14.0;
    const LANE: f64 = 84.0;
    const TOP: f64 = 6.0;
    let t0 = f64::from(hours.0) * 3600.0;
    let span = f64::from(hours.1 + 1) * 3600.0 - t0;
    let inner = W - 2.0 * PAD_X;
    let x = |secs: f64| PAD_X + ((secs - t0) / span).clamp(0.0, 1.0) * inner;
    let lane_top = |k: usize| TOP + k as f64 * LANE;
    // The dots of lane `k` sit in a band under its name, whose middle is here.
    let band_mid = |k: usize| lane_top(k) + 52.0;
    let h = lane_top(voices.len()) + 18.0;
    let complete = mornings.iter().position(|m| m.times.len() == voices.len());

    let mut s = format!(
        r#"<div class="ck mo-fig"><svg viewBox="0 0 {W} {h:.0}" width="100%" role="img" class="mo-svg ck-in" data-ck="marks" data-ck-dim="series" data-ck-id="running-order" data-ck-label="Left and right arrows step through the dots, one morning after another">"#,
    );
    s.push_str(&svg_a11y(
        "The morning running order",
        &format!(
            "{} lanes, one per voice of the dawn, with the clock from {} to {} along each. A dot is one morning: the moment that voice was first heard. A thin line joins the dots of one morning from lane to lane.",
            voices.len(),
            clock(hours.0 * 3600),
            clock((hours.1 + 1) * 3600),
        ),
    ));
    // Hours, down the whole picture.
    for hr in hours.0..=hours.1 + 1 {
        let xx = x(f64::from(hr) * 3600.0);
        let label = match hr {
            0 | 24 => "12a".to_owned(),
            12 => "12p".to_owned(),
            n if n < 12 => format!("{n}a"),
            n => format!("{}p", n - 12),
        };
        let _ = write!(
            s,
            r#"<line class="ck-grid" x1="{xx:.1}" y1="{y1:.1}" x2="{xx:.1}" y2="{y2:.1}"/><text class="ck-lbl mo-hr mono" x="{xx:.1}" y="{ty:.1}" text-anchor="middle">{label}</text>"#,
            y1 = TOP + 28.0,
            y2 = h - 18.0,
            ty = h - 4.0,
        );
    }
    // Lanes: the name above, how many mornings got this far at the right, a rail under. The
    // words are written last (`words`), over the lines and dots, so a line passing a name has
    // its halo behind the letters and not the letters struck through.
    let mut words = String::new();
    for (k, v) in voices.iter().enumerate() {
        let top = lane_top(k);
        let pct = if first == 0 {
            0.0
        } else {
            v.reached as f64 / first as f64 * 100.0
        };
        let _ = write!(
            s,
            r#"<rect class="ck-fill {slot}" x="{PAD_X}" y="{ky:.1}" width="9" height="9" rx="2"/><line class="ck-axis" x1="{PAD_X}" y1="{my:.1}" x2="{xr:.1}" y2="{my:.1}"/>"#,
            slot = slot_class(k),
            ky = top + 6.0,
            my = band_mid(k),
            xr = W - PAD_X,
        );
        let _ = write!(
            words,
            r#"<text class="mo-name ck-halo" x="{nx:.1}" y="{ny:.1}">{name}</text><text class="mo-count ck-lbl mono ck-halo" x="{rx:.1}" y="{ny:.1}" text-anchor="end">{count} mornings · {pct:.0}%</text>"#,
            nx = PAD_X + 15.0,
            ny = top + 15.0,
            name = escape_html(&v.name),
            rx = W - PAD_X,
            count = v.reached,
        );
    }
    // One morning's line, behind every dot: the newest drawn last of the lines so it lies on top.
    // Only the newest few have one: forty-five lines across three lanes is a hairball, and the
    // dots (every morning, all of them) already say where each voice was first heard.
    let mut lines = String::new();
    for (mi, m) in mornings.iter().enumerate().take(LINES).rev() {
        let off = spread(mi) * 11.0;
        let mut pts = String::new();
        for (k, &t) in m.times.iter().enumerate().take(voices.len()) {
            let _ = write!(pts, "{:.1},{:.1} ", x(f64::from(t)), band_mid(k) + off);
        }
        if m.times.len() >= 2 {
            let _ = write!(
                lines,
                r#"<polyline class="mo-line{now}" points="{pts}" data-ck-s="m{mi}"/>"#,
                now = if Some(mi) == complete { " is-now" } else { "" },
            );
        }
    }
    s.push_str(&lines);
    // The typical time of each voice.
    for (k, v) in voices.iter().enumerate() {
        if let Some(t) = v.typical_secs {
            let xx = x(f64::from(t));
            let _ = write!(
                s,
                r#"<line class="mo-typical" x1="{xx:.1}" y1="{y1:.1}" x2="{xx:.1}" y2="{y2:.1}"/><text class="mo-typ-t mono ck-annot ck-halo" x="{xx:.1}" y="{ty:.1}" text-anchor="middle" {clk}>{label}</text>"#,
                y1 = band_mid(k) - 16.0,
                y2 = band_mid(k) + 16.0,
                ty = band_mid(k) - 20.0,
                label = clock(t),
                clk = crate::display::clock_attr(&clock(t)),
            );
        }
    }
    // The dots: every morning, newest first (so the keyboard starts at the newest).
    let mut index = 0usize;
    for (mi, m) in mornings.iter().enumerate() {
        let off = spread(mi) * 11.0;
        for (k, &t) in m.times.iter().enumerate().take(voices.len()) {
            let (cx, cy) = (x(f64::from(t)), band_mid(k) + off);
            let is_now = Some(mi) == complete;
            let data_tip = tip(
                &voices[k].name,
                &[
                    (
                        "First heard",
                        &clock(t),
                        Some(&format!("var(--chart-{})", k.min(7) + 1)),
                    ),
                    (
                        "Typical",
                        &voices[k].typical_secs.map_or_else(|| "—".to_owned(), clock),
                        None,
                    ),
                ],
                Some(&m.date),
            );
            let _ = write!(
                s,
                r#"<circle class="mo-dot {slot}{now}" cx="{cx:.1}" cy="{cy:.1}" r="{r}" data-ck-s="m{mi}"/><circle class="ck-hit" cx="{cx:.1}" cy="{cy:.1}" r="9" data-tip="{data_tip}" data-ck-i="{index}" data-ck-f="m{mi}"/>"#,
                slot = slot_class(k),
                now = if is_now { " is-now" } else { "" },
                r = if is_now { 5 } else { 3 },
            );
            if is_now {
                let anchor = if cx > W * 0.75 { "end" } else { "start" };
                let lx = if cx > W * 0.75 { cx - 9.0 } else { cx + 9.0 };
                let _ = write!(
                    s,
                    r#"<text class="mo-now-t mono ck-annot ck-halo" x="{lx:.1}" y="{ly:.1}" text-anchor="{anchor}" data-ck-s="m{mi}" {clk}>{label}</text>"#,
                    ly = cy + 15.0,
                    label = clock(t),
                    clk = crate::display::clock_attr(&clock(t)),
                );
            }
            index += 1;
        }
    }
    s.push_str(&words);
    s.push_str("</svg>");
    if let Some(mi) = complete {
        let _ = write!(
            s,
            r#"<p class="bnb-meta mo-note">The large dots are the newest morning that got through all {n} voices, {date}; the ticks are each voice's typical time. A line that slants steadily to the right as it goes down is a morning in the usual order.</p>"#,
            n = voices.len(),
            date = escape_html(&mornings[mi].date),
        );
    } else {
        s.push_str(
            r#"<p class="bnb-meta mo-note">The ticks are each voice's typical time. A line that slants steadily to the right as it goes down is a morning in the usual order.</p>"#,
        );
    }
    s.push_str("</div>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voices() -> Vec<Voice> {
        vec![
            Voice {
                name: "American Robin".into(),
                reached: 20,
                typical_secs: Some(6 * 3600 + 23 * 60),
            },
            Voice {
                name: "Song Sparrow".into(),
                reached: 12,
                typical_secs: Some(7 * 3600 + 26 * 60),
            },
            Voice {
                name: "Northern Cardinal".into(),
                reached: 5,
                typical_secs: None,
            },
        ]
    }

    fn morning(date: &str, t: &[u32]) -> Morning {
        Morning {
            date: date.into(),
            times: t.to_vec(),
        }
    }

    #[test]
    fn too_few_voices_or_an_empty_first_step_is_the_placeholder() {
        assert_eq!(morning_order(&[], &[], (4, 8)), EMPTY);
        assert_eq!(morning_order(&voices()[..1], &[], (4, 8)), EMPTY);
        let mut v = voices();
        v[0].reached = 0;
        assert_eq!(morning_order(&v, &[], (4, 8)), EMPTY);
    }

    #[test]
    fn every_lane_says_how_many_mornings_got_that_far() {
        let svg = morning_order(&voices(), &[], (4, 8));
        assert!(
            svg.contains("American Robin")
                && svg.contains("Song Sparrow")
                && svg.contains("Northern Cardinal")
        );
        // The first step is 100% of itself; the others are shares of it.
        assert!(svg.contains("20 mornings · 100%"));
        assert!(svg.contains("12 mornings · 60%"));
        assert!(svg.contains("5 mornings · 25%"));
        assert!(svg.contains("<title>The morning running order</title>"));
        assert!(!svg.contains("aria-label"));
    }

    /// A dot sits where the voice was first heard on the clock: 06:00 on a 04:00-09:00 axis is
    /// two fifths of the way along.
    #[test]
    fn a_dot_is_where_it_was_on_the_clock() {
        let m = [morning(
            "2026-10-21",
            &[6 * 3600, 7 * 3600 + 1800, 8 * 3600],
        )];
        let svg = morning_order(&voices(), &m, (4, 8));
        let cxs: Vec<f64> = svg
            .split("<circle class=\"mo-dot ")
            .skip(1)
            .map(|c| {
                let k = c.find("cx=\"").expect("cx") + 4;
                c[k..]
                    .split('"')
                    .next()
                    .expect("v")
                    .parse()
                    .expect("number")
            })
            .collect();
        assert_eq!(cxs.len(), 3);
        let inner = 560.0 - 28.0;
        assert!((cxs[0] - (14.0 + inner * 2.0 / 5.0)).abs() < 0.2, "{cxs:?}");
        assert!((cxs[1] - (14.0 + inner * 3.5 / 5.0)).abs() < 0.2, "{cxs:?}");
        assert!(cxs[0] < cxs[1] && cxs[1] < cxs[2], "later is further right");
    }

    /// A morning that stopped at the second voice has two dots and a line between them, and a
    /// morning with one has a dot and no line: a line to nowhere would claim a third step.
    #[test]
    fn a_line_joins_only_the_steps_a_morning_reached() {
        let m = [
            morning("2026-10-21", &[6 * 3600, 7 * 3600, 8 * 3600]),
            morning("2026-10-20", &[6 * 3600, 7 * 3600]),
            morning("2026-10-19", &[6 * 3600]),
        ];
        let svg = morning_order(&voices(), &m, (4, 8));
        assert_eq!(svg.matches("<polyline").count(), 2);
        assert_eq!(svg.matches("<circle class=\"mo-dot").count(), 6);
        assert_eq!(svg.matches("class=\"ck-hit\"").count(), 6);
        // Exactly one morning is the highlighted one: the newest that got through every voice.
        assert_eq!(
            svg.matches(" is-now\"").count(),
            1 + 3,
            "line + three dots: {svg:.0}"
        );
        assert!(svg.contains(
            "The large dots are the newest morning that got through all 3 voices, 2026-10-21"
        ));

        // Only the newest few mornings are joined by a line: fifty mornings are fifty dots and
        // fourteen lines, not a hairball.
        let many: Vec<Morning> = (0..50)
            .map(|i| {
                morning(
                    &format!("2026-09-{:02}", i % 28 + 1),
                    &[6 * 3600, 7 * 3600, 8 * 3600],
                )
            })
            .collect();
        let svg = morning_order(&voices(), &many, (4, 8));
        assert_eq!(svg.matches("<polyline").count(), LINES);
        assert_eq!(svg.matches("<circle class=\"mo-dot").count(), 150);
    }

    /// The typical time is on the lane that has one and the lane that has none says nothing,
    /// not a made-up time.
    #[test]
    fn a_voice_without_a_typical_time_has_no_tick() {
        let svg = morning_order(&voices(), &[], (4, 8));
        assert_eq!(svg.matches("class=\"mo-typical\"").count(), 2);
        assert!(svg.contains("06:23") && svg.contains("07:26"));
    }

    #[test]
    fn the_median_is_the_lower_middle() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [5]), Some(5));
        assert_eq!(median(&mut [9, 1, 5]), Some(5));
        assert_eq!(median(&mut [4, 1, 3, 2]), Some(2));
    }

    #[test]
    fn a_morning_keeps_its_height_in_every_lane_and_the_spread_stays_inside() {
        for i in 0..200 {
            let s = spread(i);
            assert!((-1.0..=1.0).contains(&s), "{i}: {s}");
            assert!(
                (s - spread(i)).abs() < f64::EPSILON,
                "the same morning draws in the same place"
            );
        }
    }

    #[test]
    fn species_names_are_escaped() {
        let mut v = voices();
        v[0].name = "<script>alert(1)</script>".into();
        let svg = morning_order(&v, &[], (4, 8));
        assert!(!svg.contains("<script>"));
        assert!(svg.contains("&lt;script&gt;"));
    }
}
