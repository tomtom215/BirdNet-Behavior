//! The frame every analytic page and chart card is built in.
//!
//! Before this, each page wrote its own opening: twenty-nine `*-h1` classes, twenty-four
//! `*-lede` classes, margins that differed by two pixels and a help link that sat in the
//! eyebrow on four tabs and under the heading on two. A chart card was a `bnb-card` with a
//! hand-assembled `section-header`, and whether it used `h2` or `h3`, or said how to read the
//! chart or what the chart found, was up to whoever wrote it. The reader met six pages that
//! looked related and behaved differently.
//!
//! Three pieces, the first two thin wrappers over classes that already exist:
//!
//! * [`page_head`], the opening of a page or tab: eyebrow (with the help link in the same
//!   place on every page), headline, one lead sentence, and whatever controls the page owns.
//! * [`figure`], a chart card: a topic line that numbers itself ("Fig. 3 · …", by CSS
//!   counter, so the number follows the page and not the author), a plain title, the chart,
//!   and an optional "how to read this" caption under it. What the chart *found* is [`takeaway`], written by
//!   the chart's own partial from the numbers it already holds (see [`super::insight`]).
//!
//! * [`method`], the fourth layer of an analytic card: what computed the numbers, what was asked of
//!   it, and how to take them away (a link to the REST route that returns them, and the table as a
//!   CSV). Every analytic figure has the same four layers in the same order: what it found, the
//!   picture, a closer look, and this.
//!
//! The one definition of dawn the pages share is here too.

use std::fmt::Write as _;

use super::escape_html;

/// First hour of the dawn window the pages shade and count (05:00).
///
/// One definition for the When and Dawn tabs and Today's stat trio. Two others exist on
/// purpose and are named where they live: the morning running order on the Behavior tab
/// watches 04:00 to 08:59 because it is looking for the *first* voice, and the public API
/// keeps upstream's 04:00 to 09:59 for parity with BirdNET-Pi.
pub const DAWN_FIRST_HOUR: usize = 5;
/// Last hour of the dawn window (08:00 to 08:59), inclusive.
pub const DAWN_LAST_HOUR: usize = 8;

/// Whether `hour` (0 to 23) is inside the dawn window.
#[must_use]
pub const fn is_dawn_hour(hour: usize) -> bool {
    hour >= DAWN_FIRST_HOUR && hour <= DAWN_LAST_HOUR
}

/// The "Save as PDF" button the report pages carry in their head. Its handler is the one script
/// `homes/reports.rs` writes into the page.
pub const PRINT_BUTTON: &str = r#"<button type="button" class="bnb-btn ghost rp-pdf" data-print data-print-hide>⎙ Save as PDF</button>"#;

/// The opening of a page or tab.
pub struct Head<'a> {
    /// Small-caps line above the headline. Text.
    pub eyebrow: &'a str,
    /// The "how this works" link, already rendered (`help::help_link`), or empty. It is
    /// placed in the eyebrow on every page, so it is always in the same place.
    pub help: &'a str,
    /// The headline. Authored markup: one `<em>` for the italic accent word, if any.
    pub title: &'a str,
    /// The bold first sentence of the lede. Text; may be empty.
    pub lead: &'a str,
    /// The rest of the lede. Authored markup.
    pub lede: &'a str,
    /// Controls or pills the page owns, on the right. Authored markup, or empty.
    pub aside: &'a str,
}

/// Render a [`Head`].
#[must_use]
pub fn page_head(h: &Head<'_>) -> String {
    let lead = if h.lead.is_empty() {
        String::new()
    } else {
        format!("<b>{}</b> ", escape_html(h.lead))
    };
    format!(
        r#"<div class="page-head pg-head">
  <div class="pg-main">
    <div class="bnb-eyebrow pg-eyebrow"><span>{eyebrow}</span>{help}</div>
    <h1 class="display pg-h1">{title}</h1>
    <p class="bnb-lede pg-lede">{lead}{lede}</p>
  </div>
  {aside}
</div>"#,
        eyebrow = escape_html(h.eyebrow),
        help = h.help,
        title = h.title,
        lede = h.lede,
        aside = h.aside,
    )
}

/// A chart card.
pub struct Figure<'a> {
    /// The topic: small-caps, after the figure number. Text.
    pub topic: &'a str,
    /// The card's title, an `h2`. Text. A plain name of what is drawn; the finding goes in
    /// the chart's [`takeaway`].
    pub title: &'a str,
    /// A link at the right of the title row: `(href, label)`. Both text.
    pub action: Option<(&'a str, &'a str)>,
    /// How to read the chart: one or two sentences, authored markup, set under the chart like a
    /// journal figure's caption. Empty for none.
    pub note: &'a str,
    /// The chart, or the slot a partial fills. Authored markup.
    pub body: &'a str,
}

