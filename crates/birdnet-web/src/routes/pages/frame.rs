//! The frame every analytic page and chart card is built in.
//!
//! Before this, each page wrote its own opening: twenty-nine `*-h1` classes, twenty-four
//! `*-lede` classes, margins that differed by two pixels and a help link that sat in the
//! eyebrow on four tabs and under the heading on two. A chart card was a `bnb-card` with a
//! hand-assembled `section-header`, and whether it used `h2` or `h3`, or said how to read the
//! chart or what the chart found, was up to whoever wrote it. The reader met six pages that
//! looked related and behaved differently.
//!
//! Two pieces, both thin wrappers over classes that already exist:
//!
//! * [`page_head`], the opening of a page or tab: eyebrow (with the help link in the same
//!   place on every page), headline, one lead sentence, and whatever controls the page owns.
//! * [`figure`], a chart card: a topic line that numbers itself ("Fig. 3 · …", by CSS
//!   counter, so the number follows the page and not the author), a plain title, the chart,
//!   and an optional "how to read this" caption under it. What the chart *found* is [`takeaway`], written by
//!   the chart's own partial from the numbers it already holds (see [`super::insight`]).
//!
//! The one definition of dawn the pages share is here too.

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

#[cfg(test)]
mod tests {
    use super::*;

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
