//! A colour, a weight, a hairline or a bar radius is written once, as a token.
//!
//! The design-token page promises "components read tokens; a colour or a pixel
//! pad written into a rule is a defect". Nothing held the stylesheet to it:
//! `font-weight: 500` was typed 77 times, a half-pixel border 159 times, and a
//! new `#3a7` could land in any rule on any day with every other gate green. A
//! theme that was supposed to change every card's edge (high contrast) listed
//! five classes and missed the rest, because the edge was a literal in each.
//!
//! # What this holds
//!
//! 1. **No raw colour outside a token definition.** In `app.css`, a hex colour,
//!    `rgb()`, `hsl()`, `oklch()`, `oklab()`, `lab()`, `lch()` or `color()` may
//!    appear only as the value of a custom property (`--x: …`), which *is* a
//!    definition. `color-mix()` over tokens, `var()`, `transparent`,
//!    `currentColor` and `rgba(var(--accent-rgb), a)` are compositions and pass.
//! 2. **No raw weight, hairline or bar radius** where a token exists:
//!    `font-weight: 500`, `border: 0.5px solid`, `border-radius: 3px`. (Not in
//!    `@font-face`, where a weight is a descriptor of the file.)
//! 3. **Every colour token has a value in both themes**, or is named below as
//!    the same in both with a reason; and the dark block overrides nothing the
//!    light block does not define.
//! 4. **A ratchet on colours outside the stylesheet.** The admin pages carry
//!    their own `<style>` blocks, a chart renderer writes SVG, the layout names a
//!    browser-chrome colour: each file may hold the number of raw colour
//!    literals it holds now and no more. The number can only go down; when a
//!    file improves, lower it here (the test says so).
//!
//! 5. **No raw shadow, radius, duration, curve, layer or on-ladder gap** outside a
//!    token definition. A `box-shadow` is `var(--shadow-*)`, `--ring` or a ring
//!    drawn from `--bw*`; a radius is `var(--r-*)`; a time is `var(--dur-*)`; a
//!    curve is `var(--ease-*)` or a CSS keyword; a `z-index` is `var(--z-*)`, 0,
//!    1 or -1; a `gap`, `padding` or `margin` written as a pixel count that the
//!    `--space-*` ladder has a rung for is that rung; a `font-size` is
//!    `var(--fs-*)` or an entry in [`FONT_SIZE_EXEMPT`] with a reason. Optical
//!    gaps off the ladder (5, 7, 9, 11, 13 px) are not flagged: the design-token
//!    page says why they stay.
//!
//! Each rule has a counterpart that feeds it a violation and requires it to be
//! found, because a scanner that finds nothing is indistinguishable from a
//! stylesheet that has nothing to find.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// `/* … */` removed.
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

/// One CSS declaration and the selector chain it sits under.
struct Decl {
    context: String,
    prop: String,
    value: String,
}

/// Every declaration in `css`, with the `@font-face`-style at-rule or selector
/// it appears in. Not a CSS parser: it understands `{ }`, `;`, `( )` and
/// strings, which is all this stylesheet uses.
fn declarations(css: &str) -> Vec<Decl> {
    let css = strip_comments(css);
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0_i32;
    let mut quote: Option<char> = None;
    let flush = |cur: &mut String, stack: &[String], out: &mut Vec<Decl>| {
        let d = cur.trim().to_string();
        cur.clear();
        if let Some((p, v)) = d.split_once(':')
            && !p.contains(char::is_whitespace)
            && !p.is_empty()
        {
            out.push(Decl {
                context: stack.join(" "),
                prop: p.trim().to_string(),
                value: v.trim().to_string(),
            });
        }
    };
    for ch in css.chars() {
        if let Some(q) = quote {
            cur.push(ch);
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                cur.push(ch);
            }
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            '{' if depth == 0 => {
                stack.push(cur.trim().to_string());
                cur.clear();
            }
            '}' if depth == 0 => {
                flush(&mut cur, &stack, &mut out);
                stack.pop();
            }
            ';' if depth == 0 => flush(&mut cur, &stack, &mut out),
            _ => cur.push(ch),
        }
    }
    out
}

