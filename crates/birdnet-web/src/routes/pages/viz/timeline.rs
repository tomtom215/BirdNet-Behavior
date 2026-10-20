//! Cartesian time-series charts: streamgraph, accumulation curve, migration
//! ridgeline, and the 24-hour day strip.
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::{EMPTY, svg_a11y};
use crate::routes::pages::atoms::series_color;
use crate::routes::pages::escape_html;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The calendar day `days` after the ISO date `end`, as `("14 Oct", "Tue 14 Oct")`.
///
/// Uses the workspace's own civil-date arithmetic rather than a date crate —
/// this workspace carries none. `None` if `end` does not parse, so a malformed
/// date degrades to a chart without dates rather than a panic in a render path
/// (`panic = "abort"`, so a handler panic is a station outage).
fn civil_label(end: &str, days: i64) -> Option<(String, String)> {
    let t = birdnet_core::civil::parse_civil(end, "00:00:00")?;
    let secs = birdnet_core::civil::unix_secs_from_civil(&t) + days * 86_400;
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    let month = MONTHS.get((c.month as usize).wrapping_sub(1))?;
    // 1970-01-01 was a Thursday.
    let weekday = WEEKDAYS[(secs.div_euclid(86_400) + 4).rem_euclid(7) as usize];
    let short = format!("{} {month}", c.day);
    let long = format!("{weekday} {short}");
    Some((short, long))
}

/// The largest 1, 2 or 5 times a power of ten that does not exceed `x` (never
/// less than 1): a count a reader can hold in their head and measure against.
fn nice_count(x: f64) -> i64 {
    if x < 1.0 {
        return 1;
    }
    let mut mag = 10_f64.powf(x.log10().floor());
    let mut f = x / mag;
    // `log10` of an exact power of ten can land a hair under it.
    if f >= 10.0 {
        mag *= 10.0;
        f /= 10.0;
    }
    let m = if f >= 5.0 {
        5.0
    } else if f >= 2.0 {
        2.0
    } else {
        1.0
    };
    (m * mag) as i64
}

