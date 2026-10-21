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

use super::kit;
use super::{EMPTY, svg_a11y};
use crate::routes::pages::atoms::series_color;
use crate::routes::pages::escape_html;

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

/// The `y` of every edge of a centred stack, one list per edge, bottom edge first.
///
/// `counts[k][d]` is series `k` on day `d`. There are `counts.len() + 1` edges: the
/// first is the bottom of the lowest band, the last the top of the highest, and band
/// `k` lies between edge `k` and edge `k + 1`: so two neighbours share an edge, and
/// cannot leave a gap or overlap. The stack is centred on `mid` on every day (a
/// "themeriver"): the day's total is split evenly above and below, so there is no
/// zero line to read a value against (the scale bar and the tooltip carry the values).
/// `scale` is units of `y` per detection.
pub(crate) fn stack_edges(counts: &[Vec<i64>], days: usize, mid: f64, scale: f64) -> Vec<Vec<f64>> {
    let at = |k: usize, d: usize| counts[k].get(d).copied().unwrap_or(0) as f64;
    let mut edges = vec![vec![0.0; days]; counts.len() + 1];
    for d in 0..days {
        let total: f64 = (0..counts.len()).map(|k| at(k, d)).sum();
        let mut cum = -total / 2.0;
        edges[0][d] = mid - cum * scale;
        for k in 0..counts.len() {
            cum += at(k, d);
            edges[k + 1][d] = mid - cum * scale;
        }
    }
    edges
}

/// Where a label `width` by `height` units can sit entirely inside a band, if
/// anywhere: the centre `(x, y)` with the most clearance.
///
/// `xs[d]` is each day's x and `top[d]`, `bottom[d]` the band's edges there (`y`
/// grows downward, so `top < bottom`). The label may sit anywhere in `x_min..=x_max`;
/// at each candidate position every day under it, and the label's own two ends
/// (interpolated between days), must have the whole text height between the edges.
/// A band too thin or too short for the text returns `None`, and the legend and the
/// tooltip carry it instead: the text is never clipped or struck through by its own band.
pub(crate) fn place_in_band(
    xs: &[f64],
    top: &[f64],
    bottom: &[f64],
    width: f64,
    height: f64,
    (x_min, x_max): (f64, f64),
) -> Option<(f64, f64)> {
    if xs.len() < 2 || width > x_max - x_min {
        return None;
    }
    let edge_at = |e: &[f64], x: f64| -> f64 {
        // Linear between the two days either side of `x`.
        let i = xs.partition_point(|&v| v <= x).clamp(1, xs.len() - 1);
        let (xa, xb) = (xs[i - 1], xs[i]);
        let t = if xb > xa { ((x - xa) / (xb - xa)).clamp(0.0, 1.0) } else { 0.0 };
        e[i - 1] + (e[i] - e[i - 1]) * t
    };
    let mut best: Option<(f64, f64, f64)> = None; // (clearance, x, y)
    for &x in xs {
        let cx = x.clamp(x_min + width / 2.0, x_max - width / 2.0);
        let (l, r) = (cx - width / 2.0, cx + width / 2.0);
        let mut hi = edge_at(top, l).max(edge_at(top, r));
        let mut lo = edge_at(bottom, l).min(edge_at(bottom, r));
        for d in 0..xs.len() {
            if xs[d] >= l && xs[d] <= r {
                hi = hi.max(top[d]);
                lo = lo.min(bottom[d]);
            }
        }
        let clearance = lo - hi - height;
        if clearance >= 2.0 && best.is_none_or(|b| clearance > b.0 + 1e-9) {
            best = Some((clearance, cx, (hi + lo) / 2.0));
        }
    }
    best.map(|(_, x, y)| (x, y))
}

/// The columns an axis labels: at most `max` of the `days`, evenly spaced, the first
/// and the last always among them.
pub(crate) fn axis_columns(days: usize, max: usize) -> Vec<usize> {
    if days == 0 {
        return Vec::new();
    }
    let n = days.min(max.max(2));
    let mut v: Vec<usize> = (0..n)
        .map(|i| ((i as f64) * (days - 1) as f64 / (n - 1).max(1) as f64).round() as usize)
        .collect();
    v.dedup();
    v
}

