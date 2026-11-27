//! The co-occurrence graph as a ladder matrix, a ranked list, and the figure that holds
//! them beside the chord diagram.
//!
//! # Why a ladder, and why it is the figure
//!
//! A matrix can say three things about a pair: heard together more often than chance, less
//! often, or no differently. A chord diagram can say only the first: every ribbon is a
//! positive, and an arc's length is the sum of its ribbons, so a species that avoids the
//! others, or is independent of them, has nothing drawn (CHT-08). The ladder is therefore the
//! figure at every width, coloured on the diverging ramp by how many times chance the pair
//! shared its blocks, and only where the pair stands out from the permutation null
//! (`birdnet_behavioral::null`); a pair within chance is an empty cell. The ring stays, folded
//! away, fed the same verdicts, and says that it shows only the positives.
//!
//! The matrix is symmetric, so drawing it whole says every pair twice. The ladder draws each
//! pair once, as a staircase, and writes each name at the foot of its own step, where the
//! eye arrives after following a cell across its row and then up its column: across to a
//! name, up to a name. Species stand in the same order as on the ring
//! ([`super::cograph::chord_order`]).
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

#[cfg(test)]
use super::cograph::share_step;
use super::cograph::{CoGraph, Link, Verdict, chord_order, lift_step, lift_words};
use super::kit::{text_width, tip};
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

/// The class a cell of the ladder is painted with: the diverging ramp's warm side for "more
/// often than chance", its cool side for "less often", and the empty track for a pair within
/// chance. A step is `1..=3` ([`lift_step`]).
fn cell_class(verdict: Verdict, lift: Option<f64>) -> String {
    match (verdict, lift) {
        (Verdict::More, Some(l)) => format!("ck-fill ck-dp{}", lift_step(l)),
        (Verdict::Less, Some(l)) => format!("ck-fill ck-dn{}", lift_step(l)),
        _ => "ck-track".to_owned(),
    }
}

/// What a pair's verdict is called, in a reading and in the table.
#[must_use]
pub const fn verdict_words(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::More => "together more often than chance",
        Verdict::Less => "together less often than chance",
        Verdict::Chance => "within chance",
    }
}