/// Centred stacked-area ("themeriver") of per-day counts. Each series is
/// `(common_name, daily_counts)`; all vectors should share a length.
///
/// `end_date` is the calendar date of the **last** column, used to label the
/// time axis and the readout. `None` omits the dates but still draws the scale.
///
/// # What a reader can take from it
///
/// A centred stack has no baseline to read a value against: the thickness of a
/// band on one day *is* the number, and nothing printed it. It had a hairline
/// and a tiny "26/day peak" and three date ticks, and a reader asking "was that
/// day five or fifty" could not answer. Now:
///
/// * a **scale bar** in the left margin, as tall as `nice` detections in a day
///   (a 1, 2 or 5 times a power of ten, about two fifths of the busiest day), so
///   a band's thickness can be held against something;
/// * the **busiest day** marked on the chart with its count;
/// * a **readout** under the chart: move a pointer or a finger across it and the
///   column under it is printed, day, total and the species that made it
///   (`static/chart-text.js` reads `data-names` / `data-cols`);
/// * a **table** behind a disclosure with the same facts per species, which is
///   also what a keyboard or a screen reader uses: the chart itself is one
///   image, with each band's name and total in its `<title>`.
///
/// Colours are `--chart-1` … `--chart-8` by rank, with a surface-coloured seam
/// between bands and no transparency, so what is painted is what was graded.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn streamgraph(series: &[(String, Vec<i64>)], end_date: Option<&str>) -> String {
    let days = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    if series.is_empty() || days < 2 {
        return EMPTY.to_string();
    }

    let w = 760.0_f64;
    // Left margin for the scale bar; a few units on the right so the last
    // column's seam is not clipped.
    let pad_l = 46.0_f64;
    let pad_r = 4.0_f64;
    let plot_w = w - pad_l - pad_r;
    let plot_h = 220.0_f64;
    // Room under the plot for the date axis.
    let h = plot_h + 22.0;
    let mid = plot_h / 2.0;
    let step = plot_w / (days - 1) as f64;
    let x_at = |d: usize| pad_l + d as f64 * step;

    let count = |s: &(String, Vec<i64>), d: usize| s.1.get(d).copied().unwrap_or(0);
    let totals: Vec<i64> = (0..days)
        .map(|d| series.iter().map(|s| count(s, d)).sum())
        .collect();
    let (peak_day, max_total) = totals
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|&(d, t)| (t, std::cmp::Reverse(d)))
        .unwrap_or((0, 0));
    let max_f = max_total.max(1) as f64;
    // The fattest day fills ~85% of the height.
    let scale = (plot_h * 0.85) / max_f;
    let val_y = |v: f64| mid - v * scale;
    let nice = nice_count(max_f * 0.4);

    let mut svg = String::with_capacity(8 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart"><div class="viz-scroll" tabindex="0" role="group" aria-label="Activity streamgraph, scrolls sideways on a narrow screen"><svg width="100%" viewBox="0 -14 {w:.0} {vh:.0}" preserveAspectRatio="none" role="img" class="viz-svg-block viz-stream" data-readout data-x0="{pad_l:.0}" data-x1="{x1:.0}" data-names="{names}" data-cols="{cols}">"#,
        x1 = pad_l + plot_w,
        // 14 units of headroom above the plot for the busiest day's count.
        vh = h + 14.0,
        names = escape_html(
            &series
                .iter()
                .map(|(n, _)| n.replace(['|', ';'], "/"))
                .collect::<Vec<_>>()
                .join("|")
        ),
        cols = {
            let mut s = String::new();
            for d in 0..days {
                let label = end_date
                    .and_then(|e| civil_label(e, d as i64 - (days as i64 - 1)))
                    .map_or_else(|| format!("Day {}", d + 1), |(_, long)| long);
                if d > 0 {
                    s.push(';');
                }
                let _ = write!(s, "{label}:");
                for (k, ser) in series.iter().enumerate() {
                    if k > 0 {
                        s.push(',');
                    }
                    let _ = write!(s, "{}", count(ser, d));
                }
            }
            s
        },
    );
    svg.push_str(&svg_a11y(
        "Activity streamgraph",
        "Detections per day over time as a centred stack; each coloured band is one species and its thickness is that day's count.",
    ));

    // Stack bands from a centred baseline, bottom of the legend order first.
    //
    // `series_color(rank)`, not `species_color(name)`: several species are shown
    // together here, and a hash put pairs of them 2–3° apart in hue.
    for (rank, (name, counts)) in series.iter().enumerate() {
        let color = series_color(rank, series.len());
        let mut top = String::new();
        let mut bottom: Vec<(f64, f64)> = Vec::with_capacity(days);
        for d in 0..days {
            let below: f64 = series.iter().take(rank).map(|s| count(s, d)).sum::<i64>() as f64;
            let val = counts.get(d).copied().unwrap_or(0) as f64;
            let lower = below - totals.get(d).copied().unwrap_or(0) as f64 / 2.0;
            let upper = lower + val;
            let x = x_at(d);
            let _ = write!(
                top,
                "{}{x:.1},{y:.1} ",
                if d == 0 { "M" } else { "L" },
                y = val_y(upper)
            );
            bottom.push((x, val_y(lower)));
        }
        let mut path = top;
        for (x, y) in bottom.iter().rev() {
            let _ = write!(path, "L{x:.1},{y:.1} ");
        }
        path.push('Z');
        let total: i64 = counts.iter().sum();
        let _ = write!(
            svg,
            r#"<path d="{path}" fill="{color}" stroke="var(--card-top)" stroke-width="1.2" stroke-linejoin="round" vector-effect="non-scaling-stroke"><title>{n} — {total} in this period</title></path>"#,
            n = escape_html(name),
        );
    }

    // The scale bar: `nice` detections in one day, to hold a band against.
    let bar_half = nice as f64 * scale / 2.0;
    let bx = pad_l - 12.0;
    let _ = write!(
        svg,
        r#"<g stroke="var(--fg-3)" stroke-width="1.5" fill="none" stroke-linecap="butt" vector-effect="non-scaling-stroke"><line x1="{bx:.1}" y1="{a:.1}" x2="{bx:.1}" y2="{b:.1}"/><line x1="{bx0:.1}" y1="{a:.1}" x2="{bx1:.1}" y2="{a:.1}"/><line x1="{bx0:.1}" y1="{b:.1}" x2="{bx1:.1}" y2="{b:.1}"/></g><text class="mono" x="{bx:.1}" y="{ty:.1}" text-anchor="middle" font-size="10" fill="var(--fg-3)">{nice}</text>"#,
        a = mid - bar_half,
        b = mid + bar_half,
        bx0 = bx - 4.0,
        bx1 = bx + 4.0,
        ty = mid - bar_half - 5.0,
    );

    // The busiest day, marked where it happened.
    let px = x_at(peak_day);
    let py = val_y(max_f / 2.0);
    let anchor = if peak_day * 5 < days {
        "start"
    } else if peak_day * 5 > days * 4 {
        "end"
    } else {
        "middle"
    };
    let _ = write!(
        svg,
        r#"<line x1="{px:.1}" y1="{py:.1}" x2="{px:.1}" y2="{ly:.1}" stroke="var(--fg-3)" stroke-width="1" vector-effect="non-scaling-stroke"/><text class="mono" x="{px:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10" fill="var(--fg-2)">{max_total}</text>"#,
        ly = py - 5.0,
        ty = py - 8.0,
    );

    // Time axis: first, middle and last column, so the period is unambiguous.
    for (frac, anchor) in [(0.0_f64, "start"), (0.5, "middle"), (1.0, "end")] {
        let back = ((1.0 - frac) * (days - 1) as f64).round() as i64;
        let label = end_date
            .map(|end| {
                civil_label(end, -back).map_or_else(|| end.chars().skip(5).collect(), |(s, _)| s)
            })
            .unwrap_or_default();
        if label.is_empty() {
            continue;
        }
        let x = pad_l + frac * plot_w;
        let _ = write!(
            svg,
            r#"<text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10" fill="var(--fg-3)">{label}</text>"#,
            ty = h - 4.0,
        );
    }

    // The hairline the readout moves; hidden until it is asked for.
    let _ = write!(
        svg,
        r#"<line class="sg-cursor" x1="{pad_l:.0}" y1="0" x2="{pad_l:.0}" y2="{plot_h:.0}" stroke="var(--fg)" stroke-width="1" vector-effect="non-scaling-stroke" visibility="hidden"/>"#,
    );
    svg.push_str("</svg></div>");

    // The readout line, idle until a pointer is on the chart.
    let _ = write!(
        svg,
        r#"<p class="bnb-meta viz-readout">The bar at the left is {nice} detection{s} in a day. The busiest day had {max_total}. Move across the chart to read a day.</p>"#,
        s = if nice == 1 { "" } else { "s" },
    );

    // Legend.
    svg.push_str(r#"<div class="viz-legend">"#);
    // Same rank-based colour the bands use. A legend keyed differently from
    // the chart it explains is worse than no legend.
    for (rank, (name, _)) in series.iter().enumerate() {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta viz-legend-item"><span class="viz-swatch" data-style="background:{c}"></span>{n}</span>"#,
            c = series_color(rank, series.len()),
            n = escape_html(name),
        );
    }
    svg.push_str("</div>");

    // The same facts as a table: what a keyboard or a screen reader uses, and
    // what to copy a number from.
    svg.push_str(
        r#"<details class="viz-table"><summary>Show as a table</summary><div class="viz-table-scroll"><table class="pt-tbl"><caption class="sr-only">Detections per species over the period shown</caption><thead><tr><th scope="col">Species</th><th scope="col" class="num">Detections</th><th scope="col">Busiest day</th><th scope="col" class="num">That day</th></tr></thead><tbody>"#,
    );
    for (rank, (name, counts)) in series.iter().enumerate() {
        let total: i64 = counts.iter().sum();
        let (best_day, best) = counts
            .iter()
            .copied()
            .enumerate()
            .max_by_key(|&(d, v)| (v, std::cmp::Reverse(d)))
            .unwrap_or((0, 0));
        let when = if best == 0 {
            "\u{2014}".to_string()
        } else {
            end_date
                .and_then(|e| civil_label(e, best_day as i64 - (days as i64 - 1)))
                .map_or_else(|| format!("Day {}", best_day + 1), |(_, long)| long)
        };
        let _ = write!(
            svg,
            r#"<tr><th scope="row"><span class="viz-swatch" data-style="background:{c}"></span> {n}</th><td class="num">{total}</td><td>{when}</td><td class="num">{best}</td></tr>"#,
            c = series_color(rank, series.len()),
            n = escape_html(name),
        );
    }
    svg.push_str("</tbody></table></div></details></div>");
    svg
}

