//! `cached_fragment`'s third argument is what a **failed** computation renders.
//!
//! The helper takes one fallback and a closure returning `Option<String>`, so
//! `None` has to mean exactly one thing. Three of its four call sites read it
//! as "the query failed" and pass error wording. `migration.rs` read it as
//! "there is no data" and passed `RIDGELINE_EMPTY` /`DIVERSITY_EMPTY`, so a
//! DuckDB or SQLite failure on the Migration tab rendered
//! "No migratory species detected yet this year." to an operator whose station
//! was fine and whose database was not.
//!
//! Observed failing against the shipped tree, naming both migration call sites.
//!
//! This is a source scan rather than a rendering test on purpose: the failure
//! path needs a broken database to reach, and the mistake is visible in the
//! call itself.

use std::path::Path;

/// Extract the identifier passed as `cached_fragment`'s third argument.
///
/// Calls are written across several lines, so this walks forward from the call
/// and splits on top-level commas rather than matching a single line.
fn fallback_args(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("cached_fragment(") {
        rest = &rest[at + "cached_fragment(".len()..];
        // Walk to the matching close paren, recording top-level comma offsets.
        let (mut depth, mut commas, mut end) = (0i32, Vec::new(), rest.len());
        for (i, ch) in rest.char_indices() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' if depth == 0 => {
                    end = i;
                    break;
                }
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => commas.push(i),
                _ => {}
            }
        }
        if commas.len() >= 3 {
            let arg = rest[commas[1] + 1..commas[2]].trim();
            out.push(arg.to_string());
        }
        rest = &rest[end.min(rest.len())..];
    }
    out
}

/// The text of `const NAME: &str = r#"..."#;` in the same file.
fn const_value(src: &str, name: &str) -> Option<String> {
    let at = src.find(&format!("const {name}:"))?;
    let rest = &src[at..];
    let eq = rest.find('=')?;
    let after = &rest[eq + 1..];
    let (open, close) = if let Some(i) = after.find("r#\"") {
        (i + 3, "\"#")
    } else {
        (after.find('"')? + 1, "\"")
    };
    let body = &after[open..];
    Some(body[..body.find(close)?].to_string())
}

#[test]
fn every_cached_fragment_fallback_is_error_wording() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut checked = 0usize;
    let mut stack = vec![src_root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for arg in fallback_args(&text) {
                // The helper's own definition has no call to inspect.
                if arg.contains("fallback") {
                    continue;
                }
                checked += 1;
                // Resolve the constant to its literal and judge the *wording*,
                // not the name: `TS_FALLBACK` says "Analytics temporarily
                // unavailable", which is correct, and an earlier version of
                // this gate flagged it purely for not ending in `_ERR`.
                let Some(body) = const_value(&text, &arg) else {
                    offenders.push(format!(
                        "{}  cached_fragment(.., {arg}, ..) — could not resolve \
                         this constant, so its wording was never checked",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    continue;
                };
                let lower = body.to_lowercase();
                let says_failure = ["unavailable", "could not", "couldn't", "failed", "error"]
                    .iter()
                    .any(|w| lower.contains(w));
                if !says_failure {
                    offenders.push(format!(
                        "{}  cached_fragment(.., {arg}, ..) renders {body:?}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        }
    }
    assert!(
        checked >= 15,
        "the scanner found only {checked} cached_fragment call sites; it has \
         probably stopped parsing them and is passing for the wrong reason"
    );
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "`cached_fragment`'s fallback renders when the computation FAILED, so it \
         must say so. A `*_EMPTY` constant here reports a broken database as an \
         empty yard. Return the empty body from the closure as `Some(..)` \
         instead:\n  {}",
        offenders.join("\n  ")
    );
}
