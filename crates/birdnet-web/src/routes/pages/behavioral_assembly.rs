//! The Sequences tab's "chorus assembling" card: how fast the station's dawn voices come together.
//!
//! The picture is `viz::assembly`; this module makes the card around it, in the four layers every
//! analytic card has: one sentence of what was found, the picture (with the order the voices join
//! in), the table of the same numbers, and a panel of how they were made and how to take them away.
//!
//! The sentence is a function of the numbers and says its basis (how many mornings, which window),
//! and it is silent where there is nothing to say: a station whose dawn has fewer than
//! [`MIN_VOICES`] regular voices has no chorus to watch assembling, and one that has heard the
//! first voice on fewer than [`MIN_MORNINGS`] mornings has too few to read a typical one from.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;

use birdnet_behavioral::types::Assembly;

use super::viz::assembly::{Chorus, window_label};
use super::viz::chorus_figure;
use super::{escape_html, frame, simple_url_encode};

/// How many days back the card looks.
pub(super) const DAYS: i64 = 90;
/// The most voices it follows: the regular dawn voices, in the order they typically open.
pub(super) const VOICES: usize = 6;
/// Fewer regular voices than this is not a chorus.
pub(super) const MIN_VOICES: usize = 3;
/// Fewer mornings than this (a week) cannot say what a typical one is.
pub(super) const MIN_MORNINGS: usize = 7;

/// What the card is made from.
pub(super) struct Card<'a> {
    /// The voices as the station stores their names, in the order they join.
    pub stored: &'a [String],
    /// The same voices as the station displays them.
    pub shown: &'a [String],
    /// The engine's rows, newest morning first.
    pub rows: &'a [Assembly],
    /// The windows asked, in minutes.
    pub windows: &'a [u32],
    /// First and last hour of the morning looked in, inclusive.
    pub hours: (u32, u32),
    /// The first date counted.
    pub since: &'a str,
    /// The extension's version, when it reported one.
    pub version: Option<&'a str>,
}

/// The sentence that says what the picture found, or `None` when no second voice ever joined on
/// half of the mornings (then [`fewer_than_half`] says that).
///
/// "Together" is defined by the Method panel and the table: that many of the usual voices, in
/// their usual order, within that long of each other at some point in the morning.
#[must_use]
pub(super) fn headline(c: &Chorus) -> String {
    let n = c.mornings;
    let top = c.voices;
    let last = c.windows.len() - 1;
    let at = |i: usize| window_label(c.windows[i]);
    match (c.half_by(top), c.half_by(2)) {
        (Some(all), Some(two)) if top > 2 => format!(
            "On half of your {n} mornings all {top} voices were together, in order, within {}; two were together within {}.",
            at(all),
            at(two),
        ),
        (Some(all), _) => format!(
            "On half of your {n} mornings both voices were together, in order, within {}.",
            at(all),
        ),
        (None, Some(two)) => {
            // The most voices that half of the mornings had by the end, and when they had them.
            let k = (2..top)
                .rev()
                .find(|&k| c.half_by(k).is_some())
                .unwrap_or(2);
            let when = c.half_by(k).unwrap_or(two);
            let pct = c.percent(top, last).round();
            let tail = if c.reached[top - 1][last] == 0 {
                format!(
                    "all {top} never came together, in order, within {}.",
                    at(last)
                )
            } else {
                format!(
                    "all {top} came together, in order, within {} on {pct:.0}% of them.",
                    at(last)
                )
            };
            format!(
                "On half of your {n} mornings {k} voices were together, in order, within {}; {tail}",
                at(when),
            )
        }
        (None, None) => fewer_than_half(n),
    }
}

/// The sentence for a station whose second voice joins the first, in order, on fewer than half of
/// the mornings: said as a finding, since it is one.
#[must_use]
pub(super) fn fewer_than_half(mornings: usize) -> String {
    format!(
        "A second voice joined the first, in order, on fewer than half of your {mornings} mornings."
    )
}