/// Monotonic life-list accumulation: `points` is `(label, cumulative_total)`
/// in chronological order. Renders a filled step-up area with end-value label.
#[must_use]
pub fn accumulation_curve(points: &[(String, i64)]) -> String {
    if points.len() < 2 {
        return EMPTY.to_string();
    }
    let w = 720.0_f64;
    let h = 200.0_f64;
    let pad_l = 6.0;
    let pad_b = 22.0;
    // Headroom for the total, written above the curve's end (see the test).
    let pad_t = 26.0;
    let plot_h = h - pad_b - pad_t;
    let n = points.len();
    let max = points.last().map_or(1, |(_, v)| *v).max(1) as f64;
    let step = (w - pad_l) / (n - 1) as f64;
    let x_at = |i: usize| pad_l + i as f64 * step;
    let y_at = |v: i64| pad_t + plot_h - (v as f64 / max) * plot_h;

    let mut line = String::new();
    for (i, (_, v)) in points.iter().enumerate() {
        let _ = write!(
            line,
            "{}{x:.1},{y:.1} ",
            if i == 0 { "M" } else { "L" },
            x = x_at(i),
            y = y_at(*v)
        );
    }
    let area = format!(
        "{line}L{x:.1},{base:.1} L{x0:.1},{base:.1} Z",
        x = x_at(n - 1),
        x0 = x_at(0),
        base = pad_t + plot_h,
    );

    let mut svg = format!(
        r#"<svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" class="viz-svg-block">"#
    );
    svg.push_str(&svg_a11y(
        "Species accumulation over time",
        "A rising line showing how many distinct species have been recorded as time goes on.",
    ));
    let _ = write!(
        svg,
        r#"<line x1="{pad_l:.1}" y1="{base:.1}" x2="{w:.1}" y2="{base:.1}" stroke="var(--hairline)" stroke-width="0.5"/>"#,
        base = pad_t + plot_h,
    );
    let _ = write!(
        svg,
        r#"<path d="{area}" fill="var(--chart-1)" fill-opacity="0.12"/><path d="{line}" fill="none" stroke="var(--chart-1)" stroke-width="1.6"/>"#,
    );
    let (lx, ly) = (x_at(n - 1), y_at(points[n - 1].1));
    let _ = write!(
        svg,
        r#"<circle cx="{lx:.1}" cy="{ly:.1}" r="2.6" fill="var(--chart-1)"/><text class="mono" x="{tx:.1}" y="{ty:.1}" text-anchor="end" font-size="11" fill="var(--fg-2)">{total} species</text>"#,
        tx = lx - 4.0,
        ty = ly - 8.0,
        total = points[n - 1].1,
    );
    let stride = (n / 6).max(1);
    for (i, (label, _)) in points.iter().enumerate() {
        if i % stride == 0 || i == n - 1 {
            // The first and last labels are anchored inward: centred on the plot's
            // edge, half of each hung outside the SVG, and the label-fitting script
            // (static/chart-text.js) then cut them to "25-…" and "26-…".
            let anchor = if i == 0 {
                "start"
            } else if i == n - 1 {
                "end"
            } else {
                "middle"
            };
            let _ = write!(
                svg,
                r#"<text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="8" fill="var(--fg-4)">{label}</text>"#,
                x = x_at(i),
                ty = h - 6.0,
            );
        }
    }
    svg.push_str("</svg>");
    svg
}