/// Render a [`Figure`].
#[must_use]
pub fn figure(f: &Figure<'_>) -> String {
    let action = f.action.map_or_else(String::new, |(href, label)| {
        format!(
            r#"<a class="action" href="{}">{}</a>"#,
            escape_html(href),
            escape_html(label)
        )
    });
    let note = if f.note.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="bnb-meta fig-note">{}</p>"#, f.note)
    };
    format!(
        r#"<div class="bnb-card pad fig">
  <div class="section-header"><div><div class="bnb-eyebrow"><span class="fig-no" aria-hidden="true"></span>{topic}</div><h2 class="sh-h">{title}</h2></div>{action}</div>
  {body}
  {note}
</div>"#,
        topic = escape_html(f.topic),
        title = escape_html(f.title),
        body = f.body,
    )
}

/// What a chart found, as one sentence above the chart. `html` is authored markup: the
/// functions in [`super::insight`] return it with their text escaped and their times marked,
/// so a caller cannot hand this a raw string by accident.
#[must_use]
pub fn takeaway(html: &str) -> String {
    format!(r#"<p class="fig-take">{html}</p>"#)
}

/// What was done to get a figure's numbers, and how to take them away: the fourth layer of an
/// analytic card (at a glance, the picture, a closer look, then this).
pub struct Method<'a> {
    /// What computed the numbers, as authored markup: [`behavioral`] for the extension's
    /// functions, or a plain sentence for a query that is only SQL.
    pub computed: &'a str,
    /// What was asked, as `(label, value)`: the window, the hours, the days, the voices. Text.
    pub asked: &'a [(&'a str, String)],
    /// What a number in the figure counts, in a sentence or two. Authored markup, or empty.
    pub note: &'a str,
    /// The REST routes (with their queries) that return the same numbers, as `(label, href)`;
    /// empty where there is none, because a link is never made up.
    pub json: &'a [(&'a str, String)],
    /// The tables a button saves as CSV, as `(button label, table id, file name without
    /// extension)`; empty for none.
    pub csv: &'a [(&'a str, &'a str, &'a str)],
}

/// How a figure says it was computed by one of `duckdb-behavioral`'s functions: the function
/// in code, and the extension's version when the engine reported one (and that it did not, when
/// it did not, because a method with its version missing is not the same method).
#[must_use]
pub fn behavioral(function: &str, version: Option<&str>) -> String {
    let version = version.map_or_else(
        || "version not reported".to_owned(),
        |v| format!("version {}", escape_html(v)),
    );
    format!(
        "<code>{}</code>, from the <a href=\"https://github.com/tomtom215/duckdb-behavioral\">duckdb-behavioral</a> extension, {version}",
        escape_html(function),
    )
}

