//! `docs/book/reference/design-tokens.md` states what `app.css` declares.
//!
//! The reference page is the one place a person learns what a token means and
//! what value it carries in each theme, and nothing tied it to the stylesheet:
//! a token could be retuned, added or deleted in `app.css` and the page would
//! go on describing the old one. This reads both and fails when they differ.
//!
//! # What is compared
//!
//! * Every token declared in the light `:root` block, and every one in the
//!   dark block, must appear in the page — a new token cannot ship
//!   undocumented.
//! * Every token the page names must exist in the stylesheet — a deleted token
//!   cannot stay documented.
//! * Every value the page gives, per theme, must equal the stylesheet's, after
//!   comments and whitespace are normalised. A theme with no override of its
//!   own inherits the light value, so a three-column row (token, value, use)
//!   asserts both themes agree.
//!
//! Rows are read from the page's markdown tables: the first cell is the token
//! in backticks, and the shape of the row says what the other cells are.
//!
//! | Cells | Shape | Compared |
//! |---|---|---|
//! | 4 | token, light, dark, use | light and dark values |
//! | 3 | token, value, use | the value, which both themes must share |
//! | 2 | alias, resolves to | the `var(--x)` it is declared as |
//!
//! Component-local custom properties (`--dot`, `--sp`, `--n`, …) are set on an
//! element, not in the two theme blocks, and are not tokens in this sense.

use std::collections::BTreeMap;
use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// `/* … */` removed, so prose that mentions a token is not a declaration.
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

/// One whitespace-normalised value: runs of whitespace become one space and the
/// space just inside a bracket or after a comma is dropped, so a value wrapped
/// over two lines in the stylesheet equals the same value on one line.
fn norm(v: &str) -> String {
    let collapsed = v.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(", ", ",")
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Declarations of the first block whose selector is exactly `selector`:
/// `--name: value` pairs, split on `;` outside parentheses.
fn token_block(css: &str, selector: &str) -> BTreeMap<String, String> {
    let needle = format!("{selector} {{");
    let at = css
        .find(&needle)
        .unwrap_or_else(|| panic!("no block `{selector} {{` in app.css"));
    let body_start = at + needle.len();
    let body_end = css[body_start..].find("\n}").map_or_else(
        || panic!("block `{selector}` never closes"),
        |i| body_start + i,
    );
    let body = &css[body_start..body_end];

    let mut out = BTreeMap::new();
    let mut depth = 0_i32;
    let mut current = String::new();
    let mut flush = |decl: &mut String| {
        let d = decl.trim().to_string();
        decl.clear();
        let Some(rest) = d.strip_prefix("--") else {
            return;
        };
        if let Some((name, value)) = rest.split_once(':') {
            out.insert(format!("--{}", name.trim()), norm(value));
        }
    };
    for ch in body.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ';' if depth == 0 => flush(&mut current),
            _ => current.push(ch),
        }
    }
    flush(&mut current);
    out
}

/// A table row: the token, then the remaining cells with backticks removed.
struct Row {
    token: String,
    cells: Vec<String>,
    line: usize,
}

fn doc_rows(md: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for (i, line) in md.lines().enumerate() {
        let t = line.trim();
        if !t.starts_with("| `--") {
            continue;
        }
        let cells: Vec<String> = t
            .trim_matches('|')
            .split(" | ")
            .map(|c| c.trim().trim_matches('`').trim().to_string())
            .collect();
        let token = cells[0].clone();
        assert!(
            token.starts_with("--"),
            "line {}: first cell is not a token: {t}",
            i + 1
        );
        rows.push(Row {
            token,
            cells: cells[1..].to_vec(),
            line: i + 1,
        });
    }
    rows
}

struct Sheet {
    light: BTreeMap<String, String>,
    dark: BTreeMap<String, String>,
}

impl Sheet {
    fn load() -> Self {
        let css = strip_comments(&read("static/css/app.css"));
        Self {
            light: token_block(&css, ":root,\n:root[data-theme=\"light\"]"),
            dark: token_block(&css, ":root[data-theme=\"dark\"]"),
        }
    }

    /// What the theme resolves a token to: its own override, else the light value.
    fn dark_value(&self, t: &str) -> Option<&String> {
        self.dark.get(t).or_else(|| self.light.get(t))
    }
}

