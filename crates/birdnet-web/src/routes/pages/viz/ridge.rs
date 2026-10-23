//! The migration ridgeline: one ridge per species across the calendar year, and under them
//! how many species were heard each week, on the same weeks.
//!
//! # What the picture claims
//!
//! * A ridge's **height** is that species' detections in a week as a share of *its own*
//!   busiest week. Heights are not comparable between species and the caption says so: a
//!   wren heard twenty times a week and a robin heard two hundred have the same peak, which
//!   is the point of the chart (when each is here, not how loud) and also the reason it
//!   cannot say who is commoner. The tooltip carries the real counts.
//! * Its **colour** is the season it peaks in: the spring window, the fall window, or
//!   neither. Species are told apart by the name beside the ridge, so the colour is free to
//!   say something, and twelve species do not need twelve hues (the palette has eight).
//! * Its **shape** between weeks is a monotone cubic: the curve never rises above a week's
//!   peak or dips below a quiet one, so smoothing cannot invent a peak.
//! * **Weeks that have not happened yet** are hatched, not drawn as zero.
//!
//! The lane under the ridges is distinct species heard per week, on the same x axis; one
//! pointer reading speaks for both (the kit's "cols" mode: a crosshair over a week lists every
//! species heard in it and says how many kinds that was).
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
#![allow(clippy::items_after_statements, clippy::many_single_char_names)]

use std::fmt::Write as _;

use super::kit::{self, Col};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// Columns of the chart: the year in weeks of seven days, the last absorbing the remainder
/// (`WEEK_COLUMN` in `pages/migration.rs`).
pub const WEEKS: usize = 52;

/// Which window of the year a species peaks in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Season {
    /// Peak inside the spring migration window.
    Spring,
    /// Peak inside the fall migration window.
    Fall,
    /// Peak at some other time of year.
    Elsewhere,
}

impl Season {
    /// The kit colour class of a ridge that peaks in this window.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Spring => "ck-c1",
            Self::Fall => "ck-c5",
            Self::Elsewhere => "ck-cother",
        }
    }
}

/// An inclusive range of week columns.
pub type Weeks = (usize, usize);

/// The season a week belongs to.
pub fn season_of(week: usize, spring: Weeks, fall: Weeks) -> Season {
    if (spring.0..=spring.1).contains(&week) {
        Season::Spring
    } else if (fall.0..=fall.1).contains(&week) {
        Season::Fall
    } else {
        Season::Elsewhere
    }
}

/// The spring and fall migration windows, in week columns, for a station at latitude `lat`
/// (`None` where the station has no configured location).
///
/// The windows are the northern temperate ones: spring from the start of March to the third
/// week of May, fall from the last week of August to the first week of November. South of
/// the equator the seasons are the other way round, so the windows swap, and a station there
/// no longer sees its austral spring called fall. Near the equator neither window means much;
/// the chart still draws them and the legend says which weeks they are.
pub fn migration_windows(lat: Option<f64>) -> (Weeks, Weeks) {
    const MARCH_TO_MAY: Weeks = (8, 20);
    const AUGUST_TO_OCTOBER: Weeks = (34, 44);
    if lat.is_some_and(|l| l < 0.0) {
        (AUGUST_TO_OCTOBER, MARCH_TO_MAY)
    } else {
        (MARCH_TO_MAY, AUGUST_TO_OCTOBER)
    }
}

/// One species' year.
pub struct Ridge {
    /// The name as the station displays it.
    pub name: String,
    /// Detections in each week column.
    pub counts: [f64; WEEKS],
}

impl Ridge {
    /// The week column with the most detections (the first of equals).
    pub fn peak_week(&self) -> usize {
        let mut best = 0;
        for (w, &c) in self.counts.iter().enumerate() {
            if c > self.counts[best] {
                best = w;
            }
        }
        best
    }
}

/// Everything the chart needs.
pub struct Phenology<'a> {
    /// The ridges, top to bottom.
    pub ridges: &'a [Ridge],
    /// Distinct species heard in each week column.
    pub diversity: &'a [i64; WEEKS],
    /// The calendar year drawn.
    pub year: i32,
    /// The week column of today, or `None` when the year drawn is over (nothing is "not yet").
    pub today_week: Option<usize>,
    /// The spring window, in week columns.
    pub spring: Weeks,
    /// The fall window.
    pub fall: Weeks,
}

