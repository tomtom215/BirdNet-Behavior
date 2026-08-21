//! A closed `<details>` must not fetch what it is hiding.
//!
//! # What this is defending
//!
//! htmx fires `hx-trigger="load"` when an element is processed into the DOM.
//! Visibility has nothing to do with it, and a closed `<details>` **is** in the
//! DOM — verified in a real browser, along with `revealed`, which also fires
//! while closed because a zero-size element counts as revealed.
//!
//! So `hx-trigger="load"` inside a collapsed disclosure is the worst of both:
//! the reader is spared the content and the station renders it anyway. Measured
//! on a live station, `/patterns?tab=trends` issued ten panel requests to paint
//! two visible charts; `?tab=together` shipped 28 KB of co-occurrence matrix and
//! pair table nobody had asked to see. Both scale with the history, so the
//! waste grows over a station's life.
//!
//! The working trigger is `toggle from:closest details once, intersect once`:
//! `toggle` fires when the disclosure opens, and `intersect once` still loads
//! the panel immediately if a future edit marks the `<details>` `open`.
//!
//! This gate is structural rather than behavioural because the alternative — a
//! browser — is not available to `cargo test`. It reads the shipped templates
//! and the route sources that emit markup, and fails on any `load` trigger
//! inside a `<details>` that has no `open` attribute.

use std::path::{Path, PathBuf};

/// Files that carry `<details>` markup: the templates, and the route modules
/// that build HTML in Rust.
fn markup_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in ["templates", "src"] {
        collect(&root.join(dir), &mut out);
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("html" | "rs")
        ) {
            out.push(path);
        }
    }
}

/// Byte offsets of every `<details …>` opening tag that carries no `open`
/// attribute, paired with the offset just past it.
fn closed_details_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(rel) = text[i..].find("<details") {
        let start = i + rel;
        let Some(close_rel) = text[start..].find('>') else {
            break;
        };
        let tag_end = start + close_rel + 1;
        let tag = &text[start..tag_end];
        // `open` as a bare attribute, not as part of a longer word.
        let is_open = tag
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .any(|w| w == "open");
        if !is_open {
            let body_end = text[tag_end..]
                .find("</details>")
                .map_or(bytes.len(), |r| tag_end + r);
            spans.push((tag_end, body_end));
        }
        i = tag_end;
    }
    spans
}

#[test]
fn no_hidden_panel_fetches_itself_before_it_is_opened() {
    let mut offenders = Vec::new();
    for path in markup_sources() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (start, end) in closed_details_spans(&text) {
            let body = &text[start..end];
            let mut from = 0;
            while let Some(rel) = body[from..].find(r#"hx-trigger="load"#) {
                let at = from + rel;
                // `hx-trigger="load"` exactly, not `hx-trigger="load, …"` —
                // both are eager, so match the prefix and report either.
                let line = text[..start + at].matches('\n').count() + 1;
                offenders.push(format!("{}:{line}", path.display()));
                from = at + 1;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these panels are inside a collapsed <details> and still fetch \
         themselves on page load, which renders and ships content nobody has \
         asked to see. Use `hx-trigger=\"toggle from:closest details once, \
         intersect once\"` instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// A panel that has deliberately not loaded must not claim to be loading.
///
/// # The defect this was written for
///
/// Deferring the fetch is only half the change. The placeholder those panels
/// carried said "Loading…", and once the fetch stopped happening at page load
/// that text stopped being temporary: `/correlation` rendered a permanent
/// "Loading…" inside each collapsed section, in every theme and viewport, for
/// as long as the reader left it closed.
///
/// It is not hypothetical and it was not caught here — the browser sweep caught
/// it, on all four theme x viewport states at once, because the `.pt-disc` CSS
/// keeps collapsed content in flow (`offsetParent` is non-null) rather than
/// `display: none`. This gate exists so the next one is caught by
/// `cargo test` instead of by a four-minute Playwright run.
///
/// The convention the other seven converted panels already follow is
/// `class="htmx-indicator"`, which `app.css` renders `display: none` until htmx
/// puts `.htmx-request` on the element — so the words appear exactly while a
/// request is in flight, which is when they are true.
#[test]
fn a_deferred_panel_does_not_advertise_a_load_that_is_not_happening() {
    /// The trigger that makes a panel deferred.
    const DEFERRED: &str = r#"hx-trigger="toggle from:closest details"#;

    let mut offenders: Vec<String> = Vec::new();
    for path in markup_sources() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut from = 0;
        while let Some(rel) = text[from..].find(DEFERRED) {
            let at = from + rel;
            from = at + 1;
            // The element's own content: from the end of its opening tag to
            // the first closing tag after it. Scoping to the panel is what
            // keeps this from flagging placeholders that belong to something
            // else in the same disclosure — the polar clock's SVG `<text>` is
            // filled by the page's own script, not by htmx, and is none of
            // this gate's business.
            let Some(gt) = text[at..].find('>').map(|r| at + r + 1) else {
                continue;
            };
            let inner_end = text[gt..].find("</").map_or(text.len(), |r| gt + r);
            let inner = &text[gt..inner_end];
            if !inner.to_lowercase().contains("loading") {
                continue;
            }
            if inner.contains("htmx-indicator") {
                continue;
            }
            let line = text[..gt].matches('\n').count() + 1;
            offenders.push(format!("{}:{line}  {}", path.display(), inner.trim()));
        }
    }
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "these placeholders belong to panels that do not fetch until their \
         <details> is opened, so the words are shown to a reader for whom \
         nothing is loading. Mark them `class=\"htmx-indicator\"` (hidden until \
         htmx sets .htmx-request) or use a skeleton:\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart: an `open` disclosure *should* still load eagerly, so the
/// scanner must not be a blanket ban on `load` near a `<details>`.
#[test]
fn an_open_disclosure_is_not_flagged() {
    let sample = r#"<details open><div hx-get="/x" hx-trigger="load"></div></details>"#;
    assert!(
        closed_details_spans(sample).is_empty(),
        "a <details open> must not be treated as collapsed"
    );
    let sample = r#"<details class="pt-disc"><div hx-get="/x" hx-trigger="load"></div></details>"#;
    assert_eq!(
        closed_details_spans(sample).len(),
        1,
        "a collapsed <details> must be found"
    );
}