/// Render a [`Method`]: a disclosure closed by default, so a figure reads as its picture first.
///
/// The CSV button is `hidden` until `static/figure-export.js` shows it, so a page with no script
/// has no control that does nothing; the JSON link is an ordinary link.
#[must_use]
pub fn method(m: &Method<'_>) -> String {
    let mut s = String::from(
        r#"<details class="pt-disc bh-method"><summary>Method and export</summary><div><dl class="pt-def">"#,
    );
    let _ = write!(s, "<dt>Computed with</dt><dd>{}</dd>", m.computed);
    for (label, value) in m.asked {
        let _ = write!(
            s,
            "<dt>{}</dt><dd>{}</dd>",
            escape_html(label),
            escape_html(value)
        );
    }
    s.push_str("</dl>");
    if !m.note.is_empty() {
        let _ = write!(s, r#"<p class="bnb-meta">{}</p>"#, m.note);
    }
    if !m.json.is_empty() || !m.csv.is_empty() {
        s.push_str(r#"<p class="bh-method-links">"#);
        for (label, href) in m.json {
            let _ = write!(
                s,
                r#"<a class="bnb-btn ghost" href="{}" type="application/json">{}</a>"#,
                escape_html(href),
                escape_html(label)
            );
        }
        for (label, table, name) in m.csv {
            let _ = write!(
                s,
                r##"<button type="button" class="bnb-btn ghost" data-csv-table="#{}" data-csv-name="{}" hidden>{}</button>"##,
                escape_html(table),
                escape_html(name),
                escape_html(label)
            );
        }
        s.push_str("</p>");
    }
    s.push_str("</div></details>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel says what computed the numbers and with which version, then what was asked, and
    /// it offers only the exports it has: a missing route is no link, not a dead one.
    #[test]
    fn the_method_panel_names_the_function_the_version_and_what_was_asked() {
        let computed = behavioral("window_funnel", Some("0.10.1"));
        let html = method(&Method {
            computed: &computed,
            asked: &[("Hours", "04:00 to 08:59".to_owned())],
            note: "A depth is how many voices.",
            json: &[(
                "The numbers as JSON",
                "/api/v2/analytics/assembly?since=2026-08-01".to_owned(),
            )],
            csv: &[("Download the table as CSV", "as-table", "chorus")],
        });
        assert!(
            html.starts_with(r#"<details class="pt-disc bh-method"><summary>Method and export"#)
        );
        assert!(
            html.contains("<code>window_funnel</code>") && html.contains("version 0.10.1"),
            "{html}"
        );
        assert!(
            html.contains("<dt>Hours</dt><dd>04:00 to 08:59</dd>"),
            "{html}"
        );
        assert!(
            html.contains(r#"href="/api/v2/analytics/assembly?since=2026-08-01""#),
            "{html}"
        );
        assert!(
            html.contains(r##"data-csv-table="#as-table" data-csv-name="chorus" hidden"##),
            "the button waits for its script: {html}"
        );
        // Closed by default: a figure leads with its picture.
        assert!(!html.contains("open"), "{html}");
    }

    #[test]
    fn a_missing_version_is_said_and_a_missing_export_is_not_offered() {
        let computed = behavioral("sessionize", None);
        assert!(computed.contains("version not reported"), "{computed}");
        let html = method(&Method {
            computed: &computed,
            asked: &[],
            note: "",
            json: &[],
            csv: &[],
        });
        assert!(
            !html.contains("bh-method-links"),
            "no export, no row of links: {html}"
        );
        assert!(!html.contains("<a class=\"bnb-btn"), "{html}");
        assert!(!html.contains("bnb-meta"), "no note, no paragraph: {html}");
    }

    #[test]
    fn what_was_asked_and_the_version_are_text_not_markup() {
        let computed = behavioral("<b>f</b>", Some("<i>1</i>"));
        let html = method(&Method {
            computed: &computed,
            asked: &[("<x>", "<script>alert(1)</script>".to_owned())],
            note: "",
            json: &[("<j>", "/a?b=\"><script>".to_owned())],
            csv: &[("<c>", "a\"b", "n")],
        });
        assert!(
            !html.contains("<script>") && !html.contains("<b>f") && !html.contains("<i>1"),
            "{html}"
        );
        assert!(!html.contains(r#"?b="><"#), "{html}");
        assert!(
            html.contains("&lt;x&gt;") && html.contains("&lt;script&gt;"),
            "{html}"
        );
        assert!(
            html.contains("&lt;j&gt;") && html.contains("&lt;c&gt;"),
            "labels are text: {html}"
        );
    }

    #[test]
    fn the_dawn_window_is_five_to_eight() {
        let hours: Vec<usize> = (0..24).filter(|&h| is_dawn_hour(h)).collect();
        assert_eq!(hours, vec![5, 6, 7, 8]);
    }

    #[test]
    fn a_head_puts_the_help_link_in_the_eyebrow_and_the_lead_in_bold() {
        let html = page_head(&Head {
            eyebrow: "Patterns · when",
            help: r#"<a class="help" href="/help/">?</a>"#,
            title: "When the yard is <em>alive</em>",
            lead: "When your birds are out.",
            lede: "Four views of the same days.",
            aside: "",
        });
        let eyebrow = html
            .split(r#"<div class="bnb-eyebrow pg-eyebrow">"#)
            .nth(1)
            .and_then(|s| s.split("</div>").next())
            .expect("an eyebrow");
        assert!(eyebrow.contains("Patterns · when"), "{eyebrow}");
        assert!(eyebrow.contains(r#"class="help""#), "{eyebrow}");
        assert!(
            html.contains("<b>When your birds are out.</b> Four views"),
            "{html}"
        );
        assert!(html.contains("<em>alive</em>"));
    }

    #[test]
    fn text_fields_are_escaped_and_authored_ones_are_not() {
        let html = page_head(&Head {
            eyebrow: "<script>",
            help: "",
            title: "A <em>b</em>",
            lead: "x & y",
            lede: "<code>z</code>",
            aside: "",
        });
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(!html.contains("<script>"));
        assert!(html.contains("<b>x &amp; y</b>"));
        assert!(html.contains("<code>z</code>"));
    }

    #[test]
    fn a_figure_numbers_itself_with_css_and_names_one_h2() {
        let html = figure(&Figure {
            topic: "Hour by hour",
            title: "Detections by hour",
            action: Some(("/patterns?tab=dawn", "See who sings when →")),
            note: "Totals for every hour.",
            body: "<div id=\"slot\"></div>",
        });
        assert!(
            html.contains(r#"class="fig-no" aria-hidden="true""#),
            "{html}"
        );
        assert_eq!(html.matches("<h2 ").count(), 1);
        assert!(html.contains(r#"href="/patterns?tab=dawn""#));
        assert!(html.contains(r#"<div id="slot"></div>"#));
        assert!(html.contains("fig-note"));
        // The caption sits under the chart, as in a journal.
        assert!(
            html.find(r#"id="slot""#) < html.find("fig-note"),
            "the how-to-read note follows the chart: {html}"
        );
    }

    #[test]
    fn a_figure_without_a_note_or_action_has_neither() {
        let html = figure(&Figure {
            topic: "t",
            title: "T",
            action: None,
            note: "",
            body: "",
        });
        assert!(
            !html.contains("fig-note") && !html.contains("class=\"action\""),
            "{html}"
        );
    }

    #[test]
    fn a_takeaway_is_one_paragraph_of_the_markup_it_is_given() {
        assert_eq!(
            takeaway(r#"Peaks at <span class="clk" data-clk="07:00">07:00</span>."#),
            r#"<p class="fig-take">Peaks at <span class="clk" data-clk="07:00">07:00</span>.</p>"#
        );
    }
}
