//! ACC-09 / RES-30: a confidence score is the classifier's output, not a
//! measured hit rate, so no surface a reader sees words it as certainty.
//!
//! Scans every template, static script and Rust source of the web crate for
//! the phrases the review found ("very sure", "near-certain", "very few false
//! positives") and a short list of their siblings. A comment or test that
//! *quotes* one to forbid it names it in this file only.

use std::path::Path;

const FORBIDDEN: [&str; 4] = [
    "very sure",
    "near-certain",
    "very few false positives",
    "guaranteed correct",
];

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x, "rs" | "html" | "js" | "css"))
        {
            out.push(p);
        }
    }
}

#[test]
fn no_surface_calls_a_score_certainty() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for sub in ["src", "templates", "static"] {
        walk(&root.join(sub), &mut files);
    }
    assert!(
        files.len() > 50,
        "the scan reached the crate: {}",
        files.len()
    );
    let mut hits = Vec::new();
    for f in &files {
        if f.ends_with("htmx.min.js") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let lower = text.to_lowercase();
        for phrase in FORBIDDEN {
            if lower.contains(phrase) {
                hits.push(format!("{}: {phrase:?}", f.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a score is worded as certainty in:\n{}",
        hits.join("\n")
    );
}
