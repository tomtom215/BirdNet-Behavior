//! A value in the URL is text on the page, never markup.
//!
//! `inject_script_nonce` stamps the per-request nonce onto **every** `<script`
//! in the finished body, an attacker's included, so any renderer that puts a
//! query value into HTML unescaped is a script-execution bug that the strict
//! CSP does not stop. Two were found by an audit, one at a time:
//! `/quarantine?filter=` (written into an `hx-get="…"` attribute) and
//! `/pages/weekly-content?week=` (checked only for being ten characters long).
//! The page-title bug before them was the same class and was fixed alone.
//!
//! This is the sweep. It requests every page and partial route the visual-QA
//! sweep lists, that `src/routes/pages` registers, or that those pages load
//! with `hx-get`, with **every** `String`-typed parameter name found in
//! `src/routes` set at once to a probe, and fails on any response that puts
//! the probe back as markup:
//!
//! * `ZQ"><zq-probe>ZQ` and a ten-character `"><zq-p>12` (the weekly report
//!   accepts a `week` only if it is ten characters long) must not reappear
//!   with their `<`;
//! * `ZQ" zq-attr="1` must not reappear inside a tag, where its `"` would end
//!   an attribute and `zq-attr` would become one.
//!
//! A handler that rejects the combined query (a parameter it types as a number
//! or a date, set to a probe that is neither) answers 4xx without rendering
//! anything, so each route that did is asked again one parameter at a time.
//!
//! What this does not cover: a value the handler stores and a later page
//! renders (`a_stored_setting_is_shown_as_text.rs` holds that side), POST
//! bodies, and JSON responses, which are not HTML.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::rate_limit::RateLimitConfig;
use birdnet_web::server::build_router_with_rate_limit;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const LONG_TAG: &str = "ZQ\"><zq-probe>ZQ";
const TEN_CHAR_TAG: &str = "\"><zq-p>12";
const ATTRIBUTE: &str = "ZQ\" zq-attr=\"1";

/// Routes that do something a read-only sweep must not trigger, or that
/// stream and never end.
const SKIPPED: &[&str] = &[
    "/admin/system/backup/full",
    "/admin/support-bundle",
    "/api/",
    "/r/",
    "/help/",
    "/logout",
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable source directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `pub name: Option<String>,` / `name: String,` field names in `src/routes`:
/// every query or form field a handler reads as text.
fn string_params() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src/routes"), &mut files);
    let mut out = BTreeSet::new();
    for file in files {
        let src = std::fs::read_to_string(&file).expect("readable source");
        for line in src.lines() {
            let line = line.trim();
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let Some((name, ty)) = line.split_once(": ") else {
                continue;
            };
            if matches!(ty.trim_end_matches(','), "String" | "Option<String>")
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
            {
                out.insert(name.to_owned());
            }
        }
    }
    out
}

/// String literals that start with `/` in `qa.mjs`'s `ROUTES` table, the same
/// parse `every_link_on_every_page_resolves.rs` uses.
fn qa_pages() -> BTreeSet<String> {
    let src = std::fs::read_to_string(crate_dir().join("../../tools/visual-qa/qa.mjs"))
        .expect("qa.mjs is readable");
    let table = src
        .split_once("export const ROUTES = [")
        .expect("qa.mjs exports a ROUTES table")
        .1
        .split_once("\n];")
        .expect("ROUTES table is closed")
        .0;
    let mut out = BTreeSet::new();
    for line in table.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for quote in ['\'', '`'] {
            let mut rest = line;
            while let Some(start) = rest.find(quote) {
                rest = &rest[start + 1..];
                let Some(end) = rest.find(quote) else { break };
                let candidate = &rest[..end];
                if candidate.starts_with('/') && !candidate.contains("${") {
                    out.insert(candidate.split('?').next().unwrap_or(candidate).to_owned());
                }
                rest = &rest[end + 1..];
            }
        }
    }
    out
}

/// Paths registered with `.route("/…"` under `src/routes/pages`, without path
/// parameters: the page and partial layer.
fn registered_page_routes() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src/routes/pages"), &mut files);
    let mut out = BTreeSet::new();
    for file in files {
        let src = std::fs::read_to_string(&file).expect("readable source");
        let mut rest = src.as_str();
        while let Some(i) = rest.find(".route(") {
            rest = &rest[i + ".route(".len()..];
            let trimmed = rest.trim_start();
            if let Some(after) = trimmed.strip_prefix('"')
                && let Some(end) = after.find('"')
            {
                let path = &after[..end];
                if path.starts_with('/') && !path.contains('{') && !path.contains(':') {
                    out.insert(path.to_owned());
                }
            }
        }
    }
    out
}

/// `hx-get="/pages/…"` targets in a rendered page.
fn partials_in(html: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = html;
    while let Some(i) = rest.find("hx-get=\"") {
        rest = &rest[i + "hx-get=\"".len()..];
        let Some(end) = rest.find('"') else { break };
        let raw = rest[..end].replace("&amp;", "&");
        rest = &rest[end..];
        let path = raw.split(['?', '#']).next().unwrap_or(&raw);
        if path.starts_with("/pages/") && !path.contains('{') {
            out.insert(path.to_owned());
        }
    }
    out
}

fn station() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) VALUES
           (date('now','localtime'), '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91, 'b.wav'),
           (date('now','localtime','-1 day'), '07:20:00', 'Erithacus rubecula', 'European Robin', 0.88, 'r.wav');",
    )
    .expect("seed");
    build_router_with_rate_limit(
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:")),
        RateLimitConfig {
            requests_per_second: 1e9,
            burst_capacity: u32::MAX,
            ..RateLimitConfig::default()
        },
    )
}

