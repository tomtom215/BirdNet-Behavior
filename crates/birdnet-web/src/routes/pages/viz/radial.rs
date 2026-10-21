//! The chord diagram of who is heard with whom, and the small geometry it is built from.
//!
//! One ring, one arc per species, one ribbon per link. What each mark says:
//!
//! * a ribbon's **width** and its **colour** (the sequential sun ramp, seven steps) are the
//!   same number, how much of their time the two species share, so the eye needs only one of
//!   them and the colour-blind reader has the other;
//! * an arc's **length** is the total of the ribbons that end on it, so the arcs are a
//!   measure of how much of the picture each species owns, and no arc has a stretch with
//!   nothing on it;
//! * the **order** around the ring is chosen to cross the fewest ribbons
//!   ([`super::cograph::chord_order`]), so groups that sing together sit together;
//! * names stand upright on either side of the ring, spread apart where the arcs are close
//!   (a name written along a short arc ran into its neighbour's), and joined to their arc
//!   by a hairline;
//! * the middle is empty. The old picture wrote a caption there, over the ribbons.
//!
//! The ring is complete with no script. `static/chart-kit.js` adds what a picture cannot:
//! point at a species and only its ribbons stay in the foreground, point at a ribbon and
//! only it and its two species do, and the keyboard walks the species and then the links.
//! That uses two things of the kit: `data-ck-f` on a mark (the focus its reading brings)
//! and the space-separated tokens of `data-ck-s` (what a mark belongs to).
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::similar_names
)]
#![allow(clippy::items_after_statements)]

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use super::cograph::{CoGraph, Link, chord_order, share_step};
use super::kit::{slot_class, text_width, tip};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// Radius of the ring the species' arcs stand on.
const RING: f64 = 176.0;
/// Thickness of an arc.
const ARC: f64 = 10.0;
/// Empty angle between two neighbouring arcs, in radians.
const GAP: f64 = 0.05;
/// How far past the arc a name's leader goes before it turns to meet the name.
const LEADER: f64 = 20.0;
/// Font size of a name, in SVG units.
const NAME_FONT: f64 = 12.0;
/// Height one line of names takes, so two names on one side never touch.
const NAME_LINE: f64 = 17.0;
/// Longest a name is written (in characters) before it ends in an ellipsis; the full name
/// is in the reading the arc gives.
const NAME_CHARS: usize = 26;

/// (x, y) on a circle centred at (`cx`, `cy`), at `ang` radians from the 3 o'clock axis.
pub fn polar(cx: f64, cy: f64, r: f64, ang: f64) -> (f64, f64) {
    (cx + r * ang.cos(), cy + r * ang.sin())
}

/// Annular band between `r_in` and `r_out` spanning the angular range [a0, a1].
pub fn arc_band_path(cx: f64, cy: f64, r_in: f64, r_out: f64, a0: f64, a1: f64) -> String {
    let large = i32::from(a1 - a0 > PI);
    let (xi0, yi0) = polar(cx, cy, r_in, a0);
    let (xi1, yi1) = polar(cx, cy, r_in, a1);
    let (xo0, yo0) = polar(cx, cy, r_out, a0);
    let (xo1, yo1) = polar(cx, cy, r_out, a1);
    format!(
        "M{xi0:.1},{yi0:.1} L{xo0:.1},{yo0:.1} A{r_out:.1},{r_out:.1} 0 {large} 1 {xo1:.1},{yo1:.1} L{xi1:.1},{yi1:.1} A{r_in:.1},{r_in:.1} 0 {large} 0 {xi0:.1},{yi0:.1} Z"
    )
}

/// How deep a ribbon between two points `delta` radians apart dips towards the centre,
/// as the fraction of the radius its control points sit at: a ribbon between neighbours
/// stays near the rim, one between opposite species passes close to the middle. (The old
/// ribbons all bent through the centre, which is where they became one tangle.)
pub fn dip(delta: f64) -> f64 {
    let d = (delta.rem_euclid(TAU)).min(TAU - delta.rem_euclid(TAU));
    0.62 * (1.0 - d / PI).clamp(0.0, 1.0)
}

