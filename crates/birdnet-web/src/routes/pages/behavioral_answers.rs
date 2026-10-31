//! "Who answers whom": after one bird is heard, which others are heard within a minute?
//!
//! The picture is a grid. A row is the bird that was heard first, a column the bird heard within a
//! minute after it, and the disc where they meet says how many times that happened (area and
//! colour are the same number, as in every heat grid here). It is not symmetric: that a Blackbird
//! follows a Robin says nothing about a Robin following a Blackbird, and the asymmetry is the
//! point. The counts come from the extension's `sequence_count` with a time constraint
//! ([`birdnet_behavioral::queries::answers_sql`]); what it counts and what it does not (the same
//! instant, a bird answering itself, a reply the next day) is on the type
//! ([`birdnet_behavioral::types::Answers`]) and held by a test against the real engine.
//!
//! A cell with fewer than [`MIN_EXCHANGES`] exchanges is drawn as an outline and says it is too few
//! to read: a pair seen twice is an anecdote, and a grid that coloured it would invite a conclusion.
#![deny(clippy::unwrap_used, clippy::expect_used)]
// Cell positions and sizes from counts and a dozen rows: nothing near 2^52, and `i`/`j`/`n` are the
// grid's indices.
#![allow(clippy::cast_precision_loss, clippy::many_single_char_names)]

use std::fmt::Write as _;

use birdnet_behavioral::types::Answers;

use super::viz::kit;
use super::{escape_html, group_thousands};

/// The fewest exchanges a cell needs before it is drawn as a finding.
pub(super) const MIN_EXCHANGES: i64 = 3;