/// The REST route that returns the same numbers, or `None` when a voice's name has a comma in it
/// (the route reads a comma-separated list, so it could not say which birds were meant).
#[must_use]
fn json_link(stored: &[String], since: &str, hours: (u32, u32)) -> Option<String> {
    if stored.iter().any(|s| s.contains(',')) {
        return None;
    }
    let species: Vec<String> = stored.iter().map(|s| simple_url_encode(s)).collect();
    Some(format!(
        "/api/v2/analytics/assembly?species={}&since={}&hour_start={}&hour_end={}",
        species.join(","),
        simple_url_encode(since),
        hours.0,
        hours.1,
    ))
}

/// The table of the numbers behind the picture, one row per window. The first column carries its
/// number alone for a spreadsheet (`data-csv`) and reads as minutes for a person.
fn table(c: &Chorus) -> String {
    let top = c.voices;
    let mut s = String::from(
        r#"<div class="viz-table-scroll"><table class="pt-tbl" id="as-table"><caption class="sr-only">Voices together, in order, by the time allowed</caption><thead><tr><th scope="col" data-csv="Within (minutes)">Within</th><th scope="col" class="num" data-csv="Typical morning (voices)">Typical morning</th><th scope="col" class="num" data-csv="Middle half, fewest voices">Middle half, from</th><th scope="col" class="num" data-csv="Middle half, most voices">to</th><th scope="col" class="num" data-csv="Mornings with two or more voices">Two or more</th><th scope="col" class="num" data-csv="Share with two or more (%)">%</th>"#,
    );
    let _ = write!(
        s,
        r#"<th scope="col" class="num" data-csv="Mornings with all {top} voices">All {top}</th><th scope="col" class="num" data-csv="Share with all {top} (%)">%</th></tr></thead><tbody>"#
    );
    for (i, &w) in c.windows.iter().enumerate() {
        let _ = write!(
            s,
            r#"<tr><th scope="row" data-csv="{w}">{label}</th><td class="num">{med}</td><td class="num">{lo}</td><td class="num">{hi}</td><td class="num">{two}</td><td class="num">{two_pct:.0}</td><td class="num">{all}</td><td class="num">{all_pct:.0}</td></tr>"#,
            label = window_label(w),
            med = c.median(i),
            lo = c.lower(i),
            hi = c.upper(i),
            two = c.reached[1][i],
            two_pct = c.percent(2, i),
            all = c.reached[top - 1][i],
            all_pct = c.percent(top, i),
        );
    }
    s.push_str("</tbody></table></div>");
    s
}

