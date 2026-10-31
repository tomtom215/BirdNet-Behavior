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

/// Radius of a dot for a morning other than the highlighted one, and of the highlighted one's.
const SMALL_DOT: f64 = 3.0;
const LARGE_DOT: f64 = 5.0;

/// The band of each lane, in units below the lane's top, in which words are written (the name, the
/// count, the typical time): no line is drawn through it. A line from one lane's dot to the next
/// lane's has to cross the next lane's words, and whichever way a halo is drawn it is a line
/// running through a word, which is what a heavy line did to "167 mornings · 82%". So the lines
/// are drawn in pieces and leave the band out, as a header strip would; the slant of the pieces
/// on either side still says which way the morning went.
const WORDS_TOP: f64 = 2.0;
/// See [`WORDS_TOP`]: the dots of a lane sit at 41 and below, so the band ends above them.
const WORDS_BOTTOM: f64 = 36.0;

/// A segment `(x1, y1, x2, y2)`.
type Seg = (f64, f64, f64, f64);

/// The parts of the polyline `pts` that lie outside every `(top, bottom)` band of `bands`, as the
/// segments that remain, in order. A segment that crosses a band is shortened at its edge; one
/// that lies inside it is dropped.
fn outside_bands(pts: &[(f64, f64)], bands: &[(f64, f64)]) -> Vec<Seg> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let ((x1, y1), (x2, y2)) = (w[0], w[1]);
        let dy = y2 - y1;
        // The stretches of the segment, as fractions of its length, that are inside a band.
        let mut cut: Vec<(f64, f64)> = Vec::new();
        for &(a, b) in bands {
            if dy.abs() < 1e-9 {
                if y1 >= a && y1 <= b {
                    cut.push((0.0, 1.0));
                }
            } else {
                let (t1, t2) = ((a - y1) / dy, (b - y1) / dy);
                let (lo, hi) = (t1.min(t2).max(0.0), t1.max(t2).min(1.0));
                if lo < hi {
                    cut.push((lo, hi));
                }
            }
        }
        cut.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut at = 0.0;
        let at_point = |t: f64| (x1 + (x2 - x1) * t, y1 + dy * t);
        for (lo, hi) in cut.into_iter().chain(std::iter::once((1.0, 1.0))) {
            if lo > at + 1e-9 {
                let (p, q) = (at_point(at), at_point(lo));
                out.push((p.0, p.1, q.0, q.1));
            }
            at = at.max(hi);
        }
    }
    out
}

/// `segs` as the `d` of a path: one subpath for each run of segments that meet end to start, so
/// the joins at a dot stay round.
fn path_d(segs: &[Seg]) -> String {
    let mut d = String::new();
    let mut end: Option<(f64, f64)> = None;
    for &(x1, y1, x2, y2) in segs {
        let joined = end.is_some_and(|(ex, ey)| (ex - x1).abs() < 1e-6 && (ey - y1).abs() < 1e-6);
        if joined {
            let _ = write!(d, "L {x2:.1} {y2:.1} ");
        } else {
            let _ = write!(d, "M {x1:.1} {y1:.1} L {x2:.1} {y2:.1} ");
        }
        end = Some((x2, y2));
    }
    d.trim_end().to_owned()
}

