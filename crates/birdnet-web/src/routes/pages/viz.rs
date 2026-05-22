//! Bespoke analytics visualizations rendered as inline SVG.
//!
//! Pure functions, styled by the design tokens in `static/css/app.css`:
//! - [`cooccurrence_matrix`] — who-sings-with-whom intensity grid
//! - [`streamgraph`] — centred stacked species activity over a window
//! - [`circadian_polar`] — the dawn-chorus 24-hour polar plot
//!
//! Colours come from [`super::atoms::species_color`] so a species keeps the
//! same hue across every screen.
//!
//! These are geometry-heavy SVG generators; the lint allows below cover the
//! pervasive, benign `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::atoms::{species_code, species_color};
use super::escape_html;

const EMPTY: &str = r#"<p class="bnb-meta" style="text-align:center;padding:1.5rem;">Not enough data yet for this view.</p>"#;

/// (x, y) on a circle centred at (`cx`, `cy`).
fn polar(cx: f64, cy: f64, r: f64, ang: f64) -> (f64, f64) {
    (cx + r * ang.cos(), cy + r * ang.sin())
}

/// Hour-of-day (0–24, fractional ok) → angle with midnight at the top.
fn hour_angle(h: f64) -> f64 {
    (h / 24.0) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2
}

// ───────────────────────────── co-occurrence matrix ────────────────────────

/// Symmetric N×N intensity grid. `m[i][j]` is a normalised 0–1 strength;
/// the diagonal is drawn as a neutral cell.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn cooccurrence_matrix(labels: &[String], m: &[Vec<f64>]) -> String {
    let n = labels.len();
    if n < 2 {
        return EMPTY.to_string();
    }
    let cell = 30_i32;
    let gutter = 92_i32; // room for row labels (left) and rotated col labels (top)
    let size_w = gutter + n as i32 * cell + 8;
    let size_h = gutter + n as i32 * cell + 8;

    let mut svg = format!(
        r#"<div style="overflow-x:auto;"><svg width="{size_w}" height="{size_h}" viewBox="0 0 {size_w} {size_h}" role="img" aria-label="Species co-occurrence matrix">"#
    );

    // Column labels (rotated) + row labels.
    for (i, name) in labels.iter().enumerate() {
        let code = species_code(name);
        let title = escape_html(name);
        let x = gutter + i as i32 * cell + cell / 2;
        let y = gutter + i as i32 * cell + cell / 2;
        // column header, rotated -45°
        let _ = write!(
            svg,
            r#"<text class="mono" x="{x}" y="{cy}" transform="rotate(-45 {x} {cy})" text-anchor="start" font-size="9" fill="var(--fg-3)"><title>{title}</title>{code}</text>"#,
            cy = gutter - 6,
        );
        // row label
        let _ = write!(
            svg,
            r#"<text class="mono" x="{lx}" y="{y}" text-anchor="end" dominant-baseline="middle" font-size="9" fill="var(--fg-3)"><title>{title}</title>{code}</text>"#,
            lx = gutter - 8,
        );
    }

    // Cells.
    for (i, row) in m.iter().enumerate().take(n) {
        for (j, &v) in row.iter().enumerate().take(n) {
            let x = gutter + j as i32 * cell;
            let y = gutter + i as i32 * cell;
            if i == j {
                let _ = write!(
                    svg,
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="3" fill="var(--surface-2)"/>"#,
                    w = cell - 3,
                );
                continue;
            }
            let op = 0.06 + v.clamp(0.0, 1.0) * 0.9;
            let a = escape_html(&labels[i]);
            let b = escape_html(&labels[j]);
            let pct = (v * 100.0).round() as i64;
            let _ = write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="3" fill="var(--moss)" fill-opacity="{op:.2}"><title>{a} × {b} — {pct}%</title></rect>"#,
                w = cell - 3,
            );
        }
    }

    svg.push_str("</svg></div>");
    svg
}

// ──────────────────────────────── streamgraph ──────────────────────────────

/// Centred stacked-area ("themeriver") of per-day counts. Each series is
/// `(common_name, daily_counts)`; all vectors should share a length.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn streamgraph(series: &[(String, Vec<i64>)]) -> String {
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
        r#"<div style="overflow-x:auto;"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" aria-label="Activity streamgraph" style="display:block;">"#
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
    svg.push_str(r#"<div style="display:flex;gap:12px;flex-wrap:wrap;margin-top:10px;">"#);
    for (name, _) in series {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta" style="display:inline-flex;align-items:center;gap:5px;"><span style="width:9px;height:9px;border-radius:2px;background:{c};display:inline-block;"></span>{n}</span>"#,
            c = species_color(name),
            n = escape_html(name),
        );
    }
    svg.push_str("</div>");
    svg
}

