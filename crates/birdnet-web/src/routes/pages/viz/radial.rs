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
use crate::routes::pages::atoms::series_color;
use crate::routes::pages::escape_html;

/// (x, y) on a circle centred at (`cx`, `cy`).
fn polar(cx: f64, cy: f64, r: f64, ang: f64) -> (f64, f64) {
    (cx + r * ang.cos(), cy + r * ang.sin())
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

    // The top and bottom 36 units are cropped from the viewBox: the outermost labels
    // sit tangent to the ring, so they need ~40 units beyond it, not the 110 the ring
    // was inset by, and the rest was a band of blank card above and below the chart.
    let mut svg = format!(
        r#"<svg viewBox="0 36 {size:.0} {h:.0}" width="100%" role="img" class="viz-svg-chord">"#,
        h = size - 72.0,
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
            ci = series_color(i, labels.len()),
            cj = series_color(j, labels.len()),
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
            ci = series_color(i, labels.len()),
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
            c = series_color(i, labels.len()),
        );
    }

    // Labels ride the arc (tangent), flipped on the left half.
    for (i, &(_, _, mid, _)) in arcs.iter().enumerate() {
        let label_r = r_outer + 22.0;
        let deg = mid.to_degrees() + 90.0;
        // `deg` is the arc's tangent, which reads upright only in the upper half of
        // the ring (screen y grows downward, so that is angles in (π, 2π)). The old
        // test flipped the left half instead — which turned the labels in the
        // upper-left quadrant upside down, and left the ones in the lower-right
        // quadrant tilted past vertical.
        let flip = mid.rem_euclid(std::f64::consts::TAU) < pi;
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
        // A --surface halo behind the caption: it sits over the ribbons, and grey on a
        // translucent coloured ribbon is not readable.
        r#"<text x="{cx:.1}" y="{y0:.1}" text-anchor="middle" class="display" font-size="15" fill="var(--fg-3)" stroke="var(--surface)" stroke-width="4" paint-order="stroke" stroke-linejoin="round">5-minute</text><text x="{cx:.1}" y="{y1:.1}" text-anchor="middle" class="display" font-size="15" fill="var(--fg-3)" stroke="var(--surface)" stroke-width="4" paint-order="stroke" stroke-linejoin="round">co-occurrence</text>"#,
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

    /// No species name is ever drawn upside down, wherever its arc falls.
    ///
    /// Observed failing against the old flip rule (`mid` in (π/2, 3π/2) or
    /// below -π/2): with ten species the labels in the upper-left quadrant came
    /// out rotated about 90-180 degrees, i.e. reading upside down, and the ones
    /// in the lower-right quadrant were past vertical.
    #[test]
    fn chord_labels_are_never_upside_down() {
        let n = 10;
        let labels: Vec<String> = (0..n).map(|i| format!("Species {i}")).collect();
        // Symmetric, every pair above the 0.20 ribbon threshold, strengths varied so
        // the arcs differ in length and the labels land all the way round the ring.
        let m: Vec<Vec<f64>> = (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| {
                        if i == j {
                            return 0.0;
                        }
                        let k = u8::try_from((i.min(j) * 7 + i.max(j) * 3) % 10).unwrap_or(0);
                        0.25 + f64::from(k) / 14.0
                    })
                    .collect()
            })
            .collect();
        let svg = chord_diagram(&labels, &m);
        let mut checked = 0;
        for chunk in svg.split("rotate(").skip(1) {
            let deg: f64 = chunk
                .split(')')
                .next()
                .and_then(|d| d.parse().ok())
                .expect("rotate() holds a number");
            let norm = (deg + 180.0).rem_euclid(360.0) - 180.0;
            assert!(
                norm.abs() <= 90.0 + 1e-6,
                "a label rotated {deg} degrees (normalised {norm}) reads upside down"
            );
            checked += 1;
        }
        assert_eq!(
            checked, n,
            "one rotate() per species, or the gate is vacuous"
        );
    }
}
