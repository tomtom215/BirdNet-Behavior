//! No production SQL may use `INSERT OR IGNORE`.
//!
//! # What went wrong, and why a grep is the right guard
//!
//! `INSERT OR IGNORE` reads as "absorb the duplicate". It does not: it absorbs
//! **every** constraint violation on the statement and reports success. The two
//! are indistinguishable to the caller, because both give `Ok(0)`.
//!
//! That cost this project a silent data loss. `insert_quarantine` used
//! `OR IGNORE` to absorb the `UNIQUE(date, time, sci_name)` collision when the
//! same detection is offered twice — a real and necessary thing to absorb. When
//! a fourth quarantine reason was added without widening the column's `CHECK`,
//! the same clause swallowed the CHECK violation exactly as it swallows a
//! duplicate. Every detection quarantined for the new reason was dropped on the
//! floor, `Ok(())` came back, and there was no row and no error anywhere to
//! find afterwards. It surfaced only because an end-to-end test asserted the
//! row existed.
//!
//! The fix at each site is `ON CONFLICT(<the columns that actually collide>)
//! DO NOTHING`, which absorbs precisely the intended conflict and lets a CHECK
//! or NOT NULL violation raise the error it is.
//!
//! A grep guard rather than a type-level one because the hazard is in a string:
//! nothing in Rust's type system can see inside a SQL literal, and the next
//! `OR IGNORE` will be written by someone reaching for the obvious idiom.

use std::path::{Path, PathBuf};

/// Files whose occurrences are not production writes.
///
/// `migration.rs` carries the statement inside test fixtures that deliberately
/// reproduce the old behaviour — including the counterpart proving the CHECK
/// still rejects an unknown reason.
const EXCLUDE: &[&str] = &["crates/birdnet-db/src/migration.rs"];

/// Whether a path is a test or an example rather than a production write path.
///
/// Excluded by kind rather than by name: a regression fixture may legitimately
/// need to issue the old statement to prove what it did, and listing each one
/// would make this guard a maintenance burden that gets weakened to shut up.
fn is_test_or_example(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("tests" | "examples" | "benches")
        )
    })
}

/// The forbidden idiom, assembled so this file does not trip its own guard.
///
/// `INTO` is part of the needle deliberately: prose *about* the idiom (a doc
/// comment explaining why it is not used) is not the hazard, and a guard that
/// flagged its own explanation would be turned off.
fn forbidden() -> String {
    format!("INSERT {} IGNORE INTO", "OR")
}

/// Workspace root: this test lives in `<root>/tests/`.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `src/` and `crates/`, excluding `target/`.
fn collect_sources(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["src", "crates"] {
        collect_recursive(&root.join(sub), &mut out);
    }
    out.sort();
    out
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == "target") {
            continue;
        }
        if path.is_dir() {
            collect_recursive(&path, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Scan the workspace, returning `(files actually read, offending lines)`.
///
/// The scanned count is returned rather than discarded because a guard that
/// scans nothing passes forever: excluding the whole workspace by accident
/// leaves `failures` empty and the assertion green. That is not a
/// hypothetical failure mode — it is what an over-broad exclusion looks like,
/// and it was reachable here until `the_guard_actually_scans_the_workspace`
/// existed.
fn scan(root: &Path) -> (usize, Vec<String>) {
    let needle = forbidden();
    let excluded: Vec<PathBuf> = EXCLUDE.iter().map(|r| root.join(r)).collect();
    let mut failures = Vec::new();
    let mut scanned = 0;

    for path in collect_sources(root) {
        if excluded.contains(&path) || is_test_or_example(&path) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        scanned += 1;
        let rel = path.strip_prefix(root).unwrap_or(&path).display();
        for (lineno, line) in src.lines().enumerate() {
            if line.contains(&needle) {
                failures.push(format!("  {rel}:{}  {}", lineno + 1, line.trim()));
            }
        }
    }
    (scanned, failures)
}

/// Production source files this workspace is known to have, comfortably below
/// the real count, so ordinary growth never trips it.
const MIN_SCANNED_FILES: usize = 100;

#[test]
fn the_guard_actually_scans_the_workspace() {
    // Counterpart to the guard below, and the reason it is worth anything:
    // with every path excluded there is nothing to find and the guard reports
    // success. Pinning the file count means an exclusion that swallows the
    // workspace fails loudly instead of quietly disarming the check.
    let (scanned, _) = scan(&workspace_root());
    assert!(
        scanned >= MIN_SCANNED_FILES,
        "the guard scanned only {scanned} files; an exclusion is swallowing the workspace \
         and this check is no longer protecting anything"
    );
}

#[test]
fn no_production_sql_uses_insert_or_ignore() {
    let (_, failures) = scan(&workspace_root());

    assert!(
        failures.is_empty(),
        "`INSERT OR IGNORE` absorbs every constraint violation, not just the duplicate \
         it is written for, and reports success either way — a CHECK or NOT NULL failure \
         becomes a row that silently never existed. Use \
         `ON CONFLICT(<columns>) DO NOTHING`, naming the conflict you actually mean.\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_guard_can_see_the_idiom_it_is_looking_for() {
    // Counterpart: a guard whose needle never matches passes forever and
    // protects nothing. `migration.rs` is excluded from the scan precisely
    // because it still contains the string — in the comments describing this
    // bug — which makes it the fixture that proves the needle works.
    let excluded = workspace_root().join("crates/birdnet-db/src/migration.rs");
    let src = std::fs::read_to_string(&excluded).expect("migration.rs is readable");
    assert!(
        src.contains(&forbidden()),
        "the guard's needle no longer appears in the one file known to contain it; \
         either the needle is wrong or the exclusion is now unnecessary"
    );
}
