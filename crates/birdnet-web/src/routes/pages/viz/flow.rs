//! Who is heard just before and just after: a flow with the species in the middle.
//!
//! The behavioural engine's `sequence_next_node` answers, for one species, which species were
//! heard immediately before its first detection in a session and which immediately after, and in
//! what share of the sessions that had any. Drawn as two sets of ribbons meeting at a node, the
//! width of a ribbon is that share, so "who comes before a Blue Jay" is a picture of widths
//! rather than a column of percentages to compare.
//!
//! What is *not* drawn: the shares of the five shown do not add to the whole (the rest is the
//! long tail of species the list was cut before), and the picture does not pretend they do: the
//! middle node is as tall as the larger side, and each side leaves its tail unaccounted for
//! rather than inventing an "other". The reading and the table beside it carry the counts.
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#![allow(clippy::items_after_statements, clippy::many_single_char_names)]

use std::fmt::Write as _;

use super::cograph::share_step;
use super::kit::tip;
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// One neighbour of the middle species.
pub struct Neighbour {
    /// The species as the station displays it.
    pub name: String,
    /// The share, `0..=1`, of the sessions that had a bird on this side at all in which it was
    /// the neighbour; not of every session the middle species was heard in.
    pub probability: f64,
    /// Sessions in which it was.
    pub frequency: u64,
    /// This neighbour is the middle species itself (it calls again).
    pub is_trigger: bool,
}

const W: f64 = 700.0;
const NODE_W: f64 = 12.0;
const LEFT_X: f64 = 196.0;
const MID_X: f64 = 343.0;
const MID_W: f64 = 14.0;
const RIGHT_X: f64 = W - 196.0 - NODE_W;
const GAP: f64 = 10.0;
const MIN_NODE: f64 = 14.0;
const COL_H: f64 = 220.0;
const TOP: f64 = 40.0;

/// Where one side's nodes go: `(top, height)` of each node in a column of total height
/// `avail`, stacked with [`GAP`] between, and the thickness of the ribbon each sends to the
/// middle (the true share, scaled).
fn side_layout(p: &[f64], scale: f64) -> Vec<(f64, f64, f64)> {
    let ribbon: Vec<f64> = p.iter().map(|v| (v * scale).max(1.5)).collect();
    let node: Vec<f64> = ribbon.iter().map(|r| r.max(MIN_NODE)).collect();
    let total: f64 = node.iter().sum::<f64>() + GAP * node.len().saturating_sub(1) as f64;
    let mut top = TOP + ((COL_H - total) / 2.0).max(0.0);
    let mut out = Vec::with_capacity(p.len());
    for (n, r) in node.iter().zip(&ribbon) {
        out.push((top, *n, *r));
        top += n + GAP;
    }
    out
}

/// A band from `(x0, y0..y0+t0)` to `(x1, y1..y1+t1)`, curved horizontally.
fn band(x0: f64, y0: f64, t0: f64, x1: f64, y1: f64, t1: f64) -> String {
    let xm = f64::midpoint(x0, x1);
    format!(
        "M{x0:.1},{y0:.1} C{xm:.1},{y0:.1} {xm:.1},{y1:.1} {x1:.1},{y1:.1} L{x1:.1},{yb1:.1} C{xm:.1},{yb1:.1} {xm:.1},{yb0:.1} {x0:.1},{yb0:.1} Z",
        yb0 = y0 + t0,
        yb1 = y1 + t1,
    )
}

/// A name as written beside a node: whole if it fits, else its start and an ellipsis.
fn fit(name: &str) -> String {
    if name.chars().count() <= 24 {
        name.to_owned()
    } else {
        format!("{}…", name.chars().take(23).collect::<String>().trim_end())
    }
}

