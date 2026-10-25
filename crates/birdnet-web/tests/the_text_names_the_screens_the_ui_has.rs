//! The screens the manual and the product's own sentences send a reader to
//! are the screens the navigation has.
//!
//! The top-level section was renamed from "Station" to "Settings" and its
//! "Settings" tab to "General". The nav, the page titles and most of the
//! product's own sentences moved with it; the manual did not, and neither did
//! the sr-only `<h1>` every tab opens with. A reader following "**Station →
//! Capture**" looked for a tab that no longer has that name, and one following
//! "Settings → Accounts" or "Admin → Settings → MQTT" was sent to places that
//! were never there. Nothing failed: a sentence cannot fail to compile.
//!
//! Two checks, both against what the router renders rather than against a
//! list kept here:
//!
//! 1. Each tab's `<h1>` is "{the nav's label for the section} — {the tab's
//!    label}", both read from that tab's own page.
//! 2. Every breadcrumb that starts at **Settings** in the manual, the README,
//!    the Rust sources' string literals and the installer's messages names a
//!    tab the page really has, and none starts at the old **Station**.
//!
//! What this does not check: that the third step of a breadcrumb exists on the
//! tab (several are loaded on demand, so a text match on the page would reject
//! real ones), and prose that names a screen without an arrow.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const TABS: &[&str] = &[
    "/station",
    "/station/capture",
    "/station/alerts",
    "/station/data",
    "/station/settings",
    "/station/access",
];

async fn get(uri: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    let res = birdnet_web::server::build_router(state)
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    assert!(res.status().is_success(), "{uri}: {}", res.status());
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The text between `open` and the next `close`, starting the search at the
/// first `open` in `html`.
fn between<'a>(html: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = html.find(open)? + open.len();
    let len = html[start..].find(close)?;
    Some(&html[start..start + len])
}

