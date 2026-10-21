//! The co-occurrence graph as a ladder matrix, a ranked list, and the figure that holds
//! them beside the chord diagram.
//!
//! # Why a ladder
//!
//! The chord diagram is the right picture on a wide card and the wrong one on a phone: its
//! names stand either side of the ring and need about 760 units of width, so on a 390 px
//! screen they would be drawn at about 5 px (the legibility gate's floor is 8.5), and a
//! ring's worth of ribbons in 340 px is a knot whatever order they are in. A matrix has no
//! such limit, because a cell is a cell at any size. So the figure carries both and the
//! stylesheet shows the one the screen can read (`.co-chord`, `.co-ladder`).
//!
//! The matrix is symmetric, so drawing it whole says every pair twice. The ladder draws each
//! pair once, as a staircase, and writes each name at the foot of its own step, where the
//! eye arrives after following a cell across its row and then up its column: across to a
//! name, up to a name. Species stand in the same order as on the ring
//! ([`super::cograph::chord_order`]), so groups that sing together form blocks of dark cells
//! along the diagonal.
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
#![allow(clippy::items_after_statements, clippy::many_single_char_names)]

use std::fmt::Write as _;

use super::cograph::{CoGraph, chord_order, share_step};
use super::kit::{slot_class, text_width, tip};
use super::radial::{chord_diagram, pct};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// Side of a cell, and the gap left between two.
const CELL: f64 = 24.0;
const SEAM: f64 = 2.0;
/// Font size of a name beside the ladder.
const NAME_FONT: f64 = 11.5;
/// Clearance between the last cell of a row and its name.
const NAME_GAP: f64 = 8.0;
/// Where the name begins after its colour dot: the dot's own room, less the seam.
const NAME_INDENT: f64 = 6.0 - SEAM;

/// The matrix as a staircase: each pair once, strongest in the darkest step of the
/// sequential ramp, each name at the foot of its own step.
#[must_use]
pub fn cooccurrence_matrix(g: &CoGraph) -> String {
    let n = g.n();
    if n < 2 || !g.is_square() {
        return EMPTY.to_string();
    }
    let links = g.ranked_links();
    if links.is_empty() {
        return EMPTY.to_string();
    }
    let nodes: Vec<usize> = (0..n).collect();
    let order = chord_order(&nodes, &links);

    let names: Vec<String> = g
        .names
        .iter()
        .map(|s| {
            if s.chars().count() > 26 {
                let head: String = s.chars().take(25).collect();
                format!("{}…", head.trim_end())
            } else {
                s.clone()
            }
        })
        .collect();
    // The row of position `p` has `p` cells and then its name, so the picture is as wide as the
    // furthest any name reaches.
    let width = order
        .iter()
        .enumerate()
        .map(|(p, &i)| p as f64 * CELL + NAME_GAP + NAME_INDENT + text_width(&names[i], NAME_FONT))
        .fold(0.0, f64::max)
        + 2.0;
    let height = n as f64 * CELL;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {width:.0} {height:.0}" width="100%" role="img" class="viz-svg-ladder" data-ck="marks" data-ck-label="Left and right arrows step through the pairs, up and down through a row" data-ck-id="ladder">"#,
    );
    svg.push_str(&svg_a11y(
        "Species co-occurrence matrix",
        &format!(
            "A staircase of {} species pairs; a darker cell means those two species were detected in the same five-minute block more of the time. Each name stands at the foot of its own step: read across a row to its name and up a column to the other.",
            links.len()
        ),
    ));

    // Cells, then names. The reading index counts cells in document order (row by row).
    let mut index = 0usize;
    let mut start = 0usize;
    let mut best = -1.0_f64;
    for p in 1..n {
        let i = order[p];
        for (q, &j) in order.iter().enumerate().take(p) {
            let v = g.strength[i][j];
            let x = q as f64 * CELL;
            let y = p as f64 * CELL;
            let data_tip = tip(
                &format!("{} × {}", g.names[i], g.names[j]),
                &[
                    (
                        "Of the five-minute blocks either was heard in",
                        &pct(g.share[i][j]),
                        None,
                    ),
                    (
                        "Heard together",
                        &format!("{} blocks", g.blocks[i][j]),
                        None,
                    ),
                ],
                None,
            );
            if v > best {
                best = v;
                start = index;
            }
            let class = if v > 0.0 {
                format!("ck-fill ck-q{}", share_step(v))
            } else {
                "ck-track".to_owned()
            };
            let _ = write!(
                svg,
                r#"<rect class="{class}" x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{w:.1}" rx="3" data-tip="{data_tip}" data-ck-i="{index}"/>"#,
                w = CELL - SEAM,
            );
            index += 1;
        }
    }
    for (p, &i) in order.iter().enumerate() {
        let x = p as f64 * CELL + NAME_GAP - SEAM;
        let y = p as f64 * CELL + (CELL - SEAM) / 2.0;
        let _ = write!(
            svg,
            r#"<circle class="{slot} ck-fill" cx="{cxd:.1}" cy="{y:.1}" r="3.5"/><text class="ladder-name" x="{x2:.1}" y="{y:.1}" dominant-baseline="central">{name}</text>"#,
            slot = slot_class(i),
            cxd = p as f64 * CELL + 2.0,
            x2 = x + 6.0,
            name = escape_html(&names[i]),
        );
    }
    let _ = write!(svg, "</svg>");
    // The strongest pair is where the keyboard lands first.
    svg.replacen(
        r#"data-ck-id="ladder">"#,
        &format!(r#"data-ck-id="ladder" data-ck-start="{start}">"#),
        1,
    )
}