/// Every pair as a staircase of cells: coloured only where the pair stands out from chance,
/// each name at the foot of its own step.
#[must_use]
pub fn cooccurrence_matrix(g: &CoGraph) -> String {
    let n = g.n();
    if n < 2 || !g.is_square() {
        return EMPTY.to_string();
    }
    let nodes: Vec<usize> = (0..n).collect();
    // The order clusters the pairs that stand out together (positives pull their species side
    // by side); with none, it is the busiest-first order the graph came in.
    let positives = g.ranked_links();
    let order = if positives.is_empty() {
        nodes
    } else {
        chord_order(&nodes, &positives)
    };

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
    let (more, less) = g.standing_out();

    let mut svg = format!(
        r#"<svg viewBox="0 0 {width:.0} {height:.0}" width="100%" role="img" class="viz-svg-ladder" data-ck="marks" data-ck-label="Left and right arrows step through the pairs, up and down through a row" data-ck-id="ladder">"#,
    );
    svg.push_str(&svg_a11y(
        "Species co-occurrence matrix",
        &format!(
            "A staircase of {} species pairs. A cell is coloured only where the two were heard in the same five-minute block more often (warm) or less often (cool) than chance gives in the hours each is heard in; an empty cell is within chance. {more} more often, {less} less often. Each name stands at the foot of its own step: read across a row to its name and up a column to the other.",
            g.tested(),
        ),
    ));

    // Cells, then names. The reading index counts cells in document order (row by row).
    let mut index = 0usize;
    let mut start = 0usize;
    let mut best = -1.0_f64;
    for p in 1..n {
        let i = order[p];
        for (q, &j) in order.iter().enumerate().take(p) {
            let x = q as f64 * CELL;
            let y = p as f64 * CELL;
            let verdict = g.verdict(i, j);
            let lift = g.lift(i, j);
            let chance = g
                .expected
                .get(i)
                .and_then(|r| r.get(j))
                .map(|e| format!("{e:.1} blocks"));
            let against = lift.map_or_else(
                || "not worked out".to_owned(),
                |l| format!("{}, {}", lift_words(l), verdict_words(verdict)),
            );
            let mut rows: Vec<(&str, String)> =
                vec![("Heard together", format!("{} blocks", g.blocks[i][j]))];
            if let Some(c) = chance {
                rows.push(("Chance gives", c));
            }
            rows.push(("Against chance", against));
            rows.push((
                "Of the five-minute blocks either was heard in",
                pct(g.share[i][j]),
            ));
            let row_refs: Vec<(&str, &str, Option<&str>)> =
                rows.iter().map(|(a, b)| (*a, b.as_str(), None)).collect();
            let data_tip = tip(&format!("{} × {}", g.names[i], g.names[j]), &row_refs, None);
            let strength = if verdict == Verdict::Chance {
                0.0
            } else {
                lift.map_or(0.0, |l| l.max(f64::MIN_POSITIVE).ln().abs())
            };
            if strength > best {
                best = strength;
                start = index;
            }
            let _ = write!(
                svg,
                r#"<rect class="{class}" x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{w:.1}" rx="3" data-tip="{data_tip}" data-ck-i="{index}"/>"#,
                class = cell_class(verdict, lift),
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
            slot = g.class(i),
            cxd = p as f64 * CELL + 2.0,
            x2 = x + 6.0,
            name = escape_html(&names[i]),
        );
    }
    let _ = write!(svg, "</svg>");
    // The pair furthest from chance is where the keyboard lands first.
    svg.replacen(
        r#"data-ck-id="ladder">"#,
        &format!(r#"data-ck-id="ladder" data-ck-start="{start}">"#),
        1,
    )
}

/// The pairs heard together more often than chance, furthest from it first, each with its
/// lift: the list a reader meets beside the ladder. Empty when no pair stands out.
#[must_use]
pub fn ranked_pairs(g: &CoGraph, k: usize) -> String {
    let links: Vec<Link> = g.top_links(k);
    if links.is_empty() || !g.is_square() {
        return String::new();
    }
    let mut html = String::from(r#"<ol class="co-rank">"#);
    for l in &links {
        let lift = g.lift(l.a, l.b).unwrap_or(1.0);
        let _ = write!(
            html,
            r#"<li class="co-rank-row"><span class="co-rank-names">{a}<span class="co-rank-x"> × </span>{b}</span><span class="co-rank-bar"><i class="ck-sw ck-dp{step}" data-style="width:{w:.0}%"></i></span><b class="co-rank-v">{words}</b></li>"#,
            a = escape_html(&g.names[l.a]),
            b = escape_html(&g.names[l.b]),
            step = lift_step(lift),
            w = (l.v * 100.0).max(3.0),
            words = escape_html(&lift_words(lift)),
        );
    }
    html.push_str("</ol>");
    html
}

/// What a cell means, written under the picture: the diverging key, how many pairs stand out
/// on each side of chance, how chance was worked out, and what the ring leaves out.
#[must_use]
pub fn legend(g: &CoGraph) -> String {
    if g.n() < 2 {
        return String::new();
    }
    let (more, less) = g.standing_out();
    let tested = g.tested();
    let mut steps = String::new();
    for k in (1..=3).rev() {
        let _ = write!(steps, r#"<i class="ck-sw ck-dn{k}"></i>"#);
    }
    steps.push_str(r#"<i class="ck-sw ck-track-sw"></i>"#);
    for k in 1..=3 {
        let _ = write!(steps, r#"<i class="ck-sw ck-dp{k}"></i>"#);
    }
    let found = if more + less == 0 {
        format!("Of the {tested} pairs, none stands out from chance.")
    } else {
        format!(
            "Of the {tested} pairs, {more} {v1} heard together more often than chance and {less} less often.",
            v1 = if more == 1 { "was" } else { "were" },
        )
    };
    format!(
        r#"<div class="co-legend"><div class="ck-scale"><span class="ck-steps" aria-hidden="true">{steps}</span><span class="bnb-meta">Left of the middle: heard in the same five-minute block less often than chance gives; right: more often; a stronger colour is further from chance (under 1.5 times, under 3 times, beyond). An empty cell is within chance. {found}</span></div><p class="bnb-meta ck-scale-note">{method}</p></div>"#,
        method = escape_html(&g.method),
    )
}

/// How many links the ring draws, and the choices offered.
pub const LINK_CHOICES: [usize; 3] = [12, 24, 0];

/// The sentence the figure opens with when no pair stands out: said, not left to an empty
/// picture.
pub const NOTHING_STANDS_OUT: &str = "Nothing stands out: no pair was heard together more, or less, often than chance gives in the hours each bird is heard in.";

/// The whole figure: the ladder (the figure, at every width), the pairs that stand out and
/// the key beside it, and the ring folded away under it.
///
/// `max_links` of `0` means every link the ring may draw. `base` is the URL the "links shown"
/// buttons ask, without its query (the page adds the range it already holds).
#[must_use]
pub fn co_figure(g: &CoGraph, max_links: usize, base: &str) -> String {
    if g.n() < 2 || !g.is_square() {
        return EMPTY.to_string();
    }
    let all = g.ranked_links().len();
    let (more, less) = g.standing_out();
    let mut html = String::from(r#"<div class="co-fig">"#);
    if more + less == 0 {
        let _ = write!(
            html,
            r#"<p class="co-none" role="status"><strong>{NOTHING_STANDS_OUT}</strong> The {n} busiest species are below; every cell is empty because none of their pairs differs from what their hours give.</p>"#,
            n = g.n(),
        );
    }
    let rank = ranked_pairs(g, 8);
    let aside_h = if rank.is_empty() {
        String::new()
    } else {
        r#"<h3 class="co-aside-h">Together more than chance</h3>"#.to_owned()
    };
    let _ = write!(
        html,
        r#"<div class="co-main"><div class="co-ladder ck">{ladder}</div><div class="co-aside">{aside_h}{rank}{key}</div></div>"#,
        ladder = cooccurrence_matrix(g),
        key = legend(g),
    );
    if all > 0 {
        let shown = if max_links == 0 {
            all
        } else {
            max_links.min(all)
        };
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
        let controls = if all > LINK_CHOICES[0] {
            format!(
                r#"<div class="co-controls"><span class="bnb-meta" id="co-links-l">Links shown</span><div class="seg" role="group" aria-labelledby="co-links-l">{buttons}</div></div>"#,
            )
        } else {
            String::new()
        };
        let _ = write!(
            html,
            r#"<details class="pt-disc co-ring"><summary>As a ring: only the {all} pair{s} heard together more often than chance</summary><p class="bnb-meta">A ring can draw only a positive: a pair heard together less often, or no differently, has no ribbon, and an arc is as long as the ribbons that end on it. The ladder above is the whole answer.</p>{controls}<div class="co-chord ck">{chord}</div></details>"#,
            s = if all == 1 { "" } else { "s" },
            chord = chord_diagram(g, shown),
        );
    }
    html.push_str("</div>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` species named `Species k`, nine blocks shared by every pair, and the pairs given
    /// standing out at the lift given (above 1 more often than chance, below 1 less often).
    fn graph(n: usize, pairs: &[(usize, usize, f64)]) -> CoGraph {
        let mut verdict = vec![vec![Verdict::Chance; n]; n];
        let mut expected = vec![vec![9.0; n]; n];
        let mut strength = vec![vec![0.0; n]; n];
        let top = pairs
            .iter()
            .filter(|p| p.2 > 1.0)
            .map(|p| p.2.ln())
            .fold(f64::MIN_POSITIVE, f64::max);
        for &(a, b, lift) in pairs {
            let v = if lift > 1.0 {
                Verdict::More
            } else {
                Verdict::Less
            };
            verdict[a][b] = v;
            verdict[b][a] = v;
            expected[a][b] = 9.0 / lift;
            expected[b][a] = 9.0 / lift;
            if lift > 1.0 {
                strength[a][b] = lift.ln() / top;
                strength[b][a] = lift.ln() / top;
            }
        }
        CoGraph {
            slots: crate::routes::pages::viz::kit::SpeciesSlots::default(),
            names: (0..n).map(|i| format!("Species {i}")).collect(),
            share: vec![vec![0.3; n]; n],
            strength,
            blocks: vec![vec![9; n]; n],
            expected,
            verdict,
            method: "Chance is worked out by shuffling.".to_owned(),
        }
    }

    #[test]
    fn matrix_empty_for_too_few_species() {
        assert!(cooccurrence_matrix(&CoGraph::default()).contains("Not enough data"));
        assert!(cooccurrence_matrix(&graph(1, &[])).contains("Not enough data"));
        // Three species with nothing standing out are still three pairs to show as "within
        // chance": the empty answer is drawn, not withheld.
        assert!(!cooccurrence_matrix(&graph(3, &[])).contains("Not enough data"));
    }

    /// A staircase has `n * (n - 1) / 2` cells, one per pair, each carrying its own reading;
    /// only a pair that stands out is coloured, on the side of chance it falls.
    #[test]
    fn the_ladder_has_one_cell_per_pair_coloured_only_where_it_stands_out() {
        let g = graph(5, &[(0, 1, 4.0), (1, 2, 1.3), (3, 4, 0.2)]);
        let svg = cooccurrence_matrix(&g);
        assert_eq!(svg.matches("<rect ").count(), 10, "5 species, 10 pairs");
        assert_eq!(svg.matches("data-tip=").count(), 10);
        assert_eq!(
            svg.matches("ladder-name").count(),
            5,
            "every species is named"
        );
        assert_eq!(
            svg.matches("ck-track").count(),
            7,
            "within chance is an empty cell"
        );
        assert_eq!(
            svg.matches("ck-dp3").count(),
            1,
            "4 times chance is the far step"
        );
        assert_eq!(
            svg.matches("ck-dp1").count(),
            1,
            "1.3 times is the near step"
        );
        assert_eq!(
            svg.matches("ck-dn3").count(),
            1,
            "a fifth of chance is the far cool step"
        );
        assert!(
            svg.contains("Chance gives&#9;2.2 blocks"),
            "nine blocks at four times chance: {svg}"
        );
        assert!(
            svg.contains("4.0 times chance, together more often than chance"),
            "{svg}"
        );
        assert!(svg.contains("1.0 times chance, within chance"), "{svg}");
        assert!(svg.contains("<title>Species co-occurrence matrix</title>"));
        assert!(!svg.contains("aria-label"));
    }

    /// CHT-08: a figure in which nothing differs from chance says so, colours no cell and draws
    /// no ring. Counterpart: one pair standing out is coloured, listed and on the ring.
    #[test]
    fn nothing_standing_out_is_said_and_drawn_as_nothing() {
        let none = co_figure(&graph(4, &[]), 12, "/pages/acoustic-network");
        assert!(none.contains(NOTHING_STANDS_OUT), "{none}");
        assert!(
            !none.contains("ck-fill ck-dp") && !none.contains("ck-fill ck-dn"),
            "no cell is coloured: {none}"
        );
        assert!(
            !none.contains("co-ring") && !none.contains("chord-ribbon"),
            "{none}"
        );
        assert!(
            none.contains("Of the 6 pairs, none stands out from chance."),
            "{none}"
        );
        assert!(
            none.contains("Chance is worked out by shuffling."),
            "{none}"
        );

        let one = co_figure(&graph(4, &[(1, 3, 2.5)]), 12, "/pages/acoustic-network");
        assert!(!one.contains(NOTHING_STANDS_OUT), "{one}");
        assert_eq!(one.matches(r#"class="ck-fill ck-dp2""#).count(), 1, "{one}");
        assert!(one.contains("Together more than chance"), "{one}");
        assert!(one.contains("2.5 times chance"), "{one}");
        assert!(one.contains("As a ring: only the 1 pair heard together more often than chance"));
        assert_eq!(one.matches("chord-ribbon").count(), 1);
    }

    /// A pair heard together less often than chance is on the ladder and nowhere on the ring: a
    /// ring cannot draw a negative, and says so.
    #[test]
    fn a_pair_apart_is_on_the_ladder_and_not_on_the_ring() {
        let fig = co_figure(&graph(4, &[(0, 1, 0.3), (2, 3, 3.5)]), 12, "/x");
        assert_eq!(fig.matches(r#"class="ck-fill ck-dn3""#).count(), 1, "{fig}");
        assert_eq!(fig.matches("chord-ribbon").count(), 1, "only the positive");
        assert!(fig.contains("A ring can draw only a positive"), "{fig}");
        assert!(fig.contains("1 was heard together more often than chance and 1 less often"));
    }

    /// Every name is inside the picture. A name sits after the last cell of its row, so the
    /// width is set by the furthest one: a width from the name list alone clipped the
    /// longest name when it stood at the foot.
    #[test]
    fn every_name_fits_inside_the_ladder() {
        let mut g = graph(
            6,
            &(0..6)
                .flat_map(|a| (a + 1..6).map(move |b| (a, b, 2.0)))
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

    /// The list beside the ladder holds the positives, furthest from chance first, each with
    /// its lift; a pair heard together less often is not in it.
    #[test]
    fn the_ranked_list_is_the_positives_furthest_from_chance_first() {
        let g = graph(4, &[(0, 1, 1.6), (2, 3, 5.0), (0, 3, 0.25)]);
        let html = ranked_pairs(&g, 8);
        assert_eq!(html.matches("<li ").count(), 2, "{html}");
        let first = html.find("Species 2").expect("strongest first");
        let second = html.find("Species 1").expect("second");
        assert!(first < second);
        assert!(html.contains(">5.0 times chance<"), "{html}");
        assert!(ranked_pairs(&graph(3, &[]), 5).is_empty());
    }

    #[test]
    fn the_key_names_both_sides_of_chance_and_the_method() {
        let g = graph(4, &[(0, 1, 2.0), (2, 3, 0.25)]);
        let k = legend(&g);
        assert!(k.contains("less often than chance gives"), "{k}");
        assert!(k.contains("An empty cell is within chance"), "{k}");
        assert!(k.contains("Chance is worked out by shuffling."), "{k}");
        assert_eq!(k.matches("ck-sw ck-d").count(), 6, "three steps each side");
        assert_eq!(k.matches("ck-track-sw").count(), 1, "and the middle");
    }

    /// The links control offers only choices that change the ring: with ten positives there is
    /// no "24", only "All", and the current choice is the one pressed.
    #[test]
    fn the_links_control_offers_only_what_changes_the_picture() {
        let pairs: Vec<(usize, usize, f64)> = (0..5)
            .flat_map(|a| (a + 1..5).map(move |b| (a, b, 1.5 + 0.1 * (a + b) as f64)))
            .collect();
        let few = co_figure(&graph(5, &pairs), 12, "/pages/acoustic-network");
        assert!(!few.contains("co-controls"), "nothing to show more of");

        let pairs8: Vec<(usize, usize, f64)> = (0..8)
            .flat_map(|a| (a + 1..8).map(move |b| (a, b, 1.5 + 0.05 * (a + b) as f64)))
            .collect();
        let g8 = graph(8, &pairs8);
        // 28 positives: 12, 24 and all are three different rings.
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

    /// The ladder is the figure at every width; the ring is folded under it.
    #[test]
    fn the_ladder_is_the_figure_and_the_ring_is_folded() {
        let g = graph(4, &[(0, 1, 3.0), (1, 2, 2.0), (2, 3, 1.5)]);
        let fig = co_figure(&g, 12, "/pages/acoustic-network");
        let ladder = fig.find("viz-svg-ladder").expect("ladder");
        let ring = fig
            .find("<details class=\"pt-disc co-ring\">")
            .expect("ring folded");
        assert!(ladder < ring);
        assert!(fig.contains("viz-svg-chord"));
    }

    #[test]
    fn a_lift_has_a_step_on_either_side() {
        assert_eq!(lift_step(1.2), 1);
        assert_eq!(lift_step(1.0 / 1.2), 1);
        assert_eq!(lift_step(2.0), 2);
        assert_eq!(lift_step(0.5), 2);
        assert_eq!(lift_step(3.0), 3);
        assert_eq!(lift_step(0.1), 3);
        let _ = share_step(0.5);
    }
}