/// Whether segment `s` meets the box `(x0, y0, x1, y1)`: an end inside it, or a crossing of an edge.
fn meets_box(s: Seg, b: (f64, f64, f64, f64)) -> bool {
    // Not named `inside`: `tests/database_work_stays_off_the_async_runtime.rs` resolves a bare call
    // by name, and read this closure as `birdnet-migrate`'s `inside`, which reads the filesystem.
    let holds = |x: f64, y: f64| x >= b.0 && x <= b.2 && y >= b.1 && y <= b.3;
    if holds(s.0, s.1) || holds(s.2, s.3) {
        return true;
    }
    let cross = |a: Seg, c: Seg| {
        let d = (a.2 - a.0) * (c.3 - c.1) - (a.3 - a.1) * (c.2 - c.0);
        if d.abs() < 1e-12 {
            return false;
        }
        let t = ((c.0 - a.0) * (c.3 - c.1) - (c.1 - a.1) * (c.2 - c.0)) / d;
        let u = ((c.0 - a.0) * (a.3 - a.1) - (c.1 - a.1) * (a.2 - a.0)) / d;
        (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
    };
    [
        (b.0, b.1, b.2, b.1),
        (b.2, b.1, b.2, b.3),
        (b.2, b.3, b.0, b.3),
        (b.0, b.3, b.0, b.1),
    ]
    .into_iter()
    .any(|e| cross(s, e))
}

/// Roughly the box `(x0, y0, x1, y1)` of a word of `chars` characters at font size `fs` whose
/// baseline is at `y`, anchored at `x` (`start`, `middle` or `end`). A character is taken to be
/// 0.6 em wide, a little generous for a proportional face and right for the mono one; the line
/// runs from 0.8 em above the baseline to 0.25 em below it.
fn word_box(x: f64, y: f64, chars: usize, fs: f64, anchor: &str) -> (f64, f64, f64, f64) {
    let w = chars as f64 * fs * 0.6;
    let x0 = match anchor {
        "end" => x - w,
        "middle" => x - w / 2.0,
        _ => x,
    };
    (x0, y - fs * 0.8, x0 + w, y + fs * 0.25)
}

/// Where to write the time beside a large dot at `(cx, cy)`: `(x, baseline y, anchor)`.
///
/// The times are `HH:MM`, five characters of the small mono face. The candidates are the places
/// beside, under and over the dot, the side towards the middle of the picture first; the first
/// that the highlighted morning's own line (`heavy`, as drawn), the lane words (`words`) and the
/// dots (`dots`, the centre of each, all drawn at the small radius) do not touch, and that stays
/// inside the picture (`w_picture` wide), wins. The line leaves a dot towards the next voice, so the free side
/// depends on where the morning went: a steep line, or one that doubles back, reaches the usual
/// spot beside the dot. When every candidate is spoilt, the one with the least wrong with it: a
/// word or a line weighs ten dots, so a crowded lane gives up a dot's face before it gives up a
/// name or lets the line strike the time.
fn time_label_spot(
    (cx, cy): (f64, f64),
    w_picture: f64,
    heavy: &[Seg],
    words: &[(f64, f64, f64, f64)],
    dots: &[(f64, f64)],
) -> (f64, f64, &'static str) {
    const FS: f64 = 10.0;
    let towards_left = cx > w_picture * 0.75;
    let (near, far) = if towards_left {
        (-9.0, 9.0)
    } else {
        (9.0, -9.0)
    };
    let side = |dx: f64| if dx < 0.0 { "end" } else { "start" };
    let candidates = [
        (cx + near, cy + 15.0, side(near)),
        (cx + far, cy + 15.0, side(far)),
        (cx + near, cy + 4.0, side(near)),
        (cx + far, cy + 4.0, side(far)),
        (cx, cy + 24.0, "middle"),
        (cx + near, cy - 7.0, side(near)),
        (cx + far, cy - 7.0, side(far)),
        (cx, cy - 13.0, "middle"),
    ];
    let spoilt = |&(x, y, anchor): &(f64, f64, &str)| {
        let b = word_box(x, y, 5, FS, anchor);
        let padded = (b.0 - 1.5, b.1 - 1.5, b.2 + 1.5, b.3 + 1.5);
        let off_picture = b.0 < 2.0 || b.2 > w_picture - 2.0;
        let overlaps = |o: &(f64, f64, f64, f64)| {
            padded.0 < o.2 && o.0 < padded.2 && padded.1 < o.3 && o.1 < padded.3
        };
        let covered_dots = dots
            .iter()
            .filter(|&&(dx, dy)| {
                overlaps(&(
                    dx - SMALL_DOT,
                    dy - SMALL_DOT,
                    dx + SMALL_DOT,
                    dy + SMALL_DOT,
                ))
            })
            .count();
        (heavy.iter().filter(|s| meets_box(**s, padded)).count()
            + words.iter().filter(|o| overlaps(o)).count())
            * 10
            + covered_dots
            + usize::from(off_picture) * 1000
    };
    candidates
        .into_iter()
        .min_by_key(spoilt)
        .unwrap_or_else(|| (cx + near, cy + 15.0, side(near)))
}

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
    // Where each lane's words are, for the times that are written beside the large dots.
    let mut word_boxes: Vec<(f64, f64, f64, f64)> = Vec::new();
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
        let count_text = format!("{} mornings · {pct:.0}%", v.reached);
        word_boxes.push(word_box(
            PAD_X + 15.0,
            top + 15.0,
            v.name.chars().count(),
            13.0,
            "start",
        ));
        word_boxes.push(word_box(
            W - PAD_X,
            top + 15.0,
            count_text.chars().count(),
            11.0,
            "end",
        ));
        if let Some(t) = v.typical_secs {
            word_boxes.push(word_box(
                x(f64::from(t)),
                band_mid(k) - 20.0,
                5,
                10.0,
                "middle",
            ));
        }
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
    let bands: Vec<(f64, f64)> = (0..voices.len())
        .map(|k| (lane_top(k) + WORDS_TOP, lane_top(k) + WORDS_BOTTOM))
        .collect();
    let mut lines = String::new();
    let mut heavy: Vec<Seg> = Vec::new();
    for (mi, m) in mornings.iter().enumerate().take(LINES).rev() {
        let off = spread(mi) * 11.0;
        let pts: Vec<(f64, f64)> = m
            .times
            .iter()
            .enumerate()
            .take(voices.len())
            .map(|(k, &t)| (x(f64::from(t)), band_mid(k) + off))
            .collect();
        if m.times.len() >= 2 {
            let drawn = outside_bands(&pts, &bands);
            if Some(mi) == complete {
                heavy.clone_from(&drawn);
            }
            let _ = write!(
                lines,
                r#"<path class="mo-line{now}" d="{d}" data-ck-s="m{mi}"/>"#,
                now = if Some(mi) == complete { " is-now" } else { "" },
                d = path_d(&drawn),
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
    // Where morning `mi`'s dot for voice `k` goes, at `t`: a morning sits at the same height in
    // every lane. The drawing and the times that keep clear of the dots both read it from here.
    let centre = |mi: usize, k: usize, t: u32| (x(f64::from(t)), band_mid(k) + spread(mi) * 11.0);
    let every_dot: Vec<(f64, f64)> = mornings
        .iter()
        .enumerate()
        .flat_map(|(mi, m)| {
            (m.times.iter().enumerate().take(voices.len())).map(move |(k, &t)| centre(mi, k, t))
        })
        .collect();
    // The dots: every morning, newest first (so the keyboard starts at the newest).
    let mut index = 0usize;
    for (mi, m) in mornings.iter().enumerate() {
        for (k, &t) in m.times.iter().enumerate().take(voices.len()) {
            let (cx, cy) = centre(mi, k, t);
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
                r = if is_now { LARGE_DOT } else { SMALL_DOT },
            );
            if is_now {
                let (lx, ly, anchor) =
                    time_label_spot((cx, cy), W, &heavy, &word_boxes, &every_dot);
                let _ = write!(
                    s,
                    r#"<text class="mo-now-t mono ck-annot ck-halo" x="{lx:.1}" y="{ly:.1}" text-anchor="{anchor}" data-ck-s="m{mi}" {clk}>{label}</text>"#,
                    label = clock(t),
                    clk = crate::display::clock_attr(&clock(t)),
                );
            }
            index += 1;
        }
    }
    s.push_str(&words);
    s.push_str("</svg>");
    s.push_str(&note(
        complete.map(|mi| (voices.len(), mornings[mi].date.as_str())),
    ));
    s.push_str("</div>");
    s
}

/// The sentence under the picture. `highlighted` is the number of voices and the date of the
/// newest morning that went through all of them, when there is one: the large dots are that
/// morning's.
fn note(highlighted: Option<(usize, &str)>) -> String {
    match highlighted {
        Some((n, date)) => format!(
            r#"<p class="bnb-meta mo-note">The large dots are the newest morning that got through all {n} voices, {date}; the ticks are each voice's typical time. A line that slants steadily to the right as it goes down is a morning in the usual order.</p>"#,
            date = escape_html(date),
        ),
        None => r#"<p class="bnb-meta mo-note">The ticks are each voice's typical time. A line that slants steadily to the right as it goes down is a morning in the usual order.</p>"#.to_owned(),
    }
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
        assert_eq!(svg.matches("<path class=\"mo-line").count(), 2);
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
        assert_eq!(svg.matches("<path class=\"mo-line").count(), LINES);
        assert_eq!(svg.matches("<circle class=\"mo-dot").count(), 150);
    }

    /// With nothing in the way a time goes beside its dot on the side towards the middle of the
    /// picture (to the right of a dot on the left, to the left of one on the right); a word where
    /// that spot is sends it elsewhere, and a line through it does too.
    #[test]
    fn a_time_goes_towards_the_middle_and_gives_way_to_a_word_or_a_line() {
        let spot = |cx, heavy: &[Seg], words: &[(f64, f64, f64, f64)]| {
            time_label_spot((cx, 100.0), 560.0, heavy, words, &[])
        };
        assert_eq!(spot(100.0, &[], &[]), (109.0, 115.0, "start"));
        assert_eq!(spot(500.0, &[], &[]), (491.0, 115.0, "end"));
        // A word on the usual spot: the next is used.
        let word = (105.0, 105.0, 150.0, 120.0);
        let moved = spot(100.0, &[], &[word]);
        assert_ne!(moved, (109.0, 115.0, "start"), "{moved:?}");
        // A line through the usual spot: likewise.
        let line = (105.0, 118.0, 150.0, 108.0);
        assert_ne!(spot(100.0, &[line], &[]), (109.0, 115.0, "start"));
        // A word far away changes nothing.
        assert_eq!(
            spot(100.0, &[], &[(300.0, 0.0, 340.0, 10.0)]),
            (109.0, 115.0, "start")
        );
    }

    /// The note names the large dots only when there are some, and writes the date as text.
    #[test]
    fn the_note_names_the_large_dots_only_when_there_are_some() {
        let with = note(Some((3, "2026-10-21<b>")));
        assert!(with.contains("The large dots are the newest morning that got through all 3 voices, 2026-10-21&lt;b&gt;;"));
        assert!(with.ends_with("is a morning in the usual order.</p>"));
        let without = note(None);
        assert!(!without.contains("large dots"), "{without}");
        assert!(without.starts_with(
            r#"<p class="bnb-meta mo-note">The ticks are each voice's typical time."#
        ));
    }

    /// The small dots as drawn, `(cx, cy)` of every `mo-dot` that is not the highlighted morning's.
    fn drawn_small_dots(svg: &str) -> Vec<(f64, f64)> {
        svg.split("<circle class=\"mo-dot")
            .skip(1)
            .filter(|c| !c.split('"').next().unwrap_or("").contains("is-now"))
            .filter_map(|c| {
                let num = |a: &str| {
                    c.split(&format!("{a}=\""))
                        .nth(1)?
                        .split('"')
                        .next()?
                        .parse::<f64>()
                        .ok()
                };
                Some((num("cx")?, num("cy")?))
            })
            .collect()
    }

    /// Whether the word box overlaps the circle of a small dot at `(dx, dy)`.
    fn covers(b: (f64, f64, f64, f64), (dx, dy): (f64, f64)) -> bool {
        b.0 < dx + SMALL_DOT && dx - SMALL_DOT < b.2 && b.1 < dy + SMALL_DOT && dy - SMALL_DOT < b.3
    }

    /// A dot where the usual spot is sends the time elsewhere, and a dot elsewhere does nothing; a
    /// crowd of dots does not outweigh a line through the time, which is the worse of the two.
    #[test]
    fn a_time_gives_way_to_a_dot_but_gives_a_dot_up_before_a_line() {
        let usual = (109.0, 115.0, "start");
        let spot = |heavy: &[Seg], dots: &[(f64, f64)]| {
            time_label_spot((100.0, 100.0), 560.0, heavy, &[], dots)
        };
        assert_eq!(spot(&[], &[]), usual);
        let under = [(120.0, 112.0)];
        let moved = spot(&[], &under);
        assert_ne!(moved, usual);
        let b = word_box(moved.0, moved.1, 5, 10.0, moved.2);
        assert!(!covers(b, under[0]), "{moved:?} still covers the dot");
        // A dot far from every candidate changes nothing.
        assert_eq!(spot(&[], &[(300.0, 100.0)]), usual);
        // Two dots in each of the other seven spots, one in the usual one, and a line through the
        // usual one: the usual spot has the fewest dots and the most wrong, and the time still
        // goes where the line is not. (Were a line to weigh what a dot does, the usual spot
        // would tie with the others and win, being first.)
        let crowd = [
            (130.0, 115.0),
            (66.0, 115.0),
            (75.0, 115.0),
            (130.0, 100.0),
            (122.0, 100.0),
            (66.0, 100.0),
            (75.0, 100.0),
            (100.0, 124.0),
            (92.0, 124.0),
            (130.0, 90.0),
            (122.0, 90.0),
            (66.0, 90.0),
            (75.0, 90.0),
            (100.0, 84.0),
            (108.0, 84.0),
        ];
        let line = (105.0, 118.0, 150.0, 108.0);
        assert_eq!(
            spot(&[], &crowd),
            usual,
            "without the line the usual spot costs least"
        );
        let chosen = spot(&[line], &crowd);
        let b = word_box(chosen.0, chosen.1, 5, 10.0, chosen.2);
        assert!(!meets_box(line, b), "{chosen:?} is struck by the line");
    }

    /// In the picture: the highlighted morning's time is not written over another morning's dot
    /// when there is room beside it. The first render of the book's figure had "08:22" with a
    /// dot under its first digit. The same data without that one dot is the counterpart: the
    /// time is where it always is.
    #[test]
    fn a_time_is_not_written_over_another_mornings_dot() {
        let v = voices();
        let target = 5 * 3600;
        // A morning whose dots sit, in every lane, at the height that puts one under the time.
        let blocker = (1..400)
            .find(|&mi| (-2.0..=3.0).contains(&(spread(mi) * 11.0)))
            .expect("some morning sits at that height");
        let build = |with_blocker: bool| {
            // Each later voice was earlier on the clock, so the morning's line leaves every dot
            // down and to the left and the spot to its right is free of it.
            let mut ms = vec![morning(
                "2026-10-26",
                &[target, 4 * 3600 + 1800, 4 * 3600 + 900],
            )];
            // Fillers far along the clock, so they are nowhere near the time.
            ms.extend(
                (1..blocker)
                    .map(|i| morning(&format!("2026-09-{:02}", i % 28 + 1), &[8 * 3600 + 1800])),
            );
            // The blocker sits ten minutes after the target.
            ms.push(morning(
                "2026-09-01",
                &[if with_blocker {
                    target + 640
                } else {
                    8 * 3600 + 1800
                }],
            ));
            ms
        };
        let usual = {
            let svg = morning_order(&v, &build(false), (4, 8));
            let now: Vec<_> = drawn_words(&svg)
                .into_iter()
                .filter(|(k, _)| k == "mo-now-t")
                .collect();
            assert_eq!(now.len(), 3);
            now[0].1
        };
        let svg = morning_order(&v, &build(true), (4, 8));
        // The radius the times keep clear of is the radius drawn (3 for a morning, 5 for the
        // highlighted one), not only a number the test and the placement agree on.
        assert!(svg.contains(r#"cx="139.3" cy="#) && svg.contains(r#"r="3" data-ck-s"#));
        assert!(svg.contains(r#"r="5" data-ck-s"#));
        let dots = drawn_small_dots(&svg);
        let near_usual = |d: &(f64, f64)| covers(usual, *d);
        assert!(
            dots.iter().any(near_usual),
            "the premise: a dot sits where the time usually goes ({usual:.0?}); dots {dots:.0?}"
        );
        let now: Vec<_> = drawn_words(&svg)
            .into_iter()
            .filter(|(k, _)| k == "mo-now-t")
            .collect();
        assert_eq!(now.len(), 3);
        for (_, b) in &now {
            assert!(
                !dots.iter().any(|d| covers(*b, *d)),
                "a time at {b:.0?} is written over a dot"
            );
        }
        assert_ne!(now[0].1, usual, "the time did not move off the dot");
    }

    /// A dot at the end of the clock, with its line doubling back across the whole picture, has the
    /// usual spot for its time struck by that line and the spots on the right off the edge: the
    /// time is put where neither is so, and every word stays inside the picture.
    #[test]
    fn a_time_at_the_edge_is_neither_struck_nor_off_the_picture() {
        let (v, _) = a_late_newest_morning();
        let m = [morning(
            "2026-10-26",
            &[8 * 3600 + 3540, 6 * 3600, 7 * 3600],
        )];
        let svg = morning_order(&v, &m, (4, 8));
        let heavy: Vec<Seg> = drawn_lines(&svg)
            .into_iter()
            .filter(|(now, _)| *now)
            .flat_map(|(_, s)| s)
            .collect();
        let words = drawn_words(&svg);
        assert!(words.iter().any(|(k, _)| k == "mo-now-t"));
        for (kind, b) in &words {
            assert!(
                b.0 >= 0.0 && b.2 <= 560.0,
                "{kind} at {b:.0?} leaves the 560-wide picture"
            );
            if kind == "mo-now-t" {
                assert!(
                    !heavy.iter().any(|s| meets(*s, *b)),
                    "a time at {b:.0?} is struck by its own line"
                );
            }
        }
    }

    /// A segment loses exactly the part of it inside a band, and what is left meets the band's
    /// edges: one band takes the middle out, two take two, and a flat segment is kept or dropped
    /// whole.
    #[test]
    fn a_line_loses_only_the_part_inside_a_band() {
        let down = [(0.0, 0.0), (100.0, 100.0)];
        assert_eq!(
            outside_bands(&down, &[(20.0, 40.0)]),
            vec![(0.0, 0.0, 20.0, 20.0), (40.0, 40.0, 100.0, 100.0)]
        );
        let two = outside_bands(&down, &[(20.0, 40.0), (60.0, 70.0)]);
        assert_eq!(two.len(), 3, "{two:?}");
        assert_eq!((two[1].1, two[1].3), (40.0, 60.0));
        // The same line drawn upward is cut at the same places.
        let up = outside_bands(&[(100.0, 100.0), (0.0, 0.0)], &[(20.0, 40.0)]);
        assert_eq!(up, vec![(100.0, 100.0, 40.0, 40.0), (20.0, 20.0, 0.0, 0.0)]);
        // A flat segment inside a band is dropped, one outside it kept, and a segment clear of
        // every band is returned as it was.
        assert!(outside_bands(&[(0.0, 30.0), (50.0, 30.0)], &[(20.0, 40.0)]).is_empty());
        assert_eq!(
            outside_bands(&[(0.0, 50.0), (50.0, 50.0)], &[(20.0, 40.0)]),
            vec![(0.0, 50.0, 50.0, 50.0)]
        );
        assert_eq!(
            outside_bands(&down, &[(200.0, 300.0)]),
            vec![(0.0, 0.0, 100.0, 100.0)]
        );
    }

    /// Pieces that meet at a dot stay one subpath (so the join is round), and a gap starts a new one.
    #[test]
    fn pieces_that_meet_are_one_subpath_and_a_gap_starts_another() {
        let d = path_d(&[
            (0.0, 0.0, 10.0, 10.0),
            (10.0, 10.0, 20.0, 5.0),
            (30.0, 5.0, 40.0, 9.0),
        ]);
        assert_eq!(d, "M 0.0 0.0 L 10.0 10.0 L 20.0 5.0 M 30.0 5.0 L 40.0 9.0");
        assert_eq!(path_d(&[]), "");
    }

    /// Leaving the words' band out takes nothing from the dots: every line still starts at its
    /// first dot and ends at its last, because a dot is never inside a band.
    #[test]
    fn a_line_still_runs_from_its_first_dot_to_its_last() {
        let (v, m) = a_late_newest_morning();
        let svg = morning_order(&v, &m, (4, 8));
        let heavy: Vec<Seg> = drawn_lines(&svg)
            .into_iter()
            .filter(|(now, _)| *now)
            .flat_map(|(_, s)| s)
            .collect();
        let dots: Vec<(f64, f64)> = svg
            .split("<circle class=\"mo-dot")
            .skip(1)
            .filter(|c| {
                c.split('"').next().unwrap_or("").contains("is-now") || c.contains(" is-now\"")
            })
            .filter_map(|c| {
                let num = |a: &str| {
                    c.split(&format!("{a}=\""))
                        .nth(1)?
                        .split('"')
                        .next()?
                        .parse::<f64>()
                        .ok()
                };
                Some((num("cx")?, num("cy")?))
            })
            .take(3)
            .collect();
        assert_eq!(dots.len(), 3, "the three large dots");
        let near =
            |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 0.2 && (p.1 - q.1).abs() < 0.2;
        assert!(
            heavy.iter().any(|s| near((s.0, s.1), dots[0])),
            "starts at the first dot"
        );
        assert!(
            heavy.iter().any(|s| near((s.2, s.3), dots[2])),
            "ends at the last dot"
        );
    }

    // ── what the reader sees: no line runs through a word ─────────────────────────────────────
    //
    // The figure used to draw its words last, with a halo, so that a line passing a name was
    // hidden behind the letters. That left the newest morning's heavy line cutting through
    // "167 mornings · 82%" and the times beside its dots whenever the morning was a late one,
    // because the lane labels sit between one lane's dots and the next lane's. These read the
    // markup back as geometry (the lines as segments, each word as a box from its anchor and an
    // estimated width) and ask the question the eye asks.

    type Seg = (f64, f64, f64, f64);

    /// Every drawn line of `svg` as `(is the highlighted morning, its segments)`. Reads both
    /// `<polyline points="x,y x,y">` and `<path d="M x y L x y M x y L x y">`.
    fn drawn_lines(svg: &str) -> Vec<(bool, Vec<Seg>)> {
        let mut out = Vec::new();
        for el in svg.split('<').filter(|e| e.contains("class=\"mo-line")) {
            let now = el.contains("is-now");
            let mut segs = Vec::new();
            if let Some(i) = el.find("points=\"") {
                let pts: Vec<(f64, f64)> = el[i + 8..]
                    .split('"')
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .filter_map(|p| p.split_once(','))
                    .filter_map(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
                    .collect();
                segs.extend(pts.windows(2).map(|w| (w[0].0, w[0].1, w[1].0, w[1].1)));
            } else if let Some(i) = el.find(" d=\"") {
                let d = el[i + 4..].split('"').next().unwrap_or("");
                let mut cur = (0.0, 0.0);
                let mut toks = d.split_whitespace();
                while let Some(t) = toks.next() {
                    if t == "M" || t == "L" {
                        let (Some(x), Some(y)) = (toks.next(), toks.next()) else {
                            break;
                        };
                        let to: (f64, f64) = (x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                        if t == "L" {
                            segs.push((cur.0, cur.1, to.0, to.1));
                        }
                        cur = to;
                    }
                }
            }
            out.push((now, segs));
        }
        out
    }

    /// Every word of the lanes as `(class, box)`, the box `(x0, y0, x1, y1)` from the anchor and a
    /// width estimated at 0.6 em a character (a little generous for a proportional face, right
    /// for the mono one), a line from 0.8 em above the baseline to 0.25 em below.
    fn drawn_words(svg: &str) -> Vec<(String, (f64, f64, f64, f64))> {
        let mut out = Vec::new();
        for el in svg.split("<text ").skip(1) {
            let class = el
                .split("class=\"")
                .nth(1)
                .and_then(|c| c.split('"').next())
                .unwrap_or("");
            let Some(kind) = ["mo-name", "mo-count", "mo-typ-t", "mo-now-t"]
                .into_iter()
                .find(|k| class.split_whitespace().any(|c| c == *k))
            else {
                continue;
            };
            let num = |attr: &str| -> f64 {
                el.split(&format!("{attr}=\""))
                    .nth(1)
                    .and_then(|v| v.split('"').next())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0)
            };
            let anchor = el
                .split("text-anchor=\"")
                .nth(1)
                .and_then(|v| v.split('"').next())
                .unwrap_or("start");
            let text = el
                .split('>')
                .nth(1)
                .and_then(|t| t.split('<').next())
                .unwrap_or("");
            let fs = match kind {
                "mo-name" => 13.0,
                "mo-count" => 11.0,
                _ => 10.0,
            };
            let w = text.chars().count() as f64 * fs * 0.6;
            let (x, y) = (num("x"), num("y"));
            let x0 = match anchor {
                "end" => x - w,
                "middle" => x - w / 2.0,
                _ => x,
            };
            out.push((kind.to_owned(), (x0, y - fs * 0.8, x0 + w, y + fs * 0.25)));
        }
        out
    }

    /// Whether segment `s` meets the box (an endpoint inside, or it crosses an edge).
    fn meets(s: Seg, b: (f64, f64, f64, f64)) -> bool {
        let inside = |x: f64, y: f64| x >= b.0 && x <= b.2 && y >= b.1 && y <= b.3;
        if inside(s.0, s.1) || inside(s.2, s.3) {
            return true;
        }
        let cross = |a: Seg, c: Seg| {
            let d = (a.2 - a.0) * (c.3 - c.1) - (a.3 - a.1) * (c.2 - c.0);
            if d.abs() < 1e-12 {
                return false;
            }
            let t = ((c.0 - a.0) * (c.3 - c.1) - (c.1 - a.1) * (c.2 - c.0)) / d;
            let u = ((c.0 - a.0) * (a.3 - a.1) - (c.1 - a.1) * (a.2 - a.0)) / d;
            (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
        };
        [
            (b.0, b.1, b.2, b.1),
            (b.2, b.1, b.2, b.3),
            (b.2, b.3, b.0, b.3),
            (b.0, b.3, b.0, b.1),
        ]
        .into_iter()
        .any(|e| cross(s, e))
    }

    /// A station whose newest complete morning is a late one (08:00, 08:13, 08:22, as on the
    /// fixture, where the heavy line ran through the right-hand count of the lanes below it), with
    /// a dozen earlier mornings spread across the dawn.
    fn a_late_newest_morning() -> (Vec<Voice>, Vec<Morning>) {
        let mut m = vec![morning(
            "2026-10-26",
            &[8 * 3600, 8 * 3600 + 780, 8 * 3600 + 1320],
        )];
        for i in 0..12u32 {
            m.push(morning(
                &format!("2026-10-{:02}", 25 - i),
                &[
                    5 * 3600 + i * 1100,
                    6 * 3600 + i * 900,
                    6 * 3600 + 1800 + i * 800,
                ],
            ));
        }
        let mut v = voices();
        v[2].typical_secs = Some(8 * 3600 + 400);
        (v, m)
    }

    /// No line crosses a name, a count or a typical time, in any lane: the lines are drawn in
    /// pieces that leave out the band the words are written in.
    #[test]
    fn no_line_runs_through_a_name_a_count_or_a_typical_time() {
        let (v, m) = a_late_newest_morning();
        let svg = morning_order(&v, &m, (4, 8));
        let lines = drawn_lines(&svg);
        assert!(lines.len() >= 2, "the fixture draws lines: {}", lines.len());
        assert!(
            lines.iter().any(|(now, s)| *now && !s.is_empty()),
            "the highlighted morning has a line to test"
        );
        let words: Vec<_> = drawn_words(&svg)
            .into_iter()
            .filter(|(k, _)| k != "mo-now-t")
            .collect();
        assert!(
            words.len() >= 3 * 2,
            "names and counts of three lanes: {}",
            words.len()
        );
        let mut struck = Vec::new();
        for (kind, b) in &words {
            for (now, segs) in &lines {
                for s in segs {
                    if meets(*s, *b) {
                        struck.push(format!(
                            "{kind} {b:.0?} by a {} line {s:.0?}",
                            if *now { "heavy" } else { "thin" }
                        ));
                    }
                }
            }
        }
        assert!(
            struck.is_empty(),
            "lines run through words:\n  {}",
            struck.join("\n  ")
        );
    }

    /// The times written beside the highlighted morning's large dots are not struck by that
    /// morning's own line: each is put on a side the line does not use.
    #[test]
    fn the_times_beside_the_large_dots_are_not_struck_by_their_own_line() {
        // A shallow line (the voices a quarter of an hour apart, as on the fixture), a steep one
        // (an hour and a half between voices) and one that doubles back to the left, which is
        // what the figure is for. A steep line is the one that reaches the label put beside a
        // dot, because it travels sideways as much as down.
        let (v, base) = a_late_newest_morning();
        let with_newest = |t: [u32; 3]| {
            let mut m = a_late_newest_morning().1;
            m[0] = morning("2026-10-26", &t);
            m
        };
        let cases = [
            ("shallow", base),
            ("steep", with_newest([5 * 3600, 6 * 3600 + 1800, 8 * 3600])),
            (
                "doubling back",
                with_newest([8 * 3600 + 3000, 6 * 3600, 7 * 3600]),
            ),
        ];
        for (what, m) in cases {
            let svg = morning_order(&v, &m, (4, 8));
            let heavy: Vec<Seg> = drawn_lines(&svg)
                .into_iter()
                .filter(|(now, _)| *now)
                .flat_map(|(_, s)| s)
                .collect();
            assert!(!heavy.is_empty(), "{what}");
            let now_times: Vec<_> = drawn_words(&svg)
                .into_iter()
                .filter(|(k, _)| k == "mo-now-t")
                .collect();
            assert_eq!(
                now_times.len(),
                3,
                "{what}: a time for each of the three large dots"
            );
            for (_, b) in &now_times {
                for s in &heavy {
                    assert!(
                        !meets(*s, *b),
                        "{what}: a time at {b:.0?} is struck by the heavy line {s:.0?}"
                    );
                }
            }
        }
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