/// The strongest pairs, ranked, each with a bar in the colour its ribbon has: the list a phone
/// reads in place of the ring, and the first thing a screen reader meets.
#[must_use]
pub fn ranked_pairs(g: &CoGraph, k: usize) -> String {
    let links = g.top_links(k);
    if links.is_empty() || !g.is_square() {
        return String::new();
    }
    let mut html = String::from(r#"<ol class="co-rank">"#);
    for l in &links {
        let share = g.share[l.a][l.b];
        let _ = write!(
            html,
            r#"<li class="co-rank-row"><span class="co-rank-names">{a}<span class="co-rank-x"> × </span>{b}</span><span class="co-rank-bar"><i class="ck-sw ck-q{step}" data-style="width:{w:.0}%"></i></span><b class="co-rank-v">{p}</b></li>"#,
            a = escape_html(&g.names[l.a]),
            b = escape_html(&g.names[l.b]),
            step = share_step(l.v),
            w = (l.v * 100.0).max(3.0),
            p = pct(share),
        );
    }
    html.push_str("</ol>");
    html
}

/// What a ribbon means, written under the picture: the ramp, the measured share at its two
/// ends, and what an arc's length is.
#[must_use]
pub fn legend(g: &CoGraph, shown: usize, available: usize) -> String {
    let links = g.top_links(shown);
    if links.is_empty() {
        return String::new();
    }
    let strongest = links.first().map_or(0.0, |l| g.share[l.a][l.b]);
    let weakest = links.last().map_or(0.0, |l| g.share[l.a][l.b]);
    let mut steps = String::new();
    for k in 1..=7 {
        let _ = write!(steps, r#"<i class="ck-sw ck-q{k}"></i>"#);
    }
    format!(
        r#"<div class="co-legend"><div class="ck-scale"><span class="ck-steps" aria-hidden="true">{steps}</span><span class="bnb-meta">A ribbon joins two species heard in the same five-minute block. Wider and darker means they share more of their time: {lo} to {hi} of the blocks either was heard in.</span></div><p class="bnb-meta ck-scale-note">An arc is as long as the ribbons that end on it. Showing the {shown} strongest of {available} links.</p></div>"#,
        lo = pct(weakest),
        hi = pct(strongest),
        shown = links.len().min(shown),
    )
}

/// How many links the figure draws, and the choices offered.
pub const LINK_CHOICES: [usize; 3] = [12, 24, 0];

/// The whole figure: the ring (wide screens), the ladder (narrow ones), the ranked list and
/// the key.
///
/// `max_links` of `0` means every link. `base` is the URL the "links shown" buttons ask,
/// without its query (the page adds the range it already holds).
#[must_use]
pub fn co_figure(g: &CoGraph, max_links: usize, base: &str) -> String {
    let all = g.ranked_links().len();
    if all == 0 || !g.is_square() {
        return EMPTY.to_string();
    }
    let shown = if max_links == 0 {
        all
    } else {
        max_links.min(all)
    };
    let mut html = String::from(r#"<div class="co-fig">"#);
    // Choices that would show the same picture are not offered: asking for 24 of 9 links is
    // asking for all of them.
    let mut buttons = String::new();
    let mut seen: Vec<usize> = Vec::new();
    for choice in LINK_CHOICES {
        let effective = if choice == 0 { all } else { choice.min(all) };
        if seen.contains(&effective) {
            continue;
        }
        seen.push(effective);
        let everything = effective == all;
        let ask = if everything { 0 } else { choice };
        let label = if everything {
            "All".to_owned()
        } else {
            choice.to_string()
        };
        let active = effective == shown;
        let _ = write!(
            buttons,
            r##"<button type="button" class="btn{on}" aria-pressed="{active}" hx-get="{base}?links={ask}" hx-include="#days-hidden" hx-target="#acoustic-network" hx-swap="innerHTML">{label}</button>"##,
            on = if active { " active" } else { "" },
        );
    }
    if all > LINK_CHOICES[0] {
        let _ = write!(
            html,
            r#"<div class="co-controls"><span class="bnb-meta" id="co-links-l">Links shown</span><div class="seg" role="group" aria-labelledby="co-links-l">{buttons}</div></div>"#,
        );
    }
    let _ = write!(
        html,
        r#"<div class="co-ladder ck">{ladder}</div><div class="co-main"><div class="co-chord ck">{chord}</div><div class="co-aside"><h3 class="co-aside-h">Strongest pairs</h3>{rank}{key}</div></div>"#,
        chord = chord_diagram(g, shown),
        rank = ranked_pairs(g, 8),
        key = legend(g, shown, all),
        ladder = cooccurrence_matrix(g),
    );
    html.push_str("</div>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;

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
            blocks: vec![vec![9; n]; n],
        }
    }

    #[test]
    fn matrix_empty_for_small_or_unlinked_input() {
        assert!(cooccurrence_matrix(&CoGraph::default()).contains("Not enough data"));
        assert!(cooccurrence_matrix(&graph(1, &[])).contains("Not enough data"));
        assert!(cooccurrence_matrix(&graph(3, &[])).contains("Not enough data"));
    }

    /// A staircase has `n * (n - 1) / 2` cells, one per pair, each carrying its own reading.
    #[test]
    fn the_ladder_has_one_cell_per_pair() {
        let g = graph(5, &[(0, 1, 1.0), (1, 2, 0.5), (3, 4, 0.2)]);
        let svg = cooccurrence_matrix(&g);
        assert_eq!(svg.matches("<rect ").count(), 10, "5 species, 10 pairs");
        assert_eq!(svg.matches("data-tip=").count(), 10);
        assert_eq!(
            svg.matches("ladder-name").count(),
            5,
            "every species is named"
        );
        // Empty pairs are drawn as empty cells, not as the palest step: "never heard together"
        // must not read as "a little".
        assert_eq!(svg.matches("ck-track").count(), 7);
        assert!(svg.contains("<title>Species co-occurrence matrix</title>"));
        assert!(!svg.contains("aria-label"));
    }

    /// Every name is inside the picture. A name sits after the last cell of its row, so the
    /// width is set by the furthest one: a width from the name list alone clipped the
    /// longest name when it stood at the foot.
    #[test]
    fn every_name_fits_inside_the_ladder() {
        let mut g = graph(
            6,
            &(0..6)
                .flat_map(|a| (a + 1..6).map(move |b| (a, b, 0.5)))
                .collect::<Vec<_>>(),
        );
        g.names[5] = "Ruby-throated Hummingbird".to_owned();
        g.names[0] = "Yellow-rumped Warbler".to_owned();
        let svg = cooccurrence_matrix(&g);
        let vb: Vec<f64> = svg
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("viewBox")
            .split_whitespace()
            .map(|v| v.parse().expect("number"))
            .collect();
        let mut checked = 0;
        for chunk in svg.split("<text class=\"ladder-name\" ").skip(1) {
            let at = |name: &str| -> f64 {
                let k = chunk.find(&format!("{name}=\"")).expect("attr") + name.len() + 2;
                chunk[k..]
                    .split('"')
                    .next()
                    .expect("value")
                    .parse()
                    .expect("number")
            };
            let name = chunk
                .split_once('>')
                .expect("text")
                .1
                .split('<')
                .next()
                .expect("name");
            let right = at("x") + text_width(name, NAME_FONT);
            assert!(
                right <= vb[0],
                "{name} ends at {right}, the picture is {} wide",
                vb[0]
            );
            checked += 1;
        }
        assert_eq!(checked, 6);
    }

    #[test]
    fn the_ranked_list_is_strongest_first_with_its_share() {
        let g = graph(4, &[(0, 1, 0.5), (2, 3, 1.0), (0, 3, 0.25)]);
        let html = ranked_pairs(&g, 2);
        assert_eq!(html.matches("<li ").count(), 2);
        let first = html.find("Species 2").expect("strongest first");
        let second = html.find("Species 0").expect("second");
        assert!(first < second);
        assert!(
            html.contains(">40%<"),
            "1.0 strength is 0.4 share here: {html}"
        );
        assert!(ranked_pairs(&graph(3, &[]), 5).is_empty());
    }

    #[test]
    fn the_key_says_what_a_ribbon_and_an_arc_are() {
        let g = graph(4, &[(0, 1, 1.0), (2, 3, 0.25)]);
        let k = legend(&g, 12, 2);
        assert!(k.contains("same five-minute block"), "{k}");
        assert!(
            k.contains("An arc is as long as the ribbons that end on it"),
            "{k}"
        );
        assert!(
            k.contains("10% to 40%"),
            "the measured ends, not the scaled ones: {k}"
        );
        assert_eq!(
            k.matches("ck-sw ck-q").count(),
            7,
            "all seven steps of the ramp"
        );
    }

    /// The links control offers only choices that change the picture: with nine links there is
    /// no "24", only "All", and the current choice is the one pressed.
    #[test]
    fn the_links_control_offers_only_what_changes_the_picture() {
        let pairs: Vec<(usize, usize, f64)> = (0..5)
            .flat_map(|a| (a + 1..5).map(move |b| (a, b, 0.3 + 0.05 * (a + b) as f64)))
            .collect();
        let g = graph(5, &pairs);
        // Ten links: 12 would show them all, so "12" is the same as "All".
        let few = co_figure(&g, 12, "/pages/acoustic-network");
        assert!(!few.contains("co-controls"), "nothing to show more of");

        let g8 = graph(
            8,
            &(0..8)
                .flat_map(|a| (a + 1..8).map(move |b| (a, b, 0.2 + 0.01 * (a + b) as f64)))
                .collect::<Vec<_>>(),
        );
        // 28 links: 12, 24 and all are three different pictures.
        let fig = co_figure(&g8, 12, "/pages/acoustic-network");
        assert_eq!(fig.matches("hx-get=").count(), 3, "{fig}");
        assert!(
            fig.contains(r#"aria-pressed="true" hx-get="/pages/acoustic-network?links=12""#),
            "{fig}"
        );
        let all = co_figure(&g8, 0, "/pages/acoustic-network");
        assert!(
            all.contains(r#"aria-pressed="true" hx-get="/pages/acoustic-network?links=0""#),
            "{all}"
        );
        assert_eq!(all.matches("chord-ribbon").count(), 28);
        assert_eq!(fig.matches("chord-ribbon").count(), 12);
    }

    /// Both pictures are always in the figure; which one the reader sees is the stylesheet's
    /// call, so a screen that cannot read the ring is never left with an empty card.
    #[test]
    fn the_figure_carries_the_ring_and_the_ladder() {
        let g = graph(4, &[(0, 1, 1.0), (1, 2, 0.5), (2, 3, 0.3)]);
        let fig = co_figure(&g, 12, "/pages/acoustic-network");
        assert!(fig.contains("co-chord") && fig.contains("viz-svg-chord"));
        assert!(fig.contains("co-ladder") && fig.contains("viz-svg-ladder"));
        assert!(fig.contains("Strongest pairs"));
    }
}
