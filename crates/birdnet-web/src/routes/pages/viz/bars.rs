//! Column charts: detections by hour of the day, and by day.
//!
//! Both are the same drawing (a bar per slot, rounded at the data end and square at
//! the baseline, on a labelled axis from zero), differing in what a slot is and in
//! what they can honestly say about it. They replace three hand-built renderers that
//! each drew flat single-colour bars with a number over every one.
//!
//! * **Which chart for the hours.** A 24-hour chart can be a bar chart or a radial
//!   "clock". The page it lives on already has the radial one (the dawn chorus, a
//!   different question: who sings when, hour by hour, species by species). Totals by
//!   hour are a *comparison of magnitudes* — is 06:00 twice 09:00 — and length on a
//!   common baseline is the encoding people read most accurately; on a clock the same
//!   number is an arc whose area grows with the radius, and the two ends (23:00 and
//!   00:00) that are neighbours in time are the two ends of the page in a bar chart,
//!   which is the only cost of the bars. That cost is paid by shading the night: a
//!   band behind the hours between sunset and sunrise says "these are one night".
//! * **Colour.** One chart, one hue unless the colour means something: dawn and dusk
//!   against the rest (emphasis), or a count through the sequential ramp where the
//!   chart's own question is "how busy" (Today).
//! * **Honesty.** The baseline is zero, always. An hour with nothing is an empty slot
//!   with a tooltip that says so, not a missing bar that reads as missing data. Only
//!   the peak is labelled on the chart: a number over every bar is a table drawn badly.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::kit::{self, Linear};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// What colours a bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fill {
    /// The sequential ramp by count: the chart's question is "how busy".
    Seq,
    /// Dawn (05–08) and dusk (18–21) in the brand hue, the rest neutral.
    DawnDusk,
    /// One categorical slot for every bar (0-based).
    Slot(usize),
}

/// Everything an hour-of-day chart needs.
pub struct Hourly<'a> {
    /// `data-ck-id`: a refresh under the same id does not replay the entrance.
    pub id: &'a str,
    /// The chart's accessible name.
    pub name: &'a str,
    /// One honest sentence for a screen reader.
    pub desc: &'a str,
    /// What is counted, plural (`detections`).
    pub unit: &'a str,
    /// Count per hour, 00 to 23.
    pub counts: [i64; 24],
    /// How the bars are coloured.
    pub fill: Fill,
    /// Drawing width in viewBox units.
    pub width: f64,
    /// Height of the plot area in viewBox units.
    pub plot_h: f64,
    /// Sunrise and sunset as fractional hours, when the station has a location.
    pub solar: Option<(f64, f64)>,
    /// The current hour as a fraction, for a chart of today.
    pub now_h: Option<f64>,
    /// For each hour, the species heard most then and how many.
    pub top: Option<&'a [Vec<(String, i64)>; 24]>,
    /// `(hour, °C)` samples drawn as a second, separately scaled strip below.
    pub temps: &'a [(f64, f64)],
    /// A "Show as a table" disclosure with all 24 rows.
    pub table: bool,
}

fn clock(h: usize) -> String {
    format!("{h:02}:00")
}

/// `19.5` → `19:30`.
pub fn fmt_clock(h: f64) -> String {
    let total_min = (h * 60.0).round();
    format!(
        "{:.0}:{:02.0}",
        (total_min / 60.0).floor() % 24.0,
        total_min % 60.0
    )
}