/// The flow for `trigger` with its `before` and `after` neighbours.
#[must_use]
pub fn neighbours_flow(trigger: &str, before: &[Neighbour], after: &[Neighbour]) -> String {
    if before.is_empty() && after.is_empty() {
        return EMPTY.to_string();
    }
    let sum = |v: &[Neighbour]| v.iter().map(|n| n.probability).sum::<f64>();
    let biggest = sum(before).max(sum(after)).max(0.05);
    let scale = COL_H / biggest * 0.82;
    let pmax = before
        .iter()
        .chain(after)
        .map(|n| n.probability)
        .fold(f64::MIN_POSITIVE, f64::max);
    let left = side_layout(
        &before.iter().map(|n| n.probability).collect::<Vec<_>>(),
        scale,
    );
    let right = side_layout(
        &after.iter().map(|n| n.probability).collect::<Vec<_>>(),
        scale,
    );
    let side_h = |l: &[(f64, f64, f64)]| l.iter().map(|x| x.2).sum::<f64>();
    let mid_h = side_h(&left).max(side_h(&right)).max(MIN_NODE);
    let mid_top = TOP + (COL_H - mid_h) / 2.0;
    let h = TOP + COL_H + 16.0;

    let mut s = format!(
        r#"<div class="ck fl-sankey"><svg viewBox="0 0 {W} {h:.0}" width="100%" role="img" class="fl-svg ck-in" data-ck="marks" data-ck-dim="series" data-ck-id="neighbours" data-ck-label="Left and right arrows step through the neighbours, before first, then after">"#,
    );
    s.push_str(&svg_a11y(
        &format!("Species heard just before and just after {trigger}"),
        &format!(
            "A flow with {trigger} in the middle. On the left the species most often heard just before it, on the right those heard just after; a wider ribbon is a larger share of the sessions."
        ),
    ));
    let _ = write!(
        s,
        r#"<text class="ck-lbl fl-head" x="{lx:.1}" y="14" text-anchor="end">Heard just before</text><text class="ck-lbl fl-head" x="{rx:.1}" y="14" text-anchor="start">Heard just after</text>"#,
        lx = LEFT_X + NODE_W,
        rx = RIGHT_X,
    );

    let mut index = 0usize;
    let mut flows = String::new();
    let mut nodes = String::new();
    let mut offset = 0.0;
    for (i, (n, &(top, node_h, ribbon))) in before.iter().zip(&left).enumerate() {
        let token = format!("b{i}");
        let data_tip = neighbour_tip(trigger, n, "before");
        let mid_y = mid_top + offset;
        offset += ribbon;
        let _ = write!(
            flows,
            r#"<path class="fl-flow ck-q{step}" d="{d}" data-tip="{data_tip}" data-ck-i="{index}" data-ck-s="{token}" data-ck-f="{token}"/>"#,
            step = share_step(n.probability / pmax),
            d = band(
                LEFT_X + NODE_W,
                top + (node_h - ribbon) / 2.0,
                ribbon,
                MID_X,
                mid_y,
                ribbon,
            ),
        );
        let _ = write!(
            nodes,
            r#"<rect class="fl-node" x="{LEFT_X}" y="{top:.1}" width="{NODE_W}" height="{node_h:.1}" rx="3" data-ck-s="{token}"/><text class="fl-name" x="{tx:.1}" y="{ty:.1}" text-anchor="end" dominant-baseline="central" data-tip="{data_tip}" data-ck-i="{index}" data-ck-s="{token}" data-ck-f="{token}">{name}<tspan class="fl-pct mono"> {pct:.0}%</tspan></text>"#,
            tx = LEFT_X - 8.0,
            ty = top + node_h / 2.0,
            name = escape_html(&fit(&n.name)),
            pct = n.probability * 100.0,
        );
        index += 1;
    }
    let mut offset = 0.0;
    for (i, (n, &(top, node_h, ribbon))) in after.iter().zip(&right).enumerate() {
        let token = format!("a{i}");
        let data_tip = neighbour_tip(trigger, n, "after");
        let mid_y = mid_top + offset;
        offset += ribbon;
        let _ = write!(
            flows,
            r#"<path class="fl-flow ck-q{step}" d="{d}" data-tip="{data_tip}" data-ck-i="{index}" data-ck-s="{token}" data-ck-f="{token}"/>"#,
            step = share_step(n.probability / pmax),
            d = band(
                MID_X + MID_W,
                mid_y,
                ribbon,
                RIGHT_X,
                top + (node_h - ribbon) / 2.0,
                ribbon,
            ),
        );
        let again = if n.is_trigger { " (calls again)" } else { "" };
        let _ = write!(
            nodes,
            r#"<rect class="fl-node" x="{RIGHT_X}" y="{top:.1}" width="{NODE_W}" height="{node_h:.1}" rx="3" data-ck-s="{token}"/><text class="fl-name" x="{tx:.1}" y="{ty:.1}" text-anchor="start" dominant-baseline="central" data-tip="{data_tip}" data-ck-i="{index}" data-ck-s="{token}" data-ck-f="{token}">{name}{again}<tspan class="fl-pct mono"> {pct:.0}%</tspan></text>"#,
            tx = RIGHT_X + NODE_W + 8.0,
            ty = top + node_h / 2.0,
            name = escape_html(&fit(&n.name)),
            pct = n.probability * 100.0,
        );
        index += 1;
    }
    s.push_str(&flows);
    let _ = write!(
        s,
        r#"<rect class="fl-mid ck-c1 ck-fill" x="{MID_X}" y="{mid_top:.1}" width="{MID_W}" height="{mid_h:.1}" rx="3"/><text class="fl-trigger" x="{cx:.1}" y="{ty:.1}" text-anchor="middle">{name}</text>"#,
        cx = MID_X + MID_W / 2.0,
        ty = mid_top - 10.0,
        name = escape_html(&fit(trigger)),
    );
    s.push_str(&nodes);
    s.push_str("</svg></div>");
    s
}