/// Complaints about `rows` measured against `sheet`, one string each.
fn problems(sheet: &Sheet, rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    let documented: Vec<&str> = rows.iter().map(|r| r.token.as_str()).collect();

    for t in sheet.light.keys().chain(sheet.dark.keys()) {
        if !documented.contains(&t.as_str()) {
            out.push(format!("{t} is declared in app.css and not documented"));
        }
    }

    for r in rows {
        let Some(light) = sheet
            .light
            .get(&r.token)
            .or_else(|| sheet.dark.get(&r.token))
        else {
            out.push(format!(
                "line {}: {} is documented and not declared in app.css",
                r.line, r.token
            ));
            continue;
        };
        let dark = sheet.dark_value(&r.token).unwrap_or(light);
        let want = |s: &str| norm(s);
        match r.cells.len() {
            // token | light | dark | use
            n if n >= 3 => {
                if want(&r.cells[0]) != *light {
                    out.push(format!(
                        "line {}: {} light is `{}` in the page, `{light}` in app.css",
                        r.line, r.token, r.cells[0]
                    ));
                }
                if want(&r.cells[1]) != *dark {
                    out.push(format!(
                        "line {}: {} dark is `{}` in the page, `{dark}` in app.css",
                        r.line, r.token, r.cells[1]
                    ));
                }
            }
            // token | value | use  (or alias | resolves to)
            2 | 1 => {
                let doc = want(&r.cells[0]);
                // An alias row names its target bare: `--surface` for var(--surface).
                let doc = if doc.starts_with("--") && !doc.contains(' ') {
                    format!("var({doc})")
                } else {
                    doc
                };
                if doc != *light {
                    out.push(format!(
                        "line {}: {} is `{}` in the page, `{light}` in app.css",
                        r.line, r.token, r.cells[0]
                    ));
                }
                if dark != light {
                    out.push(format!(
                        "line {}: {} has a dark value `{dark}` the row does not give — use the four-column form",
                        r.line, r.token
                    ));
                }
            }
            _ => out.push(format!("line {}: {} has no values", r.line, r.token)),
        }
    }
    out
}

#[test]
fn the_page_states_what_the_stylesheet_declares() {
    let sheet = Sheet::load();
    assert!(
        sheet.light.len() > 60 && sheet.dark.len() > 20,
        "parsed only {} light and {} dark tokens: the block parser has stopped \
         reading app.css, which would make every other check here vacuous",
        sheet.light.len(),
        sheet.dark.len()
    );
    let rows = doc_rows(&read("../../docs/book/reference/design-tokens.md"));
    assert!(
        rows.len() > 40,
        "read {} token rows from design-tokens.md: the table parser found nothing to compare",
        rows.len()
    );
    let mut p = problems(&sheet, &rows);
    p.sort();
    p.dedup();
    assert!(
        p.is_empty(),
        "design-tokens.md and app.css disagree ({} problems):\n  {}\n\
         The page is the reference for what a token is worth in each theme; change \
         whichever is wrong, in the same commit as the other.",
        p.len(),
        p.join("\n  ")
    );
}

/// The counterpart: the comparison must tell a wrong value, a missing token
/// and a stale token apart from a clean page, or a test that passes proves
/// nothing about what it would catch.
#[test]
fn the_comparison_notices_each_way_the_page_can_drift() {
    let mut light = BTreeMap::new();
    light.insert("--bg".into(), "oklch(98.5% 0.004 80)".into());
    light.insert("--r-sm".into(), "6px".into());
    light.insert("--bg-card".into(), "var(--surface)".into());
    let mut dark = BTreeMap::new();
    dark.insert("--bg".into(), "oklch(12% 0.008 250)".into());
    let sheet = Sheet { light, dark };

    let row = |token: &str, cells: &[&str]| Row {
        token: token.into(),
        cells: cells.iter().map(|s| (*s).to_string()).collect(),
        line: 1,
    };
    let clean = [
        row(
            "--bg",
            &["oklch(98.5% 0.004 80)", "oklch(12% 0.008 250)", "Page."],
        ),
        row("--r-sm", &["6px", "Buttons."]),
        row("--bg-card", &["--surface"]),
    ];
    assert_eq!(
        problems(&sheet, &clean),
        Vec::<String>::new(),
        "a clean page must be clean"
    );

    let wrong_dark = [
        row(
            "--bg",
            &["oklch(98.5% 0.004 80)", "oklch(13% 0.008 250)", "Page."],
        ),
        row("--r-sm", &["6px", "Buttons."]),
        row("--bg-card", &["--surface"]),
    ];
    assert!(
        problems(&sheet, &wrong_dark)
            .iter()
            .any(|p| p.contains("--bg dark")),
        "a retuned dark value must be reported"
    );

    let missing = [
        row(
            "--bg",
            &["oklch(98.5% 0.004 80)", "oklch(12% 0.008 250)", "Page."],
        ),
        row("--bg-card", &["--surface"]),
    ];
    assert!(
        problems(&sheet, &missing)
            .iter()
            .any(|p| p.contains("--r-sm is declared")),
        "a token the page omits must be reported"
    );

    let stale = [
        row(
            "--bg",
            &["oklch(98.5% 0.004 80)", "oklch(12% 0.008 250)", "Page."],
        ),
        row("--r-sm", &["6px", "Buttons."]),
        row("--bg-card", &["--surface"]),
        row("--gone", &["1px", "Old."]),
    ];
    assert!(
        problems(&sheet, &stale)
            .iter()
            .any(|p| p.contains("--gone is documented")),
        "a token the stylesheet no longer declares must be reported"
    );

    // A three-column row asserts both themes agree; a token with a dark
    // override documented that way is a drift, not a clean row.
    let hides_dark = [
        row("--bg", &["oklch(98.5% 0.004 80)", "Page."]),
        row("--r-sm", &["6px", "Buttons."]),
        row("--bg-card", &["--surface"]),
    ];
    assert!(
        problems(&sheet, &hides_dark)
            .iter()
            .any(|p| p.contains("--bg has a dark value")),
        "a three-column row must not paper over a dark override"
    );
}