/// An hour-of-day column chart.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn hourly(o: &Hourly<'_>) -> String {
    let max = o.counts.iter().copied().max().unwrap_or(0);
    if max <= 0 {
        return EMPTY.to_string();
    }
    let total: i64 = o.counts.iter().sum();
    let peak = o
        .counts
        .iter()
        .enumerate()
        .max_by_key(|&(h, &c)| (c, std::cmp::Reverse(h)))
        .map_or(0, |(h, _)| h);

    let top = kit::count_ceiling(max as f64);
    let rules = kit::count_ticks(top, 4);
    let widest = rules
        .iter()
        .map(|&v| kit::text_width(&kit::compact(v), 10.0))
        .fold(0.0_f64, f64::max);
    let w = o.width;
    let (pad_l, pad_r) = (kit::axis_margin(widest, w), 6.0);
    let plot_w = w - pad_l - pad_r;
    let plot_h = o.plot_h;
    let slot = plot_w / 24.0;
    let bar_w = (slot * 0.66).min(22.0);
    let y = Linear::new((0.0, top), (plot_h, 0.0));
    let x_of = |hour: f64| pad_l + hour / 24.0 * plot_w;

    let head = 22.0;
    let axis_rows = if o.solar.is_some() { 36.0 } else { 20.0 };
    let temp_h = if o.temps.len() >= 2 { 58.0 } else { 0.0 };
    let h_total = head + plot_h + axis_rows + temp_h;

    // Colour class per bar.
    let class_of = |h: usize, c: i64| -> String {
        match o.fill {
            Fill::Seq => format!("ck-q{}", kit::seq_step(c as f64 / max as f64)),
            Fill::DawnDusk => {
                if (5..=8).contains(&h) || (18..=21).contains(&h) {
                    "ck-c1".to_string()
                } else {
                    "ck-cother".to_string()
                }
            }
            Fill::Slot(k) => kit::slot_class(k),
        }
    };
    let mut classes: Vec<String> = o
        .counts
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c > 0)
        .map(|(h, &c)| class_of(h, c))
        .collect();
    classes.sort();
    classes.dedup();

    let mut svg = String::with_capacity(12 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><div class="viz-scroll"><svg width="100%" viewBox="0 -{head:.0} {w:.0} {h_total:.0}" role="img" class="viz-svg-block ck-in" data-ck="marks" data-ck-id="{id}" data-ck-dim="index" data-ck-start="{peak}" data-ck-label="Left and right move from hour to hour.">"#,
        id = escape_html(o.id),
    );
    svg.push_str(&svg_a11y(o.name, o.desc));
    let g = kit::id_safe(&format!("g-{}", o.id));
    svg.push_str(&kit::gradient_defs(&g, &classes));

    // Night, behind everything: the hours between sunset and sunrise are one night.
    if let Some((rise, set)) = o.solar {
        let (a, b) = (x_of(rise.clamp(0.0, 24.0)), x_of(set.clamp(0.0, 24.0)));
        if b > a {
            let _ = write!(
                svg,
                r#"<rect class="ck-track" x="{pad_l:.1}" y="0" width="{wa:.1}" height="{plot_h:.1}"/><rect class="ck-track" x="{b:.1}" y="0" width="{wb:.1}" height="{plot_h:.1}"/>"#,
                wa = a - pad_l,
                wb = pad_l + plot_w - b,
            );
        }
    }

    // Rules and their labels.
    for &v in &rules {
        let yy = y.at(v);
        if v > 0.0 {
            let _ = write!(
                svg,
                r#"<line class="ck-grid" x1="{pad_l:.1}" y1="{yy:.1}" x2="{x2:.1}" y2="{yy:.1}"/>"#,
                x2 = pad_l + plot_w,
            );
        }
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{tx:.1}" y="{ty:.1}" text-anchor="end" font-size="10">{l}</text>"#,
            tx = pad_l - 6.0,
            ty = yy + 3.5,
            l = kit::compact(v),
        );
    }

    // Bars.
    svg.push_str("<g>");
    let before_now = |h: usize| o.now_h.is_none_or(|n| (h as f64) <= n);
    for (h, &c) in o.counts.iter().enumerate() {
        if c <= 0 || !before_now(h) {
            continue;
        }
        let bh = plot_h - y.at(c as f64);
        let x = pad_l + h as f64 * slot + (slot - bar_w) / 2.0;
        let cls = class_of(h, c);
        let d = kit::bar_path(x, y.at(c as f64), bar_w, bh, 4.0);
        let _ = write!(
            svg,
            r#"<path class="ck-grow" d="{d}" fill="url(#{g}-{cls})" data-ck-i="{h}" data-style="--i:{h}"/>"#,
        );
    }
    svg.push_str("</g>");
    let _ = write!(
        svg,
        r#"<line class="ck-axis" x1="{pad_l:.1}" y1="{plot_h:.1}" x2="{x2:.1}" y2="{plot_h:.1}"/>"#,
        x2 = pad_l + plot_w,
    );

    // The peak, and only the peak, is written on the chart.
    let now_x = o.now_h.map(x_of);
    let peak_x = pad_l + peak as f64 * slot + slot / 2.0;
    let pill_clash = now_x.is_some_and(|n| (n - peak_x).abs() < 40.0);
    if before_now(peak) && !pill_clash {
        let _ = write!(
            svg,
            r#"<text class="mono ck-annot ck-halo" x="{peak_x:.1}" y="{ty:.1}" text-anchor="middle" font-size="10">{v}</text>"#,
            ty = y.at(o.counts[peak] as f64) - 5.0,
            v = kit::compact(o.counts[peak] as f64),
        );
    }

    // Hours on the axis, every third, and the sun under them.
    for hour in (0..24).step_by(3) {
        let x = pad_l + f64::from(hour) * slot + slot / 2.0;
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{x:.1}" y="{ty:.1}" text-anchor="middle" font-size="10">{hour:02}</text>"#,
            ty = plot_h + 15.0,
        );
    }
    if let Some((rise, set)) = o.solar {
        let items = [
            ("\u{2600}", "sunrise", rise, "start"),
            ("\u{263e}", "sunset", set, "end"),
        ];
        let mut last_end = f64::NEG_INFINITY;
        for (glyph, word, at, anchor) in items {
            let label = format!("{glyph} {word} {}", fmt_clock(at));
            let tw = kit::text_width(&label, 10.0);
            let x = x_of(at.clamp(0.0, 24.0));
            let (l, r) = if anchor == "start" {
                (x, x + tw)
            } else {
                (x - tw, x)
            };
            if l < last_end + 6.0 {
                continue; // the two sit too close at this latitude: the first one says it
            }
            last_end = r;
            let _ = write!(
                svg,
                r#"<line class="ck-axis" x1="{x:.1}" y1="{t0:.1}" x2="{x:.1}" y2="{t1:.1}"/><text class="mono ck-annot" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10">{label}</text>"#,
                t0 = plot_h + 19.0,
                t1 = plot_h + 23.0,
                ty = plot_h + 33.0,
            );
        }
    }

    // The temperature, on its own labelled scale beneath, never on the bars' axis:
    // two measures on one plot read as a relationship the chart cannot vouch for.
    if o.temps.len() >= 2 {
        let t_min = o.temps.iter().map(|t| t.1).fold(f64::INFINITY, f64::min);
        let t_max = o
            .temps
            .iter()
            .map(|t| t.1)
            .fold(f64::NEG_INFINITY, f64::max);
        let span = (t_max - t_min).max(1.0);
        let base = plot_h + axis_rows + 6.0;
        let (y_top, y_bot) = (base + 4.0, base + 30.0);
        let ty = Linear::new((t_min, t_min + span), (y_bot, y_top));
        let pts: Vec<(f64, f64)> = o
            .temps
            .iter()
            .map(|&(hr, t)| (x_of(hr), ty.at(t)))
            .collect();
        let line = kit::monotone_path(&pts, false);
        let area = format!(
            "{line} L{:.1},{y_bot:.1} L{:.1},{y_bot:.1} Z",
            pts[pts.len() - 1].0,
            pts[0].0
        );
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{tx:.1}" y="{ly:.1}" text-anchor="end" font-size="10" {hi_attr}>{hi_text}</text><text class="mono ck-lbl" x="{tx:.1}" y="{ly2:.1}" text-anchor="end" font-size="10" {lo_attr}>{lo_text}</text><path class="ck-area ck-c5 ck-fill" d="{area}" opacity=".14"/><path class="ck-line ck-c5 ck-drawn" d="{line}" pathLength="1"/>"#,
            tx = pad_l - 6.0,
            ly = y_top + 4.0,
            ly2 = y_bot,
            hi_attr = crate::display::temperature_attr(t_max, 0, false),
            lo_attr = crate::display::temperature_attr(t_min, 0, false),
            hi_text = crate::display::format_temperature(
                t_max,
                0,
                crate::display::TemperatureUnit::Celsius,
                false
            ),
            lo_text = crate::display::format_temperature(
                t_min,
                0,
                crate::display::TemperatureUnit::Celsius,
                false
            ),
        );
        let _ = write!(
            svg,
            r#"<text class="ck-lbl" x="{x:.1}" y="{ly:.1}" font-size="10">Temperature</text>"#,
            x = pad_l,
            ly = base + 46.0,
        );
    }

    // Now.
    if let Some(n) = o.now_h.filter(|n| (0.0..24.0).contains(n)) {
        let nx = x_of(n);
        let _ = write!(
            svg,
            r#"<line class="ck-axis" x1="{nx:.1}" y1="-4" x2="{nx:.1}" y2="{plot_h:.1}" stroke-width="1.5"/><rect class="ck-pill" x="{px:.1}" y="-20" width="32" height="15" rx="7.5"/><text class="mono ck-pill-t" x="{tx:.1}" y="-9.5" text-anchor="middle" font-size="9">now</text>"#,
            px = (nx - 16.0).clamp(pad_l, pad_l + plot_w - 32.0),
            tx = nx.clamp(pad_l + 16.0, pad_l + plot_w - 16.0),
        );
    }

    // Hit boxes: the whole slot, so an empty hour can be read and the pointer never
    // has to find a 2-unit sliver.
    for (h, &c) in o.counts.iter().enumerate() {
        let x = pad_l + h as f64 * slot;
        let heading = format!("{}\u{2013}{}", clock(h), clock((h + 1) % 24));
        let not_yet = !before_now(h);
        let tip = if not_yet {
            kit::tip(&heading, &[], Some("Not yet"))
        } else {
            let share = if total > 0 {
                format!("{:.0}%", c as f64 * 100.0 / total as f64)
            } else {
                "0%".to_string()
            };
            let count_s = kit::thousands(c);
            let mut rows: Vec<(String, String, Option<String>)> =
                vec![(capitalise(o.unit), count_s, None)];
            if c > 0 {
                rows.push(("Share of the day".to_string(), share, None));
            }
            if let Some(top) = o.top {
                for (name, n) in top[h].iter().take(3) {
                    rows.push((name.clone(), kit::thousands(*n), None));
                }
            }
            let note = if c == 0 {
                Some("None heard this hour".to_string())
            } else {
                None
            };
            let borrowed: Vec<(&str, &str, Option<&str>)> = rows
                .iter()
                .map(|(a, b, k)| (a.as_str(), b.as_str(), k.as_deref()))
                .collect();
            let solar_note = o.solar.and_then(|(rise, set)| {
                let within = |t: f64| t >= h as f64 && t < (h + 1) as f64;
                if within(rise) {
                    Some(format!("Sunrise {}", fmt_clock(rise)))
                } else if within(set) {
                    Some(format!("Sunset {}", fmt_clock(set)))
                } else {
                    None
                }
            });
            kit::tip(
                &heading,
                &borrowed,
                note.as_deref().or(solar_note.as_deref()),
            )
        };
        let _ = write!(
            svg,
            r#"<rect class="ck-hit" x="{x:.1}" y="0" width="{slot:.1}" height="{plot_h:.1}" data-ck-i="{h}" data-tip="{tip}"/>"#,
        );
    }
    svg.push_str("</svg></div>");
    let _ = write!(
        svg,
        r#"<p class="sr-only viz-readout">{}</p>"#,
        escape_html(&format!(
            "{} {} in all. Busiest hour {} with {}.",
            kit::thousands(total),
            o.unit,
            clock(peak),
            kit::thousands(o.counts[peak])
        )),
    );

    if o.table {
        svg.push_str(
            r#"<details class="viz-table"><summary>Show as a table</summary><div class="viz-table-scroll"><table class="pt-tbl"><caption class="sr-only">Count for each hour of the day</caption><thead><tr><th scope="col">Hour</th><th scope="col" class="num">Count</th><th scope="col" class="num">Share</th></tr></thead><tbody>"#,
        );
        for (h, &c) in o.counts.iter().enumerate() {
            let _ = write!(
                svg,
                r#"<tr><th scope="row">{}</th><td class="num">{}</td><td class="num">{:.0}%</td></tr>"#,
                crate::display::clock(&clock(h)),
                kit::thousands(c),
                if total > 0 {
                    c as f64 * 100.0 / total as f64
                } else {
                    0.0
                },
            );
        }
        svg.push_str("</tbody></table></div></details>");
    }
    svg.push_str("</div>");
    svg
}

