//! Polar charts: the dawn-chorus circadian plot and the acoustic co-occurrence
//! chord diagram, plus the small geometry helpers they share.
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
use crate::routes::pages::atoms::species_color;
use crate::routes::pages::escape_html;

/// (x, y) on a circle centred at (`cx`, `cy`).
fn polar(cx: f64, cy: f64, r: f64, ang: f64) -> (f64, f64) {
    (cx + r * ang.cos(), cy + r * ang.sin())
}

/// Hour-of-day (0–24, fractional ok) → angle with midnight at the top.
fn hour_angle(h: f64) -> f64 {
    (h / 24.0) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2
}

/// 24-hour polar plot. Each series is `(common_name, [hourly_value; 24])`.
///
/// Each species occupies its own concentric **ribbon** centred on a baseline
/// circle; the ribbon swells in and out around that baseline where the species
/// is active, normalised to its own daily peak so every rhythm reads clearly.
/// A night wedge, 3-hour ticks, sunrise/sunset markers and a dashed "now" hand
/// orient the reader. `now_h` is the current hour-of-day (0–24); pass a value
/// outside that range to hide the hand.
#[must_use]
pub fn circadian_polar(series: &[(String, [f64; 24])], now_h: f64) -> String {
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
        r#"<svg width="{size:.0}" height="{size:.0}" viewBox="0 0 {size:.0} {size:.0}" role="img" class="viz-svg-center">"#
    );
    svg.push_str(&svg_a11y(
        "Dawn chorus circadian plot",
        "A 24-hour clock face with midnight at the top; each species' ribbon swells at the hours of day it sang most.",
    ));

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
pub fn chord_diagram(labels: &[String], m: &[Vec<f64>]) -> String {
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
        r#"<svg viewBox="0 0 {size:.0} {size:.0}" width="100%" role="img" class="viz-svg-chord">"#
    );
    svg.push_str(&svg_a11y(
        "Acoustic co-occurrence network",
        "Species arranged on a ring; a thicker ribbon joins two species that were heard together in the same five-minute window more often.",
    ));

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

#[cfg(test)]
mod tests {
    use super::*;

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
        // Accessible name + description replace the bare aria-label.
        assert!(svg.contains("<title>Dawn chorus circadian plot</title>"));
        assert!(svg.contains("<desc>A 24-hour clock face"));
        assert!(!svg.contains("aria-label"));
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
        // Accessible name + description replace the bare aria-label.
        assert!(svg.contains("<title>Acoustic co-occurrence network</title>"));
        assert!(svg.contains("<desc>Species arranged on a ring;"));
        assert!(!svg.contains("aria-label"));
    }
}
