//! Picture credits: who made the pictures this station shows, and on what terms.
//!
//! A photograph shown under a Creative Commons licence has to be shown with its author, its
//! source and its licence. For years the station fetched pictures from Wikipedia, AviCommons and
//! Flickr and printed none of that anywhere: the credit existed, for two of the three, only in
//! the `description` of a JSON answer no page read. The credit is now captured when a picture is
//! fetched (`birdnet_integrations::species_images::ImageCredit`), kept beside the file, and shown
//! in three places:
//!
//! * under the picture on the species page ([`credit_html`]);
//! * in the footer of every page, as a link to this one, which lists every credited picture the
//!   station holds for a bird it has heard ([`page`]);
//! * in the kiosk plate's foot, which names the works its plates are after and points here. The
//!   public-domain plates themselves are listed here too, with the books they come from.
//!
//! The kiosk engraves photographs, which is an adaptation, and the page says so.

use std::fmt::Write as _;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::routing::get;
use birdnet_integrations::species_images::ImageCredit;

use super::frame::{Figure, Head, figure, page_head};
use super::{escape_html, render_page_for_request};
use crate::state::AppState;

/// Routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/system/credits", get(page))
}

/// `url` when it is an `http` or `https` address, which is all a credit may link to: it comes
/// from a provider's data, and `javascript:` is also a URL.
#[must_use]
pub fn safe_href(url: &str) -> Option<&str> {
    let url = url.trim();
    (url.starts_with("https://") || url.starts_with("http://")).then_some(url)
}

/// `label` as a link to `url` when `url` is one a credit may link to, else as plain text.
fn link(url: &str, label: &str) -> String {
    safe_href(url).map_or_else(
        || escape_html(label),
        |href| {
            format!(
                r#"<a href="{}" target="_blank" rel="noopener noreferrer">{}</a>"#,
                escape_html(href),
                escape_html(label)
            )
        },
    )
}

/// One picture's credit as a line of markup: the title and author, then the source and the
/// licence, each a link where the provider gave an address. Every field is escaped.
#[must_use]
pub fn credit_html(credit: &ImageCredit) -> String {
    let mut out = String::new();
    if !credit.title.is_empty() {
        let _ = write!(out, "“{}”", escape_html(&credit.title));
    }
    if !credit.author.is_empty() {
        let _ = write!(
            out,
            "{}{}",
            if out.is_empty() { "By " } else { " by " },
            escape_html(&credit.author)
        );
    }
    for part in [
        link(&credit.source_url, &credit.source),
        link(&credit.licence_url, &credit.licence),
    ] {
        if !part.is_empty() {
            if !out.is_empty() {
                out.push_str(" · ");
            }
            out.push_str(&part);
        }
    }
    out
}

/// A row for the page: `(display name, scientific name, credit)`.
type Row = (String, String, ImageCredit);

/// The pictures to list from `cache`: every species the station has heard whose cached picture
/// has a credit, and how many have a picture that does not yet.
fn collect(
    state: &AppState,
    cache: Option<std::sync::Arc<birdnet_integrations::species_images::ImageCache>>,
) -> Result<(Vec<Row>, usize), birdnet_db::sqlite::DbError> {
    let Some(cache) = cache else {
        return Ok((Vec::new(), 0));
    };
    let names = state.names_handle();
    let species = state.with_read_db(|conn| birdnet_db::sqlite::top_species(conn, 10_000))?;
    let mut rows = Vec::new();
    let mut pending = 0;
    for s in species {
        let Some(img) = cache.get_cached(&s.sci_name) else {
            continue;
        };
        match img.credit {
            Some(credit) => rows.push((
                names.name(&s.sci_name, &s.com_name).to_string(),
                s.sci_name,
                credit,
            )),
            None if img.cached_path.is_some() => pending += 1,
            None => {}
        }
    }
    rows.sort_by_key(|row| row.0.to_lowercase());
    Ok((rows, pending))
}

