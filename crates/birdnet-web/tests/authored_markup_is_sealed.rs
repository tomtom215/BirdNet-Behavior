//! Every `<script>` and `<style>` this crate writes, and every page template it
//! embeds, goes through `seal` before a visitor's text is put beside it.
//!
//! The nonce is stamped onto sealed tags only (`src/seal.rs`). Rendering every
//! page (`every_script_on_every_page_carries_the_nonce.rs`) finds a tag that
//! was missed on a page that gets rendered; this reads the source, so it also
//! finds one on a page the crawl does not reach (an error page, a log viewer
//! that needs a live stream, a player for a clip that must exist), and it
//! refuses the shape that cannot be sealed correctly: a tag written inside the
//! same literal as a value, where "sealing the literal" would seal whatever
//! was substituted into it.
//!
//! What counts as sealed, per occurrence of `<script` or `<style` in non-test,
//! non-comment code:
//!
//! * it sits in a `static … Sealed` item (`Sealed::new(…)`), or
//! * the same or one of the two preceding lines calls `seal::seal(`.
//!
//! And every `include_str!` of a file under `templates/` sits in `Sealed::new(`.
//!
//! Heuristic, as a text scan must be; the two self-tests at the bottom hold the
//! cases it is built to separate.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The part of a source file that ships: everything before a `#[cfg(test)]`
/// module, with `//` comment lines blanked (line numbers are kept).
fn shipped(source: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in source.lines() {
        if line.trim() == "#[cfg(test)]" {
            break;
        }
        let t = line.trim_start();
        lines.push(if t.starts_with("//") {
            String::new()
        } else {
            line.to_owned()
        });
    }
    lines
}

/// The line without a leading visibility (`pub`, `pub(crate)`, `pub(super)`).
fn without_visibility(line: &str) -> &str {
    let Some(rest) = line.strip_prefix("pub") else {
        return line;
    };
    let rest = rest
        .strip_prefix(|c: char| c == '(')
        .map_or(rest, |r| r.split_once(')').map_or(rest, |(_, after)| after));
    rest.trim_start()
}

fn opens_a_tag(line: &str) -> bool {
    ["<script", "<style"].iter().any(|tag| {
        let mut from = 0;
        while let Some(i) = line[from..].find(tag) {
            let at = from + i + tag.len();
            if line[at..]
                .chars()
                .next()
                .is_none_or(|c| c.is_ascii_whitespace() || c == '>' || c == '/' || c == '\\')
            {
                return true;
            }
            from = at;
        }
        false
    })
}

/// 1-based line numbers of authored tags that are not sealed.
fn unsealed(source: &str) -> Vec<usize> {
    let lines = shipped(source);
    let mut bad = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !opens_a_tag(line) {
            continue;
        }
        let near_seal = (i.saturating_sub(2)..=i).any(|j| lines[j].contains("seal::seal("));
        // The enclosing item: the last line at column 0 that starts one.
        let item = (0..=i).rev().find(|&j| {
            let l = without_visibility(&lines[j]);
            ["static ", "const ", "fn ", "async fn ", "let "]
                .iter()
                .any(|p| l.starts_with(p))
        });
        let in_sealed_static = item.is_some_and(|j| {
            let head: String = lines[j..=(j + 2).min(lines.len() - 1)].join(" ");
            without_visibility(&head).starts_with("static ") && head.contains("Sealed")
        });
        if !(near_seal || in_sealed_static) {
            bad.push(i + 1);
        }
    }
    bad
}

/// 1-based line numbers of template includes outside `Sealed::new(`.
fn unsealed_templates(source: &str) -> Vec<usize> {
    let lines = shipped(source);
    let mut bad = Vec::new();
    for (i, _) in lines.iter().enumerate() {
        let window = lines[i..(i + 3).min(lines.len())].join(" ");
        let here = &lines[i];
        if here.contains("include_str!(") || (here.trim_end().ends_with("include_str!(")) {
            let tail = &window[window.find("include_str!(").unwrap_or(0)..];
            let is_template = tail
                .split(')')
                .next()
                .is_some_and(|a| a.contains("templates/"));
            let before: String = lines[i.saturating_sub(2)..=i].join(" ");
            if is_template && !before.contains("Sealed::new(") {
                bad.push(i + 1);
            }
        }
    }
    bad
}