/// What the primary nav calls the section the six tabs belong to.
fn nav_label(html: &str) -> String {
    between(html, r#"<a href="/station" class="topnav-link"#, "</a>")
        .and_then(|a| a.split_once('>'))
        .map(|(_, text)| text.trim().to_owned())
        .expect("the primary nav links /station")
}

/// The labels of the sub-tab row, in order.
fn tab_labels(html: &str) -> Vec<String> {
    let row = between(html, r#"<nav class="bnb-subtabs""#, "</nav>").expect("sub-tab row");
    row.split(r#"<span class="l">"#)
        .skip(1)
        .filter_map(|s| s.split("</span>").next())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn every_station_tab_names_itself_as_the_nav_does() {
    let mut wrong = Vec::new();
    for (i, tab) in TABS.iter().enumerate() {
        let html = get(tab).await;
        let section = nav_label(&html);
        let label = &tab_labels(&html)[i];
        let want = format!("{section} — {label}");
        let h1 = between(&html, r#"<h1 class="sr-only">"#, "</h1>").unwrap_or("(none)");
        if h1 != want {
            wrong.push(format!("{tab}: <h1> is {h1:?}, the nav says {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// One breadcrumb found in text: where it starts and what follows each arrow.
#[derive(Debug, PartialEq, Eq)]
struct Crumb {
    head: String,
    steps: Vec<String>,
}

const ARROWS: &[&str] = &["→", "->", "&rarr;"];

/// Text up to the next thing that ends a step in a breadcrumb: emphasis or
/// code or link markup, a bracket, a quote, a comma, a colon, a semicolon, an
/// arrow, or a full stop before a space.
fn step_end(s: &str) -> usize {
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        let sentence_end = c == '.' && s[i + 1..].starts_with(' ');
        if matches!(
            c,
            '*' | '`' | '[' | ']' | '(' | ')' | '<' | '"' | ',' | ';' | ':' | '→' | '—' | '–'
        ) || sentence_end
            || s[i..].starts_with("->")
            || s[i..].starts_with("&rarr;")
        {
            end = i;
            break;
        }
    }
    end
}

/// Every breadcrumb in `text` whose first word is `Settings` or `Station`.
///
/// `text` is one line: a breadcrumb the source wrapped must still be read, so
/// callers join first (see [`flatten`]).
fn crumbs(text: &str) -> Vec<Crumb> {
    let mut found = Vec::new();
    for head in ["Settings", "Station"] {
        let mut from = 0;
        while let Some(at) = text[from..].find(head) {
            let start = from + at;
            from = start + head.len();
            let before = text[..start].chars().next_back();
            if before.is_some_and(char::is_alphanumeric) {
                continue;
            }
            // macOS's own "System Settings", and GitHub's repository Settings.
            let lead = &text[..start];
            if ["System ", "Repo ", "repo ", "GitHub ", "repository "]
                .iter()
                .any(|w| lead.ends_with(w))
            {
                continue;
            }
            let mut rest = text[from..].trim_start_matches(['*', '_']).trim_start();
            if arrow_len(rest) == 0 {
                continue;
            }
            let mut steps = Vec::new();
            loop {
                rest = rest[arrow_len(rest)..].trim_start_matches([' ', '*', '_']);
                let end = step_end(rest);
                steps.push(rest[..end].trim().to_owned());
                rest = rest[end..].trim_start_matches(['*', '_', ' ']);
                if !ARROWS.iter().any(|a| rest.starts_with(a)) {
                    break;
                }
            }
            found.push(Crumb {
                head: head.to_owned(),
                steps,
            });
            // Resume after the breadcrumb just read: a step can be named
            // "Detection Settings", and the word inside it starts no path of
            // its own. Reading it as one reported 19 correct paths as
            // misdirections, all of the shape "Settings → Capture → Detection
            // Settings → Minimum Confidence".
            from = from.max(text.len() - rest.len());
        }
    }
    found
}

fn arrow_len(rest: &str) -> usize {
    ARROWS
        .iter()
        .find(|a| rest.starts_with(**a))
        .map_or(0, |a| a.len())
}

/// A file as one line of what a reader would read: no comment lines in Rust,
/// a string continuation (`\` at the end of a line) joined without a space,
/// every other line break a single space, Markdown quote markers dropped.
fn flatten(path: &Path, source: &str) -> String {
    let is_rust = path.extension().is_some_and(|e| e == "rs");
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let line = line.trim_start().trim_start_matches('>').trim_start();
        if is_rust && line.starts_with("//") {
            continue;
        }
        if let Some(cont) = line.strip_suffix('\\') {
            out.push_str(cont);
        } else {
            out.push_str(line);
            out.push(' ');
        }
    }
    out
}

fn files_under(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // `_generated` is the rendered copy of the Markdown beside it.
            if name != "_generated" && name != "node_modules" && name != "target" {
                files_under(&path, extensions, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| extensions.iter().any(|x| e == *x))
        {
            out.push(path);
        }
    }
}

fn corpus(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.join("README.md")];
    files_under(&root.join("docs/book"), &["md"], &mut files);
    files_under(&root.join("src"), &["rs"], &mut files);
    files_under(&root.join("installer/lib"), &["sh"], &mut files);
    if let Ok(crates) = std::fs::read_dir(root.join("crates")) {
        for c in crates.flatten() {
            files_under(&c.path().join("src"), &["rs"], &mut files);
        }
    }
    files.sort();
    files
}

#[tokio::test]
async fn the_text_sends_readers_to_tabs_the_page_has() {
    let labels = tab_labels(&get("/station").await);
    assert_eq!(labels.len(), TABS.len(), "{labels:?}");
    let section = nav_label(&get("/station").await);

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = corpus(&root);
    assert!(
        files.len() > 100,
        "the scan found {} files — is the workspace layout what it was?",
        files.len()
    );

    let mut wrong = Vec::new();
    let mut checked = 0_usize;
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path.strip_prefix(&root).unwrap_or(path).display();
        for crumb in crumbs(&flatten(path, &source)) {
            if crumb.head == "Station" {
                wrong.push(format!(
                    "{rel}: \"Station → {}\" — the section is called {section:?}",
                    crumb.steps.join(" → ")
                ));
                continue;
            }
            let first = &crumb.steps[0];
            // "Open Settings →" is a button's arrow, not a path.
            if first.is_empty() {
                continue;
            }
            checked += 1;
            let names_a_tab = labels.iter().any(|l| {
                first
                    .strip_prefix(l.as_str())
                    .is_some_and(|tail| !tail.starts_with(char::is_alphanumeric))
            });
            if !names_a_tab {
                wrong.push(format!(
                    "{rel}: \"Settings → {}\" — the tabs are {labels:?}",
                    crumb.steps.join(" → ")
                ));
            }
        }
    }
    assert!(
        checked > 15,
        "only {checked} Settings breadcrumbs were found; the scan has stopped seeing them"
    );
    assert!(
        wrong.is_empty(),
        "{} misdirections:\n{wrong:#?}",
        wrong.len()
    );
}

/// The scan's own counterpart: it reads what a person reads, finds a
/// breadcrumb wherever the source wrapped it, and ignores the sentences that
/// only look like one.
#[test]
fn the_scan_reads_breadcrumbs_the_way_a_reader_does() {
    let one = |s: &str| crumbs(s).pop().expect("a breadcrumb");

    let c = one("open **Settings → Capture → Species**, then");
    assert_eq!(
        (c.head.as_str(), c.steps),
        ("Settings", vec!["Capture".into(), "Species".into()])
    );

    let c = one("under Settings → Alerts. Next sentence");
    assert_eq!(c.steps, vec!["Alerts".to_string()]);

    let c = one(r#"<a href="/station/data">Settings → Data</a>"#);
    assert_eq!(c.steps, vec!["Data".to_string()]);

    assert_eq!(one("the **Station → Data** tab").head, "Station");
    assert_eq!(
        one("see [Station → Health](../x.md)").steps,
        vec!["Health".to_string()]
    );
    assert_eq!(
        one("under Settings -> Capture").steps,
        vec!["Capture".to_string()]
    );

    // A step that is itself called "... Settings" is a step, not a second
    // breadcrumb: one path in, one path out. Counterpart: a real second
    // breadcrumb later in the same line is still found.
    let all = crumbs("under Settings → Capture → Detection Settings → Minimum Confidence (0–1)");
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(
        all[0].steps,
        vec![
            "Capture".to_string(),
            "Detection Settings".to_string(),
            "Minimum Confidence".to_string()
        ]
    );
    let two = crumbs("under Settings → Capture → Species, or Settings → Nowhere → X");
    assert_eq!(two.len(), 2, "{two:?}");
    assert_eq!(two[1].steps[0], "Nowhere");

    // Wrapped lines are joined before the scan, so the arrow and its step meet.
    let flat = flatten(
        Path::new("x.md"),
        "the suspect list under Station →\nData, then\n",
    );
    assert_eq!(one(&flat).steps, vec!["Data".to_string()]);
    let flat = flatten(
        Path::new("x.rs"),
        "\"add one under Settings → \\\n      Capture\"",
    );
    assert_eq!(one(&flat).steps, vec!["Capture".to_string()]);

    // Not breadcrumbs.
    assert!(crumbs("macOS: System Settings → Privacy & Security").is_empty());
    assert!(crumbs("the Settings page opens").is_empty());
    assert!(crumbs("Resettings → x").is_empty());
    assert_eq!(
        flatten(Path::new("x.rs"), "// Settings → Detection\nlet x = 1;").trim(),
        "let x = 1;"
    );
}
