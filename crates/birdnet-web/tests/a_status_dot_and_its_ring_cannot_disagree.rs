//! A status dot's fill and its contrast ring are derived from one property.
//!
//! `--dawn` is an amber *fill*, and a 6px shape carrying no border is judged on
//! its fill alone. Measured in a browser against the surfaces it actually sits
//! on, in light mode: 2.85:1 on `--bg`, 2.97:1 on `--surface`, and **2.45:1 on
//! the `--dawn-soft` pill fill** — the "Not recording" badge in the top nav, which
//! is the most-seen dot in the product. WCAG 1.4.11 asks 3:1 of a meaningful
//! graphic. Every other dot already measured 4.3–10.8 in both themes.
//!
//! `.bnb-dot` therefore carries a ring mixed from its own hue toward `--fg`,
//! which darkens in light and lightens in dark from one declaration. Both the
//! fill and the ring read `--dot`, so a variant sets one value and the two
//! cannot disagree — measured after: every dot clears 3:1 on its own
//! background, the amber one at 5.65 via its ring.
//!
//! That only holds while every variant sets `--dot`. A rule that recolours a
//! dot with `background:` instead moves the fill and leaves the ring on the
//! previous hue, which is both wrong-looking and silently back under 3:1 for
//! amber. Observed failing by reverting one variant to `background:`.
//!
//! `outline` rather than `box-shadow` is load-bearing and also checked:
//! `bnb-pulse` animates `box-shadow` on `.bnb-dot.live`, so a ring declared
//! that way would be overwritten on the animation's first frame.

use std::path::Path;

/// Rule blocks whose selector targets a dot, as `(selector, body)`.
fn dot_rules(css: &str) -> Vec<(String, String)> {
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
        if selector.contains(".bnb-dot")
            || selector.contains(".live-dot")
            || selector.contains(".bkr-dot")
        {
            out.push((selector.to_string(), body.to_string()));
        }
        rest = &rest[open + close + 1..];
    }
    out
}

#[test]
fn every_dot_variant_recolours_through_the_dot_property() {
    let css =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("static/css/app.css"))
            .expect("app.css is readable");

    let rules = dot_rules(&css);
    assert!(
        rules.len() >= 8,
        "the rule scanner found only {} dot rules; it has probably stopped \
         parsing app.css and would pass for the wrong reason",
        rules.len()
    );

    let mut offenders = Vec::new();
    // Tracked per base rule, not as one flag: `.bnb-dot` and `.live-dot` are
    // two independent dots, and an earlier version of this test let one keep
    // its ring while the other lost it — caught by observing that removing
    // `.bnb-dot`'s outline still passed.
    let mut ringed: Vec<&str> = Vec::new();
    let mut bases_seen: Vec<&str> = Vec::new();
    for (selector, body) in &rules {
        let flat: String = body.chars().filter(|c| !c.is_whitespace()).collect();

        // The base rule is the only one allowed to paint `background: var(--dot)`,
        // and it is the one that must declare the ring.
        let is_base = selector.trim() == ".bnb-dot" || selector.trim() == ".live-dot";
        if is_base {
            bases_seen.push(selector.trim());
            if flat.contains("outline:") && flat.contains("var(--dot)") {
                ringed.push(selector.trim());
            }
            assert!(
                !flat.contains("box-shadow:"),
                "{selector} declares the ring with box-shadow, which `bnb-pulse` \
                 animates on .bnb-dot.live and would overwrite. Use outline."
            );
            continue;
        }

        // Any other dot rule that changes colour must do it through `--dot`.
        let sets_bg = flat.contains("background:") || flat.contains("background-color:");
        if sets_bg {
            offenders.push(format!("{selector}  {{{}}}", body.trim()));
        }
    }

    for base in [".bnb-dot", ".live-dot"] {
        assert!(
            bases_seen.contains(&base),
            "app.css no longer declares a base `{base}` rule; this gate is \
             guarding nothing and should be revisited"
        );
        assert!(
            ringed.contains(&base),
            "`{base}` no longer declares an `outline` derived from `var(--dot)`, \
             so its dots have no contrast ring and amber is back under 3:1 on \
             every surface it sits on. Ringed: {ringed:?}"
        );
    }
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "these rules recolour a status dot with `background:` instead of \
         `--dot:`, which moves the fill and leaves the contrast ring on the \
         previous hue:\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart: `--dot` must still actually drive the fill, or the rule
/// above could be satisfied by a family that sets a property nothing reads.
#[test]
fn the_dot_property_drives_the_fill_and_the_ring() {
    let css =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("static/css/app.css"))
            .expect("app.css is readable");
    let base = dot_rules(&css)
        .into_iter()
        .find(|(sel, _)| sel.trim() == ".bnb-dot")
        .expect("app.css still declares a base .bnb-dot rule");
    let flat: String = base.1.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        flat.contains("background:var(--dot)"),
        "the base .bnb-dot rule does not paint `background: var(--dot)`, so \
         setting --dot in a variant would change nothing: {}",
        base.1.trim()
    );
    assert!(
        flat.contains("--dot:"),
        "the base .bnb-dot rule sets no default --dot, so an unqualified dot \
         has no fill at all: {}",
        base.1.trim()
    );
}
