//! Every link, partial and asset a page references resolves to a route.
//!
//! Found by crawling a running station: the five **Export** buttons on
//! Station → Data (detections CSV, species CSV, eBird, BirdDB.txt, Raven) all
//! answered 404 — the routes are nested under `/api/v2` and the buttons linked
//! the bare paths, under a comment reading "every one a route that exists".
//! So did the Backups card's "Open the log viewer", the admin overview's CSV
//! quick link, and the dawn chorus's "Phenology →". The species page's "See
//! all →" and "View in today's log" went to `/?q=…`, which the Today page
//! ignores. Every other gate was green: axe grades what renders, the visual
//! sweep looks for overflow and broken images, and nothing followed a link.
//!
//! This renders every page the visual-QA sweep covers (`tools/visual-qa/qa.mjs`
//! `ROUTES`, the one list), follows the `/pages/…` partials they load one
//! level down, and requests every local URL in `href`, `src`, `hx-get` and
//! `action`. Anything but a success, a redirect or a `405` fails; a `405`
//! means the route exists for another method (a POST form's action).
//!
//! Not followed, and why: species photos (`/api/v2/species/image/…`) fetch
//! from Wikipedia; clip audio and spectrograms (`/api/v2/recordings/…`,
//! `/api/v2/spectrogram/…`) depend on files on disk; the full backup and the
//! support bundle tar the station; `/r/…` share tokens are signed per link;
//! `/help/…` is the mdBook build served from disk, and every help link is
//! checked against `docs/book` by `the_help_book_is_reached_with_a_trailing_slash.rs`.

use std::collections::{BTreeMap, BTreeSet};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::rate_limit::RateLimitConfig;
use birdnet_web::server::build_router_with_rate_limit;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const NOT_FOLLOWED: &[&str] = &[
    "/api/v2/species/image/",
    "/api/v2/recordings/",
    "/api/v2/spectrogram/",
    "/admin/system/backup/full",
    "/admin/support-bundle",
    "/r/",
    "/help/",
];

/// The pages `qa.mjs` sweeps: its `ROUTES` table, parsed (the same parse
/// `qa_routes_cover_the_navigation.rs` uses, so there is one list).
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

/// Local URLs in the attributes that navigate or load.
fn references(html: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for attr in ["href=\"", "src=\"", "hx-get=\"", "action=\""] {
        let mut rest = html;
        while let Some(i) = rest.find(attr) {
            rest = &rest[i + attr.len()..];
            let Some(end) = rest.find('"') else { break };
            let raw = rest[..end].replace("&amp;", "&");
            rest = &rest[end..];
            if raw.starts_with('/')
                && !raw.starts_with("//")
                && !raw.contains('{')
                && !NOT_FOLLOWED.iter().any(|p| raw.starts_with(p))
            {
                out.insert(raw.split('#').next().unwrap_or(&raw).to_owned());
            }
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
    // The rate limiter answers a crawl this size with 429 — which is not 404,
    // and the first draft of this gate passed against every dead link for
    // exactly that reason. It is lifted here, and anything that is not a
    // success, a redirect or a 405 now fails rather than passing unexamined.
    build_router_with_rate_limit(
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:")),
        RateLimitConfig {
            requests_per_second: 1e9,
            burst_capacity: u32::MAX,
            ..RateLimitConfig::default()
        },
    )
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
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
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn every_link_on_every_page_resolves() {
    let app = station();
    let seeds = pages();
    assert!(
        seeds.len() > 30,
        "the qa.mjs ROUTES parse found only {seeds:?}"
    );

    // Page (and one level of partials) → the references it makes.
    let mut referenced_from: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut queue: Vec<(String, u8)> = seeds.iter().map(|p| (p.clone(), 0)).collect();
    let mut rendered = BTreeSet::new();
    while let Some((page, depth)) = queue.pop() {
        if !rendered.insert(page.clone()) {
            continue;
        }
        let (status, html) = get(&app, &page).await;
        if !status.is_success() {
            continue; // Its own status is judged as a reference, or it redirects.
        }
        for r in references(&html) {
            if depth == 0 && r.starts_with("/pages/") {
                queue.push((r.clone(), 1));
            }
            referenced_from.entry(r).or_default().insert(page.clone());
        }
    }
    assert!(
        referenced_from.len() > 100,
        "the crawl found only {} references; is the extraction broken?",
        referenced_from.len()
    );

    let mut dead = Vec::new();
    for (target, from) in &referenced_from {
        let (status, _) = get(&app, target).await;
        let resolved = status.is_success()
            || status.is_redirection()
            || status == StatusCode::METHOD_NOT_ALLOWED;
        if !resolved {
            dead.push(format!("{status} {target}  (linked from {from:?})"));
        }
    }
    assert!(
        dead.is_empty(),
        "{} link(s) do not resolve:\n  {}",
        dead.len(),
        dead.join("\n  ")
    );
}