const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Day of the year (1-based) on which each month of `year` starts.
fn month_starts(year: i32) -> [u32; 12] {
    let feb = if is_leap(year) { 29 } else { 28 };
    let lens = [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut out = [1_u32; 12];
    let mut d = 1;
    for (i, l) in lens.iter().enumerate() {
        out[i] = d;
        d += l;
    }
    out
}

/// Where a day of the year sits on the week axis. A week column is a point at the middle of
/// its seven days (its fourth), so day `d` is at `(d - 4) / 7`, and the last column runs to the
/// end of the year.
pub fn week_pos(day_of_year: u32) -> f64 {
    ((f64::from(day_of_year) - 4.0) / 7.0).clamp(0.0, (WEEKS - 1) as f64)
}

/// `1 Apr` for the `n`th day (1-based) of `year`.
fn day_label(year: i32, n: u32) -> String {
    let starts = month_starts(year);
    let m = starts.iter().rposition(|&s| s <= n).unwrap_or(0);
    format!("{} {}", n - starts[m] + 1, kit::MONTHS[m])
}

/// The first and last day of the year (1-based) a week column covers; the last column runs to
/// 31 December.
pub const fn week_days(year: i32, week: usize) -> (u32, u32) {
    let first = week as u32 * 7 + 1;
    let last = if week == WEEKS - 1 {
        if is_leap(year) { 366 } else { 365 }
    } else {
        first + 6
    };
    (first, last)
}

/// `2 Apr – 8 Apr` for a week column.
pub fn week_label(year: i32, week: usize) -> String {
    let (a, b) = week_days(year, week);
    format!("{} – {}", day_label(year, a), day_label(year, b))
}

/// A name short enough for the gutter beside a ridge ("Ruby-throated Hummingbird" is
/// "R. Hummingbird"): the way a birder abbreviates, which keeps species that share a first word
/// apart. The whole name is in the ridge's title, the readout and the table.
fn short_name(name: &str) -> String {
    if name.chars().count() <= 22 {
        return name.to_owned();
    }
    match name.split_once(' ') {
        Some((first, rest)) if !rest.is_empty() => {
            format!("{}. {rest}", first.chars().next().unwrap_or('?'))
        }
        _ => name.to_owned(),
    }
}

/// The ridgeline and its lane, in a kit chart (`data-ck="cols"`), with the table under it.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn ridgeline(p: &Phenology<'_>) -> String {
    let n = p.ridges.len();
    if n == 0 {
        return EMPTY.to_string();
    }
    const W: f64 = 1200.0;
    const PAD_L: f64 = 184.0;
    const PAD_R: f64 = 22.0;
    const TOP: f64 = 44.0;
    const LANE_H: f64 = 60.0;
    let inner_w = W - PAD_L - PAD_R;
    let x = |w: f64| PAD_L + w / (WEEKS as f64 - 1.0) * inner_w;
    let step = inner_w / (WEEKS as f64 - 1.0);
    let row_h = (340.0 / n as f64).clamp(34.0, 60.0);
    let amp = row_h * 1.85;
    let base_of = |i: usize| TOP + amp + i as f64 * row_h;
    let ridges_bottom = base_of(n - 1) + 6.0;
    let lane_top = ridges_bottom + 34.0;
    let lane_base = lane_top + LANE_H;
    let h = lane_base + 30.0;
    let last_week = p.today_week.unwrap_or(WEEKS - 1).min(WEEKS - 1);

    let classes: Vec<String> = [Season::Spring, Season::Fall, Season::Elsewhere]
        .iter()
        .map(|s| s.class().to_owned())
        .collect();

    // What a column says: the real counts, and how many kinds were heard.
    let series: Vec<(String, String)> = p
        .ridges
        .iter()
        .map(|r| {
            let s = season_of(r.peak_week(), p.spring, p.fall);
            let var = match s {
                Season::Spring => "var(--chart-1)",
                Season::Fall => "var(--chart-5)",
                Season::Elsewhere => "var(--chart-other)",
            };
            (r.name.clone(), var.to_owned())
        })
        .collect();
    let cols: Vec<Col> = (0..WEEKS)
        .map(|w| {
            let future = w > last_week;
            let label = format!("Week {} · {}", w + 1, week_label(p.year, w));
            if future {
                Col {
                    label,
                    values: vec![Some(0.0); n],
                    note: Some("This week has not happened yet.".to_owned()),
                }
            } else {
                let kinds = p.diversity[w];
                let under_way = p.today_week == Some(w);
                Col {
                    label,
                    values: p.ridges.iter().map(|r| Some(r.counts[w])).collect(),
                    note: Some(format!(
                        "{kinds} species heard{}",
                        if under_way { " so far this week" } else { "" }
                    )),
                }
            }
        })
        .collect();
    let attrs = kit::cols_attrs(
        &cols,
        &series,
        x(0.0),
        x(WEEKS as f64 - 1.0),
        false,
        "detections",
        None,
    );

    let mut s = format!(
        r#"<div class="ck rd-fig"><div class="rd-scroll"><svg viewBox="0 0 {W} {h:.0}" width="100%" role="img" class="rd-svg ck-in" data-ck-id="migration-ridges" data-ck-dim="series" data-ck-label="Left and right arrows read each week, up and down bring one species forward"{attrs}>"#,
    );
    s.push_str(&svg_a11y(
        "Twelve months of migrants",
        &format!(
            "{n} ridges, one per species, across the {} weeks of {}: a ridge is how many of that species were heard each week, as a share of its own busiest week. Below them, how many different species were heard each week. Spring and fall migration windows are shaded.",
            WEEKS, p.year
        ),
    ));
    s.push_str(&kit::gradient_defs("rd", &classes));
    s.push_str(&kit::nodata_pattern("rd"));

    // The migration windows, behind everything, with their names.
    for (win, class, name) in [
        (p.spring, "rd-band rd-band-spring", "Spring migration"),
        (p.fall, "rd-band rd-band-fall", "Fall migration"),
    ] {
        let (x0, x1) = (x(win.0 as f64 - 0.5), x(win.1 as f64 + 0.5));
        let _ = write!(
            s,
            r#"<rect class="{class}" x="{x0:.1}" y="{TOP}" width="{w:.1}" height="{hh:.1}"/><text class="rd-band-name ck-annot" x="{cx:.1}" y="14" text-anchor="middle">{name}</text><text class="rd-band-weeks ck-lbl mono" x="{cx:.1}" y="28" text-anchor="middle">weeks {a}–{b}</text>"#,
            w = x1 - x0,
            hh = lane_base - TOP,
            cx = f64::midpoint(x0, x1),
            a = win.0 + 1,
            b = win.1 + 1,
        );
    }

    // Months.
    for (m, &d) in month_starts(p.year).iter().enumerate() {
        let xm = x(week_pos(d));
        let _ = write!(
            s,
            r#"<line class="ck-grid" x1="{xm:.1}" y1="{TOP}" x2="{xm:.1}" y2="{lane_base:.1}"/><text class="ck-lbl rd-month" x="{tx:.1}" y="{ty:.1}" text-anchor="start">{label}</text>"#,
            tx = xm + 3.0,
            ty = lane_base + 16.0,
            label = kit::MONTHS[m],
        );
    }

    // Ridges, the top one first and so behind: each has an opaque base the colour of the card
    // under its fill, so the ridge in front hides the one behind it where they overlap, the way
    // a joyplot does.
    for (i, ridge) in p.ridges.iter().enumerate() {
        let base = base_of(i);
        let max = ridge
            .counts
            .iter()
            .copied()
            .fold(0.0_f64, f64::max)
            .max(f64::MIN_POSITIVE);
        let season = season_of(ridge.peak_week(), p.spring, p.fall);
        let pts: Vec<(f64, f64)> = (0..=last_week)
            .map(|w| (x(w as f64), base - ridge.counts[w] / max * amp))
            .collect();
        let line = kit::monotone_path(&pts, false);
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        let area = format!(
            "{line} L{:.1},{base:.1} L{:.1},{base:.1} Z",
            last.0, first.0
        );
        let name = escape_html(&ridge.name);
        let _ = write!(
            s,
            r#"<g class="rd-ridge {class} ck-reveal" data-ck-s="{i}"><line class="ck-grid" x1="{PAD_L}" y1="{base:.1}" x2="{xr:.1}" y2="{base:.1}"/><path class="rd-mask" d="{area}"/><path class="rd-area" d="{area}" fill="url(#rd-{class})"/><path class="ck-line rd-line" d="{line}"/><title>{name}</title></g>"#,
            class = season.class(),
            xr = x(WEEKS as f64 - 1.0),
        );
    }

    // The peaks and the names go over every ridge, not under the next one's fill.
    for (i, ridge) in p.ridges.iter().enumerate() {
        let base = base_of(i);
        let season = season_of(ridge.peak_week(), p.spring, p.fall);
        let name = escape_html(&short_name(&ridge.name));
        // The peak.
        let pw = ridge.peak_week();
        if pw <= last_week && ridge.counts[pw] > 0.0 {
            let _ = write!(
                s,
                r#"<circle class="ck-dot {class}" data-ck-s="{i}" cx="{px:.1}" cy="{py:.1}" r="3.5"/>"#,
                class = season.class(),
                px = x(pw as f64),
                py = base - amp,
            );
        }
        // The name, and when it peaks, on the left.
        let (peak_a, _) = week_days(p.year, pw);
        let _ = write!(
            s,
            r#"<text class="rd-name ck-sticky ck-halo" x="{nx:.1}" y="{ny:.1}" text-anchor="end" data-ck-s="{i}">{name}</text><text class="rd-sub mono ck-sticky ck-halo" x="{nx:.1}" y="{sy:.1}" text-anchor="end" data-ck-s="{i}">peak {when}</text>"#,
            nx = PAD_L - 12.0,
            ny = base - 7.0,
            sy = base + 9.0,
            when = day_label(p.year, peak_a),
        );
    }

    // The lane: kinds heard each week.
    let max_kinds = p.diversity.iter().copied().max().unwrap_or(0).max(1) as f64;
    let _ = write!(
        s,
        r#"<text class="rd-name ck-sticky ck-halo" x="{nx:.1}" y="{ny:.1}" text-anchor="end">Species heard</text><text class="rd-sub mono ck-sticky ck-halo" x="{nx:.1}" y="{sy:.1}" text-anchor="end">each week</text><line class="ck-axis" x1="{PAD_L}" y1="{lane_base:.1}" x2="{xr:.1}" y2="{lane_base:.1}"/>"#,
        nx = PAD_L - 12.0,
        ny = lane_top + LANE_H / 2.0 - 2.0,
        sy = lane_top + LANE_H / 2.0 + 11.0,
        xr = x(WEEKS as f64 - 1.0),
    );
    let mut peak_kinds = (0_usize, 0_i64);
    for w in 0..=last_week {
        let v = p.diversity[w];
        if v > peak_kinds.1 {
            peak_kinds = (w, v);
        }
        if v <= 0 {
            continue;
        }
        let bh = (v as f64 / max_kinds * LANE_H).max(1.5);
        let bw = (step * 0.72).max(2.0);
        let season = season_of(w, p.spring, p.fall);
        let _ = write!(
            s,
            r#"<path class="rd-bar rd-bar-{class}" d="{d}"/>"#,
            class = match season {
                Season::Spring => "spring",
                Season::Fall => "fall",
                Season::Elsewhere => "other",
            },
            d = kit::bar_path(x(w as f64) - bw / 2.0, lane_base - bh, bw, bh, 2.0),
        );
    }
    if peak_kinds.1 > 0 {
        let _ = write!(
            s,
            r#"<text class="ck-annot rd-peak-kinds ck-halo" x="{px:.1}" y="{py:.1}" text-anchor="middle">{k} species</text>"#,
            px = x(peak_kinds.0 as f64),
            py = (lane_base - peak_kinds.1 as f64 / max_kinds * LANE_H - 6.0).max(lane_top - 6.0),
            k = peak_kinds.1,
        );
    }

    // Weeks that have not happened: hatched over everything, so they read as unknown and not as
    // none.
    if last_week < WEEKS - 1 {
        let x0 = x(last_week as f64 + 0.5);
        let _ = write!(
            s,
            r#"<rect x="{x0:.1}" y="{TOP}" width="{w:.1}" height="{hh:.1}" fill="url(#rd-nd)" pointer-events="none"/><text class="ck-lbl rd-notyet" x="{tx:.1}" y="{ty:.1}" text-anchor="start">not yet</text>"#,
            w = x(WEEKS as f64 - 1.0) - x0,
            hh = lane_base - TOP,
            tx = x0 + 8.0,
            ty = TOP + 14.0,
        );
    }

    // Today.
    if let Some(tw) = p.today_week {
        let xt = x(tw as f64);
        let _ = write!(
            s,
            r#"<line class="rd-today ck-scroll-to" x1="{xt:.1}" y1="{y1:.1}" x2="{xt:.1}" y2="{lane_base:.1}"/><rect class="ck-pill" x="{rx:.1}" y="{ry:.1}" width="46" height="15" rx="3"/><text class="ck-pill-t rd-today-t" x="{xt:.1}" y="{ty:.1}" text-anchor="middle">today</text>"#,
            y1 = TOP - 8.0,
            rx = xt - 23.0,
            ry = TOP - 24.0,
            ty = TOP - 13.0,
        );
    }

    let _ = write!(
        s,
        r#"<line class="ck-cursor" x1="0" y1="{TOP}" x2="0" y2="{lane_base:.1}" visibility="hidden"/>"#,
    );
    s.push_str("</svg></div>");
    s.push_str(&key(p));
    s.push_str(&table(p));
    s.push_str("</div>");
    s
}

