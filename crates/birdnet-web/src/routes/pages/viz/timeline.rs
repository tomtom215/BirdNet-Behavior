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

use super::EMPTY;
use crate::routes::pages::atoms::{species_code, species_color};
use crate::routes::pages::escape_html;

/// Centred stacked-area ("themeriver") of per-day counts. Each series is
/// `(common_name, daily_counts)`; all vectors should share a length.
#[must_use]
pub fn streamgraph(series: &[(String, Vec<i64>)]) -> String {
    let days = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    if series.is_empty() || days < 2 {
        return EMPTY.to_string();
    }

    let w = 760.0_f64;
    let h = 220.0_f64;
    let mid = h / 2.0;
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
    let scale = (h * 0.85) / max_total;
    let val_y = |v: f64| mid - v * scale;

    let mut svg = format!(
        r#"<div class="viz-scroll"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" aria-label="Activity streamgraph" class="viz-svg-block">"#
    );

    // Stack bands from a centred baseline.
    for (name, counts) in series {
        let color = species_color(name);
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
    for (name, _) in series {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta viz-legend-item"><span class="viz-swatch" data-style="background:{c}"></span>{n}</span>"#,
            c = species_color(name),
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
        r#"<svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" aria-label="Species accumulation over time" class="viz-svg-block">"#
    );
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
    for (i, (name, _)) in series.iter().enumerate() {
        let color = species_color(name);
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
        r#"<div class="viz-scroll"><svg width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" role="img" aria-label="Migration phenology ridgeline">{defs}"#
    );

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
        let color = species_color(name);
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

/// Full-width 24-hour timeline for "today": night bands behind, an hourly
/// histogram (moss-soft), one colour-coded dot per detection placed by time
/// (x) and confidence (y), sunrise/sunset markers and a dashed "now" line.
///
/// `hourly` is the per-hour detection count; `dots` is `(hour 0–24, colour,
/// confidence 0–1)`; `sunrise`/`sunset`/`now_h` are hours-of-day (0–24).
#[must_use]
pub fn day_strip(
    hourly: &[i64; 24],
    dots: &[(f64, String, f64)],
    sunrise: f64,
    sunset: f64,
    now_h: f64,
) -> String {
    let max = hourly.iter().copied().max().unwrap_or(0);
    if max == 0 && dots.is_empty() {
        return EMPTY.to_string();
    }
    let w = 960.0_f64;
    let h = 132.0_f64;
    let base = 108.0_f64; // histogram baseline / dot floor
    let bar_ceiling = 40.0_f64; // tallest a bar may reach
    let hw = w / 24.0;
    let max_f = max.max(1) as f64;
    let x_of = |hour: f64| hour / 24.0 * w;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {w:.0} {h:.0}" width="100%" height="auto" role="img" aria-label="Detections across the day" class="viz-svg-block">"#
    );

    // Night bands (midnight→sunrise, sunset→midnight).
    let sunrise_x = x_of(sunrise);
    let sunset_x = x_of(sunset);
    let _ = write!(
        svg,
        r#"<rect x="0" y="0" width="{sunrise_x:.1}" height="{base:.1}" fill="var(--night)" fill-opacity="0.07"/><rect x="{sunset_x:.1}" y="0" width="{rw:.1}" height="{base:.1}" fill="var(--night)" fill-opacity="0.07"/>"#,
        rw = w - sunset_x,
    );

    // Hourly histogram bars.
    for (hour, &c) in hourly.iter().enumerate() {
        if c > 0 {
            let bh = (c as f64 / max_f) * (base - bar_ceiling);
            let x = hour as f64 * hw + 1.5;
            let y = base - bh;
            let _ = write!(
                svg,
                r#"<rect x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" rx="1.5" fill="var(--moss-soft)"/>"#,
                bw = hw - 3.0,
            );
        }
    }

    // Baseline.
    let _ = write!(
        svg,
        r#"<line x1="0" y1="{base:.1}" x2="{w:.0}" y2="{base:.1}" stroke="var(--hairline)" stroke-width="1"/>"#
    );

    // Detection dots — x by time, y by confidence (higher = nearer the top).
    for (hr, color, conf) in dots.iter().take(800) {
        let x = x_of(*hr);
        let y = (base - 6.0) - conf.clamp(0.0, 1.0) * (base - 18.0);
        let _ = write!(
            svg,
            r#"<circle data-species-fill="1" cx="{x:.1}" cy="{y:.1}" r="2.4" fill="{color}" fill-opacity="0.85"/>"#
        );
    }

    // Sunrise / sunset markers.
    for (hh, glyph, col) in [
        (sunrise, "\u{2600}", "var(--dawn-ink)"),
        (sunset, "\u{263e}", "var(--fg-3)"),
    ] {
        let mx = x_of(hh);
        let _ = write!(
            svg,
            r#"<line x1="{mx:.1}" y1="0" x2="{mx:.1}" y2="{base:.1}" stroke="var(--border)" stroke-width="0.75" stroke-dasharray="1 3"/><text x="{mx:.1}" y="13" text-anchor="middle" font-size="13" fill="{col}">{glyph}</text>"#,
        );
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
            ty = h - 4.0,
        );
    }

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamgraph_empty_and_basic() {
        assert!(streamgraph(&[]).contains("Not enough data"));
        let series = vec![
            ("Blue Jay".to_string(), vec![1, 3, 2, 5, 4]),
            ("American Robin".to_string(), vec![0, 2, 4, 3, 1]),
        ];
        let svg = streamgraph(&series);
        assert!(svg.contains("<svg") && svg.contains("<path"));
        assert!(svg.contains("Blue Jay"));
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
    }

    #[test]
    fn day_strip_empty_and_basic() {
        let zero = [0i64; 24];
        assert!(day_strip(&zero, &[], 6.0, 19.0, 12.0).contains("Not enough"));
        let mut hourly = [0i64; 24];
        hourly[6] = 4;
        hourly[7] = 7;
        let dots = vec![
            (6.5, "var(--moss)".to_string(), 0.9),
            (7.25, "var(--dawn)".to_string(), 0.7),
        ];
        let svg = day_strip(&hourly, &dots, 6.0, 19.5, 13.0);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<circle")); // detection dots
        assert!(svg.contains("now")); // current-time pill
    }
}
