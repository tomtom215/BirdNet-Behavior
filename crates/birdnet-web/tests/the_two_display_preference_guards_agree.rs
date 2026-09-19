//! The app has two pre-paint display-preference guards, and they drifted.
//!
//! `templates/layout.html` carries an inline one for the main shell;
//! `static/theme-guard.js` is loaded by the standalone shells (`admin/mod.rs`,
//! and the pages that have not folded into a Station tab). The latter's own
//! comment said it "mirrors the inline guard in layout.html" while reading two
//! of the four `localStorage` keys the inline one reads.
//!
//! Observed failing against the shipped tree: `theme-guard.js` was missing
//! `bnb-motion` and `bnb-contrast`. Measured in a browser with both set, the
//! `<html>` element on `/admin/doctor` carried neither `data-motion` nor
//! `data-contrast`, while `/` carried both — so Reduced motion and High
//! contrast were silently dropped on every remaining standalone admin page.
//!
//! Two guards for one job is the real defect; this test is the cheap way to
//! stop them disagreeing until one of them goes away.

use std::path::Path;

/// Every `localStorage` key a guard reads, in the two spellings used.
fn keys_read(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in ["localStorage.getItem('", "localStorage.getItem(\""] {
        let mut rest = src;
        while let Some(at) = rest.find(marker) {
            let after = &rest[at + marker.len()..];
            let end = after.find(['\'', '"']).unwrap_or(0);
            if end > 0 {
                out.push(after[..end].to_string());
            }
            rest = &after[end..];
        }
    }
    // `read('theme')` style indirection: pick up the call sites too.
    let mut rest = src;
    while let Some(at) = rest.find("read('") {
        let after = &rest[at + 6..];
        if let Some(end) = after.find('\'') {
            out.push(after[..end].to_string());
            rest = &after[end..];
        } else {
            break;
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn both_pre_paint_guards_read_the_same_preference_keys() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let layout = std::fs::read_to_string(root.join("templates/layout.html"))
        .expect("layout.html is readable");
    let guard = std::fs::read_to_string(root.join("static/theme-guard.js"))
        .expect("theme-guard.js is readable");

    // Only the first <script> of layout.html is the pre-paint guard; later
    // scripts read other things (the theme toggle writes, it does not gate
    // paint).
    let inline = layout
        .split("</script>")
        .next()
        .expect("layout.html opens with its FOUC guard");

    let a = keys_read(inline);
    let b = keys_read(&guard);
    assert!(
        a.len() >= 4,
        "the inline guard scanner found only {a:?}; it has probably stopped \
         parsing and would pass for the wrong reason"
    );
    assert_eq!(
        a, b,
        "the two pre-paint display-preference guards read different keys.\n  \
         layout.html inline: {a:?}\n  static/theme-guard.js: {b:?}\n\
         A key only one of them reads is a preference silently dropped on \
         whichever shell the other serves."
    );
}
