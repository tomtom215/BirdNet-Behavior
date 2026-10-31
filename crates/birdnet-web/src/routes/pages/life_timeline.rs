//! The life list as a timeline of firsts: each species stands on the date it was first heard.
//!
//! `static/discovery.js` draws it from the JSON this module prints into one attribute of the card;
//! the same facts are in a table behind a disclosure, so the picture is never the only way to read
//! them, and a browser with no script still has the list ("New to the list" and the growth curve
//! are on the page beside it).
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;

use serde_json::json;

use super::{escape_html, group_thousands, simple_url_encode};

/// More species than any station has heard; a limit that keeps the attribute small however long a
/// station has run. The newest are kept.
const MAX_SPECIES: usize = 1500;

/// One species and when it joined the list.
pub(super) struct First {
    /// The stored common name (what the species page looks a bird up by).
    pub com: String,
    /// The scientific name.
    pub sci: String,
    /// The first day it was heard, `YYYY-MM-DD`.
    pub first: String,
    /// How many times it has been heard since.
    pub count: i64,
}

/// The timeline card, or nothing when the list has too little in it to be a timeline (fewer than
/// two species, or no first date that reads as a date).
pub(super) fn timeline_card(
    today: &str,
    firsts: &[First],
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let is_date = |d: &str| {
        d.len() >= 10
            && d.get(..10).is_some_and(|d| {
                d.bytes().enumerate().all(|(i, b)| match i {
                    4 | 7 => b == b'-',
                    _ => b.is_ascii_digit(),
                })
            })
    };
    let mut rows: Vec<&First> = firsts.iter().filter(|f| is_date(&f.first)).collect();
    if rows.len() < 2 {
        return String::new();
    }
    // Oldest first, ties by name; when there are more than the limit, the newest are kept.
    rows.sort_by(|a, b| {
        a.first
            .get(..10)
            .cmp(&b.first.get(..10))
            .then_with(|| a.com.cmp(&b.com))
    });
    if rows.len() > MAX_SPECIES {
        rows.drain(..rows.len() - MAX_SPECIES);
    }
    let from = rows[0].first.get(..10).unwrap_or(&rows[0].first);
    let data = json!({
        "from": from,
        "to": today,
        "species": rows.iter().map(|f| json!({
            "name": names.name(&f.sci, &f.com),
            "key": f.com,
            "sci": f.sci,
            "first": f.first.get(..10).unwrap_or(&f.first),
            "count": f.count,
            "code": super::atoms::species_code(&f.com),
            "img": format!("/api/v2/species/image/{}/file", simple_url_encode(&f.sci)),
        })).collect::<Vec<_>>(),
    });
    let mut html = String::with_capacity(4096 + rows.len() * 120);
    let _ = write!(
        html,
        r#"<div class="bnb-card pad" id="life-timeline"><div class="section-header"><div><div class="bnb-eyebrow">Firsts</div><h2 class="sh-h">When each bird joined your list</h2></div></div><div class="dsc" data-dsc data-dsc-json="{json}"></div>"#,
        json = escape_html(&data.to_string()),
    );
    html.push_str(r#"<details class="pt-disc"><summary>See the numbers — every species and the day it was first heard</summary><div class="viz-table-scroll"><table class="pt-tbl"><caption class="sr-only">Each species on the life list, newest first</caption><thead><tr><th scope="col">Species</th><th scope="col">First heard</th><th scope="col" class="num">Heard since</th></tr></thead><tbody>"#);
    for f in rows.iter().rev() {
        let _ = write!(
            html,
            r#"<tr><th scope="row"><a href="/species/detail?name={enc}">{name}</a></th><td>{first}</td><td class="num">{count}</td></tr>"#,
            enc = simple_url_encode(&f.com),
            name = escape_html(names.name(&f.sci, &f.com)),
            first = escape_html(f.first.get(..10).unwrap_or(&f.first)),
            count = group_thousands(f.count),
        );
    }
    html.push_str("</tbody></table></div></details></div>");
    // The one authored tag on this card, sealed so it gets the page's nonce; nothing a visitor can
    // influence is in it, and nothing from a name is put beside it before sealing.
    html.push_str(&crate::seal::seal(&format!(
        r#"<script type="module" src="/static/discovery.js?v={}"></script>"#,
        env!("CARGO_PKG_VERSION")
    )));
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(com: &str, date: &str, count: i64) -> First {
        First {
            com: com.into(),
            sci: format!("Sci {com}"),
            first: date.into(),
            count,
        }
    }

    fn english() -> birdnet_core::i18n::NameLocalizer {
        birdnet_core::i18n::NameLocalizer::english()
    }

    /// The card names the newest bird, orders the JSON oldest first, escapes what a name can carry,
    /// and keeps the stored name beside the shown one (the species page looks a bird up by it).
    #[test]
    fn the_card_orders_firsts_oldest_first_and_escapes_names() {
        let rows = vec![
            first("Painted Bunting", "2026-08-06", 12),
            first("Blue Jay", "2025-11-02", 900),
            first("<i>Evil</i>", "2025-11-02", 3),
        ];
        let html = timeline_card("2026-10-31", &rows, &english());
        // The table behind the picture lists the newest first.
        let first_row = html
            .split_once("<tbody>")
            .map(|(_, r)| r)
            .and_then(|r| r.split_once("</tr>"))
            .map(|(row, _)| row)
            .expect("a first table row");
        assert!(
            first_row.contains("Painted Bunting") && first_row.contains("2026-08-06"),
            "{first_row}"
        );
        let attr = html
            .split_once(r#"data-dsc-json=""#)
            .and_then(|(_, r)| r.split_once('"'))
            .map(|(a, _)| {
                a.replace("&quot;", "\"")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&")
            })
            .expect("the data attribute");
        let v: serde_json::Value = serde_json::from_str(&attr).expect("the attribute is JSON");
        let names: Vec<&str> = v["species"]
            .as_array()
            .expect("species")
            .iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert_eq!(
            names,
            ["<i>Evil</i>", "Blue Jay", "Painted Bunting"],
            "oldest first, ties by name"
        );
        assert_eq!(v["from"], "2025-11-02");
        assert_eq!(v["species"][1]["key"], "Blue Jay");
        assert!(
            !html.contains("<i>Evil"),
            "a name is data, never markup: {html}"
        );
        assert!(html.contains("&lt;i&gt;Evil"), "{html}");
    }

    /// Too little for a timeline, or dates that are not dates, is no card at all.
    #[test]
    fn a_list_too_short_or_without_dates_has_no_timeline() {
        let en = english();
        assert_eq!(timeline_card("2026-10-31", &[], &en), "");
        assert_eq!(
            timeline_card("2026-10-31", &[first("A", "2026-01-01", 1)], &en),
            ""
        );
        let junk = vec![first("A", "soon", 1), first("B", "2026-01-01", 1)];
        assert_eq!(
            timeline_card("2026-10-31", &junk, &en),
            "",
            "one usable date is not a timeline"
        );
    }
}
