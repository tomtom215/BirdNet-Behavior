//! Every stylesheet link must carry the build version.
//!
//! # The trap
//!
//! `routes::static_files` serves `app.css` and `print.css` with
//! `Cache-Control: public, max-age=31536000, immutable`. `immutable` is not a
//! hint — it instructs the browser not to revalidate *even on an explicit
//! reload*. At a bare URL like `/static/css/app.css` that means an operator who
//! updates their station gets the new binary serving new HTML against **last
//! year's stylesheet** in every browser that had visited before, for up to a
//! year, with nothing they can do about it short of clearing site data.
//!
//! The service worker does not save it either: it versions its own caches by
//! build hash, but `Cache.addAll()` fetches through the ordinary HTTP cache
//! unless the request is constructed with `cache: 'reload'`, which it is not.
//! So the precache warmed the same stale entry.
//!
//! # And the fallback the browser asks for on its own
//!
//! A document with no `rel="icon"` requests `/favicon.ico` unprompted. The
//! shared layout names its icons explicitly and so was fine; the seven full
//! documents rendered outside it were not, and every one of them logged a 404
//! and showed a blank tab. The visual-QA sweep caught it the first time
//! `/login` was in the route table — which is the whole argument for
//! `qa_routes_cover_the_navigation.rs` in one line.
//!
//! `immutable` is the right header — for a URL that changes when the bytes do.
//! The fix is the `?v=…` query, and this gate is what keeps a new full-document
//! page from forgetting it. Six documents are rendered outside the shared
//! layout (the admin shell, the log viewer, onboarding, kiosk, the standalone
//! audio player, the share page); each one is a place the query has to be
//! written again.

use std::path::{Path, PathBuf};

/// Markup sources: the shipped templates, and the route modules that build full
/// documents in Rust.
fn markup_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    collect(&root.join("templates"), &mut out, &["html"]);
    collect(&root.join("src"), &mut out, &["rs"]);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, exts: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out, exts);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path);
        }
    }
}

/// The two stylesheets served `immutable`.
const IMMUTABLE_STYLESHEETS: &[&str] = &["/static/css/app.css", "/static/css/print.css"];

#[test]
fn no_stylesheet_is_linked_without_a_version_query() {
    let mut offenders: Vec<String> = Vec::new();

    for file in markup_sources() {
        // `static_files.rs` *defines* the routes and must name them bare.
        if file.ends_with("static_files.rs") {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in src.lines().enumerate() {
            // Only `<link>`s matter: a comment or a doc-string may mention the
            // path, and a test may request it bare on purpose.
            if !line.contains("<link") {
                continue;
            }
            for sheet in IMMUTABLE_STYLESHEETS {
                let Some(at) = line.find(sheet) else { continue };
                let after = &line[at + sheet.len()..];
                if !after.starts_with("?v=") {
                    offenders.push(format!("{}:{}  {}", file.display(), n + 1, line.trim()));
                }
            }
        }
    }

    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "these stylesheet links have no `?v=` query, so `immutable` will pin \
         them for a year across every future release:\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart: the scan has to be finding the links at all.
///
/// A path typo, a rename, or a `<link>` written some other way would make the
/// test above pass by looking at nothing.
#[test]
fn the_scan_actually_finds_the_stylesheet_links() {
    let found = markup_sources()
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|s| {
            s.lines()
                .filter(|l| l.contains("<link") && l.contains("/static/css/app.css"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .count();
    assert!(
        found >= 5,
        "found only {found} app.css <link> tags; the six full documents rendered \
         outside the shared layout plus the layout itself should all appear — \
         the scan is looking in the wrong place"
    );
}

/// The fallback favicon must be routed.
///
/// Seven full documents are rendered outside the shared layout and none of them
/// names an icon, so the browser asks for `/favicon.ico` and used to get a 404:
/// a blank tab and a console error on every one, including `/login`, which is
/// the first screen a stranger sees. Routing the fallback covers them all,
/// including the next one someone writes.
#[test]
fn the_favicon_fallback_is_routed() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/static_files.rs"),
    )
    .expect("static_files.rs is readable");
    assert!(
        src.contains(r#".route("/favicon.ico""#),
        "/favicon.ico is not routed; every document without an explicit \
         rel=\"icon\" will 404 for it"
    );
}

/// The service worker must precache the same URL the pages request.
///
/// Precaching the bare path while the documents request `?v=…` warms an entry
/// nothing will ever ask for, and leaves the versioned one to come off the
/// network on every first paint after an update.
#[test]
fn the_service_worker_precaches_the_versioned_stylesheets() {
    let sw = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("static/sw.js"))
        .expect("sw.js is readable");
    let precache = sw
        .split_once("const PRECACHE = [")
        .expect("sw.js has a PRECACHE list")
        .1
        .split_once("];")
        .expect("PRECACHE is closed")
        .0;

    for sheet in IMMUTABLE_STYLESHEETS {
        let versioned = format!("{sheet}?v=${{BUILD}}");
        assert!(
            precache.contains(&versioned),
            "sw.js precaches {sheet} without the build query; it will warm a URL \
             no page requests"
        );
    }
}