/// Status and, for an HTML response only, the body. A response that is not
/// HTML is not read: a stream would never end.
async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router responds");
    let status = res.status();
    let is_html = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if !is_html {
        return (status, None);
    }
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    (status, Some(String::from_utf8_lossy(&body).into_owned()))
}

fn encode(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Does `needle` occur in `html` inside a tag: after a `<` with no `>` since?
fn inside_a_tag(html: &str, needle: &str) -> bool {
    html.match_indices(needle).any(|(at, _)| {
        let before = &html[..at];
        before
            .rfind('<')
            .is_some_and(|lt| before.rfind('>').is_none_or(|gt| lt > gt))
    })
}

/// What, if anything, `html` does with the probes in it.
fn violation(html: &str) -> Option<&'static str> {
    if html.contains("<zq-probe") || html.contains("<zq-p>") {
        Some("an injected tag came back as a tag")
    } else if inside_a_tag(html, ATTRIBUTE) {
        Some("an injected quote ended an attribute")
    } else {
        None
    }
}

fn query(params: &[&str], value: &str) -> String {
    params
        .iter()
        .map(|p| format!("{p}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

#[tokio::test]
async fn a_value_in_the_url_is_never_put_back_as_markup() {
    let app = station();
    let params: Vec<String> = string_params().into_iter().collect();
    let param_refs: Vec<&str> = params.iter().map(String::as_str).collect();
    assert!(
        params.len() > 40
            && params.iter().any(|p| p == "filter")
            && params.iter().any(|p| p == "week"),
        "the parameter scan found {params:?}; has the field layout changed?"
    );

    // Routes: the sweep's pages, the registered page layer, and every partial
    // those pages load.
    let mut routes: BTreeSet<String> = qa_pages();
    routes.extend(registered_page_routes());
    assert!(routes.len() > 60, "found only {routes:?}");
    let mut partials = BTreeSet::new();
    for route in &routes {
        if let (status, Some(html)) = get(&app, route).await
            && status.is_success()
        {
            partials.extend(partials_in(&html));
        }
    }
    routes.extend(partials);
    routes.retain(|r| !SKIPPED.iter().any(|s| r.starts_with(s)));

    let mut found = Vec::new();
    let mut probed_ok = 0usize;
    for route in &routes {
        for value in [LONG_TAG, TEN_CHAR_TAG, ATTRIBUTE] {
            let (status, body) = get(&app, &format!("{route}?{}", query(&param_refs, value))).await;
            if let Some(html) = body.as_deref().filter(|_| status.is_success()) {
                probed_ok += 1;
                if let Some(what) = violation(html) {
                    found.push(format!("{route} (all parameters = {value}): {what}"));
                }
                continue;
            }
            if !status.is_client_error() {
                continue;
            }
            // Rejected as a whole. One parameter at a time.
            for param in &param_refs {
                let (status, body) =
                    get(&app, &format!("{route}?{}", query(&[param], value))).await;
                if let Some(html) = body.as_deref().filter(|_| status.is_success()) {
                    probed_ok += 1;
                    if let Some(what) = violation(html) {
                        found.push(format!("{route}?{param}={value}: {what}"));
                    }
                }
            }
        }
    }

    // The precondition: the sweep rendered a great many responses, so a
    // handler that answers 4xx to everything cannot pass it by default.
    assert!(
        probed_ok > routes.len(),
        "only {probed_ok} responses were examined for {} routes",
        routes.len()
    );
    assert!(
        found.is_empty(),
        "{} response(s) put a URL value back as markup:\n  {}",
        found.len(),
        found.join("\n  ")
    );
}

#[test]
fn the_probes_are_recognised_when_they_come_back_raw() {
    // Counterpart: the check can fail. Each shape it exists to catch, and the
    // escaped forms it must let through.
    assert!(violation(&format!("<p>{LONG_TAG}</p>")).is_some());
    assert!(violation(&format!("<p>{TEN_CHAR_TAG}</p>")).is_some());
    assert!(violation(&format!("<div hx-get=\"/x?f={ATTRIBUTE}\">")).is_some());
    assert!(violation("<p>ZQ&quot;&gt;&lt;zq-probe&gt;ZQ</p>").is_none());
    assert!(violation("<div hx-get=\"/x?f=ZQ&quot; zq-attr=&quot;1\">").is_none());
    // A quote in a text node ends nothing.
    assert!(violation(&format!("<p>{ATTRIBUTE}</p>")).is_none());
    assert_eq!(
        TEN_CHAR_TAG.len(),
        10,
        "the weekly report wants ten characters"
    );
}

#[tokio::test]
async fn the_values_the_pages_do_understand_still_work() {
    // Counterpart: the fixes drop what is not a date or a filter, not what is.
    let app = station();

    let (status, body) = get(&app, "/pages/weekly-content?week=2026-03-09").await;
    let body = body.expect("html");
    assert!(status.is_success());
    assert!(
        body.contains("2026-03-09 &ndash; 2026-03-15"),
        "a valid week must be the week shown"
    );

    let (_, body) = get(&app, "/quarantine?filter=all").await;
    let body = body.expect("html");
    assert!(
        body.contains("hx-get=\"/pages/quarantine-list?filter=all\""),
        "a known filter must reach the list request"
    );
    assert!(
        body.contains("class=\"qz-filter-link active\" ") || body.contains("qz-filter-link active"),
        "and select its tab"
    );

    // Anything else falls back to the default rather than being echoed.
    let (_, body) = get(&app, "/quarantine?filter=%22%3Ex").await;
    assert!(
        body.expect("html")
            .contains("hx-get=\"/pages/quarantine-list?filter=pending\"")
    );
}
