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
use crate::routes::pages::atoms::{series_color, species_code};
use crate::routes::pages::escape_html;

/// Shift an ISO `YYYY-MM-DD` date by `days`, returning `MM-DD` for an axis
/// label.
///
/// Uses the workspace's own civil-date arithmetic rather than a date crate —
/// this workspace carries none — and returns the input's tail unchanged if it
/// does not parse, so a malformed date degrades to a wrong-looking label rather
/// than a panic in a render path (`panic = "abort"`, so a handler panic is a
/// station outage).
fn shift_iso_date(end: &str, days: i64) -> String {
    let parsed = birdnet_core::civil::parse_civil(end, "00:00:00");
    let Some(t) = parsed else {
        return end.chars().skip(5).collect();
    };
    let secs = birdnet_core::civil::unix_secs_from_civil(&t) + days * 86_400;
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    format!("{:02}-{:02}", c.month, c.day)
}

/// Centred stacked-area ("themeriver") of per-day counts. Each series is
/// `(common_name, daily_counts)`; all vectors should share a length.
///
/// `end_date` is the calendar date of the **last** column, used to label the
/// time axis. `None` omits the date labels but still draws the scale.
///
/// # Why this draws axes at all
///
/// It did not, and the omission mattered more than it looks. With no dates
/// along x and no counts along y, a reader could not tell what period was shown
/// or whether a band meant five detections or five hundred — only the shape was
/// legible, and with near-flat data the shape is a stack of stripes. A chart
/// without a scale is decoration; this is meant to be read.
#[must_use]
pub fn streamgraph(series: &[(String, Vec<i64>)], end_date: Option<&str>) -> String {
    let days = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    if series.is_empty() || days < 2 {
        return EMPTY.to_string();
    }

    let w = 760.0_f64;
    // Room under the plot for the date axis.
    let plot_h = 220.0_f64;
    let h = plot_h + 20.0;
    let mid = plot_h / 2.0;
    let step = w / (days - 1) as f64;

    // Per-day total → vertical scale so the fattest day fills ~85% of height.
    let mut max_total = 1.0_f64;
    for d in 0..days {
        let total: i64 = series
            .iter()
            .map(|(_, v)| v.get(d).copied().unwrap_or(0))
            .sum();
        max_total = max_total.max(total as f64);
    }
    let scale = (plot_h * 0.85) / max_total;
    let val_y = |v: f64| mid - v * scale;

    let mut svg = format!(
        r#"<div class="viz-scroll"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" class="viz-svg-block">"#
    );
    svg.push_str(&svg_a11y(
        "Activity streamgraph",
        "Detections per day over time as a centred stack; each coloured band is one species and its thickness is that day's count.",
    ));

    // Scale. Without these the chart said nothing about magnitude or period.
    // A centred stack is symmetric about `mid`, so the readable quantity is the
    // total thickness at the fattest day: mark that, and the midline.
    let _ = write!(
        svg,
        r#"<line x1="0" y1="{mid:.1}" x2="{w:.0}" y2="{mid:.1}" stroke="var(--hairline)" stroke-width="0.5" stroke-dasharray="2 3"/>"#
    );
    let peak_top = val_y(max_total / 2.0);
    let _ = write!(
        svg,
        r#"<line x1="0" y1="{peak_top:.1}" x2="{w:.0}" y2="{peak_top:.1}" stroke="var(--hairline)" stroke-width="0.5"/><text class="mono" x="4" y="{ly:.1}" font-size="9" fill="var(--fg-4)">{peak:.0}/day peak</text>"#,
        ly = peak_top - 3.0,
        peak = max_total,
    );

    // Time axis: first, middle and last column, so the period is unambiguous.
    if let Some(end) = end_date {
        for (frac, anchor) in [(0.0_f64, "start"), (0.5, "middle"), (1.0, "end")] {
            let back = ((1.0 - frac) * (days - 1) as f64).round() as i64;
            let label = shift_iso_date(end, -back);
            let x = frac * w;
            let _ = write!(
                svg,
                r#"<text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="{anchor}" font-size="9" fill="var(--fg-4)">{label}</text>"#,
                ty = h - 4.0,
            );
        }
    }

    // Stack bands from a centred baseline.
    //
    // `series_color(rank, total)`, not `species_color(name)`: several species
    // are shown together here, and the hash palette put pairs of them 2–3° apart
    // in hue at constant lightness — indistinguishable. See `atoms`.
    for (rank, (name, counts)) in series.iter().enumerate() {
        let color = series_color(rank, series.len());
        let mut top = String::new();
        let mut bottom: Vec<(f64, f64)> = Vec::with_capacity(days);
        for d in 0..days {
            let total: f64 = series
                .iter()
                .map(|(_, v)| v.get(d).copied().unwrap_or(0))
                .sum::<i64>() as f64;
            let below: f64 = series
                .iter()
                .take_while(|(n2, _)| n2 != name)
                .map(|(_, v)| v.get(d).copied().unwrap_or(0))
                .sum::<i64>() as f64;
            let val = counts.get(d).copied().unwrap_or(0) as f64;
            let lower = below - total / 2.0;
            let upper = lower + val;
            let x = d as f64 * step;
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
        let _ = write!(
            svg,
            r#"<path d="{path}" fill="{color}" fill-opacity="0.82" stroke="{color}" stroke-width="0.5" stroke-opacity="0.5"><title>{n}</title></path>"#,
            n = escape_html(name),
        );
    }

    svg.push_str("</svg></div>");
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
    let pad_t = 12.0;
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
        r#"<path d="{area}" fill="var(--moss)" fill-opacity="0.12"/><path d="{line}" fill="none" stroke="var(--moss)" stroke-width="1.6"/>"#,
    );
    let (lx, ly) = (x_at(n - 1), y_at(points[n - 1].1));
    let _ = write!(
        svg,
        r#"<circle cx="{lx:.1}" cy="{ly:.1}" r="2.6" fill="var(--moss)"/><text class="mono" x="{tx:.1}" y="{ty:.1}" text-anchor="end" font-size="11" fill="var(--moss-ink)">{total} species</text>"#,
        tx = lx - 4.0,
        ty = (ly - 6.0).max(pad_t + 8.0),
        total = points[n - 1].1,
    );
    let stride = (n / 6).max(1);
    for (i, (label, _)) in points.iter().enumerate() {
        if i % stride == 0 || i == n - 1 {
            let _ = write!(
                svg,
                r#"<text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="middle" font-size="8" fill="var(--fg-4)">{label}</text>"#,
                x = x_at(i),
                ty = h - 6.0,
            );
        }
    }
    svg.push_str("</svg>");
    svg
}