/// The rows and columns of the grid, shortest unambiguous name first.
fn short(name: &str) -> String {
    if name.chars().count() <= 17 {
        return name.to_owned();
    }
    let cut: String = name.chars().take(16).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// One plain sentence about the strongest reply, and the most one-sided pair when there is one.
///
/// `None` when no pair reaches [`MIN_EXCHANGES`]: the grid then says so in its own words.
fn headline(a: &Answers, names: &[String]) -> Option<String> {
    let n = a.species.len();
    let mut best: Option<(usize, usize, i64)> = None;
    for i in 0..n {
        for j in 0..n {
            let v = a.exchanges[i][j];
            if i != j && v >= MIN_EXCHANGES && best.is_none_or(|(_, _, b)| v > b) {
                best = Some((i, j, v));
            }
        }
    }
    let (i, j, v) = best?;
    if v < MIN_EXCHANGES * 2 {
        // At the floor a pair is a hint, and a headline would read it as a finding.
        return Some(format!(
            "Nothing stands out yet: the most often one bird was heard within a minute after another is {v} times ({} then {}).",
            names[i], names[j]
        ));
    }
    let mut out = format!(
        "{} is most often heard within a minute after {}: {} times.",
        names[j],
        names[i],
        group_thousands(v)
    );
    // The most lopsided pair among those with enough to say: the reply is common one way and rare
    // the other.
    let mut lopsided: Option<(usize, usize, i64, i64)> = None;
    for p in 0..n {
        for q in (p + 1)..n {
            let (fwd, back) = (a.exchanges[p][q], a.exchanges[q][p]);
            let (hi, lo) = (fwd.max(back), fwd.min(back));
            if hi >= MIN_EXCHANGES * 2
                && hi >= lo * 3
                && lopsided.is_none_or(|(_, _, h, l)| hi - lo > h - l)
            {
                lopsided = Some(if fwd >= back {
                    (p, q, fwd, back)
                } else {
                    (q, p, back, fwd)
                });
            }
        }
    }
    if let Some((p, q, hi, lo)) = lopsided {
        if (q, p) == (j, i) {
            // The strongest reply is the one-sided pair: say how rarely it goes the other way, once.
            let _ = write!(
                out,
                " The other way round, {} is heard after {} only {}.",
                names[p],
                names[q],
                group_thousands(lo)
            );
        } else {
            let _ = write!(
                out,
                " The most one-sided pair: {} is heard after {} {} times, {} after {} only {}.",
                names[q],
                names[p],
                group_thousands(hi),
                names[p],
                names[q],
                group_thousands(lo)
            );
        }
    }
    Some(out)
}

/// The card's body: a sentence, the grid, what a disc means, and the table of the same numbers.
///
/// `shown` are the species' names in the station's language, in the order of `a.species`.
pub(super) fn render(a: &Answers, shown: &[String], window_days: u32) -> String {
    let n = a.species.len();
    if n < 2 || shown.len() != n {
        return String::from(
            r#"<p class="bnb-meta viz-empty">Not enough different birds heard yet to see who answers whom.</p>"#,
        );
    }
    let max = a
        .exchanges
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(0)
        .max(1);
    let (label_w, cell, head) = (132.0_f64, 40.0_f64, 30.0_f64);
    let w = label_w + n as f64 * cell + 8.0;
    let h = head + n as f64 * cell + 6.0;
    let r_max = cell / 2.0 - 3.0;
    let steps: Vec<String> = (1..=7).map(|k| format!("ck-q{k}")).collect();

    let mut svg = String::with_capacity(8 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><div class="hm-scroll" tabindex="0" role="group" aria-label="Who answers whom, scrolls sideways"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="hm-svg ck-in" data-ck="marks" data-ck-id="answers" data-ck-grid="{n}" data-ck-dim="index" data-ck-label="Left and right move across the birds that answer, up and down between the birds that sing first.">"#
    );
    svg.push_str(&super::viz::svg_a11y(
        "Who answers whom",
        "A grid. Each row is a bird heard first and each column a bird heard within a minute after it; the disc where they meet is how many times that happened.",
    ));
    let _ = write!(
        svg,
        "<defs>{}</defs>",
        kit::halo_gradients("answers-hg", &steps)
    );

    // Column heads: a colour mark and the bird's code.
    for (j, name) in a.species.iter().enumerate() {
        let cx = label_w + j as f64 * cell + cell / 2.0;
        let _ = write!(
            svg,
            r#"<circle class="ck-ring {slot}" cx="{cx:.1}" cy="9" r="4.5"/><text class="mono ck-lbl" x="{cx:.1}" y="25" text-anchor="middle" font-size="10">{code}</text>"#,
            slot = kit::slot_class(j),
            code = escape_html(&super::atoms::species_code(name)),
        );
    }
    let (mut halos, mut discs, mut hits) = (String::new(), String::new(), String::new());
    let mut peak: Option<(usize, usize)> = None;
    let mut peak_v = 0;
    for (i, row) in a.exchanges.iter().enumerate() {
        let y0 = head + i as f64 * cell;
        let cy = y0 + cell / 2.0;
        let _ = write!(
            svg,
            r#"<circle class="ck-ring {slot}" cx="10" cy="{cy:.1}" r="4.5"/><text class="ck-lbl" x="22" y="{ty:.1}" font-size="10.5">{name}</text><line class="ck-grid" x1="{gx0:.1}" y1="{cy:.1}" x2="{gx1:.1}" y2="{cy:.1}"/>"#,
            slot = kit::slot_class(i),
            ty = cy + 3.5,
            name = escape_html(&short(&shown[i])),
            gx0 = label_w + cell / 2.0,
            gx1 = label_w + (n as f64 - 0.5) * cell,
        );
        for (j, &v) in row.iter().enumerate() {
            let cx = label_w + j as f64 * cell + cell / 2.0;
            let idx = i * n + j;
            if i == j {
                let _ = write!(
                    svg,
                    r#"<line class="ck-grid-strong" x1="{x0:.1}" y1="{cy:.1}" x2="{x1:.1}" y2="{cy:.1}"/>"#,
                    x0 = cx - 5.0,
                    x1 = cx + 5.0,
                );
                continue;
            }
            let per_100 = if a.heard[i] > 0 {
                v as f64 * 100.0 / a.heard[i] as f64
            } else {
                0.0
            };
            let heading = format!("{} \u{2192} {}", shown[i], shown[j]);
            let tip = if v >= MIN_EXCHANGES {
                let t = v as f64 / max as f64;
                let class = format!("ck-q{}", kit::seq_step(t));
                let r = (r_max * t.sqrt()).max(2.4);
                let _ = write!(
                    halos,
                    r#"<circle class="ck-halo-d" cx="{cx:.1}" cy="{cy:.1}" r="{hr:.1}" fill="url(#answers-hg-{class})" data-ck-i="{idx}"/>"#,
                    hr = r + 5.0,
                );
                let _ = write!(
                    discs,
                    r#"<circle class="ck-disc {class}" cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" data-ck-i="{idx}"/>"#
                );
                if v > peak_v {
                    peak_v = v;
                    peak = Some((i, j));
                }
                kit::tip(
                    &heading,
                    &[
                        ("Heard within a minute after", &group_thousands(v), None),
                        ("Per 100 songs of the first", &format!("{per_100:.0}"), None),
                    ],
                    Some(&format!(
                        "{} heard {} times in {} days",
                        shown[i],
                        group_thousands(a.heard[i]),
                        a.days_heard
                    )),
                )
            } else if v > 0 {
                let _ = write!(
                    discs,
                    r#"<circle class="ck-sparse" cx="{cx:.1}" cy="{cy:.1}" r="2.6" data-ck-i="{idx}"/>"#
                );
                kit::tip(
                    &heading,
                    &[("Heard within a minute after", &group_thousands(v), None)],
                    Some("Too few times to read as a pattern"),
                )
            } else {
                let _ = write!(
                    discs,
                    r#"<circle class="ck-none" cx="{cx:.1}" cy="{cy:.1}" r="1.2" data-ck-i="{idx}"/>"#
                );
                kit::tip(&heading, &[], Some("Never heard within a minute after"))
            };
            let _ = write!(
                hits,
                r#"<rect class="ck-hit" x="{x:.1}" y="{y0:.1}" width="{cell:.1}" height="{cell:.1}" data-ck-i="{idx}" data-tip="{tip}"/>"#,
                x = label_w + j as f64 * cell,
            );
        }
    }
    svg.push_str(&halos);
    svg.push_str(&discs);
    if let Some((i, j)) = peak {
        let _ = write!(
            svg,
            r#"<circle class="ck-peak" cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}"/>"#,
            cx = label_w + j as f64 * cell + cell / 2.0,
            cy = head + i as f64 * cell + cell / 2.0,
            r = r_max + 3.0,
        );
    }
    svg.push_str(&hits);
    svg.push_str("</svg></div>");

    let names: Vec<String> = shown.to_vec();
    let lead = headline(a, &names).unwrap_or_else(|| {
        format!(
            "No pair was heard within a minute of each other at least {MIN_EXCHANGES} times in these {window_days} days."
        )
    });
    let mut html = String::with_capacity(svg.len() + 2048);
    let _ = write!(
        html,
        r#"<p class="fig-take">{lead}</p>{svg}<div class="ck-scale ck-scale--discs"><span class="bnb-meta">Read across: the bird on the left was heard first; the disc says how often the bird above was heard within {within} after it.</span></div><p class="bnb-meta">The largest disc here is {max} times; a disc's area is the count. {days} days with these birds in the last {window_days}. Two birds heard at the same instant are not replies, and neither is a bird repeating itself. A small ring is fewer than {MIN_EXCHANGES} times: too few to read.</p>"#,
        lead = escape_html(&lead),
        within = if a.within_secs == 60 {
            "a minute".to_string()
        } else {
            format!("{} seconds", a.within_secs)
        },
        days = a.days_heard,
    );
    html.push_str(&table(a, shown));
    html
}

/// The same numbers as a table: every ordered pair, the one that was heard first as the row.
fn table(a: &Answers, shown: &[String]) -> String {
    let mut t = String::from(
        r#"<details class="pt-disc"><summary>See the numbers — who was heard within a minute after whom</summary><div class="viz-table-scroll"><table class="pt-tbl" id="aw-table"><caption class="sr-only">Times the column bird was heard within a minute after the row bird</caption><thead><tr><th scope="col">First heard</th><th scope="col" class="num">Songs</th>"#,
    );
    for s in shown {
        let _ = write!(
            t,
            r#"<th scope="col" class="num">{}</th>"#,
            escape_html(&short(s))
        );
    }
    t.push_str("</tr></thead><tbody>");
    for (i, row) in a.exchanges.iter().enumerate() {
        let _ = write!(
            t,
            r#"<tr><th scope="row">{}</th><td class="num">{}</td>"#,
            escape_html(&shown[i]),
            group_thousands(a.heard[i])
        );
        for (j, &v) in row.iter().enumerate() {
            if i == j {
                t.push_str("<td class=\"num\">\u{2014}</td>");
            } else {
                let _ = write!(t, r#"<td class="num">{}</td>"#, group_thousands(v));
            }
        }
        t.push_str("</tr>");
    }
    t.push_str("</tbody></table></div></details>");
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(exchanges: Vec<Vec<i64>>) -> (Answers, Vec<String>) {
        let n = exchanges.len();
        let species: Vec<String> = [
            "European Robin",
            "Eurasian Blackbird",
            "Eurasian Wren",
            "Song Thrush",
        ]
        .iter()
        .take(n)
        .map(|s| (*s).to_string())
        .collect();
        (
            Answers {
                heard: vec![100; n],
                days_heard: 20,
                within_secs: 60,
                species: species.clone(),
                exchanges,
            },
            species,
        )
    }

    /// A pair below the floor is an outline and says it is too few; a pair above it is a disc; a
    /// pair never heard is an empty mark; and the grid is not symmetric.
    #[test]
    fn a_cell_is_a_disc_an_outline_or_empty_by_how_much_there_is_to_say() {
        let (a, names) = answers(vec![vec![0, 30, 2], vec![4, 0, 0], vec![0, 0, 0]]);
        let html = render(&a, &names, 30);
        assert_eq!(
            html.matches(r#"class="ck-disc "#).count(),
            2,
            "30 and 4 are findings: {html}"
        );
        assert_eq!(
            html.matches(r#"class="ck-sparse""#).count(),
            1,
            "2 is too few: {html}"
        );
        assert!(
            html.contains("Too few times to read as a pattern"),
            "{html}"
        );
        assert!(html.contains("Never heard within a minute after"), "{html}");
        assert!(
            html.contains("European Robin \u{2192} Eurasian Blackbird"),
            "row speaks first: {html}"
        );
        assert!(html.contains("Eurasian Blackbird is most often heard within a minute after European Robin: 30 times."), "{html}");
        // A best pair at the floor is a hint, not a standout, and the grid says so.
        let (a, names) = answers(vec![vec![0, 3], vec![1, 0]]);
        assert!(
            render(&a, &names, 30).contains("Nothing stands out yet"),
            "a pair at the floor is not a headline"
        );
    }

    /// The one-sided pair is named, and a pair that is not one-sided is not made to look so.
    #[test]
    fn a_one_sided_pair_is_named_and_a_balanced_one_is_not() {
        let (a, names) = answers(vec![vec![0, 30, 0], vec![4, 0, 0], vec![0, 0, 0]]);
        let lead = headline(&a, &names).expect("a headline");
        assert!(
            lead.contains(
                "The other way round, European Robin is heard after Eurasian Blackbird only 4."
            ),
            "the strongest pair is the one-sided one, so it is said once: {lead}"
        );
        assert!(!lead.contains("The most one-sided pair"), "{lead}");
        // When the one-sided pair is not the strongest, it is named in full.
        let (a, names) = answers(vec![
            vec![0, 40, 0, 0],
            vec![30, 0, 0, 0],
            vec![0, 0, 0, 20],
            vec![0, 0, 4, 0],
        ]);
        let lead = headline(&a, &names).expect("a headline");
        assert!(
            lead.contains("The most one-sided pair: Song Thrush is heard after Eurasian Wren 20 times, Eurasian Wren after Song Thrush only 4."),
            "{lead}"
        );
        let (a, names) = answers(vec![vec![0, 30, 0], vec![25, 0, 0], vec![0, 0, 0]]);
        let lead = headline(&a, &names).expect("a headline");
        // 30 against 25 is a balanced pair: the plain sentence and nothing about the way back. (A
        // check for "one-sided" alone passes when the other-way sentence is added by mistake.)
        assert_eq!(
            lead,
            "Eurasian Blackbird is most often heard within a minute after European Robin: 30 times."
        );
    }

    /// Nothing at the floor says so in words, and fewer than two birds is no grid at all.
    #[test]
    fn nothing_to_say_is_said_and_one_bird_is_no_grid() {
        let (a, names) = answers(vec![vec![0, 2], vec![1, 0]]);
        let html = render(&a, &names, 30);
        assert!(
            html.contains("No pair was heard within a minute of each other at least 3 times"),
            "{html}"
        );
        let (one, names) = answers(vec![vec![0]]);
        assert!(render(&one, &names, 30).contains("Not enough different birds"));
    }

    /// A name is data: markup in it is escaped, and a long one is cut for the label but kept whole
    /// in the tooltip and the table.
    #[test]
    fn names_are_escaped_and_long_ones_are_cut_for_the_label_only() {
        let (mut a, mut names) = answers(vec![vec![0, 9], vec![9, 0]]);
        names[0] = "<i>Extraordinarily Long-named Warbler</i>".to_string();
        a.species[0] = names[0].clone();
        let html = render(&a, &names, 30);
        assert!(!html.contains("<i>Extra"), "{html}");
        assert!(
            html.contains("&lt;i&gt;Extraordinarily Long-named Warbler&lt;/i&gt;"),
            "whole in the table: {html}"
        );
        assert!(html.contains('\u{2026}'), "the label is cut: {html}");
    }
}
