//! Every `var(--x)` in the stylesheet must resolve to a `--x` that exists.
//!
//! # Why this is worth a test
//!
//! An undefined custom property does not warn, does not fail, and does not
//! fall back to anything sensible: the declaration is simply invalid at
//! computed-value time and the property keeps whatever it inherited. A button
//! written as
//!
//! ```css
//! .sr-btn-primary { background: var(--primary); color: #fff; }
//! ```
//!
//! against a stylesheet with no `--primary` renders as white text on the
//! *inherited* background — which is white. The button is invisible, and
//! nothing anywhere says so.
//!
//! That is not hypothetical. When this gate was written, `app.css` used five
//! variables it never defined, and four of them predated the change that
//! prompted the check:
//!
//! | Variable | Uses | What it broke |
//! |---|---|---|
//! | `--primary` | 4 | The quarantine lede link, the active filter tab's colour *and* underline, a detail-page link, and a new primary button |
//! | `--card-bg` | 2 | The image-admin form and table backgrounds |
//!
//! Each had been shipping, in both themes, looking approximately right because
//! the inherited value happened to be close enough that nobody looked twice.
//!
//! # The exceptions, and why they are named rather than pattern-matched
//!
//! Three variables are *deliberately* absent from the stylesheet because
//! something else sets them at runtime — Rust emitting a `data-style`
//! attribute, or the menu script measuring a button. Those are listed
//! individually with the mechanism that supplies each, so adding a fourth is a
//! decision somebody writes down rather than a regex that quietly swallows it.

use std::path::Path;

/// Variables the stylesheet uses but deliberately does not define, each with
/// the runtime mechanism that supplies it.
///
/// Every entry must carry a `var(--x, fallback)` in the CSS too, so the page is
/// not broken in the window before the value arrives (or if the script never
/// runs). [`every_runtime_variable_has_a_fallback`] checks that.
const SET_AT_RUNTIME: &[(&str, &str)] = &[
    (
        "--n",
        "the number of skeleton bars, emitted by `routes::pages::skeletons` as \
         a `data-style` attribute and applied via CSSOM",
    ),
    (
        "--bnb-more-x",
        "the right edge of the More button, measured by the menu script",
    ),
    (
        "--bnb-more-y",
        "the bottom edge of the More button, measured by the menu script",
    ),
];

fn stylesheet() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/css/app.css");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every `--name:` that appears as a declaration.
fn defined(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, _) in css.match_indices("--") {
        let rest = &css[idx..];
        let end = rest
            .char_indices()
            .position(|(_, c)| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        // A definition is `--name:`; a use is `var(--name`. Distinguish by
        // what follows the name, ignoring whitespace.
        if rest[end..].trim_start().starts_with(':') {
            out.push(name.to_string());
        }
    }
    out
}

/// Every `--name` that appears inside a `var(…)`.
fn used(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, _) in css.match_indices("var(") {
        let rest = css[idx + 4..].trim_start();
        if !rest.starts_with("--") {
            continue;
        }
        let end = rest
            .char_indices()
            .position(|(_, c)| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        out.push(rest[..end].to_string());
    }
    out
}

#[test]
fn every_variable_the_stylesheet_uses_is_defined_or_declared_runtime() {
    let css = stylesheet();
    let defined = defined(&css);
    let runtime: Vec<&str> = SET_AT_RUNTIME.iter().map(|(n, _)| *n).collect();

    let mut missing: Vec<String> = used(&css)
        .into_iter()
        .filter(|v| !defined.contains(v) && !runtime.contains(&v.as_str()))
        .collect();
    missing.sort();
    missing.dedup();

    assert!(
        missing.is_empty(),
        "app.css uses {} custom propert{} it never defines: {missing:?}\n\
         An undefined `var()` does not warn — the declaration is invalid at \
         computed-value time and the property silently keeps what it inherited, \
         so the rule looks approximately right and is wrong. Define it beside \
         the other tokens, or add it to `SET_AT_RUNTIME` with the mechanism \
         that supplies it.",
        missing.len(),
        if missing.len() == 1 { "y" } else { "ies" }
    );
}

#[test]
fn every_runtime_variable_has_a_fallback() {
    // A variable supplied by script is absent for the whole first paint, and
    // absent forever if the script fails to load. Each use needs a fallback, or
    // the page is broken in exactly the window a user is most likely to see.
    let css = stylesheet();
    for (name, why) in SET_AT_RUNTIME {
        let bare = format!("var({name})");
        assert!(
            !css.contains(&bare),
            "`{name}` is set at runtime ({why}) but is used as `{bare}` with no \
             fallback. Write `var({name}, <sensible default>)` so the page is \
             not broken before the value arrives."
        );
    }
}

#[test]
fn the_runtime_allowlist_has_no_stale_entries() {
    // The counterpart: an entry that stops being used, or starts being defined
    // normally, should leave the list rather than sit there authorising nothing.
    let css = stylesheet();
    let defined = defined(&css);
    let used = used(&css);
    for (name, _) in SET_AT_RUNTIME {
        assert!(
            used.iter().any(|u| u == name),
            "`{name}` is on the runtime allowlist but the stylesheet no longer \
             uses it — drop the entry"
        );
        assert!(
            !defined.contains(&(*name).to_string()),
            "`{name}` is on the runtime allowlist but app.css now defines it \
             normally — drop the entry"
        );
    }
}

#[test]
fn the_scanner_tells_a_definition_from_a_use() {
    // The parser is the thing that could make this whole gate vacuous, so it is
    // checked directly rather than trusted.
    let css = ":root { --a: red; --b:blue } .x { color: var(--a); border: var( --c , 1px); }";
    let d = defined(css);
    assert!(d.contains(&"--a".to_string()), "{d:?}");
    assert!(d.contains(&"--b".to_string()), "spacing-free form: {d:?}");
    assert!(
        !d.contains(&"--c".to_string()),
        "a use is not a definition: {d:?}"
    );

    let u = used(css);
    assert!(u.contains(&"--a".to_string()), "{u:?}");
    assert!(
        u.contains(&"--c".to_string()),
        "`var( --c , …)` with padding must still register: {u:?}"
    );
    assert!(!u.contains(&"--b".to_string()), "{u:?}");
}