// ───────────────────────── species accumulation curve ──────────────────────

/// Monotonic life-list accumulation: `points` is `(label, cumulative_total)`
/// in chronological order. Renders a filled step-up area with end-value label.
#[must_use]
pub(crate) fn accumulation_curve(points: &[(String, i64)]) -> String {
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
        r#"<svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" aria-label="Species accumulation over time" style="display:block;">"#
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

// ─────────────────────────── dawn-chorus polar ─────────────────────────────

/// 24-hour polar plot. Each series is `(common_name, [hourly_value; 24])`.
///
/// Each species occupies its own concentric **ribbon** centred on a baseline
/// circle; the ribbon swells in and out around that baseline where the species
/// is active, normalised to its own daily peak so every rhythm reads clearly.
/// A night wedge, 3-hour ticks, sunrise/sunset markers and a dashed "now" hand
/// orient the reader. `now_h` is the current hour-of-day (0–24); pass a value
/// outside that range to hide the hand.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn circadian_polar(series: &[(String, [f64; 24])], now_h: f64) -> String {
    if series.is_empty() {
        return EMPTY.to_string();
    }
    let n = series.len();
    let size = 440.0_f64;
    let cx = size / 2.0;
    let cy = size / 2.0;
    let ir = 46.0_f64;
    let or = size / 2.0 - 38.0;
    let band = (or - ir) / n as f64;
    let amp = band * 0.42;

    let mut svg = format!(
        r#"<svg width="{size:.0}" height="{size:.0}" viewBox="0 0 {size:.0} {size:.0}" role="img" aria-label="Dawn chorus circadian plot" style="max-width:100%;height:auto;display:block;margin:0 auto;">"#
    );

    // Night wedge (≈20:00 → 05:00, wrapping through midnight at the top).
    {
        let mut wedge = format!("M{cx:.1},{cy:.1} ");
        // 20:00 → 29:00 (i.e. 05:00 next day) in half-hour steps, as integers.
        for k in 40..=58 {
            let h = f64::from(k) * 0.5;
            let (x, y) = polar(cx, cy, or, hour_angle(h));
            let _ = write!(wedge, "L{x:.1},{y:.1} ");
        }
        wedge.push('Z');
        let _ = write!(
            svg,
            r#"<path d="{wedge}" fill="var(--night)" fill-opacity="0.14"/>"#
        );
    }

    // Reference rings.
    for r in [ir, or] {
        let _ = write!(
            svg,
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" fill="none" stroke="var(--hairline)" stroke-width="0.75"/>"#,
        );
    }

    // 3-hour spokes + labels.
    let labels = ["12a", "3a", "6a", "9a", "12p", "3p", "6p", "9p"];
    for (i, lbl) in labels.iter().enumerate() {
        let h = i as f64 * 3.0;
        let (sx, sy) = polar(cx, cy, ir, hour_angle(h));
        let (ex, ey) = polar(cx, cy, or, hour_angle(h));
        let _ = write!(
            svg,
            r#"<line x1="{sx:.1}" y1="{sy:.1}" x2="{ex:.1}" y2="{ey:.1}" stroke="var(--hairline)" stroke-width="0.5"/>"#,
        );
        let (lx, ly) = polar(cx, cy, or + 14.0, hour_angle(h));
        let _ = write!(
            svg,
            r#"<text class="mono" x="{lx:.1}" y="{ly:.1}" text-anchor="middle" dominant-baseline="middle" font-size="10" fill="var(--fg-4)">{lbl}</text>"#,
        );
    }

    // Sunrise (☀ ~06:00) and sunset (☾ ~19:00) markers on the outer ring.
    for (h, glyph, col) in [
        (6.0, "\u{2600}", "var(--dawn-ink)"),
        (19.0, "\u{263e}", "var(--fg-3)"),
    ] {
        let (mx, my) = polar(cx, cy, or, hour_angle(h));
        let _ = write!(
            svg,
            r#"<text x="{mx:.1}" y="{my:.1}" text-anchor="middle" dominant-baseline="central" font-size="13" fill="{col}">{glyph}</text>"#,
        );
    }

    // Per-species concentric ribbons (outer rows first so inner draw on top).
    for (i, (name, hours)) in series.iter().enumerate() {
        let color = species_color(name);
        let baseline = ir + (i as f64 + 0.5) * band;
        let row_max = hours.iter().copied().fold(0.0_f64, f64::max).max(1.0);

        // Outer edge (baseline + activity), 0..24 inclusive to close the loop.
        let mut path = String::new();
        for k in 0..=24 {
            let h = k as f64;
            let v = hours[k % 24];
            let (x, y) = polar(cx, cy, baseline + (v / row_max) * amp, hour_angle(h));
            let _ = write!(path, "{}{x:.1},{y:.1} ", if k == 0 { "M" } else { "L" });
        }
        // Inner edge (baseline - activity), traced back.
        for k in (0..=24).rev() {
            let h = k as f64;
            let v = hours[k % 24];
            let (x, y) = polar(cx, cy, baseline - (v / row_max) * amp, hour_angle(h));
            let _ = write!(path, "L{x:.1},{y:.1} ");
        }
        path.push('Z');
        let _ = write!(
            svg,
            r#"<path data-species-fill="1" d="{path}" fill="{color}" fill-opacity="0.55" stroke="{color}" stroke-width="1" stroke-opacity="0.9"><title>{n}</title></path>"#,
            n = escape_html(name),
        );
        // Faint baseline circle for the species' "silent" radius.
        let _ = write!(
            svg,
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{baseline:.1}" fill="none" stroke="{color}" stroke-width="0.5" stroke-opacity="0.25"/>"#,
        );
    }

    // Current-time hand (dashed) when a valid hour is supplied.
    if (0.0..24.0).contains(&now_h) {
        let (hx, hy) = polar(cx, cy, or, hour_angle(now_h));
        let _ = write!(
            svg,
            r#"<line x1="{cx:.1}" y1="{cy:.1}" x2="{hx:.1}" y2="{hy:.1}" stroke="var(--fg-2)" stroke-width="1.25" stroke-dasharray="2 3"/><circle cx="{cx:.1}" cy="{cy:.1}" r="2.5" fill="var(--fg-2)"/>"#,
        );
    }

    svg.push_str("</svg>");
    // Legend.
    svg.push_str(r#"<div style="display:flex;gap:12px;flex-wrap:wrap;justify-content:center;margin-top:8px;">"#);
    for (name, _) in series {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta" style="display:inline-flex;align-items:center;gap:5px;"><span style="width:9px;height:9px;border-radius:50%;background:{c};display:inline-block;"></span>{n}</span>"#,
            c = species_color(name),
            n = escape_html(name),
        );
    }
    svg.push_str("</div>");
    svg
}