/// A ribbon: two short arcs of the ring, `(a0, a1)` and `(b0, b1)` in radians, joined by
/// cubic curves.
pub fn ribbon_path(cx: f64, cy: f64, r: f64, a: (f64, f64), b: (f64, f64)) -> String {
    let mid = |e: (f64, f64)| f64::midpoint(e.0, e.1);
    let k = dip(mid(a) - mid(b));
    let p = |ang: f64| polar(cx, cy, r, ang);
    let c = |ang: f64| polar(cx, cy, r * k, ang);
    let (xa0, ya0) = p(a.0);
    let (xa1, ya1) = p(a.1);
    let (xb0, yb0) = p(b.0);
    let (xb1, yb1) = p(b.1);
    let (c1x, c1y) = c(a.1);
    let (c2x, c2y) = c(b.0);
    let (c3x, c3y) = c(b.1);
    let (c4x, c4y) = c(a.0);
    format!(
        "M{xa0:.1},{ya0:.1} A{r:.1},{r:.1} 0 0 1 {xa1:.1},{ya1:.1} C{c1x:.1},{c1y:.1} {c2x:.1},{c2y:.1} {xb0:.1},{yb0:.1} A{r:.1},{r:.1} 0 0 1 {xb1:.1},{yb1:.1} C{c3x:.1},{c3y:.1} {c4x:.1},{c4y:.1} {xa0:.1},{ya0:.1} Z"
    )
}

/// Move a column of labels apart until none is closer than `gap` to the next, staying inside
/// `lo..=hi` when there is room for them there. Input and output are in the same order
/// (`ys` ascending); a column already spaced is returned unchanged.
pub fn spread_column(ys: &[f64], gap: f64, lo: f64, hi: f64) -> Vec<f64> {
    let mut out = ys.to_vec();
    let n = out.len();
    if n == 0 {
        return out;
    }
    // Push down, then, if the last ran off the bottom, pull the whole column back up.
    for i in 1..n {
        if out[i] < out[i - 1] + gap {
            out[i] = out[i - 1] + gap;
        }
    }
    if out[n - 1] > hi {
        out[n - 1] = hi.max(out[0]);
        for i in (0..n - 1).rev() {
            if out[i] > out[i + 1] - gap {
                out[i] = out[i + 1] - gap;
            }
        }
    }
    // Pushing up can run past the top when the column is longer than the room; then the
    // names are spread from the top down and some stand below `hi`: there is no answer
    // that is inside both, and being inside the picture's top matters more than its bottom
    // margin (which has slack).
    if out[0] < lo {
        out[0] = lo;
        for i in 1..n {
            if out[i] < out[i - 1] + gap {
                out[i] = out[i - 1] + gap;
            }
        }
    }
    out
}

/// A name as written on the ring: whole if it fits, else its start and an ellipsis.
fn fit_name(name: &str) -> String {
    if name.chars().count() <= NAME_CHARS {
        name.to_owned()
    } else {
        let head: String = name.chars().take(NAME_CHARS - 1).collect();
        format!("{}…", head.trim_end())
    }
}

/// `v` in `0..=1` as a percentage for a reading: whole numbers, and "<1%" for what rounds to
/// nothing but is not nothing.
pub fn pct(v: f64) -> String {
    let p = v * 100.0;
    if p > 0.0 && p < 1.0 {
        "<1%".to_owned()
    } else {
        format!("{:.0}%", p.round())
    }
}

/// One species on the ring.
struct Node {
    /// Index into [`CoGraph::names`].
    idx: usize,
    a0: f64,
    a1: f64,
}

impl Node {
    const fn mid(&self) -> f64 {
        f64::midpoint(self.a0, self.a1)
    }
}