/// What colour and height mean, as HTML under the picture.
fn key(p: &Phenology<'_>) -> String {
    let swatch = |class: &str, text: String| {
        format!(
            r#"<span class="bnb-meta viz-legend-item"><span class="viz-swatch ck-sw {class}"></span>{text}</span>"#
        )
    };
    format!(
        r#"<div class="viz-legend rd-key">{a}{b}{c}</div><p class="bnb-meta rd-note">A ridge is a species' detections each week as a share of its own busiest week, so a rare bird and a common one reach the same height; the real counts are in the readout and the table. Colour is when it peaks. Lower: how many different species were heard.</p>"#,
        a = swatch(
            Season::Spring.class(),
            format!(
                "peaks in spring migration (weeks {}–{})",
                p.spring.0 + 1,
                p.spring.1 + 1
            )
        ),
        b = swatch(
            Season::Fall.class(),
            format!(
                "peaks in fall migration (weeks {}–{})",
                p.fall.0 + 1,
                p.fall.1 + 1
            )
        ),
        c = swatch(
            Season::Elsewhere.class(),
            "peaks at another time".to_owned()
        ),
    )
}

/// The numbers: for every species its busiest week, its total for the year, and its count in
/// every week; and, last, how many species were heard in each week.
fn table(p: &Phenology<'_>) -> String {
    let mut t = String::from(
        r#"<details class="pt-disc"><summary>See the numbers — detections per week</summary><div class="viz-table-scroll"><table class="viz-table rd-table"><thead><tr><th scope="col">Species</th><th scope="col" class="num">Busiest week</th><th scope="col" class="num">Total</th>"#,
    );
    for w in 0..WEEKS {
        let _ = write!(
            t,
            r#"<th scope="col" class="num" title="{}">{}</th>"#,
            week_label(p.year, w),
            w + 1
        );
    }
    t.push_str("</tr></thead><tbody>");
    for r in p.ridges {
        let _ = write!(
            t,
            r#"<tr><th scope="row">{name}</th><td class="num">w{pk}</td><td class="num">{total}</td>"#,
            name = escape_html(&r.name),
            pk = r.peak_week() + 1,
            total = r.counts.iter().sum::<f64>().round() as i64,
        );
        for c in &r.counts {
            let _ = write!(t, r#"<td class="num">{}</td>"#, c.round() as i64);
        }
        t.push_str("</tr>");
    }
    t.push_str(r#"<tr class="rd-kinds"><th scope="row">Species heard</th><td class="num"></td><td class="num"></td>"#);
    for k in p.diversity {
        let _ = write!(t, r#"<td class="num">{k}</td>"#);
    }
    t.push_str("</tr></tbody></table></div></details>");
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ridge(name: &str, peak_week: usize, peak: f64) -> Ridge {
        let mut counts = [0.0; WEEKS];
        for (w, c) in counts.iter_mut().enumerate() {
            let d = (w as f64 - peak_week as f64).abs();
            *c = (peak * (-(d / 2.5).powi(2)).exp()).round();
        }
        Ridge {
            name: name.to_owned(),
            counts,
        }
    }

    fn phen<'a>(ridges: &'a [Ridge], div: &'a [i64; WEEKS], today: Option<usize>) -> Phenology<'a> {
        Phenology {
            ridges,
            diversity: div,
            year: 2026,
            today_week: today,
            spring: (8, 20),
            fall: (34, 44),
        }
    }

    #[test]
    fn nothing_to_draw_is_the_empty_placeholder() {
        let div = [0; WEEKS];
        assert_eq!(ridgeline(&phen(&[], &div, Some(10))), EMPTY);
    }

    #[test]
    fn the_windows_swap_south_of_the_equator() {
        assert_eq!(migration_windows(Some(51.5)), ((8, 20), (34, 44)));
        assert_eq!(migration_windows(None), ((8, 20), (34, 44)));
        assert_eq!(migration_windows(Some(-33.9)), ((34, 44), (8, 20)));
        assert_eq!(migration_windows(Some(0.0)), ((8, 20), (34, 44)));
        assert_eq!(season_of(10, (8, 20), (34, 44)), Season::Spring);
        assert_eq!(season_of(40, (8, 20), (34, 44)), Season::Fall);
        assert_eq!(season_of(25, (8, 20), (34, 44)), Season::Elsewhere);
    }

    #[test]
    fn weeks_are_named_by_the_days_they_cover() {
        assert_eq!(week_label(2026, 0), "1 Jan – 7 Jan");
        assert_eq!(week_label(2026, 13), "2 Apr – 8 Apr");
        // The last column absorbs the year's last one or two days.
        assert_eq!(week_label(2026, 51), "24 Dec – 31 Dec");
        assert_eq!(week_days(2026, 51), (358, 365));
        assert_eq!(
            week_days(2028, 51),
            (358, 366),
            "a leap year's last column has the extra day"
        );
        // Week 0 is days 1..=7 and the next starts the day after.
        assert_eq!(week_days(2026, 1).0, 8);
    }

    /// Every ridge stands on its own baseline and never goes below it, and none rises past
    /// its own height: a smoothed curve that dipped under the baseline would draw a negative
    /// count, and one that rose past the peak would draw a week busier than any there was.
    #[test]
    fn a_ridge_stays_between_its_baseline_and_its_peak() {
        let ridges = vec![
            ridge("A", 12, 40.0),
            ridge("B", 40, 9.0),
            ridge("C", 25, 300.0),
        ];
        let mut div = [0; WEEKS];
        div.iter_mut()
            .enumerate()
            .for_each(|(w, d)| *d = (w % 9) as i64);
        let svg = ridgeline(&phen(&ridges, &div, Some(51)));
        let h: f64 = svg
            .split("viewBox=\"0 0 1200 ")
            .nth(1)
            .and_then(|x| x.split('"').next())
            .and_then(|x| x.parse().ok())
            .expect("height");
        let mut checked = 0;
        for chunk in svg.split("<path class=\"ck-line rd-line\" d=\"").skip(1) {
            let d = chunk.split('"').next().expect("d");
            let ys: Vec<f64> = d
                .split([' ', 'C', 'M'])
                .filter(|t| t.contains(','))
                .filter_map(|t| t.split(',').nth(1)?.parse().ok())
                .collect();
            assert!(!ys.is_empty());
            let (lo, hi) = ys
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), &y| (lo.min(y), hi.max(y)));
            assert!(
                lo >= -0.5 && hi <= h,
                "outside the picture: {lo}..{hi} of {h}"
            );
            // Each ridge's lowest point is its baseline (it is quiet some weeks): the curve's
            // control points never go past it.
            let base_y = hi;
            assert!(ys.iter().all(|&y| y <= base_y + 1e-6));
            checked += 1;
        }
        assert_eq!(checked, 3, "one ridge each, or the gate is vacuous");
    }

    /// A week that has not happened is hatched and has no ridge in it; when the year is over
    /// there is nothing to hatch. Weeks after today read "not happened yet", never zero.
    #[test]
    fn the_future_is_hatched_not_drawn_as_zero() {
        let ridges = vec![ridge("A", 12, 40.0)];
        let div = [3; WEEKS];
        let part = ridgeline(&phen(&ridges, &div, Some(20)));
        assert!(
            part.contains("fill=\"url(#rd-nd)\""),
            "the future is hatched"
        );
        assert!(part.contains("not yet"));
        assert!(part.contains("This week has not happened yet."));
        assert!(part.contains("class=\"rd-today ck-scroll-to\""));
        let whole = ridgeline(&phen(&ridges, &div, None));
        assert!(!whole.contains("rd-nd)"), "a finished year has no future");
        assert!(!whole.contains("rd-today"), "and no today");
    }

    /// The reading for a week: its heading names the days, its values are the real counts (not
    /// the normalised heights) and its note says how many kinds were heard.
    #[test]
    fn a_weeks_reading_carries_real_counts_and_the_diversity() {
        let ridges = vec![ridge("Barn Swallow", 14, 41.0)];
        let mut div = [0; WEEKS];
        div[14] = 17;
        let svg = ridgeline(&phen(&ridges, &div, Some(51)));
        assert!(svg.contains("Week 15 · 9 Apr – 15 Apr"), "{svg:.300}");
        assert!(svg.contains("17 species heard"));
        assert!(svg.contains("[41.0]"), "the busiest week's count, not 1.0");
        assert!(svg.contains(r#"data-ck="cols""#));
        assert!(svg.contains("data-ck-x0=") && svg.contains("data-ck-x1="));
    }

    #[test]
    fn a_long_name_is_shortened_the_way_a_birder_would() {
        assert_eq!(short_name("Ruby-throated Hummingbird"), "R. Hummingbird");
        assert_eq!(short_name("Song Sparrow"), "Song Sparrow");
        assert_eq!(short_name("Yellow-rumped Warbler"), "Yellow-rumped Warbler");
    }

    /// Colour says where a species peaks, not which species it is: two species that peak in
    /// spring share a hue, and the name beside each ridge is what tells them apart.
    #[test]
    fn colour_is_the_season_of_the_peak() {
        let ridges = vec![
            ridge("Spring bird", 12, 30.0),
            ridge("Fall bird", 40, 30.0),
            ridge("Summer bird", 26, 30.0),
        ];
        let div = [1; WEEKS];
        let svg = ridgeline(&phen(&ridges, &div, Some(51)));
        let classes: Vec<&str> = svg
            .split("<g class=\"rd-ridge ")
            .skip(1)
            .map(|c| {
                c.split('"')
                    .next()
                    .and_then(|c| c.split(' ').next())
                    .expect("class")
            })
            .collect();
        assert_eq!(classes, ["ck-c1", "ck-c5", "ck-cother"]);
    }

    /// The table has every number: a row per species of 52 weeks and one for the kinds.
    #[test]
    fn the_table_holds_every_week() {
        let ridges = vec![ridge("A", 12, 40.0), ridge("B", 40, 9.0)];
        let div = [2; WEEKS];
        let svg = ridgeline(&phen(&ridges, &div, Some(51)));
        let rows = svg.split("<tr>").count() - 1 + svg.matches("<tr class=\"rd-kinds\">").count();
        assert_eq!(rows, 4, "header, two species, species heard");
        assert_eq!(svg.matches("<td class=\"num\">").count(), 3 * (WEEKS + 2));
    }
}