// ───────────────────────── migration ridgeline ─────────────────────────────

/// Joyplot of per-species seasonal abundance. Each series is
/// `(common_name, weekly_counts)`; rows are stacked with overlap and each
/// ridge is normalised to its own peak so arrival/departure *timing* reads
/// clearly. Month ticks run along the bottom.
#[must_use]
pub(crate) fn ridgeline(series: &[(String, Vec<i64>)]) -> String {
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
        r#"<div style="overflow-x:auto;"><svg width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" role="img" aria-label="Migration phenology ridgeline">{defs}"#
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_empty_for_small_input() {
        assert!(cooccurrence_matrix(&[], &[]).contains("Not enough data"));
        assert!(cooccurrence_matrix(&["A".into()], &[vec![1.0]]).contains("Not enough data"));
    }

    #[test]
    fn matrix_renders_cells_and_labels() {
        let labels = vec!["Blue Jay".to_string(), "Northern Cardinal".to_string()];
        let m = vec![vec![1.0, 0.5], vec![0.5, 1.0]];
        let svg = cooccurrence_matrix(&labels, &m);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("BLJA"));
        assert!(svg.contains("NOCA"));
        assert!(svg.contains("fill-opacity"));
    }

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
    fn polar_empty_and_basic() {
        assert!(circadian_polar(&[], 6.0).contains("Not enough data"));
        let mut h = [0.0_f64; 24];
        h[6] = 5.0;
        h[7] = 8.0;
        let svg = circadian_polar(&[("Northern Cardinal".to_string(), h)], 6.5);
        assert!(svg.contains("<svg") && svg.contains("12a") && svg.contains("6a"));
        // The dashed current-time hand renders for an in-range hour.
        assert!(svg.contains("stroke-dasharray"));
    }
}