/// The voices in the order they join, as a numbered key whose swatches are the riser colours.
fn legend(shown: &[String]) -> String {
    let mut s =
        String::from(r#"<ol class="as-order" aria-label="The voices, in the order they join">"#);
    for (k, name) in shown.iter().enumerate() {
        let _ = write!(
            s,
            r#"<li><span class="as-n mono">{n}</span><span class="as-chip {slot}" aria-hidden="true"></span>{name}</li>"#,
            n = k + 1,
            slot = super::viz::kit::slot_class(k),
            name = escape_html(name),
        );
    }
    s.push_str("</ol>");
    s
}

/// The card's body.
#[must_use]
pub(super) fn render(card: &Card<'_>) -> String {
    let depths: Vec<Vec<u32>> = card.rows.iter().map(|a| a.depths.clone()).collect();
    let first = card.shown.first().map_or("", String::as_str);
    let Some(chorus) = Chorus::from_depths(card.windows, card.stored.len(), &depths) else {
        return format!(
            r#"<p class="bh-muted">{} has not been heard in the dawn hours since {}, so there is no chorus to watch assembling yet.</p>"#,
            escape_html(first),
            escape_html(card.since),
        );
    };
    if chorus.mornings < MIN_MORNINGS {
        return format!(
            r#"<p class="bh-muted">{} has been heard in the dawn hours on {} of the last {DAYS} days. The picture appears from {MIN_MORNINGS}, because a typical morning cannot be read from fewer.</p>"#,
            escape_html(first),
            chorus.mornings,
        );
    }
    let order = card
        .shown
        .iter()
        .map(|s| escape_html(s))
        .collect::<Vec<_>>()
        .join(" \u{2192} ");
    let mut html = format!(
        r#"<p class="bh-after">Your dawn builds in this order: <strong>{order}</strong>.</p>"#
    );
    html.push_str(&frame::takeaway(&headline(&chorus)));
    html.push_str(&chorus_figure(&chorus, card.shown));
    html.push_str(&legend(card.shown));
    let _ = write!(
        html,
        r#"<p class="bnb-meta fig-note">A disc's area is the share of the {n} mornings that had that many voices together, in the order above, within the time allowed. The figures at the right are the share that ever got that far within {last}; the bold line is the typical morning and the shaded band the middle half of mornings.</p>"#,
        n = chorus.mornings,
        last = window_label(card.windows[card.windows.len() - 1]),
    );
    let _ = write!(
        html,
        r#"<details class="pt-disc"><summary>See the numbers — every window</summary>{}</details>"#,
        table(&chorus),
    );

    let computed = frame::behavioral("window_funnel", card.version);
    let windows = card
        .windows
        .iter()
        .map(|&w| w.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let json = json_link(card.stored, card.since, card.hours);
    html.push_str(&frame::method(&frame::Method {
        computed: &computed,
        asked: &[
            ("Voices, in order", card.shown.join(" \u{2192} ")),
            ("Times allowed", format!("{windows} minutes")),
            (
                "Hours of the morning",
                format!("hour {} to hour {}, inclusive", card.hours.0, card.hours.1),
            ),
            ("Mornings", format!("the last {DAYS} days, from {}", card.since)),
            (
                "Mornings counted",
                format!("{} (those on which {first} was heard at all)", chorus.mornings),
            ),
        ],
        note: "A number counts mornings on which at least that many of the voices above, in that order, were heard within the time allowed of each other, at any point in the morning; a chain exactly as long as the time allowed counts. The typical morning is the most voices that at least half of the mornings had; the middle half runs from the most that three mornings in four had to the most that one in four had. Each voice has to be heard strictly later than the one before, so two birds in the same recording are not one joining the other. The step times of <code>window_funnel_events</code> are not used: it reports the latest occurrence of each voice before the next, so a gap between two of its steps depends on a tie-break and not on the birds.",
        json: &json
            .map(|href| ("The numbers as JSON", href))
            .into_iter()
            .collect::<Vec<_>>(),
        csv: &[("Download the table as CSV", "as-table", "chorus-assembling")],
    }));
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Vec<u32> {
        vec![1, 2, 5, 10, 30, 60]
    }

    fn rows(depths: &[&[u32]]) -> Vec<Assembly> {
        depths
            .iter()
            .enumerate()
            .map(|(i, d)| Assembly {
                date: format!("2026-05-{:02}", 20 - i),
                depths: d.to_vec(),
            })
            .collect()
    }

    fn names() -> (Vec<String>, Vec<String>) {
        let v: Vec<String> = ["Robin", "Blackbird", "Wren"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        (v.clone(), v)
    }

    fn chorus(depths: &[&[u32]]) -> Chorus {
        let r: Vec<Vec<u32>> = depths.iter().map(|d| d.to_vec()).collect();
        Chorus::from_depths(&windows(), 3, &r).expect("rows")
    }

    fn card_html(depths: &[&[u32]]) -> String {
        let (stored, shown) = names();
        let rows = rows(depths);
        render(&Card {
            stored: &stored,
            shown: &shown,
            rows: &rows,
            windows: &windows(),
            hours: (4, 8),
            since: "2026-08-02",
            version: Some("0.10.1"),
        })
    }

    /// Eight mornings in which the whole chorus is together within 10 minutes on four, so the
    /// typical morning has it by then.
    const BRISK: [&[u32]; 8] = [
        &[1, 2, 3, 3, 3, 3],
        &[1, 2, 3, 3, 3, 3],
        &[1, 1, 2, 3, 3, 3],
        &[1, 1, 2, 3, 3, 3],
        &[1, 1, 1, 1, 2, 3],
        &[1, 1, 1, 1, 2, 3],
        &[1, 1, 1, 1, 1, 1],
        &[1, 1, 1, 1, 1, 1],
    ];

    /// Said once and with its basis: how many mornings, which times.
    #[test]
    fn the_sentence_names_the_mornings_and_the_times_it_rests_on() {
        let c = chorus(&BRISK);
        assert_eq!(
            headline(&c),
            "On half of your 8 mornings all 3 voices were together, in order, within 10 min; two were together within 5 min."
        );
    }

    /// When fewer than half get all the way, the sentence says how far half did get and how often
    /// all of them did, which is a different claim from the first.
    #[test]
    fn a_chorus_that_is_whole_on_fewer_than_half_the_mornings_says_so() {
        let slow = chorus(&[
            &[1, 2, 2, 2, 2, 2],
            &[1, 1, 2, 2, 2, 2],
            &[1, 1, 1, 2, 2, 3],
            &[1, 1, 1, 1, 1, 1],
            &[1, 1, 1, 1, 1, 1],
            &[1, 1, 2, 2, 2, 2],
        ]);
        assert_eq!(
            headline(&slow),
            "On half of your 6 mornings 2 voices were together, in order, within 5 min; all 3 came together, in order, within 1 h on 17% of them."
        );
        // And when none ever did: said, not a zero percent.
        let never = chorus(&[&[1, 2, 2, 2, 2, 2], &[1, 2, 2, 2, 2, 2], &[1; 6]]);
        assert!(
            headline(&never).ends_with("all 3 never came together, in order, within 1 h."),
            "{}",
            headline(&never)
        );
        // Fewer than half ever get a second voice: that is the finding.
        let none = chorus(&[&[1; 6], &[1; 6], &[1, 1, 1, 1, 2, 2]]);
        assert_eq!(
            headline(&none),
            "A second voice joined the first, in order, on fewer than half of your 3 mornings."
        );
    }

    /// Four layers, in order: the sentence, the picture, the table, the method.
    #[test]
    fn the_card_has_its_four_layers_in_order() {
        let html = card_html(&BRISK);
        let at = |needle: &str| {
            html.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}: {html}"))
        };
        let (take, fig, table, method) = (
            at(r#"class="fig-take""#),
            at(r#"class="ck as-fig""#),
            at(r#"id="as-table""#),
            at(r#"class="pt-disc bh-method""#),
        );
        assert!(
            take < fig && fig < table && table < method,
            "{take} {fig} {table} {method}"
        );
        assert!(
            html.contains(
                "Your dawn builds in this order: <strong>Robin → Blackbird → Wren</strong>"
            ),
            "{html}"
        );
        assert!(html.contains("version 0.10.1"), "{html}");
    }

    /// The export names exactly the table that is there, and the JSON link carries what was asked
    /// so that it returns these numbers, not the route's defaults.
    #[test]
    fn the_export_points_at_this_table_and_this_question() {
        let html = card_html(&BRISK);
        assert!(html.contains(r##"data-csv-table="#as-table""##), "{html}");
        assert_eq!(html.matches(r#"id="as-table""#).count(), 1);
        assert!(
            html.contains("href=\"/api/v2/analytics/assembly?species=Robin,Blackbird,Wren&amp;since=2026-08-02&amp;hour_start=4&amp;hour_end=8\""),
            "{html}"
        );
        // The table has a row per window and the first column carries its number for a spreadsheet.
        assert_eq!(
            html.matches(r#"<th scope="row" data-csv="#).count(),
            6,
            "{html}"
        );
        assert!(
            html.contains(r#"<th scope="row" data-csv="60">1 h</th>"#),
            "{html}"
        );
    }

    /// A route that reads a comma-separated list cannot be told about a bird whose name has a
    /// comma: no link is better than one that returns another question's answer.
    #[test]
    fn a_name_with_a_comma_gets_no_json_link() {
        assert!(
            json_link(
                &["Robin".into(), "Thrush, Song".into()],
                "2026-08-02",
                (4, 8)
            )
            .is_none()
        );
        assert!(
            json_link(
                &["Robin".into(), "Song Thrush".into()],
                "2026-08-02",
                (4, 8)
            )
            .is_some_and(|l| l.contains("species=Robin,Song%20Thrush"))
        );
    }

    /// Too few mornings, and a first voice never heard, are said in words and draw nothing.
    #[test]
    fn too_little_to_read_is_said_and_draws_nothing() {
        let few = card_html(&[&[1, 2, 3, 3, 3, 3], &[1, 1, 1, 1, 1, 1]]);
        assert!(
            few.contains("Robin has been heard in the dawn hours on 2 of the last 90 days"),
            "{few}"
        );
        assert!(!few.contains("<svg"), "{few}");
        let none = card_html(&[&[0; 6], &[0; 6]]);
        assert!(
            none.contains("Robin has not been heard in the dawn hours since 2026-08-02"),
            "{none}"
        );
        assert!(!none.contains("<svg"));
        assert!(card_html(&[]).contains("has not been heard"));
    }

    /// Two voices are one pair: the sentence says "both" once and does not say "two of them" as well.
    #[test]
    fn a_chorus_of_two_says_both_once() {
        let two =
            Chorus::from_depths(&windows(), 2, &vec![vec![1, 2, 2, 2, 2, 2]; 8]).expect("rows");
        assert_eq!(
            headline(&two),
            "On half of your 8 mornings both voices were together, in order, within 2 min."
        );
    }

    /// Seven mornings is the least that draws a picture; six says how many there are instead.
    #[test]
    fn seven_mornings_draw_and_six_do_not() {
        let morning: &[u32] = &[1, 2, 3, 3, 3, 3];
        let seven = card_html(&vec![morning; 7]);
        assert!(
            seven.contains("<svg") && seven.contains("fig-take"),
            "{seven}"
        );
        let six = card_html(&vec![morning; 6]);
        assert!(!six.contains("<svg"), "{six}");
        assert!(six.contains("on 6 of the last 90 days"), "{six}");
    }

    /// A row of the table says what the picture says: here 10 minutes, where four of eight
    /// mornings have all three voices.
    #[test]
    fn a_row_of_the_table_says_what_the_picture_does() {
        let html = card_html(&BRISK);
        let row = html
            .split(r#"<th scope="row" data-csv="10">"#)
            .nth(1)
            .and_then(|r| r.split("</tr>").next())
            .expect("the 10 minute row");
        let cells: Vec<&str> = row
            .split(r#"<td class="num">"#)
            .skip(1)
            .map(|c| c.trim_end_matches("</td>"))
            .collect();
        // Typical morning 3; middle half from 1 to 3; two or more 4 (50%); all three 4 (50%).
        assert_eq!(cells, ["3", "1", "3", "4", "50", "4", "50"], "{row}");
        assert!(row.starts_with("10 min</th>"), "{row}");
    }

    /// The key names the voices in the order they join, each with the colour of its row.
    #[test]
    fn the_key_names_the_voices_in_order_with_their_colours() {
        let html = card_html(&BRISK);
        let key = html
            .split(r#"<ol class="as-order""#)
            .nth(1)
            .and_then(|r| r.split("</ol>").next())
            .expect("a key");
        let at = |s: &str| key.find(s).unwrap_or_else(|| panic!("{s} in {key}"));
        assert!(
            at("Robin") < at("Blackbird") && at("Blackbird") < at("Wren"),
            "{key}"
        );
        for (k, name) in [(1, "Robin"), (2, "Blackbird"), (3, "Wren")] {
            let item = key.split("<li>").nth(k).expect("an item");
            assert!(
                item.contains(&format!("as-chip ck-c{k}")) && item.contains(name),
                "{item}"
            );
        }
    }

    #[test]
    fn names_are_escaped_everywhere_they_are_printed() {
        let stored = vec!["<i>A</i>".to_owned(), "B".to_owned(), "C".to_owned()];
        let rows = rows(&BRISK);
        let html = render(&Card {
            stored: &stored,
            shown: &stored,
            rows: &rows,
            windows: &windows(),
            hours: (4, 8),
            since: "2026-08-02",
            version: None,
        });
        assert!(!html.contains("<i>A"), "{html}");
        assert!(html.contains("&lt;i&gt;A&lt;/i&gt;"));
        assert!(html.contains("version not reported"));
    }
}