/// Full-width 24-hour timeline for "today" (v3 spine, `Today_home.html`):
/// night bands behind, an hourly histogram, an in-strip amber temperature
/// line with a "now" reading, labelled sunrise/sunset lines and a "now"
/// pill.
///
/// The histogram (height = count) is the one clear, colour-blind-safe
/// encoding; the pre-spine per-detection hue dots were removed — hue-only
/// species coding fails for colour-blind users and is illegible at 2px.
/// Species identity lives in the Top-species rail with text banding codes.
///
/// `hourly` is the per-hour detection count; `temps` is `(hour 0–24, °C)`
/// samples for the temperature line (empty → no line); `solar` is
/// `(sunrise, sunset)` hours when the station has a location; `now_h` is the
/// current hour-of-day. All hours share one clock axis.
#[must_use]
pub fn day_strip(
    hourly: &[i64; 24],
    temps: &[(f64, f64)],
    solar: Option<(f64, f64)>,
    now_h: f64,
) -> String {
    let max = hourly.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return EMPTY.to_string();
    }
    let w = 960.0_f64;
    let h = 120.0_f64;
    let base = 96.0_f64; // histogram baseline
    let bar_ceiling = 26.0_f64; // headroom above the tallest bar
    let hw = w / 24.0;
    let max_f = max.max(1) as f64;
    let x_of = |hour: f64| hour / 24.0 * w;

    // The accessible description only promises markers that are actually
    // drawn — without a location there are no sunrise/sunset lines.
    let desc = if solar.is_some() {
        "Hourly detection counts across the day, with sunrise, sunset and now markers"
    } else {
        "Hourly detection counts across the day, with a now marker"
    };
    let mut svg = format!(
        r#"<svg viewBox="0 0 {w:.0} {h:.0}" width="100%" height="auto" role="img" class="viz-svg-block">"#
    );
    svg.push_str(&svg_a11y("Activity across the day", desc));

    // Night bands (midnight→sunrise, sunset→midnight) — only with a location.
    if let Some((sunrise, sunset)) = solar {
        let sunrise_x = x_of(sunrise);
        let sunset_x = x_of(sunset);
        let _ = write!(
            svg,
            r#"<rect x="0" y="0" width="{sunrise_x:.1}" height="{base:.1}" fill="var(--night)" fill-opacity="0.09"/><rect x="{sunset_x:.1}" y="0" width="{rw:.1}" height="{base:.1}" fill="var(--night)" fill-opacity="0.09"/>"#,
            rw = w - sunset_x,
        );
    }

    // Hourly histogram bars.
    for (hour, &c) in hourly.iter().enumerate() {
        if c > 0 {
            let bh = (c as f64 / max_f) * (base - bar_ceiling);
            let x = hour as f64 * hw + 3.0;
            let y = base - bh;
            let _ = write!(
                svg,
                r#"<rect x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" rx="2" fill="var(--chart-1)"/>"#,
                bw = hw - 6.0,
            );
        }
    }

    // Baseline.
    let _ = write!(
        svg,
        r#"<line x1="0" y1="{base:.1}" x2="{w:.0}" y2="{base:.1}" stroke="var(--border)" stroke-width="1"/>"#
    );

    // Temperature overlaid on the SAME chart — an amber line (distinct from
    // the moss count bars) so the day's activity and weather read together.
    if temps.len() >= 2 {
        let t_min = temps.iter().map(|(_, t)| *t).fold(f64::INFINITY, f64::min);
        let t_max = temps
            .iter()
            .map(|(_, t)| *t)
            .fold(f64::NEG_INFINITY, f64::max);
        let t_range = (t_max - t_min).max(1.0);
        let ty_top = 12.0_f64;
        let ty_bot = base - 10.0;
        let t_y = |v: f64| ty_top + (1.0 - (v - t_min) / t_range) * (ty_bot - ty_top);
        let mut path = String::new();
        for (i, (hour, temp)) in temps.iter().enumerate() {
            let _ = write!(
                path,
                "{}{x:.1},{y:.1}",
                if i == 0 { "M" } else { "L" },
                x = x_of(*hour),
                y = t_y(*temp)
            );
        }
        let _ = write!(
            svg,
            r#"<path d="{path}" fill="none" stroke="var(--chart-5)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"#
        );
        // The reading nearest "now", dotted and labelled.
        if let Some((hour, temp)) = temps
            .iter()
            .min_by(|a, b| (a.0 - now_h).abs().total_cmp(&(b.0 - now_h).abs()))
        {
            let nx = x_of(*hour);
            let ny = t_y(*temp);
            let _ = write!(
                svg,
                r#"<circle cx="{nx:.1}" cy="{ny:.1}" r="2.6" fill="var(--chart-5)"/><text x="{tx:.1}" y="{ty:.1}" font-size="10" fill="var(--fg-2)" stroke="var(--surface)" stroke-width="3.5" paint-order="stroke" stroke-linejoin="round" class="mono">{temp:.0}°</text>"#,
                tx = nx + 7.0,
                ty = ny - 5.0,
            );
        }
    }

    // Clear, labelled sunrise & sunset lines at the day/night boundary.
    if let Some((sunrise, sunset)) = solar {
        for (hh, glyph, label, col, anchor) in [
            (sunrise, "\u{2600}", "sunrise", "var(--dawn-ink)", "start"),
            (sunset, "\u{263e}", "sunset", "var(--fg-3)", "end"),
        ] {
            let mx = x_of(hh);
            let tx = if anchor == "start" {
                mx + 5.0
            } else {
                mx - 5.0
            };
            let _ = write!(
                svg,
                r#"<line x1="{mx:.1}" y1="0" x2="{mx:.1}" y2="{base:.1}" stroke="{col}" stroke-width="1" stroke-dasharray="2 3" stroke-opacity="0.6"/><text x="{tx:.1}" y="11" text-anchor="{anchor}" font-size="9" fill="{col}" stroke="var(--surface)" stroke-width="3" paint-order="stroke" stroke-linejoin="round" class="mono">{glyph} {label} {time}</text>"#,
                time = fmt_clock(hh),
            );
        }
    }

    // "Now" line + pill.
    if (0.0..24.0).contains(&now_h) {
        let nx = x_of(now_h);
        let _ = write!(
            svg,
            r#"<line x1="{nx:.1}" y1="0" x2="{nx:.1}" y2="{base:.1}" stroke="var(--fg)" stroke-width="1.25"/><rect x="{px:.1}" y="0" width="34" height="15" rx="7.5" fill="var(--fg)"/><text x="{tx:.1}" y="11" text-anchor="middle" font-size="9" fill="var(--bg)" class="mono">now</text>"#,
            px = (nx - 17.0).clamp(0.0, w - 34.0),
            tx = (nx).clamp(17.0, w - 17.0),
        );
    }

    // Hour ticks.
    for hour in [0, 3, 6, 9, 12, 15, 18, 21] {
        let x = f64::from(hour) * hw;
        // The first tick sits on the left edge of the drawing: centred there, half
        // of "00" was outside the viewBox and the axis read "0  03  06".
        let anchor = if hour == 0 { "start" } else { "middle" };
        let _ = write!(
            svg,
            r#"<text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="9" fill="var(--fg-4)">{hour:02}</text>"#,
            ty = h - 3.0,
        );
    }

    svg.push_str("</svg>");
    svg
}