/// What a neighbour's reading says.
fn neighbour_tip(trigger: &str, n: &Neighbour, side: &str) -> String {
    let heading = if side == "before" {
        format!("{} → {trigger}", n.name)
    } else {
        format!("{trigger} → {}", n.name)
    };
    tip(
        &heading,
        &[
            (
                if side == "before" {
                    "Of sessions with a bird before"
                } else {
                    "Of sessions with a bird after"
                },
                &format!("{:.0}%", n.probability * 100.0),
                None,
            ),
            ("Sessions", &n.frequency.to_string(), None),
        ],
        n.is_trigger.then_some("The same species calling again."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(name: &str, p: f64, f: u64) -> Neighbour {
        Neighbour {
            name: name.to_owned(),
            probability: p,
            frequency: f,
            is_trigger: false,
        }
    }

    #[test]
    fn nothing_either_side_is_the_placeholder() {
        assert_eq!(neighbours_flow("Blue Jay", &[], &[]), EMPTY);
    }

    /// A node and its ribbon are as thick as the share it stands for: a bird that comes before
    /// twice as often is twice as thick, whatever else is on the page.
    #[test]
    fn thickness_is_the_share() {
        let l = side_layout(&[0.20, 0.10, 0.05], 400.0);
        assert!(
            (l[0].2 - 80.0).abs() < 1e-9
                && (l[1].2 - 40.0).abs() < 1e-9
                && (l[2].2 - 20.0).abs() < 1e-9
        );
        // A very small share is still drawn (a hair, 1.5 units) and its node is a size a
        // pointer can find.
        let tiny = side_layout(&[0.0001], 400.0);
        assert!((tiny[0].2 - 1.5).abs() < 1e-9 && tiny[0].1 >= MIN_NODE);
    }

    /// Nodes in a column never overlap and always leave room for a name between them: two
    /// names closer than a line would be written on each other.
    #[test]
    fn nodes_leave_room_for_their_names() {
        let l = side_layout(&[0.2, 0.01, 0.01, 0.01, 0.01], 600.0);
        for w in l.windows(2) {
            assert!(w[1].0 - (w[0].0 + w[0].1) >= GAP - 1e-9, "{w:?}");
            assert!(w[0].1 >= MIN_NODE - 1e-9);
        }
    }

    #[test]
    fn the_flow_has_a_mark_for_every_neighbour_and_says_the_share_in_its_reading() {
        let before = [
            n("Northern Cardinal", 0.20, 99),
            n("Black-capped Chickadee", 0.15, 73),
        ];
        let mut again = n("Blue Jay", 0.13, 74);
        again.is_trigger = true;
        let after = [n("Northern Cardinal", 0.15, 89), again];
        let svg = neighbours_flow("Blue Jay", &before, &after);
        assert_eq!(svg.matches("<path class=\"fl-flow").count(), 4);
        assert_eq!(svg.matches("<rect class=\"fl-node\"").count(), 4);
        assert!(
            svg.contains("(calls again)"),
            "the same bird calling again is labelled"
        );
        assert!(svg.contains("Northern Cardinal → Blue Jay"));
        assert!(svg.contains("Blue Jay → Northern Cardinal"));
        assert!(svg.contains("20%") && svg.contains("99"));
        // Reading indices are unique and dense, so the keyboard visits each once.
        for i in 0..4 {
            assert!(svg.contains(&format!("data-ck-i=\"{i}\"")), "index {i}");
        }
    }

    /// Every ribbon and node lies inside the picture, however lopsided the two sides are.
    #[test]
    fn the_flow_stays_inside_its_picture() {
        let before = [n("A", 0.9, 90), n("B", 0.05, 5), n("C", 0.02, 2)];
        let after = [n("D", 0.01, 1)];
        let svg = neighbours_flow("Middle", &before, &after);
        let h: f64 = svg
            .split("viewBox=\"0 0 700 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .expect("height");
        for chunk in svg.split(" d=\"").skip(1) {
            let d = chunk.split('"').next().expect("d");
            for tok in d.split(|c: char| c.is_ascii_alphabetic() || c == ' ') {
                if let Some((x, y)) = tok.split_once(',') {
                    let (x, y): (f64, f64) = (x.parse().expect("x"), y.parse().expect("y"));
                    assert!(
                        (0.0..=W).contains(&x) && (0.0..=h).contains(&y),
                        "({x}, {y}) of {W}x{h}"
                    );
                }
            }
        }
    }

    /// The reading says what the percentage is a share of: the sessions that had a bird on that
    /// side, which is the engine's denominator, and not every session the middle bird was in.
    #[test]
    fn the_reading_says_what_the_share_is_of() {
        let svg = neighbours_flow("Blue Jay", &[n("A", 0.6, 3)], &[n("D", 1.0, 2)]);
        assert!(svg.contains("Of sessions with a bird before"), "{svg}");
        assert!(svg.contains("Of sessions with a bird after"), "{svg}");
        assert!(!svg.contains("it came just"), "the old wording is gone");
    }

    /// One side alone is drawn, with its ribbons; the empty side is just empty.
    #[test]
    fn one_side_alone_is_drawn() {
        let svg = neighbours_flow("Blue Jay", &[], &[n("A", 0.3, 30)]);
        assert_eq!(svg.matches("<path class=\"fl-flow").count(), 1);
        assert!(svg.contains("Heard just after"));
    }
}