/// The rows of a credits table. `picture` is the heading of the second column ("Picture", or
/// "Plate"); each cell carries its column's name in `data-label`, which is what a phone prints in
/// front of it when the table is stacked.
fn rows_html(rows: &[Row], picture: &str) -> String {
    let mut body = String::new();
    for (name, sci, credit) in rows {
        let _ = write!(
            body,
            r#"<tr><td data-label="Species"><b>{name}</b><div class="bnb-meta"><i>{sci}</i></div></td><td data-label="{picture}">{title}</td><td data-label="Author">{author}</td><td data-label="Source">{source}</td><td data-label="Licence">{licence}</td></tr>"#,
            name = escape_html(name),
            sci = escape_html(sci),
            title = escape_html(&credit.title),
            author = if credit.author.is_empty() {
                r#"<span class="bnb-meta">see the source page</span>"#.to_owned()
            } else {
                escape_html(&credit.author)
            },
            source = link(&credit.source_url, &credit.source),
            licence = link(&credit.licence_url, &credit.licence),
        );
    }
    body
}

/// The books the station's plates are from, as a table, with each one's licence and the Commons
/// category its plates were found in. Empty when there is no plate cache.
fn works_table() -> String {
    use birdnet_integrations::species_images::PlateIndex;
    let index = PlateIndex::embedded();
    let mut rows = String::new();
    for (_, work) in index.works_in_use() {
        let category = if work.commons_category.is_empty() {
            String::new()
        } else {
            link(
                &format!(
                    "https://commons.wikimedia.org/wiki/{}",
                    work.commons_category.replace(' ', "_")
                ),
                &work.commons_category,
            )
        };
        let _ = write!(
            rows,
            r#"<tr><td data-label="Book"><i>{}</i></td><td data-label="Author">{}</td><td data-label="Published">{}</td><td data-label="Licence">Public domain</td><td data-label="Plates found in">{category}</td></tr>"#,
            escape_html(&work.title),
            escape_html(&work.author),
            escape_html(&work.years.replace('-', "–")),
        );
    }
    format!(
        r#"<table class="sp-table pic-credits"><thead><tr><th>Book</th><th>Author</th><th>Published</th><th>Licence</th><th>Plates found in</th></tr></thead><tbody>{rows}</tbody></table>"#
    )
}