/// Everything a day-by-day column chart needs.
pub struct Daily<'a> {
    /// `data-ck-id`, also the prefix of the chart's gradient ids, so it must be unique on a page.
    pub id: &'a str,
    /// The chart's accessible name.
    pub name: &'a str,
    /// One honest sentence for a screen reader.
    pub desc: &'a str,
    /// What is counted, plural.
    pub unit: &'a str,
    /// `(ISO date, count)`, oldest first.
    pub days: &'a [(String, i64)],
    /// The categorical slot every bar takes (0-based).
    pub slot: usize,
    /// Drawing width in viewBox units.
    pub width: f64,
    /// Height of the plot area.
    pub plot_h: f64,
    /// Whether the station was listening each day (`None`: the ledger has nothing, claim nothing).
    pub listened: Option<&'a [bool]>,
    /// A hairline at the mean of the days listened, labelled.
    pub mean_line: bool,
    /// Days from this index on have not happened yet (the current week): an empty slot that
    /// says "Not yet", which is not a zero and not an unrecorded day.
    pub future_from: Option<usize>,
    /// Label the axis with the weekday (`Mon`) rather than the date (`9 Mar`).
    pub weekday_labels: bool,
    /// A "Show as a table" disclosure.
    pub table: bool,
    /// Further classes for the chart's outer `div`, each with a leading space (` viz-wide`), or
    /// empty. A chart drawn twice, once for a wide card and once for a phone, says which is which.
    pub class: &'a str,
}

