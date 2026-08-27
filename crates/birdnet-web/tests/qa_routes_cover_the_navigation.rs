//! The frontend QA gates must load the pages the navigation points at.
//!
//! # What went wrong, and why it was invisible
//!
//! `tools/visual-qa/qa.mjs` exports one `ROUTES` table, imported by both
//! `axe.mjs` (the WCAG gate) and `qa.mjs` itself (overflow / console errors /
//! broken images / stuck loaders). It was written before the v3 "six homes"
//! navigation and never updated: it lists `heatmap`, `analytics`, `migration`,
//! `timeseries`, `weekly`, `year-in-review`, `system`, `admin-audio`, … — the
//! pre-spine URLs.
//!
//! Those URLs still resolve, because `routes::redirects` 308s them to their
//! homes and Playwright follows redirects. So the homes *are* being tested. But
//! they are being tested by accident: a row named `admin-audio` writes a
//! screenshot of the Station Capture tab, a reviewer reading `shots/` cannot
//! tell what was covered, and the coverage is a property of the redirect table
//! rather than of the QA table. Retarget or retire one redirect and a home
//! silently stops being gated, with no row changing and no test failing.
//!
//! Four surfaces were in neither the table nor any redirect — `/login`, the
//! only screen an unauthenticated visitor can reach, among them.
//!
//! This test makes the relationship explicit: every top-level home, every
//! Station tab, and a short list of surfaces that belong to no home must appear
//! **by its own URL** in the route table.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A file in this crate's `src/`.
fn src(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel)
}

/// Every `field: "value"` literal for `field` in `path`.
///
/// The nav tables are `pub(crate)`, so this gate reads them the way the other
/// structural gates in this crate read the templates: from the source. Widening
/// a module's visibility to let a test see a constant would be trading a real
/// API for a convenience.
fn literals(path: &PathBuf, field: &str) -> Vec<String> {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
    let needle = format!("{field}: \"");
    let mut out = Vec::new();
    let mut rest = source.as_str();
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        if let Some(end) = rest.find('"') {
            out.push(rest[..end].to_owned());
            rest = &rest[end + 1..];
        }
    }
    out
}

/// Surfaces that are not a home and not a Station tab, but are product screens
/// a regression would reach.
///
/// `/login` is first for a reason: it is the only page an unauthenticated
/// visitor sees on a station with a password, and it was in no gate at all.
const STANDALONE: &[&str] = &[
    "/login",
    "/admin/settings",
    "/admin/audit",
    "/admin/overview",
];

/// The route strings `qa.mjs` will actually load.
///
/// Parsed rather than imported because the table is JavaScript and this is a
/// Rust gate; the alternative — keeping a second copy in Rust — is the thing
/// this test exists to prevent.
fn qa_routes() -> BTreeSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/visual-qa/qa.mjs")
        .canonicalize()
        .expect("tools/visual-qa/qa.mjs is reachable from the web crate");
    let src = std::fs::read_to_string(&path).expect("qa.mjs is readable");

    let table = src
        .split_once("export const ROUTES = [")
        .expect("qa.mjs exports a ROUTES table")
        .1;
    let table = table.split_once("\n];").expect("ROUTES table is closed").0;

    let mut out = BTreeSet::new();
    for line in table.lines() {
        // Each row is `['name', '/path…'],`, sometimes with a template literal.
        // Take the last quoted string on the line that starts with `/`.
        for quote in ['\'', '`'] {
            let mut rest = line;
            while let Some(start) = rest.find(quote) {
                rest = &rest[start + 1..];
                let Some(end) = rest.find(quote) else { break };
                let candidate = &rest[..end];
                if candidate.starts_with('/') {
                    // Normalise a query string away: a row for `/patterns?tab=dawn`
                    // covers `/patterns` as far as this gate is concerned.
                    let base = candidate.split('?').next().unwrap_or(candidate);
                    out.insert(base.to_owned());
                    out.insert(candidate.to_owned());
                }
                rest = &rest[end + 1..];
            }
        }
    }
    out
}

fn missing(expected: impl IntoIterator<Item = String>) -> Vec<String> {
    let routes = qa_routes();
    let mut missing: Vec<String> = expected
        .into_iter()
        .filter(|p| !routes.contains(p))
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

#[test]
fn every_home_is_in_the_qa_route_table_by_its_own_url() {
    let homes = literals(&src("routes/pages/nav.rs"), "path");
    assert!(
        homes.len() >= 6,
        "expected the six v3 homes in nav.rs, found {homes:?}"
    );
    let missing = missing(homes);
    assert!(
        missing.is_empty(),
        "these top-level homes are reachable from every page's navigation but \
         are not in tools/visual-qa/qa.mjs's ROUTES table under their own URL, \
         so neither the axe gate nor the visual-QA sweep names them:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn every_station_tab_is_in_the_qa_route_table() {
    let keys = literals(&src("routes/pages/homes/station.rs"), "key");
    assert!(
        keys.len() >= 6,
        "expected the six Station tabs in station.rs, found {keys:?}"
    );
    let expected = keys.into_iter().map(|k| {
        if k == "health" {
            "/station".to_owned()
        } else {
            format!("/station/{k}")
        }
    });
    let missing = missing(expected);
    assert!(
        missing.is_empty(),
        "these Station tabs are not in the QA route table:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn the_standalone_surfaces_are_in_the_qa_route_table() {
    let missing = missing(STANDALONE.iter().map(|s| (*s).to_owned()));
    assert!(
        missing.is_empty(),
        "these screens belong to no home and are in no redirect, so nothing \
         else pulls them into the gate:\n  {}",
        missing.join("\n  ")
    );
}

/// The counterpart: the parser must actually be reading the table.
///
/// Every assertion above is satisfied by a parser that returns everything, and
/// none of them would notice one that returns nothing but happens to match. This
/// pins both ends — a route that is definitely there, and one that is
/// definitely not.
#[test]
fn the_route_table_parser_reads_the_real_table() {
    let routes = qa_routes();
    assert!(
        routes.len() > 20,
        "parsed only {} routes from qa.mjs — the table format changed",
        routes.len()
    );
    assert!(
        routes.contains("/"),
        "the dashboard row is missing; the parser is not reading the table"
    );
    assert!(
        !routes.contains("/definitely-not-a-route"),
        "the parser is matching things that are not in the table"
    );
}