async fn page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let photos = state.image_cache();
    let plates = state.plate_cache();
    let has_plates = plates.is_some();
    let result = state.run_blocking(move |s| collect(s, photos)).await;
    let plate_rows = state
        .run_blocking(move |s| collect(s, plates))
        .await
        .map(|(rows, _)| rows);
    let head = page_head(&Head {
        eyebrow: "System · pictures",
        help: "",
        title: "Picture <em>credits</em>",
        lead: "Who made the pictures this station shows.",
        lede: "Photographs are fetched from Wikipedia and Wikimedia Commons and, where enabled, \
               AviCommons and Flickr, each under the licence its author chose. The kiosk \
               engraves them, which is an adaptation of the photograph.",
        aside: "",
    });
    let body = match result {
        Err(e) => {
            tracing::warn!(error = %e, "picture credits: could not read the species list");
            super::error_states::inline("the list of pictures")
        }
        Ok((rows, pending)) => {
            let table = if rows.is_empty() {
                r#"<p class="bnb-meta">No credited pictures are held yet. They are fetched the first time a bird's page or the kiosk asks for one.</p>"#.to_owned()
            } else {
                format!(
                    r#"<table class="sp-table pic-credits"><thead><tr><th>Species</th><th>Picture</th><th>Author</th><th>Source</th><th>Licence</th></tr></thead><tbody>{}</tbody></table>"#,
                    rows_html(&rows, "Picture")
                )
            };
            let pending_note = if pending == 0 {
                String::new()
            } else {
                format!(
                    r#"<p class="bnb-meta">{pending} {} cached before credits were kept and will be replaced by a credited one the first time {} shown.</p>"#,
                    if pending == 1 {
                        "picture was"
                    } else {
                        "pictures were"
                    },
                    if pending == 1 { "it is" } else { "each is" },
                )
            };
            let fig = figure(&Figure {
                topic: "Photographs",
                title: "Where each picture came from",
                action: None,
                note: "Open a source to see the original and its author; open a licence for its terms.",
                body: &format!("{table}{pending_note}"),
            });
            let plates_fig = if has_plates {
                let stored = plate_rows.as_ref().map_or_else(
                    |_| String::new(),
                    |rows| {
                        if rows.is_empty() {
                            r#"<p class="bnb-meta pic-held">No plate has been fetched yet. The kiosk fetches one the first time a bird that has one is heard.</p>"#.to_owned()
                        } else {
                            format!(
                                r#"<p class="bnb-meta pic-held">Plates this station holds</p><table class="sp-table pic-credits"><thead><tr><th>Species</th><th>Plate</th><th>Author</th><th>Source</th><th>Licence</th></tr></thead><tbody>{}</tbody></table>"#,
                                rows_html(rows, "Plate")
                            )
                        }
                    },
                );
                figure(&Figure {
                    topic: "Plates",
                    title: "The plates the kiosk draws",
                    action: None,
                    note: "Nineteenth-century plates, in the public domain, fetched from Wikimedia Commons for the birds that have one (a minority: the books are mostly of European and Asian birds). The books, then the plates this station holds.",
                    body: &format!("{}{stored}", works_table()),
                })
            } else {
                String::new()
            };
            format!(
                "{fig}{plates_fig}{}",
                r#"<div class="bnb-card pad"><div class="bnb-eyebrow">Your own pictures</div><p class="bnb-meta">A picture you put in the custom image directory is yours to credit: the kiosk draws it as it was made and prints the first line of a <code>CREDITS.txt</code> kept beside it in the foot of the plate.</p></div>"#
            )
        }
    };
    render_page_for_request(
        "Picture credits",
        &format!("{head}{body}"),
        "station",
        &headers,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credit() -> ImageCredit {
        ImageCredit {
            title: "Common <blackbird>".into(),
            author: "J. \"Jo\" Smith & Co".into(),
            source: "Wikimedia Commons".into(),
            source_url: "https://commons.wikimedia.org/wiki/File:X.jpg".into(),
            licence: "CC BY-SA 4.0".into(),
            licence_url: "https://creativecommons.org/licenses/by-sa/4.0/".into(),
        }
    }

    #[test]
    fn a_credit_names_title_author_source_and_licence_with_links() {
        let html = credit_html(&credit());
        assert!(html.contains("“Common &lt;blackbird&gt;”"), "{html}");
        assert!(
            html.contains("by J. &quot;Jo&quot; Smith &amp; Co"),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="https://commons.wikimedia.org/wiki/File:X.jpg" target="_blank" rel="noopener noreferrer">Wikimedia Commons</a>"#),
            "{html}"
        );
        assert!(html.contains(">CC BY-SA 4.0</a>"), "{html}");
        assert!(
            !html.contains("<blackbird>"),
            "escaped, not injected: {html}"
        );
    }

    /// The address comes from a provider's data. Only http(s) is linked; anything else is
    /// printed as the text it is.
    #[test]
    fn only_web_addresses_become_links() {
        let mut c = credit();
        c.source_url = "javascript:alert(1)".into();
        c.licence_url = "  data:text/html,x".into();
        let html = credit_html(&c);
        assert!(!html.contains("href=\"javascript"), "{html}");
        assert!(!html.contains("href=\"data"), "{html}");
        assert!(
            html.contains("Wikimedia Commons") && html.contains("CC BY-SA 4.0"),
            "{html}"
        );
        // The counterpart: the same fields with web addresses are linked.
        assert!(credit_html(&credit()).contains("href=\"https://"));
        assert_eq!(
            safe_href(" https://a.example/x "),
            Some("https://a.example/x")
        );
        assert_eq!(safe_href("//a.example"), None);
    }

    #[test]
    fn a_credit_with_no_author_still_reads_and_leaves_out_what_is_missing() {
        let mut c = credit();
        c.author.clear();
        c.licence_url.clear();
        let html = credit_html(&c);
        assert!(!html.contains(" by "), "{html}");
        assert!(
            html.contains("CC BY-SA 4.0") && !html.contains("licenses/by-sa"),
            "{html}"
        );
    }
}
