//! A Rust escape written inside a raw string is printed, not interpreted.
//!
//! `r#"…a source\'s…"#` is a backslash, then an apostrophe — in the page. The
//! Settings page's Audio Channels note said "a source\'s recording quality" for
//! as long as the note existed, in a string whose author wrote the apostrophe
//! the way an ordinary string needs it. Nothing failed: it is valid Rust, valid
//! HTML, and the text reads as a typo to whoever reaches that paragraph.
//!
//! Two checks, because each sees what the other cannot:
//!
//! 1. **The page** — `/admin/settings` is fetched and must carry the words and
//!    no `\'` in what a person reads (scripts and styles aside, where a
//!    backslash is the language's).
//! 2. **The source** — no raw string in the web crate or the binary contains
//!    `\'`. A raw string never needs it, so a hit is this defect; JavaScript
//!    that needs one belongs on the list below, with its reason.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// `(file, why)` for a raw string that has to carry `\'` — JavaScript inside a
/// page, where the browser reads the backslash. Empty: none does today.
const JAVASCRIPT_THAT_NEEDS_IT: &[(&str, &str)] = &[];

/// Raw string literals in `text`, as `(1-based line, body)`.
///
/// Reads `r"…"` and `r#"…"#` (any number of hashes), with an optional `b`
/// before the `r`. It does not parse Rust, so it reads a `r"` inside an
/// ordinary string as a raw string's start; the test below holds it to the
/// shapes it is meant for, and the crate has no such string today.
fn raw_strings(text: &str) -> Vec<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // A raw string starts at an `r` that is not the tail of an identifier
        // (`for"` is not one), after an optional `b`.
        let after_ident = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        let prefix_len = match bytes[i] {
            b'r' if !after_ident => 1,
            b'b' if !after_ident && bytes.get(i + 1) == Some(&b'r') => 2,
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i + prefix_len;
        let hashes = bytes[j..].iter().take_while(|&&b| b == b'#').count();
        j += hashes;
        if bytes.get(j) != Some(&b'"') {
            i += 1;
            continue;
        }
        let body_start = j + 1;
        let close: String = std::iter::once('"')
            .chain(std::iter::repeat_n('#', hashes))
            .collect();
        let Some(rel) = text[body_start..].find(&close) else {
            break;
        };
        let line = text[..body_start].matches('\n').count() + 1;
        out.push((line, &text[body_start..body_start + rel]));
        i = body_start + rel + close.len();
    }
    out
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// `file:line` for every raw string in `files` that contains `\'`.
fn escapes_in_raw_strings(files: &[(String, String)]) -> Vec<String> {
    let mut hits = Vec::new();
    for (name, text) in files {
        for (line, body) in raw_strings(text) {
            if body.contains("\\'") && !JAVASCRIPT_THAT_NEEDS_IT.iter().any(|(f, _)| f == name) {
                hits.push(format!("{name}:{line}"));
            }
        }
    }
    hits
}

#[test]
fn the_scan_can_tell_a_raw_string_from_an_ordinary_one() {
    // Found: the shape that was in `audio.rs`, with and without hashes.
    let raw = "let a = r#\"a source\\'s quality\"#;\nlet b = r\"x\\'y\";\n";
    let found = raw_strings(raw);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|(_, body)| body.contains("\\'")));
    assert_eq!(found[0].0, 1);
    assert_eq!(found[1].0, 2);

    // Not found: an ordinary string, where `\'` is an escape that works, and
    // a raw string that does not carry one.
    let ordinary = "let a = \"a source\\'s quality\";\nlet b = r#\"a source's quality\"#;\n";
    let found = raw_strings(ordinary);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(!found[0].1.contains("\\'"));

    // An identifier ending in `r` is not the start of one.
    assert!(raw_strings("for\"x\\'y\"").is_empty());
}

#[test]
fn no_raw_string_in_the_web_crate_or_the_binary_carries_a_rust_escape() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    rs_files(&root.join("src"), &mut paths);
    rs_files(&root.join("../../src"), &mut paths);
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            (
                p.strip_prefix(root)
                    .unwrap_or(p)
                    .display()
                    .to_string()
                    .replace("../../", ""),
                std::fs::read_to_string(p).expect("read source"),
            )
        })
        .collect();
    // The walk found the code: a scan of nothing is clean.
    let raw_count: usize = files.iter().map(|(_, t)| raw_strings(t).len()).sum();
    // 1,080 when this was written; the floor is a fraction of that, so deleting a
    // directory from the walk fails and ordinary churn does not.
    assert!(raw_count > 900, "scanned only {raw_count} raw strings");

    let hits = escapes_in_raw_strings(&files);
    assert!(
        hits.is_empty(),
        "a raw string carries `\\'`, which is printed as a backslash and an apostrophe; \
         write the apostrophe plainly:\n{}",
        hits.join("\n")
    );
}

/// What a person reads: the page without its scripts and styles.
fn read_by_a_person(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script").or_else(|| rest.find("<style")) {
        out.push_str(&rest[..start]);
        let tag = if rest[start..].starts_with("<script") {
            "</script>"
        } else {
            "</style>"
        };
        rest = rest[start..]
            .find(tag)
            .map_or("", |end| &rest[start + end + tag.len()..]);
    }
    out.push_str(rest);
    out
}

#[tokio::test]
async fn the_settings_page_says_source_s_without_a_backslash() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let resp = build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
    .oneshot(
        Request::builder()
            .uri("/admin/settings")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8_lossy(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    let text = read_by_a_person(&html);

    // The note is on the page: a page that dropped it would pass the next check
    // for the wrong reason.
    assert!(
        text.contains("you set a source's"),
        "the Audio Channels note is not on the settings page, or reads differently"
    );
    assert!(
        !text.contains("\\'"),
        "the settings page prints a backslash before an apostrophe"
    );
}
