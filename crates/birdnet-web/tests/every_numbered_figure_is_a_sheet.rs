//! A card that numbers itself ("Fig. 3 ·") is a figure, and a figure has the `fig` class.
//!
//! Display → Charts → Plate draws each figure as a ruled sheet of paper: `html[data-illustration="plate"]
//! .fig` in `app.css`. The number comes from `<span class="fig-no">`, which several pages write by hand
//! rather than through `frame::figure`, and a hand-written card that forgot `fig` kept the flat white
//! card with an upright title, in the middle of a page of sheets: the cards on Who-sings-together,
//! Trends, Dawn chorus, Migration and Behavior did exactly that. The marker and the class are the same
//! fact, so this reads the first and demands the second.

use std::path::{Path, PathBuf};

/// The text of the marker every numbered figure carries.
const MARKER: &str = r#"<span class="fig-no""#;

/// `(line, opening tag)` for each marker whose nearest enclosing card is not a `fig`, or has no card.
fn unsheeted(src: &str) -> Vec<(usize, String)> {
    // Rust sources write `class=\"…\"` in ordinary string literals; the markup is the same.
    let src = src.replace("\\\"", "\"");
    let mut bad = Vec::new();
    let mut from = 0;
    while let Some(at) = src[from..].find(MARKER) {
        let at = from + at;
        from = at + MARKER.len();
        let line = src[..at].matches('\n').count() + 1;
        match nearest_card(&src[..at]) {
            Some(tag) if class_tokens(&tag).any(|t| t == "fig") => {}
            Some(tag) => bad.push((line, tag)),
            None => bad.push((line, "(no card before it)".to_owned())),
        }
    }
    bad
}

/// The last opening tag in `before` whose `class` carries the token `bnb-card`.
fn nearest_card(before: &str) -> Option<String> {
    let mut found = None;
    let mut from = 0;
    while let Some(open) = before[from..].find("<div") {
        let open = from + open;
        let Some(close) = before[open..].find('>') else {
            break;
        };
        let tag = &before[open..=open + close];
        if class_tokens(tag).any(|t| t == "bnb-card") {
            found = Some(tag.to_owned());
        }
        from = open + close + 1;
    }
    found
}

/// The whitespace-separated tokens of the tag's `class="…"`, or none.
fn class_tokens(tag: &str) -> impl Iterator<Item = &str> {
    tag.split_once("class=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(classes, _)| classes)
        .unwrap_or_default()
        .split_whitespace()
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
            path.extension().and_then(|x| x.to_str()),
            Some("rs" | "html")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_card_that_numbers_itself_is_a_figure_sheet() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&root.join("src"), &mut files);
    collect(&root.join("templates"), &mut files);
    files.sort();

    let mut markers = 0;
    let mut failures = Vec::new();
    for path in files {
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        markers += src.matches(MARKER).count();
        let rel = path.strip_prefix(root).unwrap_or(&path).display();
        for (line, tag) in unsheeted(&src) {
            failures.push(format!("  {rel}:{line}  {tag}"));
        }
    }

    // The counterpart that says the scan looked at something: a bad path or a renamed marker would
    // otherwise leave this green with nothing read. Sixteen markers when this was written; the floor
    // sits below that so removing a figure does not trip it and a scan that reads nothing does.
    assert!(
        markers >= 10,
        "found only {markers} figure markers; the scan is not reading the pages"
    );
    assert!(
        failures.is_empty(),
        "a card with a Fig. number must carry the `fig` class, or the Plate chart style leaves it \
         as a flat card among sheets:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_scan_tells_a_sheet_from_a_card_and_finds_the_enclosing_one() {
    let sheet = r#"<div class="bnb-card pad fig"><span class="fig-no"></span>"#;
    let card = r#"<div class="bnb-card pad"><span class="fig-no"></span>"#;
    assert!(unsheeted(sheet).is_empty());
    assert_eq!(unsheeted(card).len(), 1);
    // The nearest card decides, not an earlier one.
    let two = format!("{sheet}</div>{card}");
    assert_eq!(unsheeted(&two).len(), 1, "{:?}", unsheeted(&two));
    // `fig` is a token, not a substring: `fig-take` and `config` do not make a figure.
    let lookalike = r#"<div class="bnb-card config fig-take"><span class="fig-no"></span>"#;
    assert_eq!(unsheeted(lookalike).len(), 1);
    // Escaped quotes, as in a Rust string literal.
    let escaped = r#"<div class=\"bnb-card pad\"><span class=\"fig-no\"></span>"#;
    assert_eq!(unsheeted(escaped).len(), 1);
    // A marker with no card around it is reported, not skipped.
    assert_eq!(unsheeted(r#"<span class="fig-no"></span>"#).len(), 1);
}
