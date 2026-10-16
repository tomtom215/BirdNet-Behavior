//! Every `<script>` and `<style>` a page ships has the nonce, and nothing
//! ships the seal.
//!
//! The nonce goes onto the tags this server *sealed* and onto nothing else
//! (`src/seal.rs`), so that a visitor's text which reached a page unescaped
//! cannot take it. The price is that authored markup nobody sealed is not
//! admitted either: the page loses a script, and the browser says so only in
//! its console. This renders every page the visual-QA sweep covers
//! (`tools/visual-qa/qa.mjs` `ROUTES`, the one list) and the `/pages/…`
//! partials they load, and fails on any opening tag without the response's
//! own nonce, and on any response that still carries the seal's attribute (the
//! seal is a secret only while it stays in the process).
//!
//! Also here: the other half of the discrimination, so that a gate that
//! rejects everything cannot pass for one that discriminates. A reflection
//! that a page would print is not given a nonce, which
//! `security::tests::a_script_reflected_into_a_page_does_not_come_out_as_a_script`
//! holds at the unit level; the last test drives the real router.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::rate_limit::RateLimitConfig;
use birdnet_web::server::build_router_with_rate_limit;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn pages() -> BTreeSet<String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/visual-qa/qa.mjs");
    let src = std::fs::read_to_string(path).expect("qa.mjs is readable");
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
                    out.insert(candidate.to_owned());
                }
                rest = &rest[end + 1..];
            }
        }
    }
    out
}

fn partials(html: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = html;
    while let Some(i) = rest.find("hx-get=\"") {
        rest = &rest[i + "hx-get=\"".len()..];
        let Some(end) = rest.find('"') else { break };
        let raw = rest[..end].replace("&amp;", "&");
        rest = &rest[end..];
        if raw.starts_with("/pages/") && !raw.contains('{') {
            out.insert(raw);
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

struct Page {
    status: StatusCode,
    csp: String,
    html: String,
}

async fn get(app: &axum::Router, uri: &str) -> Page {
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
    let csp = res
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/html"));
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    Page {
        status,
        csp,
        html: if is_html {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            String::new()
        },
    }
}

fn nonce_of(csp: &str) -> Option<&str> {
    csp.split("'nonce-").nth(1)?.split('\'').next()
}

/// The opening tags of `<script>` and `<style>` in `html`, as written.
fn open_tags(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for tag in ["<script", "<style"] {
        let mut from = 0;
        while let Some(i) = html[from..].find(tag) {
            let start = from + i;
            let after = html[start + tag.len()..].chars().next();
            from = start + tag.len();
            if after.is_some_and(|c| c.is_ascii_whitespace() || c == '>' || c == '/') {
                let end = html[start..]
                    .find('>')
                    .map_or(html.len(), |e| start + e + 1);
                out.push(&html[start..end]);
            }
        }
    }
    out
}

#[tokio::test]
async fn every_script_and_style_on_every_page_carries_the_nonce() {
    let app = station();
    let seeds = pages();
    assert!(seeds.len() > 30, "the qa.mjs ROUTES parse found {seeds:?}");

    let mut checked_tags = 0_usize;
    let mut bare = Vec::new();
    let mut leaked = Vec::new();
    let mut rendered = 0_usize;
    let mut queue: Vec<String> = seeds.into_iter().collect();
    let mut seen = BTreeSet::new();
    while let Some(uri) = queue.pop() {
        if !seen.insert(uri.clone()) {
            continue;
        }
        let page = get(&app, &uri).await;
        if !page.status.is_success() || page.html.is_empty() {
            continue;
        }
        rendered += 1;
        let nonce = nonce_of(&page.csp)
            .unwrap_or_else(|| panic!("{uri}: an HTML response with no CSP nonce: {:?}", page.csp));
        for tag in open_tags(&page.html) {
            checked_tags += 1;
            if !tag.contains(&format!("nonce=\"{nonce}\"")) {
                bare.push(format!("{uri}: {tag}"));
            }
        }
        if page.html.contains("data-bnb-seal") {
            leaked.push(uri.clone());
        }
        queue.extend(partials(&page.html));
    }

    assert!(rendered > 40, "only {rendered} pages rendered");
    assert!(
        checked_tags > 100,
        "only {checked_tags} script/style tags were examined; is the extraction broken?"
    );
    assert!(
        leaked.is_empty(),
        "the seal reached the browser on {leaked:?}"
    );
    bare.sort();
    bare.dedup();
    assert!(
        bare.is_empty(),
        "{} script/style tag(s) shipped without the nonce, so the browser will refuse them. \
         Seal the authored text they come from (`crate::seal::seal`, or `static X: Sealed`):\n  {}",
        bare.len(),
        bare.join("\n  ")
    );
}

/// The discriminating half: a page that prints a visitor's text still prints
/// it bare. `/search?q=` is escaped today (`a_query_value_is_never_echoed_as_markup`
/// holds that), so this asserts the property that holds even if an escape is
/// someday forgotten: whatever the router prints for a query it was given
/// carries no nonce.
#[tokio::test]
async fn a_script_in_a_query_is_never_given_the_nonce() {
    let app = station();
    let probe = "<script>alert(1)</script><style>*{}</style>";
    let encoded: String = form_urlencoded::byte_serialize(probe.as_bytes()).collect();
    for path in [
        "/search?q=",
        "/quarantine?filter=",
        "/species?q=",
        "/history?q=",
    ] {
        let page = get(&app, &format!("{path}{encoded}")).await;
        if page.html.is_empty() {
            continue;
        }
        let nonce = nonce_of(&page.csp).expect("nonce");
        assert!(
            !page.html.contains("<script>alert(1)</script>")
                && !page
                    .html
                    .contains(&format!("<script nonce=\"{nonce}\">alert(1)")),
            "{path}: the probe came back as a live script"
        );
        assert!(
            !page
                .html
                .contains(&format!("<style nonce=\"{nonce}\">*{{}}")),
            "{path}: the probe came back as a nonced style"
        );
    }
}
