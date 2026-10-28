//! Display → Charts → Plate: what the stylesheet and the pattern file promise each other.
//!
//! The first version hatched everything, including the sequential scales, and measured weaker than the
//! flat colours it replaced: adjacent steps of the heat-grid legend differed by 5.3 to 5.7 L* in the
//! light theme against 6.9 to 9.2 flat, because the eye reads order from lightness and not from how
//! close together some lines are. A reader said so before the measurement did ("hard to discern and
//! read at a glance"). The style now hatches categorical series only, and these hold the two ways that
//! can quietly come undone:
//!
//! * no rule gives a sequential class (`ck-q1`…`ck-m7`) a pattern;
//! * every `url(#bnb-hatch-…)` the stylesheet names is defined in `_partial_illustration_defs.html`
//!   and every pattern defined there is used, so a rename cannot leave a mark painted with nothing.
//!
//! That the patterns reach the page is `pages::tests::every_page_carries_the_hatch_patterns` in the lib.

use std::collections::BTreeSet;
use std::path::Path;

fn read(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The plate block of `app.css`: from its banner comment to the end of the file.
fn plate_block(css: &str) -> &str {
    let at = css
        .find("/* ── Plate charts")
        .expect("app.css has no 'Plate charts' block");
    &css[at..]
}

/// `/* … */` removed, so a rule described in a comment is not a rule.
fn without_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        rest = rest[open..]
            .find("*/")
            .map_or("", |close| &rest[open + close + 2..]);
    }
    out.push_str(rest);
    out
}

/// Each `selector { body }` of a flat stylesheet, as `(selector, body)`.
fn rules(css: &str) -> Vec<(String, String)> {
    css.split('}')
        .filter_map(|chunk| chunk.split_once('{'))
        .map(|(s, b)| (s.trim().to_owned(), b.trim().to_owned()))
        .collect()
}

/// Rules that name a sequential class and give it a pattern or a hatched gradient.
fn hatched_sequential_rules(css: &str) -> Vec<String> {
    rules(css)
        .into_iter()
        .filter(|(selector, body)| {
            let sequential = (1..=7).any(|n| {
                selector.contains(&format!(".ck-q{n}")) || selector.contains(&format!(".ck-m{n}"))
            });
            sequential && (body.contains("url(#bnb-hatch") || body.contains("repeating-"))
        })
        .map(|(selector, _)| selector)
        .collect()
}

/// The `[a-z0-9]+` word after each `{marker}bnb-hatch-` in `text`.
fn hatch_ids(text: &str, marker: &str) -> BTreeSet<String> {
    let needle = format!("{marker}bnb-hatch-");
    let mut ids = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        let id: String = rest
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        ids.insert(id);
    }
    ids
}

#[test]
fn a_sequential_scale_is_never_hatched() {
    let css = without_comments(plate_block(&read("static/css/app.css")));
    let hatched = hatched_sequential_rules(&css);
    assert!(
        hatched.is_empty(),
        "these rules hatch a sequential scale, which reads weaker than its flat ramp:\n  {}",
        hatched.join("\n  ")
    );
    // The counterpart: the scan is looking at rules that exist, and it flags one that hatches.
    assert!(
        rules(&css).iter().any(|(s, _)| s.contains(".ck-q1")),
        "the block no longer mentions the sequential classes; the scan has nothing to look at"
    );
    let bad = r#":root[data-illustration="plate"] .ck-q3 { --hatch: url(#bnb-hatch-1); }
        .ck-sw.ck-m2 { background: repeating-linear-gradient(45deg, red 0 1px, blue 1px 4px); }
        .ck-c1 { --hatch: url(#bnb-hatch-1); }"#;
    assert_eq!(hatched_sequential_rules(bad).len(), 2, "{bad}");
}

#[test]
fn the_stylesheet_and_the_pattern_file_name_the_same_hatches() {
    let css = without_comments(plate_block(&read("static/css/app.css")));
    let defs = read("templates/_partial_illustration_defs.html");
    let used = hatch_ids(&css, "url(#");
    let defined = hatch_ids(&defs, "id=\"");
    assert!(
        defined.len() >= 9,
        "the pattern file defines only {defined:?}; eight series and the grey one are expected"
    );
    let undefined: Vec<_> = used.difference(&defined).collect();
    let unused: Vec<_> = defined.difference(&used).collect();
    assert!(
        undefined.is_empty() && unused.is_empty(),
        "stylesheet and pattern file disagree: painted with a pattern that does not exist: \
         {undefined:?}; defined and never used: {unused:?}"
    );
}
