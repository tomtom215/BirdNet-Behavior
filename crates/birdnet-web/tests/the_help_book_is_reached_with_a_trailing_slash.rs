//! The Help link has to land on `/help/`, not `/help`.
//!
//! # The defect this is here for
//!
//! `/help` is a directory mount over mdBook's rendered output, and mdBook
//! writes its `index.html` with *relative* asset URLs:
//!
//! ```text
//! <link rel="stylesheet" href="css/general-e96d0476.css">
//! <script src="book-609e4cb8.js"></script>
//! ```
//!
//! Thirteen of them — nine stylesheets and four scripts, counted from the
//! served page. Reached at `/help/` a browser resolves each against
//! `/help/` and they load. Reached at `/help` — no trailing slash — it
//! resolves them against `/`, so every one lands on the application's own 404
//! handler. That handler answers 200-shaped HTML, `nosniff` refuses it as a
//! stylesheet or a script, and the book renders with no CSS and no JS at all:
//! no sidebar, no search, no table of contents, and the inline SVG icons
//! painted at their intrinsic size as page-wide black slabs. It does not look
//! like a styling bug, it looks like the documentation is broken.
//!
//! `templates/_partial_footer.html` linked the bare `/help` up to 0.15.0, so
//! that was the Help page for every reader who clicked the footer.
//!
//! # What is guarded
//!
//! 1. The server itself redirects `/help` → `/help/`, so a bookmark, a typed
//!    URL or a link from outside the app also lands on a working page.
//! 2. No in-app link points at the bare root, so the redirect is a safety net
//!    rather than something every reader pays a round trip for.
//!
//! Both are needed. The redirect alone leaves the shipped HTML wrong; the link
//! check alone leaves every other way in broken.

use axum::body::Body;
use axum::http::{Request, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Source trees that can contain a link a reader clicks.
const LINK_ROOTS: [&str; 2] = ["src", "templates"];

#[tokio::test]
async fn the_book_root_redirects_to_its_directory_form() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("open state");
    let app = build_router(state);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/help")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>")
        .to_owned();

    assert!(
        status.is_redirection(),
        "`/help` must redirect to `/help/` — mdBook's relative asset URLs \
         resolve against the site root without the trailing slash and the book \
         renders unstyled. Got {status} instead."
    );
    assert_eq!(
        location, "/help/",
        "`/help` redirected somewhere other than its directory form"
    );
}

/// The counterpart: the redirect existing is not a licence to ship the bad
/// link. Every in-app link goes straight to the working URL.
#[test]
fn no_in_app_link_points_at_the_bare_book_root() {
    // Built rather than written out, so this file does not match its own scan
    // if the roots ever widen to include `tests`.
    let bad = format!("href=\"{}\"", "/help");
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut offenders = Vec::new();
    for root in LINK_ROOTS {
        for entry in walk(&crate_dir.join(root)) {
            let Ok(text) = std::fs::read_to_string(&entry) else {
                continue;
            };
            for (i, line) in text.lines().enumerate() {
                if line.contains(&bad) {
                    offenders.push(format!(
                        "{}:{} {}",
                        entry.strip_prefix(crate_dir).unwrap_or(&entry).display(),
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these links point at the bare book root; mdBook's relative asset URLs \
         need the trailing slash (`/help/`) or the page loads with no CSS and \
         no JS:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every regular file under `dir`, recursively. Missing directories yield
/// nothing — a crate laid out differently should not fail here for that.
fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