/// A day-by-day column chart: one bar per day on an axis from zero, the busiest day
/// labelled, an optional mean line, and a tooltip per day. A day the station was not
/// listening is hatched and says so rather than standing as an empty bar.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn daily(o: &Daily<'_>) -> String {
    let n = o.days.len();
    if n == 0 {
        return EMPTY.to_string();
    }
    let future = |i: usize| o.future_from.is_some_and(|f| i >= f);
    let off = |i: usize| !future(i) && o.listened.is_some_and(|l| l.get(i).is_some_and(|on| !on));
    let max = o
        .days
        .iter()
        .enumerate()
        .filter(|(i, _)| !future(*i))
        .map(|(_, d)| d.1)
        .max()
        .unwrap_or(0)
        .max(0);
    let top = kit::count_ceiling(max as f64);
    let rules = kit::count_ticks(top, 4);
    let widest = rules
        .iter()
        .map(|&v| kit::text_width(&kit::compact(v), 10.0))
        .fold(0.0_f64, f64::max);
    let (w, plot_h) = (o.width, o.plot_h);
    let (pad_l, pad_r) = (kit::axis_margin(widest, w), 8.0);
    let plot_w = w - pad_l - pad_r;
    let slot = plot_w / n as f64;
    let bar_w = (slot * 0.66).min(24.0);
    let y = Linear::new((0.0, top), (plot_h, 0.0));
    let head = 14.0;
    let h_total = head + plot_h + 22.0;
    let counted: Vec<i64> = o
        .days
        .iter()
        .enumerate()
        .filter(|(i, _)| !off(*i) && !future(*i))
        .map(|(_, d)| d.1)
        .collect();
    let mean = if counted.is_empty() {
        0.0
    } else {
        counted.iter().sum::<i64>() as f64 / counted.len() as f64
    };
    let peak = o
        .days
        .iter()
        .enumerate()
        .filter(|(i, _)| !off(*i) && !future(*i))
        .max_by_key(|&(i, d)| (d.1, std::cmp::Reverse(i)))
        .map_or(0, |(i, _)| i);
    let cls = kit::slot_class(o.slot);
    let g = kit::id_safe(&format!("g-{}", o.id));
    let label_of = |iso: &str| -> (String, String) {
        kit::civil_label(iso, 0).unwrap_or_else(|| {
            let t = iso.get(5..).unwrap_or(iso).to_string();
            (t.clone(), t)
        })
    };

    let mut svg = String::with_capacity(6 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck{class}"><div class="viz-scroll"><svg width="100%" viewBox="0 -{head:.0} {w:.0} {h_total:.0}" role="img" class="viz-svg-block ck-in" data-ck="marks" data-ck-id="{id}" data-ck-dim="index" data-ck-start="{last}" data-ck-label="Left and right move from day to day.">"#,
        class = escape_html(o.class),
        id = escape_html(o.id),
        last = n - 1,
    );
    svg.push_str(&svg_a11y(o.name, o.desc));
    svg.push_str(&kit::gradient_defs(&g, std::slice::from_ref(&cls)));
    if (0..n).any(off) {
        let _ = write!(svg, "<defs>{}</defs>", kit::nodata_pattern(&g));
    }
    for &v in &rules {
        let yy = y.at(v);
        if v > 0.0 {
            let _ = write!(
                svg,
                r#"<line class="ck-grid" x1="{pad_l:.1}" y1="{yy:.1}" x2="{x2:.1}" y2="{yy:.1}"/>"#,
                x2 = pad_l + plot_w
            );
        }
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{tx:.1}" y="{ty:.1}" text-anchor="end" font-size="10">{l}</text>"#,
            tx = pad_l - 6.0,
            ty = yy + 3.5,
            l = kit::compact(v),
        );
    }
    for i in (0..n).filter(|&i| off(i)) {
        let _ = write!(
            svg,
            r#"<rect x="{x:.1}" y="0" width="{slot:.1}" height="{plot_h:.1}" fill="url(#{g}-nd)"/>"#,
            x = pad_l + i as f64 * slot,
        );
    }
    svg.push_str("<g>");
    for (i, (_, c)) in o.days.iter().enumerate() {
        if *c <= 0 || off(i) || future(i) {
            continue;
        }
        let x = pad_l + i as f64 * slot + (slot - bar_w) / 2.0;
        let d = kit::bar_path(x, y.at(*c as f64), bar_w, plot_h - y.at(*c as f64), 4.0);
        let _ = write!(
            svg,
            r#"<path class="ck-grow" d="{d}" fill="url(#{g}-{cls})" data-ck-i="{i}" data-style="--i:{i}"/>"#
        );
    }
    svg.push_str("</g>");
    let _ = write!(
        svg,
        r#"<line class="ck-axis" x1="{pad_l:.1}" y1="{plot_h:.1}" x2="{x2:.1}" y2="{plot_h:.1}"/>"#,
        x2 = pad_l + plot_w
    );
    if o.mean_line && !counted.is_empty() && mean > 0.0 {
        let my = y.at(mean);
        let x2 = pad_l + plot_w;
        let text = format!("avg {}", kit::compact((mean * 10.0).round() / 10.0));
        let lw = kit::text_width(&text, 10.0) + 4.0;
        // The label sits on the line at whichever end no bar rises through it; when both ends are
        // taken by tall bars it goes in the row above the plot, clear of the busiest day's number.
        let bar_in = |from: f64, to: f64| {
            (0..n).any(|i| {
                let l = pad_l + i as f64 * slot + (slot - bar_w) / 2.0;
                !off(i)
                    && !future(i)
                    && l + bar_w > from
                    && l < to
                    && y.at(o.days[i].1 as f64) < my - 1.0
            })
        };
        let peak_x = pad_l + peak as f64 * slot + slot / 2.0;
        let head_free = |from: f64, to: f64| {
            max == 0 || off(peak) || future(peak) || peak_x + 14.0 < from || peak_x - 14.0 > to
        };
        let (tx, anchor, ty) = if !bar_in(x2 - lw, x2) {
            (x2, "end", my - 4.0)
        } else if !bar_in(pad_l, pad_l + lw) {
            (pad_l + 2.0, "start", my - 4.0)
        } else if head_free(x2 - lw, x2) {
            (x2, "end", -3.0)
        } else {
            (pad_l + 2.0, "start", -3.0)
        };
        let _ = write!(
            svg,
            r#"<line class="ck-axis" x1="{pad_l:.1}" y1="{my:.1}" x2="{x2:.1}" y2="{my:.1}" opacity=".75"/><text class="mono ck-annot ck-halo" x="{tx:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10">{text}</text>"#,
        );
    }
    if max > 0 && !off(peak) && !future(peak) {
        let px = pad_l + peak as f64 * slot + slot / 2.0;
        // The mean's own label sits at the right edge; the peak steps aside if it would touch it.
        let near_avg_label = o.mean_line
            && px > pad_l + plot_w - 46.0
            && (y.at(max as f64) - y.at(mean)).abs() < 16.0;
        if !near_avg_label {
            let _ = write!(
                svg,
                r#"<text class="mono ck-annot ck-halo" x="{px:.1}" y="{ty:.1}" text-anchor="middle" font-size="10">{v}</text>"#,
                ty = y.at(max as f64) - 5.0,
                v = kit::compact(max as f64),
            );
        }
    }
    // At most seven labels, and fewer where the drawing is narrow: a label ("11 Oct") is about
    // 36 units wide and wants room either side, so a 360-unit drawing takes five. Six or seven in it
    // touched, and the text-fitting script then hid the last, the one that says "today".
    let max_labels = ((plot_w / 66.0).floor() as usize).clamp(2, 7);
    for i in super::timeline::axis_columns(n, max_labels) {
        let (mut short, long) = label_of(&o.days[i].0);
        if o.weekday_labels {
            short = long.chars().take(3).collect();
        }
        let x = pad_l + i as f64 * slot + slot / 2.0;
        // The end labels hang inward only where a slot is too narrow for a label to be centred
        // in it; with room (a week's seven days) every label sits under its own bar.
        let roomy = slot >= 34.0;
        let anchor = if roomy || n < 2 {
            "middle"
        } else if i == 0 {
            "start"
        } else if i + 1 == n {
            "end"
        } else {
            "middle"
        };
        let x = match anchor {
            "start" => x - slot / 2.0 + 2.0,
            "end" => x + slot / 2.0 - 2.0,
            _ => x,
        };
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10">{l}</text>"#,
            ty = plot_h + 15.0,
            l = escape_html(&short),
        );
    }
    for (i, (iso, c)) in o.days.iter().enumerate() {
        let (_, long) = label_of(iso);
        let tip = if future(i) {
            kit::tip(&long, &[], Some("Not yet"))
        } else if off(i) {
            kit::tip(&long, &[], Some("The station was not listening"))
        } else {
            let count_s = kit::thousands(*c);
            let avg_s = format!("{mean:.1}");
            let mut rows: Vec<(String, String)> = vec![(capitalise(o.unit), count_s)];
            if o.mean_line && mean > 0.0 {
                rows.push(("Average".to_string(), avg_s));
            }
            let borrowed: Vec<(&str, &str, Option<&str>)> = rows
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str(), None))
                .collect();
            kit::tip(
                &long,
                &borrowed,
                if *c == 0 { Some("None heard") } else { None },
            )
        };
        let _ = write!(
            svg,
            r#"<rect class="ck-hit" x="{x:.1}" y="0" width="{slot:.1}" height="{plot_h:.1}" data-ck-i="{i}" data-tip="{tip}"/>"#,
            x = pad_l + i as f64 * slot,
        );
    }
    svg.push_str("</svg></div>");
    let total: i64 = counted.iter().sum();
    let _ = write!(
        svg,
        r#"<p class="sr-only viz-readout">{}</p>"#,
        escape_html(&format!(
            "{} {} over {} days. Busiest day {}.",
            kit::thousands(total),
            o.unit,
            n,
            label_of(&o.days[peak].0).1
        )),
    );
    if o.table {
        svg.push_str(r#"<details class="viz-table"><summary>Show as a table</summary><div class="viz-table-scroll"><table class="pt-tbl"><caption class="sr-only">Count for each day</caption><thead><tr><th scope="col">Day</th><th scope="col" class="num">Count</th></tr></thead><tbody>"#);
        for (i, (iso, c)) in o.days.iter().enumerate() {
            let _ = write!(
                svg,
                r#"<tr><th scope="row">{}</th><td class="num">{}</td></tr>"#,
                escape_html(&label_of(iso).1),
                if future(i) {
                    "not yet".to_string()
                } else if off(i) {
                    "not listening".to_string()
                } else {
                    kit::thousands(*c)
                },
            );
        }
        svg.push_str("</tbody></table></div></details>");
    }
    svg.push_str("</div>");
    svg
}