// ── The numbers on the page ─────────────────────────────────────────────────

/// The marker a count sits between: `<!--count-light-->123<!--/count-->`.
fn marker(name: &str) -> (String, &'static str) {
    (format!("<!--count-{name}-->"), "<!--/count-->")
}

/// The count written between the `name` markers, if the page has them.
fn count_in(md: &str, name: &str) -> Option<usize> {
    let (open, close) = marker(name);
    let start = md.find(&open)? + open.len();
    let end = start + md[start..].find(close)?;
    md[start..end].trim().parse().ok()
}

/// `md` with the `name` count replaced by `n`.
fn with_count(md: &str, name: &str, n: usize) -> String {
    let (open, close) = marker(name);
    let Some(at) = md.find(&open) else {
        return md.to_string();
    };
    let start = at + open.len();
    let Some(len) = md[start..].find(close) else {
        return md.to_string();
    };
    format!("{}{n}{}", &md[..start], &md[start + len..])
}

/// "N tokens in the light block, M overrides in the dark block" is the first
/// thing the page says about its own size, so it is read from the stylesheet
/// and not typed. Run with `BNB_BLESS_DOC=1` to rewrite the two numbers.
#[test]
fn the_counts_on_the_page_are_the_stylesheets_counts() {
    let sheet = Sheet::load();
    let rel = "../../docs/book/reference/design-tokens.md";
    let md = read(rel);
    let (light, dark) = (sheet.light.len(), sheet.dark.len());

    if std::env::var_os("BNB_BLESS_DOC").is_some() {
        let blessed = with_count(&with_count(&md, "light", light), "dark", dark);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::write(&path, blessed).expect("rewrite design-tokens.md");
        return;
    }
    assert_eq!(
        count_in(&md, "light"),
        Some(light),
        "design-tokens.md says a different number of light tokens than app.css declares \
         ({light}); run `BNB_BLESS_DOC=1 cargo test -p birdnet-web --test \
         design_tokens_doc_matches_the_stylesheet` to rewrite it"
    );
    assert_eq!(
        count_in(&md, "dark"),
        Some(dark),
        "design-tokens.md says a different number of dark overrides than app.css declares \
         ({dark}); run `BNB_BLESS_DOC=1 cargo test -p birdnet-web --test \
         design_tokens_doc_matches_the_stylesheet` to rewrite it"
    );
}

/// The counterpart: the reader and the rewriter agree, and a page with no
/// marker reads as no number, which the test above treats as a failure and not
/// as a pass.
#[test]
fn the_count_markers_read_and_rewrite() {
    let md = "has <!--count-light-->0<!--/count--> and <!--count-dark-->7<!--/count-->.";
    assert_eq!(count_in(md, "light"), Some(0));
    assert_eq!(count_in(md, "dark"), Some(7));
    let fixed = with_count(&with_count(md, "light", 312), "dark", 41);
    assert_eq!(count_in(&fixed, "light"), Some(312));
    assert_eq!(count_in(&fixed, "dark"), Some(41));
    assert_eq!(count_in("no markers here", "light"), None);
    assert_eq!(with_count("no markers here", "light", 3), "no markers here");
}
