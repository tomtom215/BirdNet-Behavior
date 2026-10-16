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

/// Width of one character of a ring label, in SVG units: the 12-unit face
/// measures about 6.4 on a name's average, and a label must not be drawn narrower
/// than it really is, so this is a little over.
const LABEL_CHAR_WIDTH: f64 = 6.8;

/// Clear space kept between two neighbouring names along the ring.
const LABEL_GAP: f64 = 8.0;

/// Passes `spread_labels` makes at most. Pushing apart in pairs settles a crowded
/// stretch gradually, like heat spreading along a rod: the test's ten names, with
/// four of them jammed together, settle in 66 passes. A pass is a handful of
/// additions, so the bound is generous; it is there to end the loop when the names
/// are longer than the ring, where no arrangement leaves every pair its room.
const MAX_SPREAD_PASSES: usize = 1_000;

/// Where each name goes on the ring: its arc's middle, moved only as far as it
/// takes for neighbouring names not to overlap.
///
/// A species heard with few others has a short arc, and a name written along a
/// short arc reaches into the next one's. `mids` are the arc middles in ring order
/// and `half_widths` the half-length of each name along the ring. `radius` is how
/// far from the centre the name's inner edge is: a name is a straight run of text
/// standing on the ring's tangent, so what limits it is where its inner corners
/// are, `atan(half / radius)` either side of its middle rather than
/// `half / radius`, and the line under the name makes that edge the nearer one.
///
/// Each pair that is too close is pushed apart by half the shortfall each, repeated
/// until none is, so a crowded stretch spreads out around its own middle instead of
/// marching round the ring. When the names together are longer than the ring there
/// is no answer, and the bound on the passes ends the loop with the names spread as
/// evenly as the passes got them.
fn spread_labels(mids: &[f64], half_widths: &[f64], radius: f64) -> Vec<f64> {
    let n = mids.len();
    let mut at = mids.to_vec();
    let gap = LABEL_GAP / radius;
    for _ in 0..MAX_SPREAD_PASSES {
        let mut moved = false;
        for i in 0..n {
            let j = (i + 1) % n;
            let want = (half_widths[i] / radius).atan() + (half_widths[j] / radius).atan() + gap;
            let have = at[j] - at[i] + if j == 0 { std::f64::consts::TAU } else { 0.0 };
            if have < want - 1e-9 {
                let push = (want - have) / 2.0;
                at[i] -= push;
                at[j] += push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    at
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

    // Labels ride the arc (tangent), flipped on the left half, each moved along
    // the ring only as far as its neighbours' names need (see `spread_labels`).
    let half_widths: Vec<f64> = labels
        .iter()
        .map(|l| l.chars().count() as f64 * LABEL_CHAR_WIDTH / 2.0)
        .collect();
    let mids: Vec<f64> = arcs.iter().map(|a| a.2).collect();
    // The inner edge of a name: its anchor is at `r_outer + 22`, and the line under
    // it reaches 14 units further in.
    let label_angles = spread_labels(&mids, &half_widths, r_outer + 22.0 - 14.0);
    for (i, &mid) in label_angles.iter().enumerate() {
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

    /// One drawn label: where it sits, how it is turned, and what it says.
    struct Placed {
        x: f64,
        y: f64,
        deg: f64,
        name: String,
    }

    /// The ring's labels, read back out of the SVG.
    fn placed_labels(svg: &str) -> Vec<Placed> {
        svg.split(r#"<g transform="translate("#)
            .skip(1)
            .map(|chunk| {
                let (xy, rest) = chunk
                    .split_once(") rotate(")
                    .expect("translate then rotate");
                let (x, y) = xy.split_once(',').expect("x,y");
                let (deg, rest) = rest.split_once(")\">").expect("rotate then the text");
                let name = rest
                    .split_once(r#"fill="var(--fg)">"#)
                    .and_then(|(_, t)| t.split_once("</text>"))
                    .expect("the name")
                    .0;
                Placed {
                    x: x.parse().expect("x"),
                    y: y.parse().expect("y"),
                    deg: deg.parse().expect("deg"),
                    name: name.to_owned(),
                }
            })
            .collect()
    }

    /// Whether two labels, each a turned rectangle, share any area. The width is
    /// the text's width at 12 px (a generous 6.8 px a character, wider than the
    /// face really is) and the height covers the name and the line under it.
    fn labels_overlap(a: &Placed, b: &Placed) -> bool {
        /// A label's box: its middle, the two unit directions of its sides, and
        /// the half-length along the first and the half-height along the second.
        struct Turned {
            middle: (f64, f64),
            along: (f64, f64),
            across: (f64, f64),
            half_len: f64,
            half_height: f64,
        }
        let turned = |l: &Placed| {
            let t = l.deg.to_radians();
            let (along, across) = ((t.cos(), t.sin()), (-t.sin(), t.cos()));
            Turned {
                // Local (0, 2) is the middle of the name line and the one beneath it.
                middle: (l.x + 2.0 * across.0, l.y + 2.0 * across.1),
                along,
                across,
                half_len: l.name.chars().count() as f64 * 6.8 / 2.0,
                half_height: 12.0,
            }
        };
        let (ra, rb) = (turned(a), turned(b));
        let gap = (rb.middle.0 - ra.middle.0, rb.middle.1 - ra.middle.1);
        [ra.along, ra.across, rb.along, rb.across]
            .iter()
            .all(|axis| {
                // How far the box reaches along `axis` from its middle.
                let reach = |r: &Turned| {
                    (r.along.0 * axis.0 + r.along.1 * axis.1).abs() * r.half_len
                        + (r.across.0 * axis.0 + r.across.1 * axis.1).abs() * r.half_height
                };
                (gap.0 * axis.0 + gap.1 * axis.1).abs() < reach(&ra) + reach(&rb)
            })
    }

    /// Species that are heard with few others have short arcs, and a name
    /// written along a short arc ran into its neighbour's: Red-winged Blackbird
    /// into Black-capped Chickadee, Downy Woodpecker into White-breasted
    /// Nuthatch, on the documentation's own co-occurrence picture.
    ///
    /// Observed failing before the labels were spread apart: three pairs of the
    /// ten overlapped.
    #[test]
    fn chord_labels_do_not_run_into_each_other() {
        let names = [
            "Red-winged Blackbird",
            "Black-capped Chickadee",
            "White-breasted Nuthatch",
            "Downy Woodpecker",
            "Blue Jay",
            "Mourning Dove",
            "Northern Cardinal",
            "American Robin",
            "Song Sparrow",
            "American Goldfinch",
        ];
        // Connectedness: a few busy species and several heard with little else.
        let weight = [0.14, 0.11, 0.15, 0.09, 0.19, 0.31, 0.29, 0.37, 0.32, 0.20];
        let labels: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        let m: Vec<Vec<f64>> = (0..names.len())
            .map(|i| {
                (0..names.len())
                    .map(|j| {
                        if i == j {
                            0.0
                        } else {
                            weight[i] * weight[j] * 9.0
                        }
                    })
                    .collect()
            })
            .collect();
        let placed = placed_labels(&chord_diagram(&labels, &m));
        assert_eq!(placed.len(), names.len());
        let mut hits = Vec::new();
        for i in 0..placed.len() {
            for j in i + 1..placed.len() {
                if labels_overlap(&placed[i], &placed[j]) {
                    hits.push(format!("{} / {}", placed[i].name, placed[j].name));
                }
            }
        }
        assert!(hits.is_empty(), "labels overlap: {hits:?}");
    }

    /// The counterpart: where nothing needs moving, nothing moves. Six equal
    /// arcs put every name at the middle of its own arc.
    #[test]
    fn chord_labels_stay_on_their_arcs_when_there_is_room() {
        let labels: Vec<String> = (0..6).map(|i| format!("Species {i}")).collect();
        let m: Vec<Vec<f64>> = (0..6)
            .map(|i| (0..6).map(|j| if i == j { 0.0 } else { 0.5 }).collect())
            .collect();
        let placed = placed_labels(&chord_diagram(&labels, &m));
        for (k, p) in placed.iter().enumerate() {
            let want =
                -std::f64::consts::FRAC_PI_2 + (k as f64 + 0.5) * std::f64::consts::TAU / 6.0;
            let got = (p.y - 360.0).atan2(p.x - 360.0);
            let off = (got - want + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            assert!(off.abs() < 1e-3, "label {k} sits {off} rad from its arc");
        }
    }
}