/// Does `value` contain a colour *literal*?
///
/// A literal is a hex colour or one of the colour functions that takes numbers.
/// `color-mix(in oklch, …)` names a colour space, not a literal, and
/// `rgba(var(--accent-rgb), .2)` takes its channels from a token.
fn has_colour_literal(value: &str) -> Option<String> {
    let v = value.to_ascii_lowercase();
    let bytes = v.as_bytes();
    // #rgb / #rrggbb / #rrggbbaa
    for (i, _) in v.match_indices('#') {
        let hex = bytes[i + 1..]
            .iter()
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        let after = bytes.get(i + 1 + hex).copied();
        let boundary = after.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'-');
        if matches!(hex, 3 | 4 | 6 | 8) && boundary {
            return Some(value[i..i + 1 + hex].to_string());
        }
    }
    for f in [
        "rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "oklab(", "lab(", "lch(", "color(",
    ] {
        for (i, _) in v.match_indices(f) {
            // `color-mix(` and `foo-color(` are not `color(`.
            let before = v[..i].chars().last();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') {
                continue;
            }
            let args = &v[i + f.len()..];
            if (f == "rgba(" || f == "rgb(") && args.trim_start().starts_with("var(") {
                continue;
            }
            return Some(format!("{f}…)"));
        }
    }
    None
}

/// Violations of rules 1 and 2 in one stylesheet.
fn violations(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    for d in declarations(css) {
        let in_font_face = d.context.contains("@font-face");
        let is_definition = d.prop.starts_with("--");
        if !is_definition
            && !in_font_face
            && let Some(lit) = has_colour_literal(&d.value)
        {
            out.push(format!(
                "`{}: {}` under `{}` writes the colour {lit} instead of a token",
                d.prop,
                d.value.chars().take(60).collect::<String>(),
                d.context
            ));
        }
        if in_font_face {
            continue;
        }
        let v = d.value.replace(' ', "");
        if d.prop == "font-weight" && matches!(v.as_str(), "400" | "500" | "600" | "700") {
            out.push(format!(
                "`font-weight: {}` under `{}`: use --fw-*",
                d.value, d.context
            ));
        }
        if (d.prop == "border" || d.prop.starts_with("border-"))
            && !d.prop.contains("radius")
            && d.value.starts_with("0.5px")
        {
            out.push(format!(
                "`{}: {}` under `{}`: use var(--bw)",
                d.prop, d.value, d.context
            ));
        }
        if d.prop == "border-radius" && v == "3px" {
            out.push(format!(
                "`border-radius: 3px` under `{}`: use var(--r-2xs)",
                d.context
            ));
        }
        out.extend(scale_violations(&d));
    }
    out
}

/// The pixel ladder `--space-N` has a rung for.
const SPACE_RUNGS: &[u32] = &[2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24, 28, 32, 40, 48, 64];

/// Display sizes that are one-off glyphs and numerals and have no step on the
/// type scale: a 404's code, a gauge numeral, a recap's closing line. Each is
/// named so that a new one is a decision, not an accident.
const FONT_SIZE_EXEMPT: &[(&str, &str, &str)] = &[
    (".yir-tile-value", "40px", "the year-in-review tile numeral"),
    (".yir-close-line", "30px", "the year-in-review closing line"),
    (".rp-stat .v", "40px", "the weekly recap numeral"),
    (
        ".rp-day .dc",
        "17px",
        "a calendar day's count, sized to its cell",
    ),
    (".he-pct", "26px", "the hour-of-day gauge numeral"),
    (".pm-404-code", "48px", "the 404 code"),
    (".sh-gone-title", "48px", "the share page's gone title"),
    (".sp-count b", "38px", "the species count numeral"),
    (".sp-life-stat .v", "48px", "the life-list statistic"),
    (".pt-kpi .val", "30px", "the patterns KPI numeral"),
    (".x-daystats .v", "25px", "the day-stats numeral"),
    (".st-vital .v", "30px", "the station vital's numeral"),
    (".login-card__head .login-h1", "38px", "the sign-in title"),
    (
        ".bkr-update-ver",
        "18px",
        "the backups page's version glyph",
    ),
    (".st-status .ico", "18px", "the status icon glyph"),
    (".bnb-cmdk__icon", "18px", "the palette's search glyph"),
    (".bnb-tabbar__slot .glyph", "18px", "the tab bar's glyph"),
];

/// Whether `v` is only `var(...)`/`color-mix(...)`/keywords/numbers with no
/// length literal: that is, geometry that came from a token.
fn has_px_literal(v: &str) -> bool {
    let mut depth = 0_i32;
    let mut outside = String::new();
    let mut i = 0;
    let b = v.as_bytes();
    while i < b.len() {
        let rest = &v[i..];
        if depth == 0 && (rest.starts_with("var(") || rest.starts_with("color-mix(")) {
            // skip the whole function
            let mut d = 0_i32;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'(' => d += 1,
                    b')' => {
                        d -= 1;
                        if d == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            outside.push(' ');
            i = j;
            continue;
        }
        match b[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        outside.push(b[i] as char);
        i += 1;
    }
    outside
        .split(|c: char| c.is_whitespace() || c == ',')
        .any(|w| {
            w.strip_suffix("px").is_some_and(|n| {
                n.trim_start_matches('-')
                    .parse::<f32>()
                    .is_ok_and(|x| x != 0.0)
            })
        })
}

/// The pixel words of a value, outside any function: `8px`, `-2px`.
fn px_words(v: &str) -> Vec<u32> {
    if v.contains("calc(") || v.contains("clamp(") || v.contains("min(") || v.contains("max(") {
        return Vec::new();
    }
    v.split_whitespace()
        .filter_map(|w| w.strip_suffix("px"))
        .filter(|n| !n.starts_with('-'))
        .filter_map(|n| n.parse::<u32>().ok())
        .collect()
}

/// Rule 5 for one declaration.
fn scale_violations(d: &Decl) -> Vec<String> {
    let mut out = Vec::new();
    if d.context.contains("@keyframes") || d.prop.starts_with("--") {
        return out;
    }
    let v = d.value.as_str();
    let ctx = &d.context;
    match d.prop.as_str() {
        "box-shadow" | "text-shadow" if has_px_literal(v) => out.push(format!(
            "`{}: {v}` under `{ctx}` writes shadow geometry: use a --shadow-* token",
            d.prop
        )),
        "border-radius"
        | "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-left-radius"
        | "border-bottom-right-radius"
            if has_px_literal(v) =>
        {
            out.push(format!(
                "`{}: {v}` under `{ctx}`: use a --r-* token",
                d.prop
            ));
        }
        "z-index" => {
            let t = v.trim();
            if !(t.starts_with("var(") || matches!(t, "0" | "1" | "-1" | "auto")) {
                out.push(format!("`z-index: {t}` under `{ctx}`: use a --z-* token"));
            }
        }
        "transition"
        | "transition-duration"
        | "transition-delay"
        | "animation"
        | "animation-duration"
        | "animation-delay"
        | "transition-timing-function"
        | "animation-timing-function" => {
            if !v.contains("!important") && !ctx.contains("prefers-reduced-motion") {
                let bad = v.split(|c: char| c.is_whitespace() || c == ',').any(|w| {
                    (w.ends_with("ms") || w.ends_with('s'))
                        && w.trim_end_matches("ms")
                            .trim_end_matches('s')
                            .parse::<f32>()
                            .is_ok_and(|x| x != 0.0)
                });
                if bad {
                    out.push(format!(
                        "`{}: {v}` under `{ctx}`: use a --dur-* token",
                        d.prop
                    ));
                }
            }
            if v.contains("cubic-bezier(") {
                out.push(format!(
                    "`{}: {v}` under `{ctx}`: use a --ease-* token",
                    d.prop
                ));
            }
        }
        "font-size" => {
            let t = v.trim();
            if t.ends_with("px")
                && !t.contains('(')
                && !FONT_SIZE_EXEMPT
                    .iter()
                    .any(|(sel, size, _)| ctx.ends_with(sel) && *size == t)
            {
                out.push(format!(
                    "`font-size: {t}` under `{ctx}`: use a --fs-* token, or name it in FONT_SIZE_EXEMPT"
                ));
            }
        }
        "gap" | "row-gap" | "column-gap" | "padding" | "padding-top" | "padding-right"
        | "padding-bottom" | "padding-left" | "margin" | "margin-top" | "margin-right"
        | "margin-bottom" | "margin-left" => {
            for px in px_words(v) {
                if SPACE_RUNGS.contains(&px) {
                    out.push(format!(
                        "`{}: {v}` under `{ctx}`: {px}px is a rung of the ladder, use var(--space-{px})",
                        d.prop
                    ));
                }
            }
        }
        _ => {}
    }
    out
}

#[test]
fn the_stylesheet_writes_no_raw_colour_weight_hairline_or_bar_radius() {
    let css = read("static/css/app.css");
    let decls = declarations(&css);
    assert!(
        decls.len() > 4000,
        "the scanner found only {} declarations in app.css; it has stopped \
         reading it and would pass for the wrong reason",
        decls.len()
    );
    let v = violations(&css);
    assert!(
        v.is_empty(),
        "{} literal(s) where a token exists:\n  {}",
        v.len(),
        v.join("\n  ")
    );
}

/// The counterpart: each rule must see its own violation, and nothing else.
#[test]
fn the_scanner_sees_each_kind_of_violation_and_only_those() {
    let bad = "
        .a { color: #3a7; }
        .b { background: rgb(1, 2, 3); }
        .c { border: 0.5px solid red; }
        .d { font-weight: 500; }
        .e { border-radius: 3px; }
        .f { box-shadow: 0 0 0 1px oklch(50% 0.1 20); }
    ";
    let v = violations(bad);
    for needle in [
        "writes the colour #3a7",
        "rgb(",
        "use var(--bw)",
        "use --fw-*",
        "use var(--r-2xs)",
        "oklch(",
    ] {
        assert!(
            v.iter().any(|m| m.contains(needle)),
            "no violation mentioned `{needle}`: {v:#?}"
        );
    }
    // Eight, not six: the 3px radius is also a raw radius (rule 5) and the
    // shadow's 1px ring is also raw shadow geometry.
    assert_eq!(v.len(), 8, "{v:#?}");

    let clean = "
        :root { --x: oklch(50% 0.1 20); --y: #fff; }
        @font-face { font-family: A; font-weight: 400; src: url(a.woff2); }
        .a { color: var(--x); background: color-mix(in oklch, var(--x) 20%, transparent); }
        .b { border: var(--bw) solid var(--border); font-weight: var(--fw-medium); }
        .c { background: rgba(var(--accent-rgb), .2); outline: 2px solid currentColor; }
        .d { background: url(\"data:image/svg+xml,%3Csvg%3E\"); border-radius: var(--r-2xs); }
    ";
    assert_eq!(violations(clean), Vec::<String>::new());
}

/// The real stylesheet with one seeded violation per rule must report exactly
/// those: the test above only proves the scanner on a toy.
#[test]
fn a_violation_seeded_into_the_real_stylesheet_is_found() {
    let css = format!(
        "{}\n.seeded {{ color: #123456; font-weight: 500; border-top: 0.5px solid red; }}\n",
        read("static/css/app.css")
    );
    let v = violations(&css);
    assert_eq!(v.len(), 3, "{v:#?}");
    assert!(v.iter().all(|m| m.contains(".seeded")), "{v:#?}");
}

/// The counterpart for rule 5: each kind of raw scale literal is found, the
/// tokenised spelling of the same thing is not, and an optical gap off the
/// ladder is left alone.
#[test]
fn the_scale_rules_see_each_kind_of_literal_and_only_those() {
    let bad = "
        .a { box-shadow: 0 2px 4px red; }
        .b { border-radius: 8px; }
        .c { z-index: 40; }
        .d { transition: color .2s ease; }
        .e { transition: color var(--dur-fast) cubic-bezier(.2, 0, 0, 1); }
        .f { gap: 12px; }
        .g { padding: 6px 12px; }
        .h { font-size: 19px; }
    ";
    let v = violations(bad);
    for needle in [
        "shadow geometry",
        "use a --r-* token",
        "use a --z-* token",
        "use a --dur-* token",
        "use a --ease-* token",
        "use var(--space-12)",
        "use var(--space-6)",
        "use a --fs-* token",
    ] {
        assert!(
            v.iter().any(|m| m.contains(needle)),
            "no violation mentioned `{needle}`: {v:#?}"
        );
    }

    let clean = "
        .a { box-shadow: var(--shadow-sm), 0 0 0 var(--bw-strong) var(--moss); }
        .b { border-radius: var(--r-base) var(--r-base) 0 0; }
        .c { z-index: var(--z-sticky); }
        .d { z-index: 1; }
        .e { transition: color var(--dur-fast) ease, transform var(--dur-base) var(--ease-out); }
        .f { gap: var(--space-12); padding: 5px 9px; margin: 0 -3px; }
        .g { font-size: var(--fs-md); }
        .h { padding: calc(100% - 6px); }
        @keyframes k { 70% { box-shadow: 0 0 0 7px transparent; } }
        @media (prefers-reduced-motion: reduce) { * { animation-duration: .001ms !important; } }
    ";
    assert_eq!(violations(clean), Vec::<String>::new());
}

/// Every name in the font-size exemption list is a real declaration, so the list
/// cannot outlive the sizes it excuses.
#[test]
fn every_font_size_exemption_still_excuses_something() {
    let css = read("static/css/app.css");
    let decls = declarations(&css);
    for (sel, size, why) in FONT_SIZE_EXEMPT {
        assert!(
            decls.iter().any(|d| d.prop == "font-size"
                && d.value.trim() == *size
                && d.context.ends_with(sel)),
            "FONT_SIZE_EXEMPT names `{sel}` at {size} ({why}), and no such declaration is left: \
             delete the entry"
        );
    }
}

/// The same for rule 5: one seeded violation of each scale rule, appended to
/// the real stylesheet, is reported and nothing else is.
#[test]
fn a_scale_violation_seeded_into_the_real_stylesheet_is_found() {
    let css = format!(
        "{}\n.seeded {{ gap: 12px; border-radius: 8px; z-index: 40; transition: color .2s ease; \
         box-shadow: 0 2px 4px var(--shadow-tint); font-size: 19px; }}\n",
        read("static/css/app.css")
    );
    let v = violations(&css);
    assert_eq!(v.len(), 6, "{v:#?}");
    assert!(v.iter().all(|m| m.contains(".seeded")), "{v:#?}");
}

// ── 3. Both themes ──────────────────────────────────────────────────────────

/// Colour tokens whose value is, on purpose, the same in both themes: a fill
/// that does not change between themes, and the ink laid on it.
const THEME_INVARIANT: &[(&str, &str)] = &[
    (
        "--on-dawn",
        "dark ink on an amber fill; amber is a bright fill in both themes",
    ),
    (
        "--switch-thumb",
        "a toggle's thumb is white on its track in both themes",
    ),
    (
        "--scrim",
        "the veil behind a dialog is black at the same alpha over either page",
    ),
];

fn token_block(css: &str, selector: &str) -> BTreeMap<String, String> {
    let needle = format!("{selector} {{");
    let at = css
        .find(&needle)
        .unwrap_or_else(|| panic!("no block `{selector}`"));
    let start = at + needle.len();
    let end = css[start..]
        .find("\n}")
        .map_or_else(|| panic!("block never closes"), |i| start + i);
    let mut out = BTreeMap::new();
    for d in declarations(&format!("x {{{}}}", &css[start..end])) {
        if d.prop.starts_with("--") {
            out.insert(d.prop, d.value);
        }
    }
    out
}

fn is_colour_literal(v: &str) -> bool {
    has_colour_literal(v).is_some()
}

#[test]
fn every_colour_token_has_a_value_in_both_themes() {
    let css = strip_comments(&read("static/css/app.css"));
    let light = token_block(&css, ":root,\n:root[data-theme=\"light\"]");
    let dark = token_block(&css, ":root[data-theme=\"dark\"]");
    assert!(light.len() > 60 && dark.len() > 20, "blocks not parsed");

    let mut missing = Vec::new();
    for (k, v) in &light {
        // `--x: var(--y)` follows its target; a literal colour must be restated.
        if is_colour_literal(v)
            && !dark.contains_key(k)
            && !THEME_INVARIANT.iter().any(|(n, _)| n == k)
        {
            missing.push(format!(
                "{k}: {v} in light, nothing in dark (name it in THEME_INVARIANT with a reason, or give it a dark value)"
            ));
        }
    }
    for k in dark.keys() {
        if !light.contains_key(k) {
            missing.push(format!(
                "{k}: overridden in dark but not defined in light, so light renders it undefined"
            ));
        }
    }
    for (k, _) in THEME_INVARIANT {
        if dark.contains_key(*k) {
            missing.push(format!(
                "{k} is listed as theme-invariant but dark overrides it; remove it from THEME_INVARIANT"
            ));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

// ── A tint keeps its hue ────────────────────────────────────────────────────

/// The lines of `text` that write a mix in OKLCH, comments excluded.
fn oklch_mixes_in(label: &str, text: &str) -> Vec<String> {
    let text = text.split("#[cfg(test)]").next().unwrap_or_default();
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let t = line.trim_start();
            !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                && line.contains("color-mix(in oklch")
        })
        .map(|(n, _)| format!("{label}:{}", n + 1))
        .collect()
}

/// Every place a mix is written, by file, that mixes in OKLCH.
fn oklch_mixes() -> Vec<String> {
    let mut out = Vec::new();
    for p in source_files()
        .into_iter()
        .chain(std::iter::once(root().join("static/css/app.css")))
        .chain(std::iter::once(root().join("static/css/print.css")))
    {
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        out.extend(oklch_mixes_in(&p.display().to_string(), &text));
    }
    out
}

/// The counterpart: it finds a mix in OKLCH in code, passes the same in `OKLab`,
/// and does not mistake a comment that explains the rule for a violation.
#[test]
fn the_mix_scanner_finds_oklch_and_only_oklch() {
    let bad = "a { background: color-mix(in oklch, var(--moss) 16%, var(--surface)); }";
    assert_eq!(oklch_mixes_in("x.css", bad), ["x.css:1"]);
    let ok = "a { background: color-mix(in oklab, var(--moss) 16%, var(--surface)); }\n\
              /* color-mix(in oklch, …) turns a neutral pink */\n\
              // color-mix(in oklch, …)";
    assert!(oklch_mixes_in("x.css", ok).is_empty());
}

/// `color-mix(in oklch, …)` interpolates hue, and a neutral has none: the light
/// theme's `--surface` is `oklch(100% 0 0)`, so a 16 % tint of amber over it
/// walked from amber's hue (60) toward zero and landed on 9.6 degrees, which is
/// pink; a 16 % tint of moss (150) landed on 24, a brown. Every tinted pill,
/// toast, banner and feed row in light was a little wrong that way, and in dark
/// (where the surface's hue is 250) the waveform beside each detection mixed
/// green and grey into teal. `in oklab` interpolates the colour's coordinates
/// and has no hue to carry: a tint of X over a neutral is X, paler.
///
/// Measured in the browser, not argued: `tools/visual-qa/tokens.mjs` reads the
/// hue of what a tinted element paints.
#[test]
fn no_tint_is_mixed_in_oklch() {
    let found = oklch_mixes();
    assert!(
        found.is_empty(),
        "{} mix(es) in oklch; use `in oklab`:\n  {}",
        found.len(),
        found.join("\n  ")
    );
}

// ── High contrast has two doors and one room ───────────────────────────────

/// What a rule block declares, by its exact selector context.
fn props_under(css: &str, context: &str) -> BTreeMap<String, String> {
    declarations(css)
        .into_iter()
        .filter(|d| d.context == context)
        .map(|d| (d.prop, d.value))
        .collect()
}

/// The operator's "High contrast" preference and the OS's `prefers-contrast:
/// more` must give the same page. They are two selectors for one set of values,
/// which CSS cannot share between a media query and an attribute, so the values
/// are written twice, and a retune of one would silently leave the other behind.
#[test]
fn the_os_and_the_operators_high_contrast_declare_the_same_tokens() {
    let css = read("static/css/app.css");
    let pairs = [
        (
            ":root[data-contrast=\"high\"]",
            "@media (prefers-contrast: more) :root:not([data-contrast])",
        ),
        (
            ":root[data-contrast=\"high\"][data-theme=\"dark\"]",
            "@media (prefers-contrast: more) :root:not([data-contrast])[data-theme=\"dark\"]",
        ),
    ];
    for (operator, os) in pairs {
        let a = props_under(&css, operator);
        let b = props_under(&css, os);
        assert!(
            a.len() >= 5,
            "`{operator}` declares {} properties; the block was not found",
            a.len()
        );
        assert_eq!(
            a, b,
            "`{operator}` and `{os}` must declare the same values: an OS set to \
             more contrast and an operator who chose High contrast are asking for \
             one page"
        );
    }
}

/// The counterpart: a difference between the two is seen.
#[test]
fn a_drift_between_the_two_high_contrast_doors_is_seen() {
    let css = ":root[data-contrast=\"high\"] { --border: a; --bw: b; }\n\
               @media (prefers-contrast: more) { :root:not([data-contrast]) { --border: a; --bw: c; } }";
    assert_ne!(
        props_under(css, ":root[data-contrast=\"high\"]"),
        props_under(
            css,
            "@media (prefers-contrast: more) :root:not([data-contrast])"
        )
    );
}

// ── 4. Colours outside the stylesheet ───────────────────────────────────────

/// Raw colour literals in a non-stylesheet file: `#rgb`/`#rrggbb`, and
/// `rgb(`/`hsl(` with numbers. `&#10003;` and `href="#top"` are not colours.
fn colour_literals_in_text(src: &str) -> usize {
    let mut n = 0;
    let b = src.as_bytes();
    for (i, _) in src.match_indices('#') {
        let prev = if i == 0 { b' ' } else { b[i - 1] };
        if !matches!(
            prev,
            b':' | b' ' | b'(' | b',' | b'\'' | b'"' | b'=' | b'\t' | b'\n'
        ) {
            continue;
        }
        // An HTML id or fragment: `href="#x"`, `url(#x)`, `for="#x"`.
        // Bytes, not a `&str` slice: eight bytes back can land inside a `ü`.
        let before = &b[i.saturating_sub(8)..i];
        if before.ends_with(b"href=\"") || before.ends_with(b"url(") || before.ends_with(b"href='")
        {
            continue;
        }
        let hex = b[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        let after = b.get(i + 1 + hex).copied();
        let boundary = after.is_none_or(|c| !c.is_ascii_alphanumeric() && c != b'-' && c != b'_');
        if matches!(hex, 3 | 6 | 8) && boundary {
            n += 1;
        }
    }
    let l = src.to_ascii_lowercase();
    for f in ["rgb(", "rgba(", "hsl(", "hsla("] {
        for (i, _) in l.match_indices(f) {
            let before = l[..i].chars().last();
            // `write_png_rgba(` is a function, `foo-rgb(` is not CSS.
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                continue;
            }
            if l[i + f.len()..].trim_start().starts_with("var(") {
                continue;
            }
            n += 1;
        }
    }
    n
}

/// The number of raw colour literals each file may hold. Only ever lowered.
///
/// * `layout.html`: the two `<meta name="theme-color">` values, which are the
///   colour of the browser's own address bar and cannot be a CSS variable.
/// * `audio_player.rs`: a canvas stroke and the player's black spectrogram
///   well, in a self-contained widget.
/// * `admin/species/render.rs`: owned by the species-list work and not touched
///   by the design pass; it will lose its one when that lands.
const ALLOWED: &[(&str, usize)] = &[
    ("templates/layout.html", 2),
    ("src/routes/pages/audio_player.rs", 2),
    ("src/routes/admin/species/render.rs", 1),
];

/// Files whose colour literals are data, not styling, and so are not counted:
/// a table of colours *is* the product of the file, and a token would be the
/// wrong thing to put in it.
///
/// * `spectrogram/colormap.rs`: the perceptual colour maps a spectrogram is
///   painted with (viridis-style stops). A spectrogram is an image the server
///   draws pixel by pixel with these values; they are never written to a
///   stylesheet and a theme must not change them, or the same clip would look
///   different by day and by night.
const DATA_FILES: &[(&str, &str)] = &[(
    "src/routes/spectrogram/colormap.rs",
    "colour-map stops are the data a spectrogram is drawn from, not styling",
)];

fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("rs" | "html" | "js")
            ) && !p.to_string_lossy().ends_with("htmx.min.js")
                && !p.to_string_lossy().ends_with("htmx-sse.js")
            {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for d in ["src", "templates", "static"] {
        walk(&root().join(d), &mut out);
    }
    out
}

#[test]
fn raw_colours_outside_the_stylesheet_only_ever_go_down() {
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for p in source_files() {
        let rel = p
            .strip_prefix(root())
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        if DATA_FILES.iter().any(|(f, _)| *f == rel) {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        // Test modules quote the very literals they forbid.
        let text = text.split("#[cfg(test)]").next().unwrap_or_default();
        // A comment line is prose: it names issue numbers (`#112`), which are
        // three hex digits, and no browser reads it.
        let text: String = if p.extension().is_some_and(|e| e == "rs") {
            text.lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text.to_string()
        };
        let n = colour_literals_in_text(&text);
        if n > 0 {
            let rel = p
                .strip_prefix(root())
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            found.insert(rel, n);
        }
    }
    let mut problems = Vec::new();
    for (file, n) in &found {
        let allowed = ALLOWED
            .iter()
            .find(|(f, _)| f == file)
            .map_or(0, |(_, a)| *a);
        if *n > allowed {
            problems.push(format!(
                "{file}: {n} raw colour literal(s), {allowed} allowed — use a token (var(--x))"
            ));
        }
    }
    for (file, allowed) in ALLOWED {
        let n = found.get(*file).copied().unwrap_or(0);
        if n < *allowed {
            problems.push(format!(
                "{file}: now {n}, ALLOWED says {allowed} — lower it, so the gain is kept"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// A data-file exemption names a file that exists, so the list cannot outlive
/// the files it excuses.
#[test]
fn every_data_file_exemption_names_a_real_file() {
    for (file, why) in DATA_FILES {
        assert!(
            root().join(file).is_file(),
            "DATA_FILES excuses `{file}` ({why}), and there is no such file: delete the entry"
        );
    }
}

#[test]
fn the_literal_counter_sees_a_colour_and_ignores_an_entity_and_a_fragment() {
    assert_eq!(colour_literals_in_text("a { color: #fff; }"), 1);
    assert_eq!(
        colour_literals_in_text("fill=\"#1F1A14\" stroke='rgb(1,2,3)'"),
        2
    );
    assert_eq!(
        colour_literals_in_text("&#10003; <a href=\"#main\">x</a> url(#grad)"),
        0
    );
    assert_eq!(
        colour_literals_in_text("background: rgba(var(--accent-rgb), .2)"),
        0
    );
    assert_eq!(colour_literals_in_text("#detections-table { gap: 4px }"), 0);
    assert_eq!(colour_literals_in_text("write_png_rgba(&mut png, 1, 1)"), 0);
}