/// Centred stacked-area ("themeriver") of per-day counts. Each series is
/// `(common_name, daily_counts)`; all vectors should share a length.
///
/// `end_date` is the calendar date of the **last** column, used to label the
/// time axis and the readout. `None` omits the dates but still draws the scale.
///
/// `listened[d]` says whether the station was recording on day `d` (from the recording-
/// effort ledger); `None` means the ledger has nothing for this period, and nothing is
/// claimed. A day it was *not* listening is hatched and says so, because a stack that
/// pinches to nothing on that day would otherwise read as a silent day: a quiet day and
/// an unrecorded one are different facts.
///
/// # What a reader can take from it
///
/// A centred stack has no baseline to read a value against: the thickness of a
/// band on one day *is* the number, and nothing printed it. So:
///
/// * a **scale bar** in the left margin, as tall as `nice` detections in a day (a 1,
///   2 or 5 times a power of ten, about two fifths of the busiest day), so a band's
///   thickness can be held against something;
/// * the **busiest day** marked on the chart with its count;
/// * **direct labels** on the bands that are thick enough to carry their own name
///   (placed by [`place_in_band`], never clipped, never over a neighbour); the legend
///   names the rest;
/// * a **tooltip** (`static/chart-kit.js`): move a pointer or a finger across it, or
///   use the arrow keys, and the day under it is printed with every species' count and
///   a crosshair; resting on one band, or on its legend entry, puts that species in
///   the foreground and the rest at `--chart-dim`;
/// * a **table** behind a disclosure with the same facts per species, which is also
///   what a screen reader uses: the chart itself is one image.
///
/// # What the curve is
///
/// Each edge between bands is a monotone cubic through the daily values
/// ([`kit::monotone_path`]): smooth, but between two days it never leaves the range of
/// the two, so smoothing cannot invent a peak that was not counted. Two bands share
/// the same edge curve, so they never part or overlap.
///
/// Colours are `--chart-1` … `--chart-8` by rank (a ninth series is `--chart-other`),
/// with a surface-coloured seam between bands.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn streamgraph(
    series: &[(String, Vec<i64>)],
    end_date: Option<&str>,
    listened: Option<&[bool]>,
) -> String {
    let days = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    if series.is_empty() || days < 2 {
        return EMPTY.to_string();
    }

    let w = 760.0_f64;
    // Left margin for the scale bar; a little on the right so the last column's seam is
    // not clipped.
    let (pad_l, pad_r) = (46.0_f64, 6.0_f64);
    let plot_w = w - pad_l - pad_r;
    let plot_h = 240.0_f64;
    let head = 22.0_f64; // room above the plot for the busiest day's count
    let foot = 26.0_f64; // room under it for the date axis
    let h = head + plot_h + foot;
    let mid = plot_h / 2.0;
    let step = plot_w / (days - 1) as f64;
    let xs: Vec<f64> = (0..days).map(|d| pad_l + d as f64 * step).collect();
    let x_at = |d: usize| xs[d];

    let counts: Vec<Vec<i64>> = series
        .iter()
        .map(|(_, v)| (0..days).map(|d| v.get(d).copied().unwrap_or(0)).collect())
        .collect();
    let totals: Vec<i64> = (0..days).map(|d| counts.iter().map(|c| c[d]).sum()).collect();
    let (peak_day, max_total) = totals
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|&(d, t)| (t, std::cmp::Reverse(d)))
        .unwrap_or((0, 0));
    let max_f = max_total.max(1) as f64;
    // The fattest day fills ~88% of the height.
    let scale = (plot_h * 0.88) / max_f;
    let nice = nice_count(max_f * 0.4);
    let edges = stack_edges(&counts, days, mid, scale);
    let off = |d: usize| listened.is_some_and(|l| l.get(d).is_some_and(|on| !on));

    let band_classes: Vec<String> = (0..series.len().min(9)).map(kit::slot_class).collect();
    let mut classes_for_defs = band_classes.clone();
    classes_for_defs.dedup();
    let names: Vec<(String, String)> = series
        .iter()
        .enumerate()
        .map(|(k, (n, _))| (n.clone(), series_color(k, series.len())))
        .collect();
    let col_label = |d: usize| -> String {
        end_date
            .and_then(|e| kit::civil_label(e, d as i64 - (days as i64 - 1)))
            .map_or_else(|| format!("Day {}", d + 1), |(_, long)| long)
    };
    let cols: Vec<kit::Col> = (0..days)
        .map(|d| kit::Col {
            label: col_label(d),
            values: counts.iter().map(|c| Some(c[d] as f64)).collect(),
            note: off(d).then(|| "The station was not listening".to_string()),
        })
        .collect();

    let mut svg = String::with_capacity(16 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><div class="viz-scroll" tabindex="0" role="group" aria-label="Activity streamgraph, scrolls sideways on a narrow screen"><svg width="100%" viewBox="0 -{head:.0} {w:.0} {h:.0}" role="img" class="viz-svg-block viz-stream ck-in" data-ck-id="stream-{days}" data-ck-dim="series" data-ck-label="Left and right read each day. Up and down put one species in the foreground."{attrs}>"#,
        attrs = kit::cols_attrs(&cols, &names, pad_l, pad_l + plot_w, false, "detections", Some("All species")),
    );
    svg.push_str(&svg_a11y(
        "Activity streamgraph",
        "Detections per day over time as a centred stack; each coloured band is one species and its thickness is that day's count.",
    ));
    svg.push_str(&kit::gradient_defs("sg", &classes_for_defs));
    if (0..days).any(off) {
        let _ = write!(svg, "<defs>{}</defs>", kit::nodata_pattern("sg"));
    }
    // Days the station was not listening: hatched from edge to edge of the day.
    for d in (0..days).filter(|&d| off(d)) {
        let (a, b) = (x_at(d) - step / 2.0, x_at(d) + step / 2.0);
        let (a, b) = (a.max(pad_l), b.min(pad_l + plot_w));
        let _ = write!(
            svg,
            r#"<rect x="{a:.1}" y="0" width="{wd:.1}" height="{plot_h:.1}" fill="url(#sg-nd)"/>"#,
            wd = b - a,
        );
    }

    // The centre line the stack is balanced on.
    let _ = write!(
        svg,
        r#"<line class="ck-grid" x1="{pad_l:.0}" y1="{mid:.1}" x2="{x2:.1}" y2="{mid:.1}"/>"#,
        x2 = pad_l + plot_w,
    );

    // Bands, lowest first. `series_color(rank)`, not a hash of the name: several species
    // are shown together here, and a hash put pairs of them 2–3° apart in hue.
    svg.push_str(r#"<g class="ck-reveal">"#);
    for (k, _) in series.iter().enumerate() {
        let top: Vec<(f64, f64)> = (0..days).map(|d| (x_at(d), edges[k + 1][d])).collect();
        let bottom: Vec<(f64, f64)> = (0..days).map(|d| (x_at(d), edges[k][d])).collect();
        let path = format!(
            "{} {}Z",
            kit::monotone_path(&top, false),
            kit::monotone_path_reversed(&bottom)
        );
        let cls = kit::slot_class(k);
        let _ = write!(
            svg,
            r#"<path class="ck-seam" d="{path}" fill="url(#sg-{cls})" data-ck-s="{k}"/>"#,
        );
    }
    svg.push_str("</g>");

    // Direct labels: only where the whole name fits inside its band. A band that is
    // too thin for the full size gets a second try a notch smaller; after that the
    // legend and the tooltip carry it.
    for (k, (name, _)) in series.iter().enumerate() {
        if counts[k].iter().sum::<i64>() == 0 {
            continue;
        }
        let top = &edges[k + 1];
        let bottom = &edges[k];
        let short = {
            let words: Vec<&str> = name.split_whitespace().collect();
            if words.len() > 1 {
                let mut s: String = words[..words.len() - 1]
                    .iter()
                    .filter_map(|w| w.chars().next())
                    .flat_map(|c| [c, '.', ' '])
                    .collect();
                s.push_str(words[words.len() - 1]);
                s
            } else {
                name.clone()
            }
        };
        let cut: String = if name.chars().count() > 14 {
            format!("{}\u{2026}", name.chars().take(13).collect::<String>().trim_end())
        } else {
            name.clone()
        };
        'sizes: for font in [11.0_f64, 9.5] {
            for text in [name.clone(), short.clone(), cut.clone()] {
                let tw = kit::text_width(&text, font);
                if let Some((x, y)) = place_in_band(&xs, top, bottom, tw, font + 1.0, (pad_l + 4.0, pad_l + plot_w - 4.0)) {
                    let _ = write!(
                        svg,
                        r#"<text class="ck-bandlbl ck-halo" x="{x:.1}" y="{ty:.1}" text-anchor="middle" font-size="{font}" data-ck-s="{k}">{t}</text>"#,
                        ty = y + font * 0.36,
                        t = escape_html(&text),
                    );
                    break 'sizes;
                }
            }
        }
    }

    // The scale bar: `nice` detections in one day, to hold a band against.
    let bar_half = nice as f64 * scale / 2.0;
    let bx = pad_l - 12.0;
    let _ = write!(
        svg,
        r#"<g class="ck-axis" stroke-linecap="butt"><line x1="{bx:.1}" y1="{a:.1}" x2="{bx:.1}" y2="{b:.1}"/><line x1="{bx0:.1}" y1="{a:.1}" x2="{bx1:.1}" y2="{a:.1}"/><line x1="{bx0:.1}" y1="{b:.1}" x2="{bx1:.1}" y2="{b:.1}"/></g><text class="mono ck-lbl" x="{bx:.1}" y="{ty:.1}" text-anchor="middle" font-size="10">{n}</text>"#,
        a = mid - bar_half,
        b = mid + bar_half,
        bx0 = bx - 4.0,
        bx1 = bx + 4.0,
        ty = mid - bar_half - 6.0,
        n = kit::compact(nice as f64),
    );

    // The busiest day, marked where it happened.
    let px = x_at(peak_day);
    let py = edges[counts.len()][peak_day];
    let anchor = if peak_day * 5 < days {
        "start"
    } else if peak_day * 5 > days * 4 {
        "end"
    } else {
        "middle"
    };
    let _ = write!(
        svg,
        r#"<line class="ck-axis" x1="{px:.1}" y1="{py:.1}" x2="{px:.1}" y2="{ly:.1}"/><text class="mono ck-annot ck-halo" x="{px:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10">{peak}</text>"#,
        ly = py - 5.0,
        ty = py - 8.0,
        peak = kit::compact(max_total as f64),
    );

    // Time axis: a rule, and evenly spaced dates with a tick over each.
    let _ = write!(
        svg,
        r#"<line class="ck-axis" x1="{pad_l:.0}" y1="{plot_h:.0}" x2="{x2:.1}" y2="{plot_h:.0}"/>"#,
        x2 = pad_l + plot_w,
    );
    let cols_to_label = axis_columns(days, 8);
    for (n, &d) in cols_to_label.iter().enumerate() {
        let label = end_date
            .and_then(|e| kit::civil_label(e, d as i64 - (days as i64 - 1)))
            .map(|(s, _)| s)
            .unwrap_or_default();
        if label.is_empty() {
            continue;
        }
        let anchor = if n == 0 {
            "start"
        } else if n + 1 == cols_to_label.len() {
            "end"
        } else {
            "middle"
        };
        let x = x_at(d);
        let _ = write!(
            svg,
            r#"<line class="ck-axis" x1="{x:.1}" y1="{plot_h:.0}" x2="{x:.1}" y2="{t:.0}"/><text class="mono ck-lbl" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="10">{label}</text>"#,
            t = plot_h + 4.0,
            ty = plot_h + 17.0,
        );
    }

    // The crosshair the kit moves; hidden until it is asked for.
    let _ = write!(
        svg,
        r#"<line class="ck-cursor" x1="{pad_l:.0}" y1="0" x2="{pad_l:.0}" y2="{plot_h:.0}" visibility="hidden"/>"#,
    );
    svg.push_str("</svg></div>");

    // The reading as a line of text too, idle until a pointer is on the chart.
    let _ = write!(
        svg,
        r#"<p class="bnb-meta viz-readout">The bar at the left is {nice} detection{s} in a day. The busiest day had {max_total}. Move across the chart, or use the arrow keys, to read a day.</p>"#,
        s = if nice == 1 { "" } else { "s" },
    );

    // Legend: buttons, because hovering or pressing one puts that species in the
    // foreground. Keyed like the bands, by rank.
    svg.push_str(r#"<div class="viz-legend ck-legend">"#);
    for (rank, (name, _)) in series.iter().enumerate() {
        let _ = write!(
            svg,
            r#"<button type="button" class="bnb-meta viz-legend-item ck-leg" data-ck-s="{rank}" aria-pressed="false"><span class="viz-swatch ck-sw {cls}"></span>{n}</button>"#,
            cls = kit::slot_class(rank),
            n = escape_html(name),
        );
    }
    svg.push_str("</div>");

    // The same facts as a table: what a keyboard or a screen reader uses, and
    // what to copy a number from.
    svg.push_str(
        r#"<details class="viz-table"><summary>Show as a table</summary><div class="viz-table-scroll"><table class="pt-tbl"><caption class="sr-only">Detections per species over the period shown</caption><thead><tr><th scope="col">Species</th><th scope="col" class="num">Detections</th><th scope="col">Busiest day</th><th scope="col" class="num">That day</th></tr></thead><tbody>"#,
    );
    for (rank, (name, _)) in series.iter().enumerate() {
        let total: i64 = counts[rank].iter().sum();
        let (best_day, best) = counts[rank]
            .iter()
            .copied()
            .enumerate()
            .max_by_key(|&(d, v)| (v, std::cmp::Reverse(d)))
            .unwrap_or((0, 0));
        let when = if best == 0 {
            "\u{2014}".to_string()
        } else {
            col_label(best_day)
        };
        let _ = write!(
            svg,
            r#"<tr><th scope="row"><span class="viz-swatch ck-sw {cls}"></span> {n}</th><td class="num">{total}</td><td>{when}</td><td class="num">{best}</td></tr>"#,
            cls = kit::slot_class(rank),
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

/// "When you heard them": today's detections by hour as one column chart.
///
/// Columns on the sequential ramp, deeper for a busier hour and rounded at their tips;
/// the night before sunrise and after sunset shaded when the station has a location;
/// a "now" marker, with the hours still to come left empty and labelled "not yet";
/// and a tooltip for each hour (its count, its share of the day, the three species
/// heard most). Temperature, when there is weather, is its own labelled strip under
/// the axis, never a second scale on the bars' plot.
///
/// `hourly` is the per-hour detection count; `top` the species heard most each hour;
/// `temps` is `(hour 0–24, °C)` samples (empty means no strip); `solar` is
/// `(sunrise, sunset)` hours when the station has a location; `now_h` is the current
/// hour-of-day. All hours share one clock axis. The geometry is in [`super::bars`].
#[must_use]
pub fn day_strip(
    hourly: &[i64; 24],
    top: &[Vec<(String, i64)>; 24],
    temps: &[(f64, f64)],
    solar: Option<(f64, f64)>,
    now_h: f64,
) -> String {
    // The accessible description only promises markers that are actually drawn.
    let desc = if solar.is_some() {
        "Detections for each hour of today, with night shaded and a now marker. Each hour's count and its busiest species are in its tooltip."
    } else {
        "Detections for each hour of today, with a now marker. Each hour's count and its busiest species are in its tooltip."
    };
    super::bars::hourly(&super::bars::Hourly {
        id: "today-hourly",
        name: "Activity across the day",
        desc,
        unit: "detections",
        counts: *hourly,
        fill: super::bars::Fill::Seq,
        width: 960.0,
        plot_h: 120.0,
        solar,
        now_h: (0.0..24.0).contains(&now_h).then_some(now_h),
        top: Some(top),
        temps,
        table: false,
    })
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
        |p: &TrendPoint| kit::civil_label(&p.date, 0).map_or_else(|| p.date.clone(), |(_, long)| long);

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
        let label = kit::civil_label(&points[i].date, 0)
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
        assert!(streamgraph(&[], None, None).contains("Not enough data"));
        let series = vec![
            ("Blue Jay".to_string(), vec![1, 3, 2, 5, 4]),
            ("American Robin".to_string(), vec![0, 2, 4, 3, 1]),
        ];
        let svg = streamgraph(&series, Some("2026-08-17"), None);
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
        let svg = streamgraph(&series, Some("2026-08-17"), None);
        // Busiest day is 26 + 3 = 29; two fifths of it is 11.6, so the bar is 10.
        assert!(
            svg.contains("The bar at the left is 10 detections in a day. The busiest day had 29."),
            "{svg}"
        );
        // The columns, with weekday and date, in order, one count per species, as the
        // kit script reads them.
        assert!(
            svg.contains(
                "[[&quot;Thu 13 Aug&quot;,[1.0,0.0]],[&quot;Fri 14 Aug&quot;,[3.0,2.0]],[&quot;Sat 15 Aug&quot;,[2.0,4.0]],[&quot;Sun 16 Aug&quot;,[26.0,3.0]],[&quot;Mon 17 Aug&quot;,[4.0,1.0]]]"
            ),
            "{svg}"
        );
        assert!(
            svg.contains("[[&quot;Blue Jay&quot;,&quot;var(--chart-1)&quot;],[&quot;American Robin&quot;,&quot;var(--chart-2)&quot;]]"),
            "{svg}"
        );
        // The table is the same facts per species.
        assert!(svg.contains("Show as a table"), "{svg}");
        assert!(
            svg.contains(r#"<td class="num">36</td><td>Sun 16 Aug</td><td class="num">26</td>"#),
            "Blue Jay: 36 in all, busiest on Sun 16 Aug with 26: {svg}"
        );
    }

    /// What is painted is what was graded: every band takes its colour from a chart
    /// token through a gradient whose stops are drawn by the stylesheet, so the theme
    /// chooses the value, and nothing in the markup is a literal colour.
    #[test]
    fn the_streamgraph_paints_the_palette_and_nothing_else() {
        let series: Vec<(String, Vec<i64>)> = (0..9)
            .map(|i| (format!("Species {i}"), vec![i + 1, 2, 3]))
            .collect();
        let svg = streamgraph(&series, None, None);
        for n in 1..=8 {
            assert!(
                svg.contains(&format!(r#"fill="url(#sg-ck-c{n})""#)),
                "slot {n}: {svg}"
            );
            assert!(svg.contains(&format!(r#"class="ck-gr ck-c{n}""#)), "gradient {n}: {svg}");
            assert!(
                svg.contains(&format!(r#"data-ck-series="[[&quot;Species 0&quot;,&quot;var(--chart-1)&quot;"#)),
                "{svg}"
            );
        }
        assert!(
            svg.contains(r#"fill="url(#sg-ck-cother)""#),
            "the ninth series is the neutral, not a ninth hue: {svg}"
        );
        assert!(svg.contains("var(--chart-other)"), "{svg}");
        assert!(!svg.contains("fill-opacity"), "{svg}");
        assert!(!svg.contains("oklch(") && !svg.contains("#fff") && !svg.contains("rgb("), "{svg}");
        // The chart is the kit's: its own entrance and tooltip, and a legend of buttons.
        assert!(svg.contains(r#"data-ck="cols""#) && svg.contains("ck-in"), "{svg}");
        assert_eq!(svg.matches(r#"class="bnb-meta viz-legend-item ck-leg""#).count(), 9, "{svg}");
    }

    /// A centred stack: every day's total is split evenly about the middle line, and
    /// each band's thickness is exactly its count times the scale. Two neighbouring
    /// bands share an edge, so there is no gap or overlap between them to misread.
    #[test]
    fn the_stack_is_centred_and_each_band_is_as_thick_as_its_count() {
        let counts = vec![vec![4, 0, 10], vec![2, 6, 0], vec![0, 1, 5]];
        let (mid, scale) = (100.0, 3.0);
        let e = stack_edges(&counts, 3, mid, scale);
        assert_eq!(e.len(), 4);
        for d in 0..3 {
            let total: i64 = counts.iter().map(|c| c[d]).sum();
            // Bottom edge is below the middle line and top edge above, by the same amount.
            assert!(((e[0][d] - mid) - (mid - e[3][d])).abs() < 1e-9, "day {d} is centred");
            assert!(((e[0][d] - e[3][d]) - total as f64 * scale).abs() < 1e-9, "day {d} is as tall as its total");
            for k in 0..3 {
                let thick = e[k][d] - e[k + 1][d];
                assert!((thick - counts[k][d] as f64 * scale).abs() < 1e-9, "band {k} day {d}");
                assert!(thick >= 0.0, "a band is never upside down");
            }
        }
    }

    /// The label planner's two promises: a label it places has the whole text height
    /// between the band's edges at every day under it, and a band too thin for the
    /// text gets no label rather than one struck through by its own edge.
    #[test]
    fn a_band_label_fits_inside_its_band_or_is_not_drawn() {
        let xs: Vec<f64> = (0..20).map(|i| f64::from(i) * 10.0).collect();
        // A fat belly in the middle (days 8-12), thin at both ends.
        let thick: Vec<f64> = (0..20)
            .map(|i| if (7..=12).contains(&i) { 30.0 } else { 3.0 })
            .collect();
        let top: Vec<f64> = thick.iter().map(|t| 100.0 - t / 2.0).collect();
        let bottom: Vec<f64> = thick.iter().map(|t| 100.0 + t / 2.0).collect();
        let (x, y) = place_in_band(&xs, &top, &bottom, 40.0, 12.0, (0.0, 190.0)).expect("fits in the belly");
        assert!((70.0..=120.0).contains(&x), "centred over the belly, got {x}");
        assert!((y - 100.0).abs() < 1e-6, "vertically centred, got {y}");
        for d in 0..20 {
            if xs[d] >= x - 20.0 && xs[d] <= x + 20.0 {
                assert!(top[d] <= y - 6.0 && bottom[d] >= y + 6.0, "day {d} clips the label");
            }
        }
        // Too wide for the belly: nothing.
        assert!(place_in_band(&xs, &top, &bottom, 120.0, 12.0, (0.0, 190.0)).is_none());
        // Too tall for the band anywhere: nothing.
        assert!(place_in_band(&xs, &top, &bottom, 20.0, 40.0, (0.0, 190.0)).is_none());
        // A label never leaves the plot, even when the best belly is at its edge.
        let edge_top: Vec<f64> = (0..20).map(|i| if i >= 16 { 80.0 } else { 99.0 }).collect();
        let edge_bot: Vec<f64> = (0..20).map(|i| if i >= 16 { 120.0 } else { 101.0 }).collect();
        let (ex, _) = place_in_band(&xs, &edge_top, &edge_bot, 30.0, 12.0, (0.0, 190.0)).expect("fits at the right edge");
        assert!(ex + 15.0 <= 190.0 + 1e-9, "label ran off the plot: centre {ex}");
    }

    /// The axis labels at most `max` columns, evenly, first and last included.
    #[test]
    fn the_time_axis_labels_a_few_evenly_spaced_days() {
        assert_eq!(axis_columns(7, 6), vec![0, 1, 2, 4, 5, 6]);
        assert_eq!(axis_columns(2, 6), vec![0, 1]);
        assert_eq!(axis_columns(0, 6), Vec::<usize>::new());
        let c = axis_columns(365, 6);
        assert_eq!((c.len(), c[0], c[c.len() - 1]), (6, 0, 364));
        assert!(c.windows(2).all(|w| w[1] > w[0]));
    }

    /// Every number on the page that the data can make arbitrarily large is compacted
    /// or contained: a day with 1.6 million detections, forty-character names, and
    /// sixty species must produce a chart, not an overflow.
    #[test]
    fn the_streamgraph_survives_a_huge_count_long_names_and_a_crowd() {
        let long = "Extraordinarily Long-named Warbler of the Far Northern Highlands";
        let mut series: Vec<(String, Vec<i64>)> = vec![(long.to_string(), vec![1_600_000, 2, 900_000])];
        series.extend((0..60).map(|i| (format!("Species number {i}"), vec![i, 3, 4])));
        let svg = streamgraph(&series, Some("2026-10-21"), None);
        assert!(svg.contains("<svg") && svg.contains("1.6M"), "peak is compacted: {svg}");
        assert!(!svg.contains("NaN") && !svg.contains("inf"), "{svg}");
        // The long name is never written into the plot whole: if it fits it is a label,
        // otherwise it is only in the legend and the tooltip.
        let plot = &svg[svg.find("<svg").unwrap()..svg.find("</svg>").unwrap()];
        assert!(plot.matches(long).count() <= 1, "{plot}");
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
        let svg = streamgraph(&series, Some("2026-03-02"), None);
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
        let svg = streamgraph(&series, Some("not-a-date"), None);
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

    fn no_species() -> [Vec<(String, i64)>; 24] {
        std::array::from_fn(|_| Vec::new())
    }

    #[test]
    fn day_strip_empty_and_basic() {
        let zero = [0i64; 24];
        let none = no_species();
        assert!(day_strip(&zero, &none, &[], Some((6.0, 19.0)), 12.0).contains("Not enough"));
        let mut hourly = [0i64; 24];
        hourly[6] = 4;
        hourly[7] = 7;
        let mut top = no_species();
        top[7] = vec![("American Robin".to_string(), 5)];
        let temps: Vec<(f64, f64)> = (0..24).map(|h| (f64::from(h) + 0.5, 12.0 + f64::from(h) / 4.0)).collect();
        let svg = day_strip(&hourly, &top, &temps, Some((6.0, 19.5)), 13.0);
        assert!(svg.contains("<svg"));
        // The labelled solar markers carry real clock times.
        assert!(svg.contains("sunrise 6:00"), "{svg}");
        assert!(svg.contains("sunset 19:30"), "{svg}");
        // Temperature has its own strip, labelled, and the now marker is drawn.
        assert!(svg.contains(">Temperature<"), "{svg}");
        assert!(svg.contains(">now<"), "{svg}");
        // The busiest species of the hour are in its tooltip.
        assert!(svg.contains("American Robin&#9;5"), "{svg}");
        // Accessible name + an honest description that names the solar markers.
        assert!(svg.contains("<title>Activity across the day</title>"));
        assert!(svg.contains("<desc>Detections for each hour of today, with night shaded and a now marker."), "{svg}");
        assert!(!svg.contains("aria-label"));
        // The chart is the kit's, and does not replay its entrance on a 30-second refresh.
        assert!(svg.contains(r#"data-ck-id="today-hourly""#), "{svg}");
    }

    #[test]
    fn day_strip_without_location_or_weather_still_renders() {
        let mut hourly = [0i64; 24];
        hourly[9] = 3;
        let svg = day_strip(&hourly, &no_species(), &[], None, 10.0);
        assert!(svg.contains("<svg"));
        // No solar facts → no sun lines and no night bands (never fake them).
        assert!(!svg.contains("sunrise"));
        assert!(!svg.contains(r#"<rect class="ck-track""#));
        assert!(!svg.contains("Temperature"), "{svg}");
        // The honest description promises only the markers actually drawn.
        assert!(svg.contains("<desc>Detections for each hour of today, with a now marker."), "{svg}");
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