/// Where each drawn link ends on each species' arc, found by laying a node's ends out in
/// the order of where their partners stand, so ribbons leaving one arc do not cross each
/// other before they have left it.
///
/// Returns, for every link in `links`, `(end on a, end on b)` as `(a0, a1)` radians.
fn link_ends(nodes: &[Node], links: &[Link], scale: f64) -> Vec<((f64, f64), (f64, f64))> {
    let at = |idx: usize| nodes.iter().position(|n| n.idx == idx);
    let mut ends: Vec<((f64, f64), (f64, f64))> = vec![((0.0, 0.0), (0.0, 0.0)); links.len()];
    for node in nodes {
        let mut here: Vec<(usize, f64)> = Vec::new(); // (link index, partner's angle offset)
        for (li, l) in links.iter().enumerate() {
            let partner = if l.a == node.idx {
                l.b
            } else if l.b == node.idx {
                l.a
            } else {
                continue;
            };
            let Some(pi) = at(partner) else { continue };
            // Clockwise distance from this arc's middle to the partner's. The partner just
            // counter-clockwise is the largest, and goes at the counter-clockwise end.
            let delta = (nodes[pi].mid() - node.mid()).rem_euclid(TAU);
            here.push((li, delta));
        }
        here.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        let mut cursor = node.a0;
        for (li, _) in here {
            let w = links[li].v * scale;
            let e = (cursor, cursor + w);
            cursor += w;
            if links[li].a == node.idx {
                ends[li].0 = e;
            } else {
                ends[li].1 = e;
            }
        }
    }
    ends
}

