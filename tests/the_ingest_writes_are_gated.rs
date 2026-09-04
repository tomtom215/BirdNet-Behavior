//! Every write that records a detection event goes through the ingest gate.
//!
//! # Why a source scan, and not another behavioural test
//!
//! `PS-5`'s fix is a set: three kinds of write stop when the database is known
//! to be corrupt, and everything else keeps working. The behavioural gates in
//! `src/daemon/processor.rs` and
//! `crates/birdnet-web/tests/a_corrupt_database_stops_taking_detections.rs`
//! prove the mechanism works and that it is narrower than a read-only
//! connection. Neither can see a *fourth* per-detection write added later
//! through plain `with_db` — it would produce no failure, no warning, and no
//! alert, which is exactly what a healthy station produces.
//!
//! This is 2.5's lesson applied a third time: **a set expressed only as
//! scattered call sites cannot be checked**, so it is written down once — in
//! `AppState::with_ingest_db`'s own doc comment — and read back here. The
//! comment claims not to be a comment to be trusted; this is what makes that
//! true.
//!
//! # What it checks
//!
//! For each name in that list:
//!
//! * it appears at least once behind `with_ingest_db`, so the list cannot name
//!   a function nothing calls;
//! * it appears **nowhere** behind plain `with_db`, in any production source in
//!   the workspace.

use std::path::{Path, PathBuf};

/// Where the set is written down.
const SOURCE_OF_TRUTH: &str = "crates/birdnet-web/src/state.rs";

/// The repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The gated write names, parsed out of `with_ingest_db`'s doc comment.
///
/// The list is the run of doc-comment bullets naming one backticked path
/// each. Parsed rather than duplicated, because a second copy here would be
/// one more thing to drift.
fn gated_names() -> Vec<String> {
    let path = repo_root().join(SOURCE_OF_TRUTH);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
    let doc = source
        .split_once("pub fn with_ingest_db")
        .unwrap_or_else(|| panic!("{SOURCE_OF_TRUTH} still defines with_ingest_db"))
        .0;

    let mut names = Vec::new();
    // Walk the doc comment backwards from the signature so only *this*
    // function's bullets are read, and stop at the first gap.
    for line in doc.lines().rev() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("/// * `")
            && let Some(name) = rest.strip_suffix('`')
        {
            names.push(
                name.rsplit("::")
                    .next()
                    .unwrap_or(name)
                    .trim_end_matches("()")
                    .to_owned(),
            );
            continue;
        }
        if !names.is_empty() && !t.starts_with("///") {
            break;
        }
        if !names.is_empty() && t.starts_with("///") && !t.starts_with("/// * `") {
            break;
        }
    }
    names.reverse();
    assert!(
        names.len() >= 3,
        "the gated-write list in {SOURCE_OF_TRUTH} has shrunk to {names:?} — if \
         that is deliberate, this gate is the right place to say why"
    );
    names
}

/// Every production `.rs` file in the workspace: the binary's `src/` and each
/// crate's `src/`. Deliberately not `tests/` or `benches/` — a fixture may
/// legitimately insert a detection row directly.
fn production_sources() -> Vec<PathBuf> {
    let root = repo_root();
    let mut roots = vec![root.join("src")];
    let crates = root.join("crates");
    let mut entries: Vec<_> = std::fs::read_dir(&crates)
        .expect("crates/ is readable")
        .filter_map(Result::ok)
        .map(|e| e.path().join("src"))
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();
    roots.append(&mut entries);

    let mut out = Vec::new();
    for dir in roots {
        walk(&dir, &mut out);
    }
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Collapse runs of whitespace so a call split across lines reads as one.
fn flatten(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every window of `len` characters following each occurrence of `needle`.
fn windows_after<'a>(haystack: &'a str, needle: &str, len: usize) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = haystack[from..].find(needle) {
        let start = from + i + needle.len();
        let end = haystack.len().min(start + len);
        // Slice on a char boundary: the sources contain non-ASCII prose.
        let end = (start..=end).rev().find(|&e| haystack.is_char_boundary(e));
        if let Some(end) = end {
            out.push(&haystack[start..end]);
        }
        from = start;
    }
    out
}

/// How far after a `with_db(` opening the callee may sit. The longest real
/// call in the workspace is the outbound-queue enqueue, at about 70
/// characters once flattened; 200 leaves room without reaching the next
/// statement.
const CALL_WINDOW: usize = 200;

#[test]
fn no_production_call_site_writes_a_detection_through_the_ungated_writer() {
    let names = gated_names();
    let mut offenders: Vec<String> = Vec::new();
    let mut gated_seen: Vec<String> = Vec::new();

    for path in production_sources() {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let flat = flatten(&source);
        // `with_ingest_db(` and `with_read_db(` do not contain `with_db(` as a
        // substring, so this needle picks out the ungated writer alone.
        for window in windows_after(&flat, "with_db(", CALL_WINDOW) {
            for name in &names {
                if window.contains(name.as_str()) {
                    offenders.push(format!("{}: {name}", path.display()));
                }
            }
        }
        for window in windows_after(&flat, "with_ingest_db(", CALL_WINDOW) {
            for name in &names {
                if window.contains(name.as_str()) {
                    gated_seen.push(name.clone());
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a write that records a detection event is going through the ungated \
         writer, so a database known to be corrupt would keep taking it \
         (PS-5): {offenders:#?}"
    );
    for name in &names {
        assert!(
            gated_seen.contains(name),
            "the gated-write list names `{name}`, but nothing in the workspace \
             calls it through `with_ingest_db` — either the list is wrong or the \
             call site was lost"
        );
    }
}