/// `19.5` → `"19:30"` for the sunrise/sunset labels.
fn fmt_clock(h: f64) -> String {
    let total_min = (h * 60.0).round();
    format!(
        "{:.0}:{:02.0}",
        (total_min / 60.0).floor() % 24.0,
        total_min % 60.0
    )
}

/// One day of a trend.
#[cfg(feature = "analytics")]
pub struct TrendPoint {
    /// ISO date, `YYYY-MM-DD`.
    pub date: String,
    /// The day's value.
    pub value: f64,
    /// A smoothed value for the same day, if there is one (a moving average).
    pub smooth: Option<f64>,
}

/// The smallest "round" number (1, 1.5, 2, 2.5, 3, 4, 5, 6, 8 or 10 times a power
/// of ten) that is at least `x`: where the top of a value axis goes.
#[cfg(feature = "analytics")]
fn nice_ceiling(x: f64) -> f64 {
    if x <= 1.0 {
        return 1.0;
    }
    let mut mag = 10_f64.powf(x.log10().floor());
    if x / mag >= 10.0 {
        mag *= 10.0;
    }
    for m in [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0] {
        if m * mag >= x {
            return m * mag;
        }
    }
    10.0 * mag
}

/// A day-by-day trend as a picture, above the table it summarises.
///
/// "Is your yard getting busier?" was answered by fourteen rows of numbers in two
/// tables: the question is about a *direction*, and a column of figures makes the
/// reader do the subtraction. With `bars`, each day is a quiet neutral bar and
/// the smoothed series is the one line in the chart's colour, so the eye follows
/// the line and the bars say how noisy the days were; without, the series is the
/// line (species richness, which has no smoothing).
///
/// A value axis from zero with three rules, labelled; the first, middle and last
/// dates; every point's date and value in its `<title>`. The table is still on
/// the page, behind a disclosure, for anyone who wants to read it.
#[cfg(feature = "analytics")]
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn trend_chart(points: &[TrendPoint], unit: &str, bars: bool) -> String {
    if points.len() < 2 {
        return EMPTY.to_string();
    }
    let w = 640.0_f64;
    let h = 176.0_f64;
    let (pad_l, pad_r, pad_t, pad_b) = (34.0_f64, 8.0_f64, 10.0_f64, 22.0_f64);
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;
    let n = points.len();
    let top = points
        .iter()
        .flat_map(|p| [p.value, p.smooth.unwrap_or(0.0)])
        .fold(0.0_f64, f64::max);
    let ymax = nice_ceiling(top);
    let y_at = |v: f64| pad_t + plot_h - (v / ymax).clamp(0.0, 1.0) * plot_h;
    let slot = plot_w / n as f64;
    let x_at = |i: usize| pad_l + (i as f64 + 0.5) * slot;
    let fmt = |v: f64| {
        if (v - v.round()).abs() < 0.05 {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.1}")
        }
    };

    let mut svg = format!(
        r#"<div class="viz-scroll"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="viz-svg-block viz-trend">"#
    );
    svg.push_str(&svg_a11y(
        "Daily trend",
        &format!(
            "{unit} per day, {n} days, as {}.",
            if bars {
                "daily bars with a smoothed line over them"
            } else {
                "a line"
            }
        ),
    ));

    // Value axis: zero, half, top.
    for frac in [0.0_f64, 0.5, 1.0] {
        let v = ymax * frac;
        let y = y_at(v);
        let _ = write!(
            svg,
            r#"<line x1="{pad_l:.0}" y1="{y:.1}" x2="{x2:.1}" y2="{y:.1}" stroke="var(--hairline)" stroke-width="1" vector-effect="non-scaling-stroke"/><text class="mono" x="{tx:.0}" y="{ty:.1}" text-anchor="end" font-size="10" fill="var(--fg-3)">{label}</text>"#,
            x2 = w - pad_r,
            tx = pad_l - 6.0,
            ty = y + 3.5,
            label = fmt(v),
        );
    }

    let long_of =
        |p: &TrendPoint| civil_label(&p.date, 0).map_or_else(|| p.date.clone(), |(_, long)| long);

    if bars {
        let bw = (slot * 0.5).max(2.0);
        for (i, p) in points.iter().enumerate() {
            let y = y_at(p.value);
            let _ = write!(
                svg,
                r#"<rect x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" rx="2" fill="var(--chart-other)"><title>{d} — {v} {unit}</title></rect>"#,
                x = x_at(i) - bw / 2.0,
                bh = (y_at(0.0) - y).max(0.0),
                d = escape_html(&long_of(p)),
                v = fmt(p.value),
                unit = escape_html(unit),
            );
        }
    }

    // The line: the smoothed series where there is one, else the values.
    let line_of = |get: &dyn Fn(&TrendPoint) -> Option<f64>| -> Vec<(f64, f64)> {
        points
            .iter()
            .enumerate()
            .filter_map(|(i, p)| get(p).map(|v| (x_at(i), y_at(v))))
            .collect()
    };
    let pts = if bars {
        line_of(&|p| p.smooth)
    } else {
        line_of(&|p| Some(p.value))
    };
    if pts.len() >= 2 {
        let mut d = String::new();
        for (k, (x, y)) in pts.iter().enumerate() {
            let _ = write!(d, "{}{x:.1},{y:.1} ", if k == 0 { "M" } else { "L" });
        }
        let _ = write!(
            svg,
            r#"<path d="{d}" fill="none" stroke="var(--chart-1)" stroke-width="2.6" stroke-linejoin="round" stroke-linecap="round" vector-effect="non-scaling-stroke"/>"#
        );
    }
    if !bars {
        for (i, p) in points.iter().enumerate() {
            let _ = write!(
                svg,
                r#"<circle cx="{x:.1}" cy="{y:.1}" r="2.6" fill="var(--chart-1)" stroke="var(--card-top)" stroke-width="1" vector-effect="non-scaling-stroke"><title>{d} — {v} {unit}</title></circle>"#,
                x = x_at(i),
                y = y_at(p.value),
                d = escape_html(&long_of(p)),
                v = fmt(p.value),
                unit = escape_html(unit),
            );
        }
    }
    if let Some((x, y)) = pts.last() {
        let _ = write!(
            svg,
            r#"<circle cx="{x:.1}" cy="{y:.1}" r="3.4" fill="var(--chart-1)" stroke="var(--card-top)" stroke-width="1.5" vector-effect="non-scaling-stroke"/>"#
        );
    }

    // First, middle and last date.
    for (i, anchor) in [(0_usize, "start"), (n / 2, "middle"), (n - 1, "end")] {
        let label = civil_label(&points[i].date, 0)
            .map_or_else(|| points[i].date.clone(), |(short, _)| short);
        let x = match anchor {
            "start" => pad_l,
            "end" => w - pad_r,
            _ => x_at(i),
        };
        let _ = write!(
            svg,
            r#"<text class="mono" x="{x:.1}" y="{y:.1}" text-anchor="{anchor}" font-size="10" fill="var(--fg-3)">{label}</text>"#,
            y = h - 6.0,
            label = escape_html(&label),
        );
    }
    svg.push_str("</svg></div>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamgraph_empty_and_basic() {
        assert!(streamgraph(&[], None).contains("Not enough data"));
        let series = vec![
            ("Blue Jay".to_string(), vec![1, 3, 2, 5, 4]),
            ("American Robin".to_string(), vec![0, 2, 4, 3, 1]),
        ];
        let svg = streamgraph(&series, Some("2026-08-17"));
        assert!(svg.contains("<svg") && svg.contains("<path"));
        assert!(svg.contains("Blue Jay"));
        assert!(svg.contains("<title>Activity streamgraph</title>"));
        assert!(svg.contains("<desc>Detections per day"));
        // The chart's name is its <title>; the scroll box around it is named for
        // what it does, because on a phone it scrolls and an axe rule requires a
        // scrollable region to be reachable by keyboard.
        let tag = &svg[svg.find("<svg").unwrap()..];
        assert!(
            !tag[..tag.find('>').unwrap()].contains("aria-label"),
            "{svg}"
        );
        assert!(
            svg.contains(r#"class="viz-scroll" tabindex="0" role="group""#),
            "{svg}"
        );

        // Axes. Without them the chart carried no scale at all: a reader could
        // not tell the period shown or whether a band meant five detections or
        // five hundred.
        assert!(
            svg.contains("The bar at the left is"),
            "the vertical scale must be explained: {svg}"
        );
        assert!(
            svg.contains(">17 Aug<"),
            "the last column must carry its date: {svg}"
        );
        assert!(
            svg.contains(">13 Aug<"),
            "five days ending 17 Aug start on 13 Aug: {svg}"
        );
    }

    /// The numbers behind the picture. A stacked band's thickness is the value
    /// and nothing printed it; now the scale bar is a count a person can hold,
    /// the busiest day is marked, and each column's facts are in the markup for
    /// the readout and in a table for everyone else.
    #[test]
    fn the_streamgraph_says_how_much() {
        let series = vec![
            ("Blue Jay".to_string(), vec![1, 3, 2, 26, 4]),
            ("American Robin".to_string(), vec![0, 2, 4, 3, 1]),
        ];
        let svg = streamgraph(&series, Some("2026-08-17"));
        // Busiest day is 26 + 3 = 29; two fifths of it is 11.6, so the bar is 10.
        assert!(
            svg.contains("The bar at the left is 10 detections in a day. The busiest day had 29."),
            "{svg}"
        );
        // The columns, with weekday and date, in order, one count per species.
        assert!(
            svg.contains(r#"data-names="Blue Jay|American Robin""#),
            "{svg}"
        );
        assert!(
            svg.contains(r#"data-cols="Thu 13 Aug:1,0;Fri 14 Aug:3,2;Sat 15 Aug:2,4;Sun 16 Aug:26,3;Mon 17 Aug:4,1""#),
            "{svg}"
        );
        // The table is the same facts per species.
        assert!(svg.contains("Show as a table"), "{svg}");
        assert!(
            svg.contains(r#"<td class="num">36</td><td>Sun 16 Aug</td><td class="num">26</td>"#),
            "Blue Jay: 36 in all, busiest on Sun 16 Aug with 26: {svg}"
        );
    }

    /// What is painted is what was graded: the bands are the chart tokens, solid,
    /// with a seam, and no transparency to wash them toward the card.
    #[test]
    fn the_streamgraph_paints_the_palette_and_nothing_else() {
        let series: Vec<(String, Vec<i64>)> = (0..9)
            .map(|i| (format!("Species {i}"), vec![i + 1, 2, 3]))
            .collect();
        let svg = streamgraph(&series, None);
        for n in 1..=8 {
            assert!(
                svg.contains(&format!(r#"fill="var(--chart-{n})""#)),
                "slot {n}: {svg}"
            );
        }
        assert!(
            svg.contains(r#"fill="var(--chart-other)""#),
            "the ninth series is the neutral, not a ninth hue: {svg}"
        );
        assert!(!svg.contains("fill-opacity"), "{svg}");
        assert!(!svg.contains("oklch("), "{svg}");
    }

    /// The nice number the scale bar shows is a 1, 2 or 5 times a power of ten
    /// and never more than the value it was derived from.
    #[test]
    fn nice_counts_are_one_two_or_five() {
        for (x, want) in [
            (0.2, 1),
            (1.0, 1),
            (1.9, 1),
            (2.0, 2),
            (4.9, 2),
            (5.0, 5),
            (9.9, 5),
            (10.4, 10),
            (26.0, 20),
            (51.0, 50),
            (999.0, 500),
            (1000.0, 1000),
        ] {
            assert_eq!(nice_count(x), want, "nice_count({x})");
        }
    }

    /// Date labels must walk back correctly across a month boundary.
    ///
    /// The case a naive `day - n` gets wrong, and the reason this goes through
    /// the workspace's civil-date arithmetic rather than string maths.
    #[test]
    fn streamgraph_dates_cross_a_month_boundary() {
        let series = vec![("Blue Jay".to_string(), vec![1, 2, 3, 4, 5])];
        let svg = streamgraph(&series, Some("2026-03-02"));
        assert!(
            svg.contains(">26 Feb<"),
            "five days ending 2026-03-02 start on 26 Feb: {svg}"
        );
    }

    #[test]
    fn streamgraph_survives_an_unparseable_end_date() {
        let series = vec![("Blue Jay".to_string(), vec![1, 2])];
        // A render-path panic is a station outage (panic = abort, no catch
        // layer), so a malformed date must degrade rather than crash.
        let svg = streamgraph(&series, Some("not-a-date"));
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn accumulation_empty_and_monotonic() {
        assert!(accumulation_curve(&[]).contains("Not enough data"));
        let pts = vec![
            ("24-03".to_string(), 3),
            ("24-04".to_string(), 7),
            ("24-05".to_string(), 12),
        ];
        let svg = accumulation_curve(&pts);
        assert!(svg.contains("<svg") && svg.contains("12 species"));
        assert!(svg.contains("<title>Species accumulation over time</title>"));
        assert!(svg.contains("<desc>A rising line"));
        assert!(!svg.contains("aria-label"));
    }

    /// The total is written at the point where the curve ends, and that point is
    /// always the chart's highest, so the label was clamped to the top of the
    /// drawing — *below* the point, with the curve's last stretch running through
    /// "15 species". The plot now leaves headroom and the label sits above the point;
    /// the curve never rises to the left of its end, so nothing can cross it there.
    ///
    /// Observed failing before the fix: the label's baseline was 8 units below the
    /// end point.
    #[test]
    fn the_total_sits_above_the_curve_it_labels() {
        let pts = vec![
            ("24-03".to_string(), 3),
            ("24-04".to_string(), 14),
            ("24-05".to_string(), 15),
        ];
        let svg = accumulation_curve(&pts);
        let number_after = |s: &str, key: &str| -> f64 {
            let rest = s.split_once(key).expect(key).1;
            rest.split('"')
                .next()
                .expect("a value")
                .parse()
                .expect("a number")
        };
        let circle = svg.split("<circle").nth(1).expect("the end point");
        let end_y = number_after(circle, r#"cy=""#);
        let label = svg
            .split("<text")
            .find(|t| t.contains("15 species"))
            .expect("the total's label");
        let baseline = number_after(label, r#" y=""#);
        assert!(
            baseline <= end_y - 6.0,
            "the label's baseline ({baseline}) is not above the end point ({end_y})"
        );
        // Room for the glyphs above that baseline (the label-fitting script can
        // enlarge them to about 16 units), or the top of the drawing cuts them.
        assert!(
            baseline >= 16.0,
            "the label's baseline ({baseline}) leaves no room above it"
        );
    }

    #[test]
    fn day_strip_empty_and_basic() {
        let zero = [0i64; 24];
        assert!(day_strip(&zero, &[], Some((6.0, 19.0)), 12.0).contains("Not enough"));
        let mut hourly = [0i64; 24];
        hourly[6] = 4;
        hourly[7] = 7;
        let temps: Vec<(f64, f64)> = (0..24).map(|h| (f64::from(h) + 0.5, 12.0)).collect();
        let svg = day_strip(&hourly, &temps, Some((6.0, 19.5)), 13.0);
        assert!(svg.contains("<svg"));
        // The labelled solar lines carry real clock times.
        assert!(svg.contains("sunrise 6:00"));
        assert!(svg.contains("sunset 19:30"));
        // Temperature line + the now-reading dot.
        assert!(svg.contains("var(--dawn-ink)"));
        assert!(svg.contains("<circle"));
        assert!(svg.contains("now")); // current-time pill
        // The pre-spine per-detection hue dots are gone (a11y decision).
        assert!(!svg.contains("data-species-fill"));
        // Accessible name + an honest description that names the solar markers.
        assert!(svg.contains("<title>Activity across the day</title>"));
        assert!(svg.contains(
            "<desc>Hourly detection counts across the day, with sunrise, sunset and now markers</desc>"
        ));
        assert!(!svg.contains("aria-label"));
    }

    #[test]
    fn day_strip_without_location_or_weather_still_renders() {
        let mut hourly = [0i64; 24];
        hourly[9] = 3;
        let svg = day_strip(&hourly, &[], None, 10.0);
        assert!(svg.contains("<svg"));
        // No solar facts → no sun lines and no night bands (never fake them).
        assert!(!svg.contains("sunrise"));
        assert!(!svg.contains("var(--night)"));
        // The honest description promises only the markers actually drawn.
        assert!(
            svg.contains("<desc>Hourly detection counts across the day, with a now marker</desc>")
        );
    }

    #[cfg(feature = "analytics")]
    fn trend(values: &[f64]) -> Vec<TrendPoint> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| TrendPoint {
                date: format!("2026-10-{:02}", i + 6),
                value: *v,
                smooth: if i >= 2 { Some(*v - 1.0) } else { None },
            })
            .collect()
    }

    /// The question the Trends tab asks is about a direction, so the chart has a
    /// value axis from zero (three labelled rules), the first, middle and last
    /// dates, every day's date and value in its tooltip, and the smoothed series
    /// as the one line in the chart's colour over neutral bars.
    #[cfg(feature = "analytics")]
    #[test]
    fn the_trend_chart_has_an_axis_dates_and_one_coloured_line() {
        let svg = trend_chart(&trend(&[33.0, 31.0, 32.0, 34.0, 33.0]), "detections", true);
        for label in ["0<", "20<", "40<"] {
            assert!(svg.contains(&format!(">{label}")), "axis {label}: {svg}");
        }
        for date in [">6 Oct<", ">8 Oct<", ">10 Oct<"] {
            assert!(svg.contains(date), "{date}: {svg}");
        }
        assert!(svg.contains("Tue 6 Oct — 33 detections"), "{svg}");
        assert_eq!(
            svg.matches(r#"fill="var(--chart-other)""#).count(),
            5,
            "five bars"
        );
        assert_eq!(
            svg.matches(r#"stroke="var(--chart-1)""#).count(),
            1,
            "one line"
        );
        assert!(
            !svg.contains("fill-opacity") && !svg.contains("oklch("),
            "{svg}"
        );
    }

    /// Without smoothing the series is the line, with a dot per day.
    #[cfg(feature = "analytics")]
    #[test]
    fn a_trend_without_bars_is_a_line_with_a_dot_per_day() {
        let svg = trend_chart(&trend(&[13.0, 12.0, 14.0]), "species", false);
        assert!(!svg.contains("chart-other"), "{svg}");
        assert_eq!(
            svg.matches("<circle").count(),
            4,
            "a dot per day and the end mark: {svg}"
        );
        assert!(svg.contains("— 14 species"), "{svg}");
        assert!(trend_chart(&trend(&[1.0]), "x", true).contains("Not enough data"));
    }

    #[cfg(feature = "analytics")]
    #[test]
    fn the_axis_tops_out_on_a_round_number_at_or_above_the_data() {
        for (x, want) in [
            (0.0, 1.0),
            (1.0, 1.0),
            (14.0, 15.0),
            (35.0, 40.0),
            (26.0, 30.0),
            (51.0, 60.0),
            (100.0, 100.0),
            (101.0, 150.0),
        ] {
            assert!(
                (nice_ceiling(x) - want).abs() < 1e-9,
                "nice_ceiling({x}) = {}",
                nice_ceiling(x)
            );
        }
    }
}
