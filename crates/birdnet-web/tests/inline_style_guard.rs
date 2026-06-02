//! P2-2 endgame inline-style guard.
//!
//! `style-src 'unsafe-inline'` is gone, so **no** served HTML may carry an inline
//! `style="…"` attribute: computed values ride a `data-style` attribute applied
//! via CSSOM (see `security.rs`'s middleware), and static ones are `app.css`
//! classes. This scans the *whole* crate — every `src/**/*.rs` and
//! `templates/**/*.html` — and fails if a real inline `style=` / `style=\"`
//! attribute reappears, so the CSP relaxation can't silently creep back (see
//! `docs/RELEASE_PUNCHLIST.md` § P2-2 / P3-3).
//!
//! Not inline styles, and therefore skipped: `data-style=` and
//! `data-confirm-style=` (data-attributes — the char before `style=` is part of
//! the attribute name), Rust search-strings like `split("style=\"")` (no
//! `prop:value` colon), and `<style>` element blocks (a `style-src` nonce
//! concern, not an attribute). There is no longer any "allowed dynamic inline
//! style" — every computed value moved to `data-style`.

use std::path::{Path, PathBuf};

/// Files excluded from the scan. `_empty_states.html` is a never-served design
/// reference (its live counterpart is `pages/empty_states.rs`); it ships no
/// markup to a browser, so its inline styles are out of CSP scope.
const EXCLUDE: &[&str] = &["templates/_empty_states.html"];

/// Scan one file, returning each inline-style occurrence as `(line, payload)`.
///
/// Catches both the literal `style="…"` (templates and raw-string Rust) **and**
/// the escaped `style=\"…\"` that ordinary Rust `write!`/`format!` string
/// literals emit — the rendered HTML carries an inline style either way.
fn disallowed_inline_styles(src: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    for (lineno, line) in src.lines().enumerate() {
        scan_inline_styles(line, "style=\"", "\"", lineno, &mut hits);
        scan_inline_styles(line, "style=\\\"", "\\\"", lineno, &mut hits);
    }
    hits
}

/// Record every `{open}…{close}` inline-style run on one line.
fn scan_inline_styles(
    line: &str,
    open: &str,
    close: &str,
    lineno: usize,
    hits: &mut Vec<(usize, String)>,
) {
    let mut rest = line;
    while let Some(idx) = rest.find(open) {
        // `data-style="…"` / `data-confirm-style="…"` end in `-style="`, so when
        // the char before the match is part of an identifier it's a
        // data-attribute (the CSP-safe applier hook, or a confirm-modal token),
        // not a bare inline `style=`.
        let is_data_attr = rest[..idx]
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-');
        let after = &rest[idx + open.len()..];
        let Some(end) = after.find(close) else { break };
        let payload = &after[..end];
        // A real inline style is `prop:value…` and always contains a colon;
        // Rust search-strings (`html.split("style=\"")`) and bare data-attribute
        // values have none, so the colon gate skips those while still catching
        // every genuine inline style.
        if !is_data_attr && payload.contains(':') {
            hits.push((lineno + 1, payload.to_string()));
        }
        rest = &after[end + close.len()..];
    }
}

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Recursively collect every `.rs`/`.html` source under `src/` and `templates/`.
fn collect_sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["src", "templates"] {
        collect_recursive(&root.join(sub), &mut out);
    }
    out.sort();
    out
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out);
        } else if matches!(
            path.extension().and_then(|x| x.to_str()),
            Some("rs" | "html")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn no_inline_style_attributes_anywhere() {
    let root = crate_root();
    let excluded: Vec<PathBuf> = EXCLUDE.iter().map(|r| root.join(r)).collect();
    let mut failures = Vec::new();

    for path in collect_sources(&root) {
        if excluded.contains(&path) {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        let rel = path.strip_prefix(&root).unwrap_or(&path).display();
        for (lineno, payload) in disallowed_inline_styles(&src) {
            failures.push(format!("  {rel}:{lineno}  style=\"{payload}\""));
        }
    }

    assert!(
        failures.is_empty(),
        "P2-2 regression: inline style attributes are present. style-src no longer \
         carries 'unsafe-inline', so an inline style attribute silently won't apply \
         and throws a CSP violation. Move it to a data-style attribute (computed) or \
         an app.css class (static).\n{}",
        failures.join("\n")
    );
}

#[test]
fn guard_itself_classifies_correctly() {
    // Every real inline style is caught — there is no dynamic allowlist anymore
    // (computed values live in data-style, applied via CSSOM).
    assert_eq!(
        disallowed_inline_styles(r#"<div style="display:flex;gap:1rem;">"#).len(),
        1
    );
    assert_eq!(
        disallowed_inline_styles(r#"<div style="width:42%">"#).len(),
        1
    );
    assert_eq!(
        disallowed_inline_styles(r#"<span style="--sp:red;">"#).len(),
        1
    );
    assert_eq!(
        disallowed_inline_styles(r#"<text style="font-size:11px;fill:var(--fg-3);">"#).len(),
        1
    );

    // data-style is the CSP-safe applier hook, not an inline style.
    assert!(disallowed_inline_styles(r#"<div data-style="width:{pct}%">"#).is_empty());
    assert!(disallowed_inline_styles(r#"<span data-style="--sp:{c};background:{b}">"#).is_empty());
    // data-confirm-style is a data-attribute (confirm-modal tone token).
    assert!(disallowed_inline_styles(r#"<button data-confirm-style="danger">"#).is_empty());

    // Escaped-quote inline styles (Rust write!/format! literals) are caught too.
    assert_eq!(
        disallowed_inline_styles(r#"write!(h, "<div style=\"display:flex\">")"#).len(),
        1
    );
    assert_eq!(
        disallowed_inline_styles(r#"format!("<i style=\"width:{p}%\">")"#).len(),
        1
    );
    // …but a Rust search-string for the literal attribute is not a style (no colon).
    assert!(disallowed_inline_styles(r#"assert!(html.split("style=\"").count() == 1)"#).is_empty());
    // …nor are escaped data-attribute values.
    assert!(disallowed_inline_styles(r#"format!("<b data-style=\"width:{p}%\">")"#).is_empty());
    assert!(disallowed_inline_styles(r#"format!("<b data-confirm-style=\"danger\">")"#).is_empty());
}
