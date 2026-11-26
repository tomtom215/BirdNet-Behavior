//! Every `#[allow(clippy::…)]` and `#[expect(clippy::…)]` in `src` and
//! `crates` says why, and the functions excused from `too_many_lines` only
//! ever get fewer.
//!
//! An allow with no reason is a lint switched off with nothing to tell the
//! next reader whether the reason still holds; `too_many_lines` in particular
//! was how `start_detection_daemon` grew to 400 lines and eleven arguments
//! with nobody deciding it should. Clippy's `allow_attributes_without_reason`
//! would say the first, but only for a crate that turns it on, and says
//! nothing of the count.
//!
//! `tests/data/clippy_allows.tsv` holds the backlog this gate started with:
//! the number of clippy allows without a reason in each file that had any, and
//! the number of `too_many_lines` allows in the tree. Each number must be the
//! number in the tree, so a new allow without a reason fails here, and so does
//! one given a reason or removed until the file's number is lowered with it: a
//! number in the file only goes down. A file not listed must have none.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The data file.
const DATA: &str = "tests/data/clippy_allows.tsv";

/// The data file's key for the `too_many_lines` count.
const TOO_MANY_LINES: &str = "too_many_lines";

/// One lint attribute that names a clippy lint.
#[derive(Debug, PartialEq, Eq)]
struct Allow {
    /// 1-based line of the attribute.
    line: usize,
    /// Whether it gives a `reason = "…"`.
    has_reason: bool,
    /// How many times it names `clippy::too_many_lines`.
    too_many_lines: usize,
}

/// Every `allow(…)` or `expect(…)` attribute in `source` that names a clippy
/// lint, including one inside `cfg_attr(…)`, across as many lines as it takes.
fn clippy_allows(source: &str) -> Vec<Allow> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = source[i..].find('#').map(|n| n + i) {
        i = at + 1;
        let rest = &source[at..];
        let Some(open) = attribute_open(rest) else {
            continue;
        };
        // In a comment (a doc comment quoting one): not an attribute.
        let line_start = source[..at].rfind('\n').map_or(0, |n| n + 1);
        if source[line_start..at].contains("//") {
            continue;
        }
        // The attribute's whole text, to its closing bracket.
        let mut depth = 0_i32;
        let mut end = at + open;
        for (k, &b) in bytes[at + open..].iter().enumerate() {
            match b {
                b'[' | b'(' => depth += 1,
                b']' | b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = at + open + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let text = &source[at..end];
        let lint_list = text.contains("allow(") || text.contains("expect(");
        if lint_list && text.contains("clippy::") {
            out.push(Allow {
                line: source[..at].matches('\n').count() + 1,
                has_reason: text.contains("reason"),
                too_many_lines: text.matches("clippy::too_many_lines").count(),
            });
        }
        i = end.max(at + 1);
    }
    out
}

/// Where the bracket of an attribute opens in `text`, which starts at a `#`:
/// `#[` or `#![`, possibly with spaces; `None` for a `#` that starts none.
fn attribute_open(text: &str) -> Option<usize> {
    let after = text[1..].trim_start_matches('!');
    after.starts_with('[').then(|| text.len() - after.len())
}

/// Every `.rs` file under `dir`, skipping build output.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// What the tree holds: reasonless clippy allows per file (relative path),
/// and the `too_many_lines` total.
fn measure(root: &Path) -> (BTreeMap<String, usize>, usize) {
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("crates"), &mut files);
    let mut without = BTreeMap::new();
    let mut too_many_lines = 0;
    for file in files {
        let source = std::fs::read_to_string(&file).expect("read a source file");
        let allows = clippy_allows(&source);
        too_many_lines += allows.iter().map(|a| a.too_many_lines).sum::<usize>();
        let n = allows.iter().filter(|a| !a.has_reason).count();
        if n > 0 {
            let rel = file
                .strip_prefix(root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/");
            without.insert(rel, n);
        }
    }
    (without, too_many_lines)
}

/// The data file, as the same two things.
fn recorded(root: &Path) -> (BTreeMap<String, usize>, usize) {
    let text = std::fs::read_to_string(root.join(DATA)).expect("read the data file");
    let mut without = BTreeMap::new();
    let mut too_many_lines = None;
    for line in text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let (key, n) = line.split_once('\t').expect("key<TAB>count");
        let n: usize = n.trim().parse().expect("a count");
        if key == TOO_MANY_LINES {
            too_many_lines = Some(n);
        } else {
            without.insert(key.to_owned(), n);
        }
    }
    (
        without,
        too_many_lines.expect("the data file records the too_many_lines count"),
    )
}

#[test]
fn every_clippy_allow_says_why_and_too_many_lines_only_goes_down() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (actual, actual_tml) = measure(root);
    let (allowed, allowed_tml) = recorded(root);
    let mut problems = Vec::new();
    for (file, &n) in &actual {
        let may = allowed.get(file).copied().unwrap_or(0);
        if n > may {
            problems.push(format!(
                "{file}: {n} clippy allow(s) without `reason = \"…\"`, and {may} are \
                 recorded; give the new one a reason"
            ));
        }
    }
    for (file, &may) in &allowed {
        let n = actual.get(file).copied().unwrap_or(0);
        if n < may {
            problems.push(format!(
                "{file}: {n} clippy allow(s) without a reason, and {DATA} records {may}; \
                 lower it to {n} (it only goes down)"
            ));
        }
    }
    if actual_tml > allowed_tml {
        problems.push(format!(
            "{actual_tml} `clippy::too_many_lines` allows in src and crates, and {DATA} \
             allows {allowed_tml}: split the function instead"
        ));
    } else if actual_tml < allowed_tml {
        problems.push(format!(
            "{actual_tml} `clippy::too_many_lines` allows, and {DATA} records \
             {allowed_tml}: lower it to {actual_tml} (it only goes down)"
        ));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The parser finds what it must and nothing else: an allow with and
/// without a reason, on one line and across several, inside `cfg_attr`, an
/// `expect`, an inner attribute; and not a lint that is not clippy's, nor a
/// `#` that starts no attribute.
#[test]
fn the_parser_sees_every_shape_of_clippy_allow() {
    let src = r##"
#[allow(clippy::too_many_lines)]
fn a() {}
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "why"
)]
fn b() {}
#[cfg_attr(test, allow(clippy::unwrap_used))]
fn c() {}
#![expect(clippy::cast_lossless, reason = "why")]
#[allow(dead_code)]
fn d() { let s = "#[allow(clippy::x)] in a string"; let n = 3 # 2; }
/// Quoting `#[allow(clippy::too_many_lines)]` in a comment.
"##;
    let found = clippy_allows(src);
    assert_eq!(
        found,
        vec![
            Allow {
                line: 2,
                has_reason: false,
                too_many_lines: 1
            },
            Allow {
                line: 4,
                has_reason: true,
                too_many_lines: 1
            },
            Allow {
                line: 10,
                has_reason: false,
                too_many_lines: 0
            },
            Allow {
                line: 12,
                has_reason: true,
                too_many_lines: 0
            },
            // A string that looks like one is counted: the scan does not
            // parse Rust, and a false alarm here is a line to reword.
            Allow {
                line: 14,
                has_reason: false,
                too_many_lines: 0
            },
        ]
    );
}
