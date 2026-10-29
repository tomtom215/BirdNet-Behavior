//! Every analytic home opens the same way: one head, one `<h1>`, the help link in the eyebrow.
//!
//! Before `pages/frame.rs` each page wrote its own opening: twenty-nine `*-h1` classes, help
//! links in the eyebrow on four tabs and under the heading on two, hero-sized headings on the
//! Reports tabs and recap-sized ones on Patterns. The Patterns tabs, Species, Recordings and the
//! three Reports tabs now go through `frame::page_head`, and this holds them to it.
//!
//! Today is held to the two rules it can share (one `<h1>`, the help link in the eyebrow) and not
//! to `page_head` itself, on purpose: its head is the day's own headline (`td-phrase`), written
//! from the data and set at the larger hero size, with the status pills where the other pages have
//! a lede.
//!
//! Observed failing with the previous Reports and Recordings heads restored (`rp-hero`, `rc-head`):
//! the pages had no `pg-head`.

use axum::body::Body;
use axum::http::Request;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PAGES: &[&str] = &[
    "/patterns?tab=together",
    "/patterns?tab=when",
    "/patterns?tab=dawn",
    "/patterns?tab=migration",
    "/patterns?tab=trends",
    "/species",
    "/recordings",
    "/reports?tab=weekly",
    "/reports?tab=year",
    "/reports?tab=history",
];

async fn page(uri: &str) -> String {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES (date('now','localtime'), '06:30:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'a.wav'),
                (date('now','localtime','-1 day'), '07:10:00', 'Parus major', 'Great Tit', 0.9, 'b.wav');",
    )
    .expect("seed");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let res = birdnet_web::server::build_router(state)
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `html` with the text of every `<script>` removed: the help drawer's script carries the
/// characters `<h1>` in a comment, which is not a heading.
fn without_scripts(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        out.push_str(&rest[..open]);
        match rest[open..].find("</script>") {
            Some(close) => rest = &rest[open + close + "</script>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The text of the first element opened by `open`, up to its closing `</div>`.
fn first_div(html: &str, open: &str) -> String {
    html.split(open)
        .nth(1)
        .and_then(|s| s.split("</div>").next())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn every_home_has_one_shared_head_with_one_h1() {
    for uri in PAGES {
        let html = page(uri).await;
        assert_eq!(
            html.matches(r#"class="page-head pg-head""#).count(),
            1,
            "{uri}: expected one shared page head"
        );
        // The page shell has no <h1> of its own; a second one would be a second page title.
        let h1 = without_scripts(&html).matches("<h1").count();
        assert_eq!(h1, 1, "{uri}: {h1} <h1> elements");
        let heading = first_div(&html, r#"<h1 class="display pg-h1">"#);
        assert!(
            !heading.is_empty() || html.contains(r#"<h1 class="display pg-h1">"#),
            "{uri}: the h1 is the head's"
        );
    }
}

#[tokio::test]
async fn the_help_link_sits_in_the_eyebrow_on_every_home() {
    for uri in PAGES {
        let html = page(uri).await;
        let eyebrow = first_div(&html, r#"<div class="bnb-eyebrow pg-eyebrow">"#);
        assert!(
            eyebrow.contains(r#"class="bnb-help-link""#),
            "{uri}: the help link is not in the head's eyebrow: {eyebrow}"
        );
    }
}

/// The three Reports tabs print: the Save-as-PDF button is in the head, not on a row of its own.
#[tokio::test]
async fn the_report_tabs_carry_the_print_button_in_their_head() {
    for uri in ["/reports?tab=weekly", "/reports?tab=year"] {
        let html = page(uri).await;
        let head = html
            .split(r#"class="page-head pg-head""#)
            .nth(1)
            .and_then(|s| s.split(r#"<div class="rp-stats">"#).next())
            .unwrap_or_default();
        assert!(
            head.contains("data-print"),
            "{uri}: no print button in the head"
        );
        assert!(
            !html.contains("rp-meta"),
            "{uri}: the old print row is back"
        );
    }
}

/// Today's hero is its own markup, so it is held to what it shares with the other homes: the
/// page's one `<h1>` is the hero's, and the help link is in the eyebrow above it.
#[tokio::test]
async fn today_has_one_h1_and_the_help_link_in_its_eyebrow() {
    let html = page("/").await;
    let h1 = without_scripts(&html).matches("<h1").count();
    assert_eq!(h1, 1, "/: {h1} <h1> elements");
    let eyebrow = first_div(&html, r#"<div class="bnb-eyebrow db-eyebrow">"#);
    assert!(
        eyebrow.contains(r#"class="bnb-help-link""#),
        "/: the help link is not in the hero's eyebrow: {eyebrow:?}"
    );
}
