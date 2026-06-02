//! Bespoke analytics visualizations rendered as inline SVG.
//!
//! Pure functions, styled by the design tokens in `static/css/app.css`:
//! - `cooccurrence_matrix` — who-sings-with-whom intensity grid
//! - `streamgraph` — centred stacked species activity over a window
//! - `circadian_polar` — the dawn-chorus 24-hour polar plot
//!
//! Colours come from `super::atoms::species_color` so a species keeps the
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

const EMPTY: &str = r#"<p class="bnb-meta viz-empty">Not enough data yet for this view.</p>"#;

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
        r#"<div class="viz-scroll"><svg width="{size_w}" height="{size_h}" viewBox="0 0 {size_w} {size_h}" role="img" aria-label="Species co-occurrence matrix">"#
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
        r#"<svg width="{size:.0}" height="{size:.0}" viewBox="0 0 {size:.0} {size:.0}" role="img" aria-label="Dawn chorus circadian plot" class="viz-svg-center">"#
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
    svg.push_str(r#"<div class="viz-legend center">"#);
    for (name, _) in series {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta viz-legend-item"><span class="viz-swatch round" data-style="background:{c}"></span>{n}</span>"#,
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

// ───────────────────────── acoustic network chord ──────────────────────────

/// Ribbon path: two short arcs on the inner circle (centred on each species'
/// arc midpoint, of `w_i`/`w_j` radians) joined by quadratic curves through
/// the centre — the classic chord "ribbon".
fn chord_ribbon_path(
    cx: f64,
    cy: f64,
    r: f64,
    mid_i: f64,
    w_i: f64,
    mid_j: f64,
    w_j: f64,
) -> String {
    let (xi0, yi0) = polar(cx, cy, r, mid_i - w_i / 2.0);
    let (xi1, yi1) = polar(cx, cy, r, mid_i + w_i / 2.0);
    let (xj0, yj0) = polar(cx, cy, r, mid_j - w_j / 2.0);
    let (xj1, yj1) = polar(cx, cy, r, mid_j + w_j / 2.0);
    format!(
        "M{xi0:.1},{yi0:.1} A{r:.1},{r:.1} 0 0 1 {xi1:.1},{yi1:.1} Q{cx:.1},{cy:.1} {xj0:.1},{yj0:.1} A{r:.1},{r:.1} 0 0 1 {xj1:.1},{yj1:.1} Q{cx:.1},{cy:.1} {xi0:.1},{yi0:.1} Z"
    )
}

/// Annular band between `r_in` and `r_out` spanning the angular range [a0, a1].
fn arc_band_path(cx: f64, cy: f64, r_in: f64, r_out: f64, a0: f64, a1: f64) -> String {
    let large = i32::from(a1 - a0 > std::f64::consts::PI);
    let (xi0, yi0) = polar(cx, cy, r_in, a0);
    let (xi1, yi1) = polar(cx, cy, r_in, a1);
    let (xo0, yo0) = polar(cx, cy, r_out, a0);
    let (xo1, yo1) = polar(cx, cy, r_out, a1);
    format!(
        "M{xi0:.1},{yi0:.1} L{xo0:.1},{yo0:.1} A{r_out:.1},{r_out:.1} 0 {large} 1 {xo1:.1},{yo1:.1} L{xi1:.1},{yi1:.1} A{r_in:.1},{r_in:.1} 0 {large} 0 {xi0:.1},{yi0:.1} Z"
    )
}

/// Acoustic-network chord diagram — the co-occurrence matrix drawn as ribbons.
///
/// Each species gets an outer arc proportional to its total connectedness;
/// ribbons join pairs with strength ≥ 0.20, gradient-filled between the two
/// species' colours, weakest drawn first so strong links sit on top. Labels
/// ride tangent to the arc and flip on the left half so they never read upside
/// down. `m` is the same symmetric 0–1 matrix the matrix view consumes.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn chord_diagram(labels: &[String], m: &[Vec<f64>]) -> String {
    let n = labels.len();
    if n < 2 || m.len() < n {
        return EMPTY.to_string();
    }
    let sums: Vec<f64> = (0..n)
        .map(|i| (0..n).filter(|&j| j != i).map(|j| m[i][j]).sum())
        .collect();
    let total: f64 = sums.iter().sum();
    if total <= 0.0 {
        return EMPTY.to_string();
    }

    let size = 720.0_f64;
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = size / 2.0 - 110.0;
    let r_outer = r + 14.0;
    let pi = std::f64::consts::PI;

    // Arc ranges, proportional to connectedness, starting at the top.
    let mut acc = 0.0_f64;
    let mut arcs: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(n); // (a0, a1, mid, span)
    for &s in &sums {
        let a0 = (acc / total) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
        acc += s;
        let a1 = (acc / total) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
        arcs.push((a0, a1, f64::midpoint(a0, a1), a1 - a0));
    }

    // Upper-triangular pairs above threshold, weakest first.
    let mut ribbons: Vec<(usize, usize, f64)> = Vec::new();
    for (i, row) in m.iter().enumerate().take(n) {
        for (j, &v) in row.iter().enumerate().take(n).skip(i + 1) {
            if v >= 0.20 {
                ribbons.push((i, j, v));
            }
        }
    }
    ribbons.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut svg = format!(
        r#"<svg viewBox="0 0 {size:.0} {size:.0}" width="100%" role="img" aria-label="Acoustic co-occurrence network" class="viz-svg-chord">"#
    );

    // Gradient defs (one per ribbon, oriented midpoint → midpoint).
    svg.push_str("<defs>");
    for (idx, &(i, j, _)) in ribbons.iter().enumerate() {
        let (x1, y1) = polar(cx, cy, r, arcs[i].2);
        let (x2, y2) = polar(cx, cy, r, arcs[j].2);
        let _ = write!(
            svg,
            r#"<linearGradient id="chord-{idx}" gradientUnits="userSpaceOnUse" x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}"><stop offset="0%" stop-color="{ci}"/><stop offset="100%" stop-color="{cj}"/></linearGradient>"#,
            ci = species_color(&labels[i]),
            cj = species_color(&labels[j]),
        );
    }
    svg.push_str("</defs>");

    // Ribbons.
    for (idx, &(i, j, v)) in ribbons.iter().enumerate() {
        let w_i = if sums[i] > 0.0 {
            (v / sums[i]) * arcs[i].3
        } else {
            0.0
        };
        let w_j = if sums[j] > 0.0 {
            (v / sums[j]) * arcs[j].3
        } else {
            0.0
        };
        let path = chord_ribbon_path(cx, cy, r, arcs[i].2, w_i, arcs[j].2, w_j);
        let op = v.mul_add(0.40, 0.45);
        let _ = write!(
            svg,
            r#"<path class="chord-ribbon" data-species-fill="1" d="{path}" fill="url(#chord-{idx})" fill-opacity="{op:.2}" stroke="{ci}" stroke-opacity="0.5" stroke-width="0.7"><title>{a} × {b} — {pct}%</title></path>"#,
            ci = species_color(&labels[i]),
            a = escape_html(&labels[i]),
            b = escape_html(&labels[j]),
            pct = (v * 100.0).round() as i64,
        );
    }

    // Outer species arcs.
    for (i, &(a0, a1, _, _)) in arcs.iter().enumerate() {
        let path = arc_band_path(cx, cy, r + 3.0, r_outer, a0 + 0.005, a1 - 0.005);
        let _ = write!(
            svg,
            r#"<path data-species-fill="1" d="{path}" fill="{c}" opacity="0.92"/>"#,
            c = species_color(&labels[i]),
        );
    }

    // Labels ride the arc (tangent), flipped on the left half.
    for (i, &(_, _, mid, _)) in arcs.iter().enumerate() {
        let label_r = r_outer + 22.0;
        let deg = mid.to_degrees() + 90.0;
        let flip = (mid > pi / 2.0 && mid < pi * 1.5) || (mid < -pi / 2.0);
        let final_deg = if flip { deg + 180.0 } else { deg };
        let radius = if flip { label_r + 8.0 } else { label_r };
        let (tx, ty) = polar(cx, cy, radius, mid);
        let rho_bar = sums[i] / (n as f64 - 1.0).max(1.0);
        let _ = write!(
            svg,
            r#"<g transform="translate({tx:.1},{ty:.1}) rotate({final_deg:.1})"><text text-anchor="middle" font-size="12" font-weight="500" fill="var(--fg)">{name}</text><text class="mono" y="12" text-anchor="middle" font-size="9.5" fill="var(--fg-3)">ρ̄ {rho_bar:.2}</text></g>"#,
            name = escape_html(&labels[i]),
        );
    }

    // Centre caption.
    let _ = write!(
        svg,
        r#"<text x="{cx:.1}" y="{y0:.1}" text-anchor="middle" class="display" font-size="15" fill="var(--fg-3)">5-minute</text><text x="{cx:.1}" y="{y1:.1}" text-anchor="middle" class="display" font-size="15" fill="var(--fg-3)">co-occurrence</text>"#,
        y0 = cy - 6.0,
        y1 = cy + 12.0,
    );

    svg.push_str("</svg>");
    svg
}

// ─────────────────────────────── day strip ─────────────────────────────────

/// Full-width 24-hour timeline for "today": night bands behind, an hourly
/// histogram (moss-soft), one colour-coded dot per detection placed by time
/// (x) and confidence (y), sunrise/sunset markers and a dashed "now" line.
///
/// `hourly` is the per-hour detection count; `dots` is `(hour 0–24, colour,
/// confidence 0–1)`; `sunrise`/`sunset`/`now_h` are hours-of-day (0–24).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn day_strip(
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

    #[test]
    fn chord_empty_and_basic() {
        assert!(chord_diagram(&[], &[]).contains("Not enough data"));
        // All-zero matrix has no connectedness → empty.
        let labels = vec!["Blue Jay".to_string(), "Northern Cardinal".to_string()];
        assert!(chord_diagram(&labels, &[vec![0.0, 0.0], vec![0.0, 0.0]]).contains("Not enough"));
        // A real link renders ribbons + arcs + the centre caption.
        let m = vec![vec![0.0, 0.8], vec![0.8, 0.0]];
        let svg = chord_diagram(&labels, &m);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("chord-ribbon"));
        assert!(svg.contains("co-occurrence"));
        assert!(svg.contains("Blue Jay"));
    }
}
