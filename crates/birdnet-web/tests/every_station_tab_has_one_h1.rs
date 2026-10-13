//! Every Station tab has exactly one `<h1>`.
//!
//! The Capture tab is three older pages stitched together, and two of them
//! kept their own page heading: an `<h1>` "Audio sources" under the tab's
//! `<h2>` "Audio sources" (with an eyebrow reading "primary support surface",
//! a phrase from a design document), and an `<h1>` "Species List Management"
//! under the `<h2>` "Which birds count". Three `<h1>`s, two of them below an
//! `<h2>`: an outline a screen reader's heading list presents backwards. The
//! Alerts, Data and Access tabs had the same shape, from the bodies of the
//! admin pages they replaced (Alert Rules, Notification History, Test
//! Notification Channels, Backups & recovery, BirdNET-Pi Migration, Data
//! quality, Accounts & sessions). Every one of those admin pages now redirects
//! to its tab, except `/admin/notifications/test`, which keeps its own `<h1>`
//! in its own wrapper.
//!
//! Observed failing before the fix: capture 3, alerts 4, data 4, access 2.

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

/// The document with comments, scripts and styles removed: a comment that
/// *mentions* `<h1>` is not a heading. The help drawer's template has one.
fn markup(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let next = [
            ("<!--", "-->"),
            ("<script", "</script>"),
            ("<style", "</style>"),
        ]
        .iter()
        .filter_map(|(open, close)| rest.find(open).map(|i| (i, *close)))
        .min_by_key(|(i, _)| *i);
        let Some((i, close)) = next else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..i]);
        rest = rest[i..]
            .find(close)
            .map_or("", |j| &rest[i + j + close.len()..]);
    }
}

fn h1_count(html: &str) -> usize {
    let m = markup(html);
    m.matches("<h1>").count() + m.matches("<h1 ").count()
}

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

#[tokio::test]
async fn every_station_tab_has_one_h1() {
    let mut wrong = Vec::new();
    for tab in TABS {
        let n = h1_count(&get(tab).await);
        if n != 1 {
            wrong.push(format!("{tab}: {n}"));
        }
    }
    assert!(wrong.is_empty(), "tabs without exactly one <h1>: {wrong:?}");
}

/// The counter's own counterpart: it counts real headings, attributes and
/// all, and nothing inside a comment, script or style.
#[test]
fn the_counter_counts_headings_and_nothing_else() {
    let html = r#"<h1 class="x">A</h1><!-- an <h1> in a comment --><script>"<h1>"</script><style>/* <h1> */</style><h1>B</h1><h10>"#;
    assert_eq!(h1_count(html), 2);
}
