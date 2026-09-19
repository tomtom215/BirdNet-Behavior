//! A control whose job is to report a state must not dim the state it reports.
//!
//! # The defect
//!
//! The top-nav live-stream pill had one line of styling for its reconnect
//! state:
//!
//! ```css
//! #live-status[data-state="reconnecting"] { opacity: .6; }
//! ```
//!
//! `opacity` composites the whole subtree toward whatever is behind it, text
//! included. Measured in the running page, text over the pill fill it actually
//! sits on, at 11px / weight 500 — normal text, so WCAG 1.4.3 asks 4.5:1:
//!
//! | state          | light  | dark   |
//! |----------------|--------|--------|
//! | live           | 8.54:1 | 9.24:1 |
//! | **reconnecting** | **3.07:1** | **4.24:1** |
//! | default        | 8.54:1 | 9.24:1 |
//!
//! Both themes failed, and only in the state whose word the reader actually
//! has to read. "Live" — the state that needs no attention — was the legible
//! one. Replacing the dim with the amber pair `.bnb-pill.dawn` already uses
//! measures 7.26:1 light and 9.78:1 dark, and says "attention" rather than
//! "error", which is what a reconnect is.
//!
//! # Why a gate rather than the accessibility sweep
//!
//! `axe.mjs` did find this, once, on one route, in one theme — because the
//! socket happened to be retrying when that page was sampled. Every other run
//! was clean. A state a script reaches at runtime is graded by luck: the
//! sweep loads a page and reads whatever state it is in. This check needs no
//! browser and no luck.

use std::path::Path;

/// The stylesheet that carries the shell's styling.
fn app_css() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/css/app.css");
    std::fs::read_to_string(&path).expect("app.css must be readable")
}

/// The declarations of every rule whose selector contains `needle`.
fn declarations_for(css: &str, needle: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let selector = rest[..open]
            .rsplit(['}', '*', '/'])
            .next()
            .unwrap_or("")
            .trim();
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + close];
        if selector.contains(needle) {
            out.push((selector.to_owned(), body.trim().to_owned()));
        }
        rest = &rest[open + close + 1..];
    }
    out
}

#[test]
fn the_live_pill_is_never_dimmed() {
    let css = app_css();
    let offenders: Vec<String> = declarations_for(&css, "#live-status")
        .into_iter()
        .filter(|(_, body)| body.contains("opacity"))
        .map(|(sel, body)| format!("{sel} {{{body}}}"))
        .collect();

    assert!(
        offenders.is_empty(),
        "the live pill reports the connection state, and `opacity` composites \
         its text toward the page: the reconnect state measured 3.07:1 in \
         light and 4.24:1 in dark, against the 4.5:1 WCAG 1.4.3 asks of 11px \
         text. Use a token pair that carries its own contrast.\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart. Deleting the rule would satisfy the test above and leave
/// "Reconnecting…" looking exactly like "Live" — a silent outage, which is
/// worse than an illegible warning.
#[test]
fn the_reconnect_state_still_looks_different_from_the_live_one() {
    let css = app_css();
    let reconnecting: Vec<String> = declarations_for(&css, "#live-status")
        .into_iter()
        .filter(|(sel, _)| sel.contains("reconnecting"))
        .map(|(_, body)| body)
        .collect();

    assert!(
        !reconnecting.is_empty(),
        "nothing styles the reconnect state, so a station that has lost its \
         live stream shows the same pill as one that has not"
    );
    let all = reconnecting.join(" ");
    assert!(
        all.contains("color:") || all.contains("color "),
        "the reconnect state must restate its own text colour rather than \
         inherit the live one: {all}"
    );
    assert!(
        all.contains("background:"),
        "and its own fill, so the two states differ by more than a dot: {all}"
    );
}