#[test]
fn authored_markup_is_sealed() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    assert!(files.len() > 100, "found only {} source files", files.len());

    let mut problems = Vec::new();
    let mut tags_seen = 0_usize;
    for file in &files {
        // The module that defines the seal quotes the tags it handles.
        if file.ends_with("seal.rs") {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("readable");
        tags_seen += shipped(&text).iter().filter(|l| opens_a_tag(l)).count();
        for n in unsealed(&text) {
            problems.push(format!(
                "{}:{n}: a <script>/<style> that is not sealed",
                file.strip_prefix(&src).unwrap().display()
            ));
        }
        for n in unsealed_templates(&text) {
            problems.push(format!(
                "{}:{n}: a page template embedded without `Sealed::new(`",
                file.strip_prefix(&src).unwrap().display()
            ));
        }
    }
    assert!(
        tags_seen > 20,
        "the scan saw only {tags_seen} authored tags; is it reading the right text?"
    );
    assert!(
        problems.is_empty(),
        "authored markup that would lose its nonce, or take one it should not:\n  {}\n\
         Seal it: `static X: crate::seal::Sealed = crate::seal::Sealed::new(r#\"…\"#);` for text, \
         or `crate::seal::seal(…)` on a literal that holds no visitor text. Never put a tag in the \
         same `format!` literal as a value: hoist it into a sealed static and interpolate that.",
        problems.join("\n  ")
    );
}

#[test]
fn the_scan_flags_the_shapes_it_exists_for() {
    let bare = "fn page() -> String {\n    format!(r#\"<p>{x}</p><script>run()</script>\"#)\n}\n";
    assert_eq!(unsealed(bare), vec![2]);
    let bare_style = "const CSS: &str = r\"<style>.a{}</style>\";\n";
    assert_eq!(unsealed(bare_style), vec![1]);
    let template = "const T: &str = include_str!(\"../../templates/x.html\");\n";
    assert_eq!(unsealed_templates(template), vec![1]);
    let template_wrapped =
        "const T: &str =\n    include_str!(\n        \"../../templates/x.html\");\n";
    assert_eq!(unsealed_templates(template_wrapped), vec![2]);
}

#[test]
fn the_scan_passes_the_shapes_it_must_leave_alone() {
    let sealed = "static S: crate::seal::Sealed = crate::seal::Sealed::new(r#\"<script>run()</script>\"#);\n";
    assert!(unsealed(sealed).is_empty());
    let wrapped = "static S: crate::seal::Sealed =\n    crate::seal::Sealed::new(r#\"<style>\n.a{}\n</style>\"#);\n";
    assert!(unsealed(wrapped).is_empty());
    let sealed_in_place = "fn f() -> String {\n    crate::seal::seal(&format!(\n        \"<style>{CSS}</style>\"\n    ))\n}\n";
    assert!(unsealed(sealed_in_place).is_empty());
    let comment_only = "// the layout stamps every <script> it finds\nfn f() {}\n";
    assert!(unsealed(comment_only).is_empty());
    let in_tests = "fn f() {}\n#[cfg(test)]\nmod tests {\n    const X: &str = \"<script>alert(1)</script>\";\n}\n";
    assert!(unsealed(in_tests).is_empty());
    let not_a_tag = "const X: &str = \"<scripts> <styled-card> <script-like>\";\n";
    assert!(unsealed(not_a_tag).is_empty());
    let sealed_template = "static T: crate::seal::Sealed =\n    crate::seal::Sealed::new(include_str!(\"../../templates/x.html\"));\n";
    assert!(unsealed_templates(sealed_template).is_empty());
}