/// One row of a distribution chart.
pub struct Row<'a> {
    /// What the row is (`80-90%`).
    pub label: &'a str,
    /// How many.
    pub count: i64,
    /// A CSS colour reference (`var(--conf-high)`), never a literal.
    pub fill: String,
}

/// A horizontal distribution chart: a labelled bar per row from a zero baseline, its count at
/// its tip, its share of the whole in its tooltip.
#[must_use]
pub fn distribution(id: &str, name: &str, desc: &str, unit: &str, rows: &[Row<'_>]) -> String {
    let total: i64 = rows.iter().map(|r| r.count).sum();
    if rows.is_empty() || total <= 0 {
        return EMPTY.to_string();
    }
    let max = rows.iter().map(|r| r.count).max().unwrap_or(1).max(1);
    let label_w = rows
        .iter()
        .map(|r| kit::text_width(r.label, 10.0))
        .fold(0.0_f64, f64::max)
        + 10.0;
    let value_w = kit::text_width(&kit::compact(max as f64), 10.0) + 8.0;
    let w = 360.0_f64;
    let (bar_h, gap) = (18.0_f64, 7.0_f64);
    let plot_w = w - label_w - value_w;
    let h = rows.len() as f64 * (bar_h + gap) - gap;
    let mut svg = String::with_capacity(3 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="viz-svg-block ck-in" data-ck="marks" data-ck-id="{id}" data-ck-dim="index" data-ck-label="Up and down move from row to row." data-ck-grid="1">"#,
        id = escape_html(id),
    );
    svg.push_str(&svg_a11y(name, desc));
    let _ = write!(
        svg,
        r#"<line class="ck-axis" x1="{label_w:.1}" y1="0" x2="{label_w:.1}" y2="{h:.1}"/>"#
    );
    for (i, r) in rows.iter().enumerate() {
        let yy = i as f64 * (bar_h + gap);
        let bw = r.count as f64 / max as f64 * plot_w;
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{lx:.1}" y="{ty:.1}" text-anchor="end" font-size="10">{l}</text>"#,
            lx = label_w - 8.0,
            ty = yy + bar_h / 2.0 + 3.5,
            l = escape_html(r.label),
        );
        if r.count > 0 {
            let _ = write!(
                svg,
                r#"<path d="{d}" fill="{fill}" data-ck-i="{i}"/><text class="mono ck-annot" x="{vx:.1}" y="{ty:.1}" font-size="10">{v}</text>"#,
                d = kit::hbar_path(label_w, yy, bw, bar_h, 4.0),
                fill = escape_html(&r.fill),
                vx = label_w + bw + 6.0,
                ty = yy + bar_h / 2.0 + 3.5,
                v = kit::compact(r.count as f64),
            );
        }
        let share = format!("{:.0}%", r.count as f64 * 100.0 / total as f64);
        let count_s = kit::thousands(r.count);
        let _ = write!(
            svg,
            r#"<rect class="ck-hit" x="0" y="{y0:.1}" width="{w:.0}" height="{bh:.1}" data-ck-i="{i}" data-tip="{tip}"/>"#,
            y0 = yy - gap / 2.0,
            bh = bar_h + gap,
            tip = kit::tip(
                r.label,
                &[(&capitalise(unit), &count_s, None), ("Share", &share, None)],
                None
            ),
        );
    }
    svg.push_str("</svg></div>");
    svg
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> [i64; 24] {
        let mut c = [0_i64; 24];
        for (h, v) in [(5, 30), (6, 122), (7, 90), (12, 40), (18, 12), (21, 3)] {
            c[h] = v;
        }
        c
    }

    fn opts(counts: [i64; 24]) -> Hourly<'static> {
        Hourly {
            id: "t",
            name: "Detections by hour",
            desc: "Detections for each hour of the day.",
            unit: "detections",
            counts,
            fill: Fill::DawnDusk,
            width: 720.0,
            plot_h: 150.0,
            solar: None,
            now_h: None,
            top: None,
            temps: &[],
            table: true,
        }
    }

    fn daily_of(counts: &[i64]) -> String {
        let days: Vec<(String, i64)> = counts
            .iter()
            .enumerate()
            .map(|(i, &c)| (format!("2026-03-{:02}", i + 1), c))
            .collect();
        daily(&Daily {
            id: "d",
            name: "Detections per day",
            desc: "d",
            unit: "detections",
            days: &days,
            slot: 0,
            width: 600.0,
            plot_h: 150.0,
            listened: None,
            mean_line: true,
            future_from: None,
            weekday_labels: false,
            table: false,
            class: "",
        })
    }

    /// The mean's label is read, so no bar may rise through it: it moves to the end of the line
    /// that is clear, and above the plot when a tall bar stands at both ends.
    #[test]
    fn the_mean_label_is_not_written_across_a_bar() {
        let attrs = |svg: &str| {
            let at = svg.find(">avg ").expect("an avg label");
            let tag = &svg[svg[..at].rfind("<text").unwrap()..at];
            let get = |k: &str| {
                tag.split(&format!("{k}=\""))
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap()
                    .to_string()
            };
            (get("text-anchor"), get("y").parse::<f64>().unwrap())
        };
        // A tall bar at the right end only: the label goes to the left end, on the line.
        let (anchor, y) = attrs(&daily_of(&[1, 1, 1, 1, 1, 1, 20]));
        assert_eq!(
            anchor, "start",
            "a bar at the right end takes the label to the left"
        );
        assert!(y > 0.0, "still on the line, inside the plot");
        // Nothing tall at either end: it stays at the right, on the line.
        let (anchor, y) = attrs(&daily_of(&[1, 1, 1, 20, 1, 1, 1]));
        assert_eq!(anchor, "end");
        assert!(y > 0.0);
        // Tall at both ends: above the plot.
        let (_, y) = attrs(&daily_of(&[20, 1, 1, 1, 1, 1, 20]));
        assert!(
            y < 0.0,
            "both ends taken, so it is written above the plot, got y={y}"
        );
    }

    #[test]
    fn nothing_counted_is_not_a_chart() {
        assert!(hourly(&opts([0; 24])).contains("Not enough data"));
    }

    /// One tooltip box per hour, empty hours included, so a quiet hour can be read
    /// as quiet; and one painted bar per hour that has something.
    #[test]
    fn every_hour_has_a_reading_and_only_counted_hours_have_a_bar() {
        let svg = hourly(&opts(sample()));
        assert_eq!(svg.matches(r#"class="ck-hit""#).count(), 24, "{svg}");
        assert_eq!(svg.matches(r#"class="ck-grow""#).count(), 6, "{svg}");
        assert!(
            svg.contains("04:00\u{2013}05:00&#10;Detections&#9;0&#10;None heard this hour"),
            "{svg}"
        );
        assert!(
            svg.contains("06:00\u{2013}07:00&#10;Detections&#9;122&#10;Share of the day&#9;"),
            "{svg}"
        );
        assert!(
            svg.contains(r#"data-ck-start="6""#),
            "the keyboard lands on the peak: {svg}"
        );
    }

    /// Only the peak is written on the chart; the axis is from zero and rounded up.
    #[test]
    fn only_the_peak_is_labelled_and_the_axis_starts_at_zero() {
        let svg = hourly(&opts(sample()));
        // 122 tops out at 150? count_ceiling gives 150? no: 1,2,3,4,5,6,8 x 10^k -> 200.
        assert!(svg.contains(">200<") || svg.contains(">150<"), "{svg}");
        assert!(svg.contains(">0<"), "{svg}");
        let labels = svg.matches(r#"class="mono ck-annot ck-halo""#).count();
        assert_eq!(labels, 1, "one peak label: {svg}");
        assert!(svg.contains(">122<"), "{svg}");
    }

    /// Dawn and dusk hours get the brand hue, the rest the neutral (emphasis, not a
    /// rainbow); a count chart through the ramp gets one step per bar.
    #[test]
    fn colour_means_something() {
        let svg = hourly(&opts(sample()));
        assert!(
            svg.contains("g-t-ck-c1") && svg.contains("g-t-ck-cother"),
            "{svg}"
        );
        assert_eq!(
            svg.matches(r#"fill="url(#g-t-ck-c1)""#).count(),
            5,
            "05,06,07,18,21: {svg}"
        );
        assert_eq!(
            svg.matches(r#"fill="url(#g-t-ck-cother)""#).count(),
            1,
            "12 only: {svg}"
        );
        let mut o = opts(sample());
        o.fill = Fill::Seq;
        let s = hourly(&o);
        assert!(
            s.contains("g-t-ck-q7"),
            "the peak hour is the top step: {s}"
        );
        assert!(s.contains("g-t-ck-q"), "{s}");
        assert!(!s.contains("fill-opacity") && !s.contains("oklch("), "{s}");
    }

    /// A chart of today stops at now: later hours have no bar and say "Not yet".
    #[test]
    fn hours_after_now_are_not_yet_and_not_zero() {
        let mut o = opts(sample());
        o.now_h = Some(6.5);
        let svg = hourly(&o);
        assert_eq!(
            svg.matches(r#"class="ck-grow""#).count(),
            2,
            "hours 05 and 06 only: {svg}"
        );
        assert!(svg.contains("12:00\u{2013}13:00&#10;Not yet"), "{svg}");
        assert!(svg.contains(">now<"), "{svg}");
    }

    /// The sun is drawn only when the station has a location, and the night is the
    /// hours outside it; two labels too close to read are one label.
    #[test]
    fn night_is_shaded_only_when_the_sun_is_known() {
        assert!(!hourly(&opts(sample())).contains("sunrise"));
        let mut o = opts(sample());
        o.solar = Some((6.7, 18.0));
        let svg = hourly(&o);
        assert!(
            svg.contains("sunrise 6:42") && svg.contains("sunset 18:00"),
            "{svg}"
        );
        assert_eq!(svg.matches(r#"<rect class="ck-track""#).count(), 2, "{svg}");
        assert!(
            svg.contains("Sunrise 6:42"),
            "the hour it happens says so: {svg}"
        );
        o.solar = Some((12.0, 12.2));
        let close = hourly(&o);
        assert_eq!(
            close.matches("sunrise").count()
                + close.matches("sunset").count()
                + close.matches("Sunrise").count()
                + close.matches("Sunset").count(),
            2,
            "{close}"
        );
    }

    /// Temperature has its own strip and its own labelled extremes; it is never drawn
    /// on the count axis.
    #[test]
    fn temperature_is_its_own_scale() {
        let mut o = opts(sample());
        let temps: Vec<(f64, f64)> = (0..24)
            .map(|h| (f64::from(h) + 0.5, 10.0 + f64::from(h) / 2.0))
            .collect();
        o.temps = &temps;
        let svg = hourly(&o);
        assert!(
            svg.contains(">Temperature<") && svg.contains(">22°<") && svg.contains(">10°<"),
            "{svg}"
        );
    }

    /// A day of 1.6 million detections must not run its own axis labels off the card.
    #[test]
    fn a_huge_count_is_compacted() {
        let mut c = sample();
        c[6] = 1_600_000;
        let svg = hourly(&opts(c));
        assert!(svg.contains(">1.6M<") && svg.contains(">2M<"), "{svg}");
        assert!(
            svg.contains("1,600,000"),
            "the tooltip carries the exact number: {svg}"
        );
    }

    /// The top species of an hour are rows of its tooltip, and untrusted text stays escaped.
    #[test]
    fn the_busiest_species_of_an_hour_are_in_its_tooltip() {
        let mut top: [Vec<(String, i64)>; 24] = std::array::from_fn(|_| Vec::new());
        top[6] = vec![("American Robin".into(), 60), ("<b>Evil</b>".into(), 30)];
        let mut o = opts(sample());
        o.top = Some(&top);
        let svg = hourly(&o);
        assert!(
            svg.contains("American Robin&#9;60&#10;&lt;b&gt;Evil&lt;/b&gt;&#9;30"),
            "{svg}"
        );
        assert!(!svg.contains("<b>Evil"), "{svg}");
    }
}