/// The acoustic network as a chord diagram: the `max_links` strongest links among the species
/// in `g`, as ribbons on a ring. A species with none of them is not on the ring (it is in the
/// ranked list and the table, which hold every pair).
#[must_use]
pub fn chord_diagram(g: &CoGraph, max_links: usize) -> String {
    let links = g.top_links(max_links);
    if links.is_empty() || !g.is_square() {
        return EMPTY.to_string();
    }

    // The species with at least one link drawn, in the order that crosses the fewest.
    let mut present: Vec<usize> = (0..g.n())
        .filter(|&i| links.iter().any(|l| l.a == i || l.b == i))
        .collect();
    present.sort_unstable();
    let order = chord_order(&present, &links);

    let weight_of = |i: usize| -> f64 {
        links
            .iter()
            .filter(|l| l.a == i || l.b == i)
            .map(|l| l.v)
            .sum()
    };
    let total: f64 = order.iter().map(|&i| weight_of(i)).sum();
    let scale = (TAU - GAP * order.len() as f64) / total;

    let mut nodes: Vec<Node> = Vec::with_capacity(order.len());
    let mut cursor = -PI / 2.0 + GAP / 2.0;
    for &idx in &order {
        let span = weight_of(idx) * scale;
        nodes.push(Node {
            idx,
            a0: cursor,
            a1: cursor + span,
        });
        cursor += span + GAP;
    }
    let ends = link_ends(&nodes, &links, scale);

    // The picture: names stand either side of the ring, so the width is the ring and the
    // longest name twice over.
    let names: Vec<String> = g.names.iter().map(|n| fit_name(n)).collect();
    let name_w = order
        .iter()
        .map(|&i| text_width(&names[i], NAME_FONT))
        .fold(0.0, f64::max);
    let margin = LEADER + 14.0 + name_w;
    let w = 2.0 * (RING + ARC + margin);
    let h = 2.0 * (RING + ARC + 26.0);
    let (cx, cy) = (w / 2.0, h / 2.0);

    let mut svg = format!(
        r#"<svg viewBox="0 0 {w:.0} {h:.0}" width="100%" role="img" class="viz-svg-chord" data-ck="marks" data-ck-dim="series" data-ck-start="0" data-ck-id="chord" data-ck-label="Left and right arrows step through the species and then the links; a species brings its own ribbons forward">"#,
    );
    svg.push_str(&svg_a11y(
        "Acoustic co-occurrence network",
        &format!(
            "{} species on a ring joined by the {} strongest links; a wider, darker ribbon joins two species that were heard together in the same five-minute block more of the time. The strongest is {} with {}.",
            nodes.len(),
            links.len(),
            g.names[links[0].a],
            g.names[links[0].b],
        ),
    ));

    // Ribbons, the weakest first so the strongest lie on top. `links` is strongest first,
    // and its rank is its index; the mark's reading index counts after the arcs.
    let first_link_i = nodes.len();
    for (li, l) in links.iter().enumerate().rev() {
        let (ea, eb) = ends[li];
        let a_name = &g.names[l.a];
        let b_name = &g.names[l.b];
        let data_tip = tip(
            &format!("{a_name} × {b_name}"),
            &[
                (
                    "Of the five-minute blocks either was heard in",
                    &pct(g.share[l.a][l.b]),
                    None,
                ),
                (
                    "Heard together",
                    &format!("{} blocks", g.blocks[l.a][l.b]),
                    None,
                ),
            ],
            None,
        );
        let _ = write!(
            svg,
            r#"<path class="chord-ribbon ck-q{step}" d="{d}" data-tip="{data_tip}" data-ck-i="{i}" data-ck-s="{ta} {tb} r{li}" data-ck-f="r{li}"/>"#,
            step = share_step(l.v),
            d = ribbon_path(cx, cy, RING - 2.0, ea, eb),
            i = first_link_i + li,
            ta = l.a,
            tb = l.b,
        );
    }

    // Arcs, a hit band beyond each so a finger has something to land on, the leader and
    // the name.
    let mut sides: [Vec<(usize, f64, f64)>; 2] = [Vec::new(), Vec::new()]; // (node, natural y, x)
    for (ni, node) in nodes.iter().enumerate() {
        let m = node.mid();
        let (px, py) = polar(cx, cy, RING + ARC + LEADER, m);
        sides[usize::from(px < cx)].push((ni, py, px));
    }
    let lo = 12.0;
    let hi = h - 12.0;
    let mut label_at: Vec<(f64, f64, bool)> = vec![(0.0, 0.0, false); nodes.len()]; // (x, y, left)
    for (side, col) in sides.iter_mut().enumerate() {
        col.sort_by(|x, y| x.1.total_cmp(&y.1));
        let ys: Vec<f64> = col.iter().map(|c| c.1).collect();
        let spread = spread_column(&ys, NAME_LINE, lo, hi);
        for (k, &(ni, _, px)) in col.iter().enumerate() {
            let left = side == 1;
            let x = if left { px - 8.0 } else { px + 8.0 };
            label_at[ni] = (x, spread[k], left);
        }
    }

    for (ni, node) in nodes.iter().enumerate() {
        let i = node.idx;
        let name = &g.names[i];
        // The arc's reading: how much of the picture it owns and who its strongest partners are.
        let mut partners: Vec<(usize, f64)> = links
            .iter()
            .filter_map(|l| {
                if l.a == i {
                    Some((l.b, l.v))
                } else if l.b == i {
                    Some((l.a, l.v))
                } else {
                    None
                }
            })
            .collect();
        partners.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        let rows: Vec<(String, String)> = partners
            .iter()
            .take(4)
            .map(|&(p, _)| (g.names[p].clone(), pct(g.share[i][p])))
            .collect();
        let row_refs: Vec<(&str, &str, Option<&str>)> = rows
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str(), None))
            .collect();
        let more = partners.len().saturating_sub(4);
        let note = if more > 0 {
            format!(
                "and {more} more link{} drawn. Share of the blocks either was heard in.",
                if more == 1 { "" } else { "s" }
            )
        } else {
            "Share of the blocks either was heard in.".to_owned()
        };
        let data_tip = tip(name, &row_refs, Some(&note));
        // Who this arc belongs to: itself and its partners (reading a species keeps them), and
        // the id of each ribbon that ends here (reading a ribbon keeps its two arcs and no
        // third species that is merely linked to both).
        let tokens: String = std::iter::once(i.to_string())
            .chain(partners.iter().map(|p| p.0.to_string()))
            .chain(
                links
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.a == i || l.b == i)
                    .map(|(li, _)| format!("r{li}")),
            )
            .collect::<Vec<_>>()
            .join(" ");
        let slot = slot_class(i);

        let band = arc_band_path(cx, cy, RING, RING + ARC, node.a0 + 0.004, node.a1 - 0.004);
        let _ = write!(
            svg,
            r#"<path class="chord-arc {slot}" d="{band}" data-tip="{data_tip}" data-ck-i="{ni}" data-ck-s="{tokens}" data-ck-f="{i}"/>"#,
        );
        let hit = arc_band_path(
            cx,
            cy,
            RING + ARC,
            RING + ARC + LEADER + 10.0,
            node.a0,
            node.a1,
        );
        let _ = write!(
            svg,
            r#"<path class="ck-hit" d="{hit}" data-tip="{data_tip}" data-ck-i="{ni}" data-ck-f="{i}"/>"#,
        );

        let (lx, ly, left) = label_at[ni];
        let m = node.mid();
        let (ax, ay) = polar(cx, cy, RING + ARC + 3.0, m);
        let (kx, ky) = polar(cx, cy, RING + ARC + LEADER, m);
        let tx = if left { lx + 5.0 } else { lx - 5.0 };
        let _ = write!(
            svg,
            r#"<g data-ck-s="{tokens}"><polyline class="chord-leader" points="{ax:.1},{ay:.1} {kx:.1},{ky:.1} {tx:.1},{ly:.1}"/></g>"#,
        );
        let anchor = if left { "end" } else { "start" };
        let _ = write!(
            svg,
            r#"<text class="chord-name" x="{lx:.1}" y="{ly:.1}" text-anchor="{anchor}" dominant-baseline="central" data-tip="{data_tip}" data-ck-i="{ni}" data-ck-s="{tokens}" data-ck-f="{i}">{name}</text>"#,
            name = escape_html(&names[i]),
        );
    }

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` species named `Species k`, every pair of `pairs` linked at the strength given.
    fn graph(n: usize, pairs: &[(usize, usize, f64)]) -> CoGraph {
        let mut s = vec![vec![0.0; n]; n];
        for &(a, b, v) in pairs {
            s[a][b] = v;
            s[b][a] = v;
        }
        CoGraph {
            names: (0..n).map(|i| format!("Species {i}")).collect(),
            share: s
                .iter()
                .map(|r| r.iter().map(|v| v * 0.4).collect())
                .collect(),
            strength: s,
            blocks: vec![vec![12; n]; n],
        }
    }

    /// Every pair, strengths varied so the arcs differ in length.
    fn complete(n: usize) -> CoGraph {
        let mut pairs = Vec::new();
        for a in 0..n {
            for b in a + 1..n {
                let k = u8::try_from((a * 7 + b * 3) % 10).unwrap_or(0);
                pairs.push((a, b, 0.25 + f64::from(k) / 14.0));
            }
        }
        graph(n, &pairs)
    }

    #[test]
    fn chord_empty_and_basic() {
        assert!(chord_diagram(&CoGraph::default(), 12).contains("Not enough data"));
        assert!(chord_diagram(&graph(2, &[]), 12).contains("Not enough"));
        let svg = chord_diagram(&graph(2, &[(0, 1, 0.8)]), 12);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("chord-ribbon"));
        assert!(svg.contains("Species 0") && svg.contains("Species 1"));
        // Accessible name + description replace the bare aria-label.
        assert!(svg.contains("<title>Acoustic co-occurrence network</title>"));
        assert!(svg.contains("<desc>2 species on a ring"));
        assert!(!svg.contains("aria-label"));
    }

    /// The old diagram wrote "5-minute co-occurrence" in the middle, over the ribbons, and
    /// had to give it a halo to be readable. The middle is empty now.
    #[test]
    fn nothing_is_written_over_the_ribbons() {
        let svg = chord_diagram(&complete(6), 12);
        assert!(
            !svg.contains("co-occurrence</text>"),
            "the centre caption is back"
        );
        let size = svg
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("viewBox");
        let mut it = size
            .split_whitespace()
            .map(|v| v.parse::<f64>().expect("number"));
        let (w, h) = (it.next().expect("w"), it.next().expect("h"));
        // No text is anywhere near the middle of the ring.
        for chunk in svg.split("<text ").skip(1) {
            let at = |name: &str| -> f64 {
                let k = chunk.find(&format!("{name}=\"")).expect("attr") + name.len() + 2;
                chunk[k..]
                    .split('"')
                    .next()
                    .expect("value")
                    .parse()
                    .expect("number")
            };
            let (x, y) = (at("x"), at("y"));
            let d = (x - w / 2.0).hypot(y - h / 2.0);
            assert!(d > RING, "a label at ({x}, {y}) is inside the ring ({d})");
        }
    }

    /// A ribbon's width and its colour are one number: the strongest link gets the darkest
    /// step and the weakest a paler one, in the same order as their widths.
    #[test]
    fn width_and_colour_say_the_same_thing() {
        let g = graph(4, &[(0, 1, 1.0), (2, 3, 0.1)]);
        let svg = chord_diagram(&g, 12);
        let class_of = |tipped: &str| -> usize {
            let at = svg.find(tipped).expect("ribbon");
            let before = &svg[..at];
            let c = before.rfind("ck-q").expect("a step class") + 4;
            before[c..=c].parse().expect("a digit")
        };
        let strong = class_of("Species 0 × Species 1");
        let weak = class_of("Species 2 × Species 3");
        assert_eq!(strong, 7);
        assert!(weak < strong, "weak {weak} strong {strong}");
    }

    /// Both ends of a ribbon are as wide as the link is strong, on one scale for the whole
    /// ring, and an arc is exactly as long as the ribbons that end on it: no species owns a
    /// stretch of ring with nothing on it, and none has ribbons running off its end.
    #[test]
    fn an_arc_is_the_sum_of_the_ribbons_that_end_on_it() {
        let g = complete(7);
        let links = g.top_links(100);
        let order = chord_order(&(0..7).collect::<Vec<_>>(), &links);
        let weight_of = |i: usize| -> f64 {
            links
                .iter()
                .filter(|l| l.a == i || l.b == i)
                .map(|l| l.v)
                .sum()
        };
        let total: f64 = order.iter().map(|&i| weight_of(i)).sum();
        let scale = (TAU - GAP * 7.0) / total;
        let mut nodes = Vec::new();
        let mut cursor = 0.0;
        for &idx in &order {
            let span = weight_of(idx) * scale;
            nodes.push(Node {
                idx,
                a0: cursor,
                a1: cursor + span,
            });
            cursor += span + GAP;
        }
        let ends = link_ends(&nodes, &links, scale);
        for node in &nodes {
            let mut mine: Vec<(f64, f64)> = links
                .iter()
                .zip(&ends)
                .filter_map(|(l, e)| {
                    if l.a == node.idx {
                        Some(e.0)
                    } else if l.b == node.idx {
                        Some(e.1)
                    } else {
                        None
                    }
                })
                .collect();
            mine.sort_by(|x, y| x.0.total_cmp(&y.0));
            assert!(
                (mine[0].0 - node.a0).abs() < 1e-9,
                "ends start at the arc's start"
            );
            assert!(
                (mine[mine.len() - 1].1 - node.a1).abs() < 1e-9,
                "ends fill the arc to its end"
            );
            for pair in mine.windows(2) {
                assert!(
                    (pair[0].1 - pair[1].0).abs() < 1e-9,
                    "ends tile the arc without a gap"
                );
            }
        }
        for (l, e) in links.iter().zip(&ends) {
            let (wa, wb) = (e.0.1 - e.0.0, e.1.1 - e.1.0);
            assert!((wa - wb).abs() < 1e-9, "both ends of a ribbon are as wide");
            assert!(
                (wa - l.v * scale).abs() < 1e-9,
                "a ribbon is as wide as it is strong"
            );
        }
    }

    /// A ribbon between neighbours must not dive through the middle: the control points of
    /// the old quadratic curve were always the centre, which is what made the picture one
    /// knot. Neighbours stay near the rim, opposites pass near the middle.
    #[test]
    fn a_ribbon_between_neighbours_stays_near_the_rim() {
        assert!(dip(0.2) > 0.5, "neighbours: {}", dip(0.2));
        assert!(dip(PI) < 0.01, "opposite: {}", dip(PI));
        assert!(
            (dip(0.3) - dip(TAU - 0.3)).abs() < 1e-12,
            "either way round"
        );
        // Narrower the further apart, never wider.
        let mut last = f64::MAX;
        for k in 1..=10 {
            let d = dip(PI * f64::from(k) / 10.0);
            assert!(d <= last + 1e-12);
            last = d;
        }
        // The path is a closed figure of two arcs and two curves.
        let p = ribbon_path(300.0, 300.0, 170.0, (0.0, 0.1), (2.0, 2.2));
        assert_eq!(p.matches('A').count(), 2);
        assert_eq!(p.matches('C').count(), 2);
        assert!(p.ends_with('Z'));
    }

    /// Names stand either side of the ring, and where arcs are close they are spread apart:
    /// the old tangent names ran into their neighbours (Red-winged Blackbird into
    /// Black-capped Chickadee) on the documentation's own picture.
    ///
    /// Observed failing against a placement that wrote every name at its arc's own height
    /// (the first assertion's `unspread` variant of this very test, run by hand: four pairs
    /// of the ten closer than a line), and holding for the spread one.
    #[test]
    fn names_never_touch() {
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
        let weight: [f64; 10] = [0.14, 0.11, 0.15, 0.09, 0.19, 0.31, 0.29, 0.37, 0.32, 0.20];
        let n = names.len();
        let mut g = graph(
            n,
            &(0..n)
                .flat_map(|a| (a + 1..n).map(move |b| (a, b)))
                .map(|(a, b)| (a, b, (weight[a] * weight[b] * 9.0).min(1.0)))
                .collect::<Vec<_>>(),
        );
        g.names = names.iter().map(|s| (*s).to_owned()).collect();
        let svg = chord_diagram(&g, 100);

        struct Placed {
            x: f64,
            y: f64,
            end: bool,
            name: String,
        }
        let placed: Vec<Placed> = svg
            .split("<text class=\"chord-name\" ")
            .skip(1)
            .map(|chunk| {
                let at = |name: &str| -> String {
                    let k = chunk.find(&format!("{name}=\"")).expect("attr") + name.len() + 2;
                    chunk[k..].split('"').next().expect("value").to_owned()
                };
                Placed {
                    x: at("x").parse().expect("x"),
                    y: at("y").parse().expect("y"),
                    end: at("text-anchor") == "end",
                    name: chunk
                        .split_once('>')
                        .expect("text")
                        .1
                        .split('<')
                        .next()
                        .expect("name")
                        .to_owned(),
                }
            })
            .collect();
        assert_eq!(
            placed.len(),
            n,
            "one name per species, or the gate is vacuous"
        );
        let rect = |p: &Placed| {
            let w = text_width(&p.name, NAME_FONT);
            let (x0, x1) = if p.end {
                (p.x - w, p.x)
            } else {
                (p.x, p.x + w)
            };
            (x0, x1, p.y - NAME_FONT / 2.0, p.y + NAME_FONT / 2.0)
        };
        let mut hits = Vec::new();
        for i in 0..placed.len() {
            for j in i + 1..placed.len() {
                let (a, b) = (rect(&placed[i]), rect(&placed[j]));
                if a.0 < b.1 && b.0 < a.1 && a.2 < b.3 && b.2 < a.3 {
                    hits.push(format!("{} / {}", placed[i].name, placed[j].name));
                }
            }
        }
        assert!(hits.is_empty(), "names overlap: {hits:?}");
        // And they are inside the picture.
        let vb: Vec<f64> = svg
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("viewBox")
            .split_whitespace()
            .map(|v| v.parse().expect("number"))
            .collect();
        for p in &placed {
            let r = rect(p);
            assert!(
                r.0 >= 0.0 && r.1 <= vb[0] && r.2 >= 0.0 && r.3 <= vb[1],
                "{} is outside the picture",
                p.name
            );
        }
    }

    /// The column spreader: nothing moves when nothing needs to, and a crowd keeps its
    /// order and its gaps.
    #[test]
    fn spreading_leaves_the_room_alone_and_separates_a_crowd() {
        assert_eq!(
            spread_column(&[10.0, 40.0, 90.0], 17.0, 0.0, 200.0),
            vec![10.0, 40.0, 90.0]
        );
        let s = spread_column(&[50.0, 51.0, 52.0, 53.0], 17.0, 0.0, 200.0);
        for w in s.windows(2) {
            assert!(w[1] - w[0] >= 17.0 - 1e-9, "{s:?}");
        }
        // A crowd at the bottom edge is pulled up into the picture, not pushed out of it.
        let s = spread_column(&[190.0, 195.0, 199.0], 17.0, 0.0, 200.0);
        assert!(s[2] <= 200.0 && s[0] >= 0.0, "{s:?}");
        assert!(spread_column(&[], 17.0, 0.0, 10.0).is_empty());
    }

    /// Hovering a species must leave exactly the ribbons that touch it, and hovering a
    /// ribbon exactly that ribbon and its two species. The kit lights a mark when its
    /// `data-ck-s` tokens contain every token of the focus the reading names, so check the
    /// rule with the same function the browser applies.
    #[test]
    fn the_focus_tokens_isolate_a_species_and_a_ribbon() {
        let g = graph(5, &[(0, 1, 0.9), (0, 2, 0.5), (3, 4, 0.7), (1, 3, 0.3)]);
        let svg = chord_diagram(&g, 100);
        let marks: Vec<(String, Vec<String>)> = svg
            .split('<')
            .filter_map(|el| {
                let kind = if el.starts_with("path class=\"chord-ribbon") {
                    "ribbon"
                } else if el.starts_with("path class=\"chord-arc") {
                    "arc"
                } else {
                    return None;
                };
                let tokens = el
                    .split("data-ck-s=\"")
                    .nth(1)?
                    .split('"')
                    .next()?
                    .split(' ')
                    .map(str::to_owned)
                    .collect();
                Some((kind.to_owned(), tokens))
            })
            .collect();
        let lit = |focus: &[&str], kind: &str| -> usize {
            marks
                .iter()
                .filter(|(k, t)| k == kind && focus.iter().all(|f| t.iter().any(|x| x == f)))
                .count()
        };
        // Species 0 has two ribbons; species 3 has two; species 2 has one.
        assert_eq!(lit(&["0"], "ribbon"), 2);
        assert_eq!(lit(&["3"], "ribbon"), 2);
        assert_eq!(lit(&["2"], "ribbon"), 1);
        // A ribbon is lit only by its own two species together.
        assert_eq!(lit(&["0", "1"], "ribbon"), 1);
        assert_eq!(lit(&["0", "4"], "ribbon"), 0);
    }

    /// Reading a ribbon keeps its two species and nobody else, even a third species that is
    /// heard with both of them (a triangle): an arc lists its partners so that reading a
    /// species keeps them, and that list must not make the third species look like an end
    /// of a ribbon it only borders.
    ///
    /// Observed failing against the focus `"{a} {b}"` this ribbon used to name: the arc of the
    /// third species held both tokens and stayed in the foreground (three arcs, not two).
    #[test]
    fn reading_a_ribbon_does_not_keep_a_species_that_is_only_linked_to_both_ends() {
        let g = graph(3, &[(0, 1, 0.9), (0, 2, 0.8), (1, 2, 0.7)]);
        let svg = chord_diagram(&g, 100);
        let attr = |el: &str, name: &str| -> Vec<String> {
            el.split(&format!("{name}=\""))
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map(|t| t.split(' ').map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let ribbons: Vec<(Vec<String>, Vec<String>)> = svg
            .split('<')
            .filter(|el| el.starts_with("path class=\"chord-ribbon"))
            .map(|el| (attr(el, "data-ck-s"), attr(el, "data-ck-f")))
            .collect();
        let arcs: Vec<Vec<String>> = svg
            .split('<')
            .filter(|el| el.starts_with("path class=\"chord-arc"))
            .map(|el| attr(el, "data-ck-s"))
            .collect();
        assert_eq!((ribbons.len(), arcs.len()), (3, 3));
        let holds = |tokens: &[String], focus: &[String]| focus.iter().all(|f| tokens.contains(f));
        for (tokens, focus) in &ribbons {
            let ends: Vec<&String> = tokens.iter().filter(|t| t.chars().all(|c| c.is_ascii_digit())).collect();
            assert_eq!(ends.len(), 2, "a ribbon belongs to two species: {tokens:?}");
            let lit_ribbons = ribbons.iter().filter(|(t, _)| holds(t, focus)).count();
            let lit_arcs: Vec<usize> = arcs
                .iter()
                .enumerate()
                .filter(|(_, t)| holds(t, focus))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(lit_ribbons, 1, "only itself: {focus:?}");
            assert_eq!(lit_arcs.len(), 2, "its two species, not three: {focus:?} lit arcs {lit_arcs:?}");
            for e in ends {
                assert!(lit_arcs.contains(&e.parse::<usize>().expect("species")), "an end of the ribbon stays lit");
            }
        }
    }
}
