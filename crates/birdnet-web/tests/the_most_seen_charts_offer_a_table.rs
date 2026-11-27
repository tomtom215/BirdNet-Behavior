//! UIC-27: the most-seen charts offer their numbers as a table.
//!
//! The kit's hourly and daily column charts draw a closed "Show as a table" disclosure when asked
//! (`table: true`), at no visual cost; the Today strip, the species page's hourly and daily
//! charts, the history day, the dashboard and the weekly report asked it not to, so a screen
//! reader user or a researcher had the picture and its tooltips and nothing to read down.
//!
//! This scans the call sites outside test code: a kit chart there must not set `table: false`.
//! A new call site in another file is not covered; the list below names the files the review
//! found, and a chart added elsewhere is held by charts.md's "Adding a chart", step 3.

use std::path::Path;

/// The files that hold the most-seen kit chart call sites (UIC-27's evidence).
const SITES: &[&str] = &[
    "src/routes/pages/charts.rs",
    "src/routes/pages/weekly_report.rs",
    "src/routes/pages/viz/timeline.rs",
];

/// The source before its first `#[cfg(test)]`: test code builds charts without tables freely.
fn production(src: &str) -> &str {
    src.find("#[cfg(test)]").map_or(src, |i| &src[..i])
}

#[test]
fn no_most_seen_chart_turns_its_table_off() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    let mut sites = 0;
    for file in SITES {
        let src = std::fs::read_to_string(root.join(file)).expect("source file");
        let prod = production(&src);
        sites += prod.matches("table: true").count() + prod.matches("table: false").count();
        for (n, line) in prod.lines().enumerate() {
            if line.trim() == "table: false," {
                found.push(format!("{file}:{}", n + 1));
            }
        }
    }
    // Counterpart: the scan reads call sites at all (a renamed field would pass vacuously).
    assert!(sites >= 4, "the scan found only {sites} call sites");
    assert!(
        found.is_empty(),
        "these charts turn their table alternative off: {found:?}"
    );
}
