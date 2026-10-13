//! No stylesheet embedded in a Station tab styles a bare element.
//!
//! The tabs stitch together the bodies of older admin pages, and each brought
//! its own `<style>` block written for a page it had to itself: `label {…}`,
//! `input, textarea, select { width: 100%; margin-bottom: 1rem … }`, `h1 {…}`.
//! On a shared page those rules reached everything — the audio-source edit
//! form put each checkbox on a full-width row of its own, away from its words,
//! and the header's search box took the `margin-bottom`, which moved the
//! whole top bar on these tabs alone. A rule in an embedded block must now
//! start at a class, an id, or `:where(.scope)`.
//!
//! Observed failing against the stylesheets before they were scoped: it
//! named `label`, `input`, `h1` and the rest in the settings, species, rules
//! and migration blocks.

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const TABS: &[&str] = &[
    "/station",
    "/station/capture",
    "/station/alerts",
    "/station/data",
    "/station/settings",
    "/station/access",
];

/// Elements a stray rule could restyle across the page.
const ELEMENTS: &[&str] = &[
    "a", "article", "aside", "body", "button", "code", "details", "div", "dl", "em", "fieldset",
    "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "html", "img", "input",
    "label", "legend", "li", "main", "nav", "ol", "option", "p", "pre", "section", "select",
    "small", "span", "strong", "summary", "svg", "table", "tbody", "td", "textarea", "tfoot", "th",
    "thead", "tr", "ul",
];

/// Every selector in every `<style>` block of `html`, comments removed.
fn selectors(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<style") {
        let Some(open) = rest[i..].find('>') else {
            break;
        };
        let body_start = i + open + 1;
        let Some(len) = rest[body_start..].find("</style>") else {
            break;
        };
        let mut css = rest[body_start..body_start + len].to_string();
        while let Some(c) = css.find("/*") {
            let end = css[c..].find("*/").map_or(css.len(), |e| c + e + 2);
            css.replace_range(c..end, "");
        }
        // A selector list is whatever precedes a `{` back to the previous
        // `}`, `{` or `;`; an at-rule's own prelude is skipped.
        let mut start = 0;
        for (pos, ch) in css.char_indices() {
            match ch {
                '{' => {
                    let head = css[start..pos].trim();
                    if !head.starts_with('@') {
                        out.extend(top_level_list(head));
                    }
                    start = pos + 1;
                }
                '}' | ';' => start = pos + 1,
                _ => {}
            }
        }
        rest = &rest[body_start + len..];
    }
    out
}

/// A selector list split at its top-level commas — not the ones inside
/// `:is(input, select)`.
fn top_level_list(head: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (i, c) in head.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(head[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(head[start..].trim().to_string());
    out
}

/// Whether a selector's first compound is an element with no class or id to
/// confine it: `label`, `td code`, `input[type=text]`, `input:focus`. A
/// class-qualified `a.btn` matches only what carries the class, and passes.
fn starts_at_an_element(selector: &str) -> bool {
    let first: String = selector
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    if !ELEMENTS.contains(&first.as_str()) {
        return false;
    }
    let compound: String = selector[first.len()..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '+' && *c != '~')
        .collect();
    !compound.contains('.') && !compound.contains('#')
}

async fn page(uri: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    let res = birdnet_web::server::build_router(state)
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn no_embedded_stylesheet_styles_a_bare_element() {
    let mut found = Vec::new();
    let mut seen = 0;
    for tab in TABS {
        let sels = selectors(&page(tab).await);
        seen += sels.len();
        for s in sels.iter().filter(|s| starts_at_an_element(s)) {
            found.push(format!("{tab}: {s}"));
        }
    }
    assert!(
        seen > 50,
        "read only {seen} selectors; the parse is not reading the blocks"
    );
    found.sort();
    found.dedup();
    assert!(
        found.is_empty(),
        "page-wide element rules:\n{}",
        found.join("\n")
    );
}

/// The parser's own counterpart: it finds a bare element, and lets a scoped,
/// a class-led and an at-rule's selectors through.
#[test]
fn the_parser_tells_a_bare_element_from_a_scoped_one() {
    let html = "<style>/* label { } */ label { x: 1 } :where(.s) input { x: 1 }
        .card h2 { x: 1 } @media (max-width: 9px) { .a, select { x: 1 } }
        :where(.s) :is(input,textarea) { x: 1 } a.btn { x: 1 } input[type=text] { x: 1 }</style>";
    let bare: Vec<String> = selectors(html)
        .into_iter()
        .filter(|s| starts_at_an_element(s))
        .collect();
    assert_eq!(bare, ["label", "select", "input[type=text]"]);
}