/// Joyplot of per-species seasonal abundance. Each series is
/// `(common_name, weekly_counts)`; rows are stacked with overlap and each
/// ridge is normalised to its own peak so arrival/departure *timing* reads
/// clearly. Month ticks run along the bottom.
#[must_use]
pub fn ridgeline(series: &[(String, Vec<i64>)]) -> String {
    const MONTHS: [&str; 12] = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
    let weeks = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    if series.is_empty() || weeks < 2 {
        return EMPTY.to_string();
    }
    let n = series.len();
    let w = 760.0_f64;
    let left = 96.0_f64;
    let right = 10.0_f64;
    let plot_w = w - left - right;
    let row_step = 30.0_f64;
    let amp = 54.0_f64;
    let top = amp;
    let bottom = 22.0_f64;
    let h = top + n as f64 * row_step + bottom;
    let step_x = plot_w / (weeks - 1) as f64;
    let wk_x = |wk: f64| left + wk * step_x;

    // Per-species vertical gradients: saturated at the crest, fading to baseline.
    let mut defs = String::from("<defs>");
    for (i, (_name, _)) in series.iter().enumerate() {
        let color = series_color(i, series.len());
        let baseline = top + (i + 1) as f64 * row_step;
        let _ = write!(
            defs,
            r#"<linearGradient id="ridge-{i}" gradientUnits="userSpaceOnUse" x1="0" y1="{y0:.1}" x2="0" y2="{y1:.1}"><stop offset="0%" stop-color="{color}" stop-opacity="0.78"/><stop offset="100%" stop-color="{color}" stop-opacity="0.07"/></linearGradient>"#,
            y0 = baseline - amp,
            y1 = baseline,
        );
    }
    defs.push_str("</defs>");

    let mut svg = format!(
        r#"<div class="viz-scroll"><svg width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" role="img">"#
    );
    svg.push_str(&svg_a11y(
        "Migration phenology ridgeline",
        "One ridge per species across the year; each ridge peaks in the weeks that species was most abundant, with spring and fall migration bands behind.",
    ));
    svg.push_str(&defs);

    // Spring (~weeks 12–21) and fall (~weeks 30–43) migration bands, behind.
    let yb = h - bottom;
    for (w0, w1, tok, lbl) in [
        (12.0, 21.0, "var(--moss)", "spring"),
        (30.0, 43.0, "var(--dawn)", "fall"),
    ] {
        let x = wk_x(w0);
        let bw = wk_x(w1) - x;
        let _ = write!(
            svg,
            r#"<rect x="{x:.1}" y="{top:.1}" width="{bw:.1}" height="{bh:.1}" fill="{tok}" fill-opacity="0.06"/><text class="bnb-eyebrow" x="{tx:.1}" y="{top:.1}" font-size="8" fill="var(--fg-4)">{lbl}</text>"#,
            bh = yb - top,
            tx = x + 3.0,
        );
    }

    // Month ticks + faint guides.
    for (m, label) in MONTHS.iter().enumerate() {
        let x = left + (m as f64 / 12.0) * plot_w;
        let _ = write!(
            svg,
            r#"<line x1="{x:.1}" y1="{top:.1}" x2="{x:.1}" y2="{yb:.1}" stroke="var(--hairline)" stroke-width="0.5"/><text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="middle" font-size="8" fill="var(--fg-4)">{label}</text>"#,
            ty = h - 6.0,
        );
    }

    // Ridges, top row first so lower rows overlap in front.
    for (i, (name, vals)) in series.iter().enumerate() {
        let row_max = vals.iter().copied().max().unwrap_or(1).max(1) as f64;
        let baseline = top + (i + 1) as f64 * row_step;
        let color = series_color(i, series.len());
        let mut path = String::new();
        for (wk, &v) in vals.iter().enumerate() {
            let x = wk_x(wk as f64);
            let y = baseline - (v as f64 / row_max) * amp;
            let _ = write!(path, "{}{x:.1},{y:.1} ", if wk == 0 { "M" } else { "L" });
        }
        let area = format!(
            "{path}L{xe:.1},{baseline:.1} L{x0:.1},{baseline:.1} Z",
            xe = wk_x((vals.len() - 1) as f64),
            x0 = left,
        );
        let _ = write!(
            svg,
            r#"<path data-species-fill="1" d="{area}" fill="url(#ridge-{i})" stroke="{color}" stroke-width="1.4" stroke-opacity="0.95"/><text class="mono" x="{lx:.1}" y="{ly:.1}" text-anchor="end" dominant-baseline="middle" font-size="9" fill="var(--fg-3)"><title>{full}</title>{code}</text>"#,
            lx = left - 8.0,
            ly = baseline - 4.0,
            full = escape_html(name),
            code = species_code(name),
        );
    }

    svg.push_str("</svg></div>");
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
                r#"<rect x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" rx="2" fill="var(--moss)" fill-opacity="0.6"/>"#,
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
            r#"<path d="{path}" fill="none" stroke="var(--dawn-ink)" stroke-width="1.5" stroke-opacity="0.8" stroke-linecap="round" stroke-linejoin="round"/>"#
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
                r#"<circle cx="{nx:.1}" cy="{ny:.1}" r="2.6" fill="var(--dawn-ink)"/><text x="{tx:.1}" y="{ty:.1}" font-size="10" fill="var(--dawn-ink)" stroke="var(--surface)" stroke-width="3.5" paint-order="stroke" stroke-linejoin="round" class="mono">{temp:.0}°</text>"#,
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
        let _ = write!(
            svg,
            r#"<text class="mono" x="{x:.1}" y="{ty:.1}" text-anchor="middle" font-size="9" fill="var(--fg-4)">{hour:02}</text>"#,
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
        assert!(!svg.contains("aria-label"));

        // Axes. Without them the chart carried no scale at all: a reader could
        // not tell the period shown or whether a band meant five detections or
        // five hundred.
        assert!(
            svg.contains("/day peak"),
            "the vertical scale must be labelled: {svg}"
        );
        assert!(
            svg.contains("08-17"),
            "the last column must carry its date: {svg}"
        );
        assert!(
            svg.contains("08-13"),
            "five days ending 08-17 start on 08-13: {svg}"
        );
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
            svg.contains("02-26"),
            "five days ending 2026-03-02 start on 02-26: {svg}"
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

    #[test]
    fn ridgeline_empty_and_basic() {
        assert!(ridgeline(&[]).contains("Not enough data"));
        let s = vec![
            ("Blue Jay".to_string(), vec![0, 1, 3, 5, 2, 0]),
            ("Magnolia Warbler".to_string(), vec![0, 0, 0, 2, 6, 1]),
        ];
        let svg = ridgeline(&s);
        assert!(svg.contains("<svg") && svg.contains("BLJA"));
        assert!(svg.contains("<title>Migration phenology ridgeline</title>"));
        assert!(svg.contains("<desc>One ridge per species"));
        assert!(!svg.contains("aria-label"));
        // <title>/<desc> precede <defs> so they are the SVG's first children.
        let svg_open = svg.find("<svg").unwrap();
        assert!(svg[svg_open..].find("<title>") < svg[svg_open..].find("<defs>"));
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
}
