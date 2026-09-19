//! `print.css` forces a light palette; every token that flips must be in it.
//!
//! The sheet re-declares the design tokens under
//! `:root, :root[data-theme="dark"], :root[data-contrast="high"]` so a printed
//! page is legible on white paper whatever the screen theme was. That only
//! works for the tokens it actually lists, and it listed 22 of the 29 that
//! differ between light and dark.
//!
//! Observed failing against the shipped tree, naming the seven it missed:
//! `--sp-ink`, `--on-fill`, `--on-moss`, `--accent-rgb`, `--night`,
//! `--paper`, `--warning`. The worst was `--sp-ink`: it is near-white in dark
//! mode and `.bnb-avatar` mixes each species' identity hue 62% toward it, so a
//! dark-theme operator printing the weekly report got invisible banding-code
//! avatars on every row.
//!
//! Computed from app.css rather than maintained by hand, so a token added with
//! a dark override in a later change cannot be forgotten here.

use std::collections::BTreeMap;
use std::path::Path;

/// Strip `/* … */` so prose mentioning a token is not read as a declaration.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        match rest[open + 2..].find("*/") {
            Some(close) => rest = &rest[open + 2 + close + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The `--name: value` pairs of the first block whose selector line starts
/// with `needle`.
fn token_block(src: &str, needle: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(at) = src.find(needle) else {
        return out;
    };
    let after = &src[at..];
    let Some(open) = after.find('{') else {
        return out;
    };
    let Some(close) = after[open..].find("\n}") else {
        return out;
    };
    for line in after[open + 1..open + close].lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("--") else {
            continue;
        };
        if let Some((name, value)) = rest.split_once(':') {
            out.insert(
                format!("--{}", name.trim()),
                value.trim().trim_end_matches(';').to_string(),
            );
        }
    }
    out
}

#[test]
fn print_css_overrides_every_token_that_differs_between_themes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/css");
    let app = strip_comments(&std::fs::read_to_string(root.join("app.css")).expect("app.css"));
    let print =
        strip_comments(&std::fs::read_to_string(root.join("print.css")).expect("print.css"));

    // The light block is written `:root,\n:root[data-theme="light"] {`.
    let light = token_block(&app, "\n:root,");
    let dark = token_block(&app, "\n:root[data-theme=\"dark\"] {");
    assert!(
        light.len() > 30 && dark.len() > 10,
        "the token-block scanner found {} light and {} dark declarations; it \
         has probably stopped parsing app.css and would pass for the wrong reason",
        light.len(),
        dark.len()
    );

    let flipped: Vec<&String> = dark
        .iter()
        .filter(|(k, v)| light.get(*k).map(String::as_str) != Some(v.as_str()))
        .map(|(k, _)| k)
        .collect();

    let forced = token_block(&print, "\n  :root,");
    let missing: Vec<&&String> = flipped
        .iter()
        .filter(|k| !forced.contains_key(**k))
        .collect();

    assert!(
        missing.is_empty(),
        "print.css forces a light palette but does not override {} token(s) \
         that differ between light and dark, so their DARK values reach the \
         page on paper this sheet has already forced white:\n  {}",
        missing.len(),
        missing
            .iter()
            .map(|k| format!("{k}  (dark: {})", dark[**k]))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
