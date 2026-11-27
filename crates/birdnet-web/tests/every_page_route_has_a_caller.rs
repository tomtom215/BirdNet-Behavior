//! Every `/pages/…` partial and every full page the server routes is called from somewhere
//! a reader reaches (UIC-05, TD-6).
//!
//! A routed partial nothing loads is maintained, tested and sometimes pre-warmed for no
//! reader, and one that writes (`/pages/today-relabel`) is an endpoint behind the sign-in with
//! no control that posts to it. TD-6 found them by a one-off grep; this is that grep as a gate.
//!
//! A path counts as called when its text appears in `crates/birdnet-web/{src,templates,static}`
//! anywhere other than the `.route(` that registers it, a comment, or a test module. A URL built
//! from pieces (`"/pages/" + name`) is invisible to it, which is why the exemptions are a named
//! list with a reason each rather than a heuristic. Seen failing with an empty list: it named
//! the sixteen routes TD-6 records.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Routed with no caller, each recorded in `docs/TECH_DEBT.md` TD-6 as the maintainer's decision
/// (link or delete: "what the interface promises"). Removing a route, or linking it, removes its
/// line here; a new orphan is not added here without a TD-6 entry.
const EXEMPT: &[&str] = &[
    "/pages/disk-status",
    "/pages/history-dates",
    "/pages/today-lock",
    "/pages/today-unlock",
    "/pages/today-relabel",
    "/pages/stats",
    "/pages/hero-status",
    "/pages/species-list",
    "/pages/hourly-chart",
    "/pages/daily-chart",
    "/pages/most-recent",
    "/pages/confidence-chart",
    "/pages/activity-heatmap",
    "/pages/life-accumulation",
    "/pages/ts-heatmap",
    "/player/{filename}",
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "vendor" || n == "fonts") {
                continue;
            }
            files(&p, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x, "rs" | "html" | "js" | "mjs"))
        {
            out.push(p);
        }
    }
}

/// Where a Rust file's test modules start: the first `#[cfg(test)]` whose item is a `mod`. A
/// test module is the last thing in the files that have one; a `#[cfg(test)]` function in the
/// middle of a file is not a module, and the code after it is read.
fn tests_start(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find("#[cfg(test)]") {
        let at = from + i;
        if text[at + "#[cfg(test)]".len()..]
            .trim_start()
            .starts_with("mod ")
        {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// The source with test modules and `//` comments removed.
fn reader_facing(path: &Path, text: &str) -> String {
    let body = if path.extension().is_some_and(|x| x == "rs") {
        tests_start(text).map_or(text, |i| &text[..i])
    } else {
        text
    };
    body.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("* ") || t.starts_with("/*"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The routed paths: the first string literal after each `.route(`, whichever line it is on.
fn routed(sources: &[(PathBuf, String)]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, text) in sources {
        let mut rest = text.as_str();
        while let Some(i) = rest.find(".route(") {
            rest = &rest[i + ".route(".len()..];
            let Some(q) = rest.find('"') else { break };
            let lit = &rest[q + 1..];
            let Some(end) = lit.find('"') else { break };
            let path = &lit[..end];
            if path.starts_with("/pages/") || path.starts_with("/player/") {
                out.insert(path.to_owned());
            }
        }
    }
    out
}

/// Whether `path` is named anywhere other than in a `.route(` registration.
fn called(path: &str, sources: &[(PathBuf, String)]) -> bool {
    // A path with a parameter is called by its fixed prefix (`/player/` + a file name).
    let needle = path.split('{').next().unwrap_or(path);
    sources.iter().any(|(_, text)| {
        let mut from = 0;
        while let Some(i) = text[from..].find(needle) {
            let at = from + i;
            let before = &text[..at];
            let registration = before
                .rfind(".route(")
                .is_some_and(|r| !before[r..].contains(')') && !before[r..].contains(';'));
            // The character after the needle must end the path or start its query, so
            // `/pages/today` is not called by `/pages/today-lock`. A backslash ends it too: the
            // escaped quote of a path inside a Rust string (`\"/pages/x\"`).
            let next = text[at + needle.len()..].chars().next();
            let whole = needle.ends_with('/')
                || next.is_none_or(|c| {
                    matches!(c, '?' | '"' | '\'' | '`' | '#' | '&' | ' ' | '<' | '\\')
                });
            if !registration && whole {
                return true;
            }
            from = at + needle.len();
        }
        false
    })
}

fn sources() -> Vec<(PathBuf, String)> {
    let root = crate_dir();
    let mut paths = Vec::new();
    for d in ["src", "templates", "static"] {
        files(&root.join(d), &mut paths);
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let text = fs::read_to_string(&p).ok()?;
            let kept = reader_facing(&p, &text);
            Some((p, kept))
        })
        .collect()
}

fn orphans() -> Vec<String> {
    let src = sources();
    routed(&src)
        .into_iter()
        .filter(|p| !called(p, &src))
        .collect()
}

#[test]
fn every_routed_page_has_a_caller_or_a_named_exemption() {
    let found = orphans();
    let unexplained: Vec<&String> = found
        .iter()
        .filter(|p| !EXEMPT.contains(&p.as_str()))
        .collect();
    assert!(
        unexplained.is_empty(),
        "routed, and nothing a reader reaches calls them (link them, delete them, or record \
         them in TD-6 and here):\n  {}",
        unexplained
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// The list does not outlive what it lists: an exempted route that gained a caller, or was
/// deleted, comes off it.
#[test]
fn every_exemption_is_still_an_orphan() {
    let found = orphans();
    let stale: Vec<&&str> = EXEMPT
        .iter()
        .filter(|p| !found.iter().any(|f| f == *p))
        .collect();
    assert!(
        stale.is_empty(),
        "no longer orphaned; remove from EXEMPT and TD-6: {stale:?}"
    );
}

/// The counterpart: the scan sees routes and their callers. The History partials are routed in
/// `history.rs` and called from markup in the same file; a scan that found no routes, or called
/// everything an orphan, would pass the first gate for the wrong reason.
#[test]
fn the_scan_sees_a_called_route_as_called() {
    let src = sources();
    let routes = routed(&src);
    assert!(routes.len() > 40, "the scan found the routes: {routes:?}");
    assert!(routes.contains("/pages/history-chart"));
    assert!(called("/pages/history-chart", &src));
    assert!(!called("/pages/history-dates", &src));
}
