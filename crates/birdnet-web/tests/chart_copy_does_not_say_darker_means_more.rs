//! Chart copy never tells the reader that more is darker, or lighter (UIR-04).
//!
//! Every sequential ramp in this app runs dark-to-light on paper and
//! light-to-dark in the dark theme (`the_heat_ramp_runs_one_way_and_its_ends_can_be_seen`
//! in `the_chart_palette_can_be_told_apart.rs` holds both directions). So a
//! sentence such as "darker squares were busier" is true in one theme and tells
//! the reader to read the chart backwards in the other. The activity grid says
//! "a stronger colour means more", which is true in both, and that is the
//! wording every chart takes.
//!
//! This scans the text the crate can serve: every `src/**/*.rs` (outside its
//! comments and its `#[cfg(test)]` module), every `templates/**/*.html` (outside
//! its comments), and every first-party `static/*.js` (outside its comments),
//! for the words in [`LIGHTNESS_WORDS`]. Wrapped Rust strings are joined
//! (a `\` continuation and the indentation after it) and so are lines, so a
//! phrase split across two source lines is still one phrase. A use that is
//! true in both themes is a named entry in [`EXEMPT`], and every entry must
//! still match something, so the list cannot outlive what it excuses.

use std::path::{Path, PathBuf};

/// Words that describe a colour by its lightness, which flips with the theme.
const LIGHTNESS_WORDS: &[&str] = &[
    "darker", "lighter", "paler", "deeper", "brighter", "fainter",
];

/// Uses that are true in both themes, as `(file from the crate root, phrase)`.
/// The phrase is matched against the joined, comment-free text of that file.
const EXEMPT: &[(&str, &str)] = &[
    // The spectrogram colour maps are fixed images (viridis, magma, grey): they
    // do not follow the theme, so "louder is lighter" is the same in both.
    (
        "src/routes/admin/settings/render/system.rs",
        "Louder is always lighter",
    ),
    // A canvas composite operation's name, not text a reader sees.
    ("static/skysong.js", "\"lighter\""),
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, ext: &str, recurse: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recurse {
                walk(&path, ext, recurse, out);
            }
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// Remove every `open … close` span (a block comment).
fn strip_spans(src: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find(open) {
        out.push_str(&rest[..i]);
        match rest[i + open.len()..].find(close) {
            Some(j) => rest = &rest[i + open.len() + j + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Drop whole-line `//` comments (doc comments included).
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where the file's `#[cfg(test)] mod …` begins, or its length.
fn test_module_start(src: &str) -> usize {
    let mut from = 0;
    while let Some(i) = src[from..].find("#[cfg(test)]") {
        let at = from + i;
        let after = src[at + "#[cfg(test)]".len()..].trim_start();
        if after.starts_with("mod ")
            || after.starts_with("pub mod ")
            || after.starts_with("pub(crate) mod ")
        {
            return at;
        }
        from = at + 1;
    }
    src.len()
}

/// The text a file can put in front of a reader, joined onto one line.
fn served_text(rel: &str, src: &str) -> String {
    let text = if rel.ends_with(".rs") {
        // The unit tests at the foot of a module are not served (a single
        // `#[cfg(test)]` item elsewhere in the file is left in the scan).
        let src = &src[..test_module_start(src)];
        strip_line_comments(&strip_spans(src, "/*", "*/"))
    } else if rel.ends_with(".js") {
        strip_line_comments(&strip_spans(src, "/*", "*/"))
    } else {
        strip_spans(&strip_spans(src, "<!--", "-->"), "{#", "#}")
    };
    // A Rust string continuation: `\` at the end of a line and the indentation
    // of the next are not in the string.
    let mut joined = String::with_capacity(text.len());
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        joined.push_str(trimmed);
        if joined.ends_with('\\') && rel.ends_with(".rs") {
            joined.pop();
        } else if lines.peek().is_some() {
            joined.push(' ');
        }
    }
    joined
}

/// Every lightness word in `text`, as `(word, a window of context)`.
fn lightness_words(text: &str) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    let bytes = text.as_bytes();
    let mut start = None;
    for i in 0..=bytes.len() {
        let is_word = i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_');
        match (is_word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                let word = text[s..i].to_ascii_lowercase();
                if LIGHTNESS_WORDS.contains(&word.as_str()) {
                    let lo = text.floor_char_boundary(s.saturating_sub(60));
                    let hi = text.ceil_char_boundary((i + 60).min(text.len()));
                    hits.push((word, text[lo..hi].to_string()));
                }
                start = None;
            }
            _ => {}
        }
    }
    hits
}

fn sources() -> Vec<(String, String)> {
    let root = crate_root();
    let mut files = Vec::new();
    walk(&root.join("src"), "rs", true, &mut files);
    walk(&root.join("templates"), "html", true, &mut files);
    // First-party scripts only: `static/vendor` is not ours to word.
    walk(&root.join("static"), "js", false, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .expect("under the crate")
                .to_string_lossy()
                .replace('\\', "/");
            let src = std::fs::read_to_string(&p).expect("read a source file");
            (rel, src)
        })
        .collect()
}

#[test]
fn chart_copy_does_not_describe_more_as_darker_or_lighter() {
    let files = sources();
    assert!(
        files
            .iter()
            .any(|(r, _)| r == "src/routes/pages/history.rs"),
        "the scan must reach the page files"
    );
    let mut found = Vec::new();
    for (rel, src) in &files {
        let mut text = served_text(rel, src);
        for (file, phrase) in EXEMPT {
            if file == rel {
                text = text.replace(phrase, " ");
            }
        }
        for (word, ctx) in lightness_words(&text) {
            found.push(format!("{rel}: \"{word}\" in …{ctx}…"));
        }
    }
    assert!(
        found.is_empty(),
        "chart copy names a lightness, which flips with the theme; say \"a stronger colour\" \
         (or add a named exemption if the sentence is true in both themes):\n{}",
        found.join("\n")
    );
}

/// Counterpart: every exemption still excuses a real use, and the scanner
/// still sees a lightness word in a joined, wrapped Rust string.
#[test]
fn every_exemption_still_matches_and_the_scanner_sees_a_wrapped_word() {
    let files = sources();
    for (file, phrase) in EXEMPT {
        let src = files
            .iter()
            .find(|(r, _)| r == file)
            .map(|(_, s)| s.as_str())
            .unwrap_or_else(|| panic!("exempt file {file} is gone"));
        assert!(
            served_text(file, src).contains(phrase),
            "exemption ({file}, {phrase}) no longer matches anything: remove it"
        );
    }
    let wrapped = "let s = \"a wider, \\\n        darker ribbon\";\n// a darker comment\n";
    let hits = lightness_words(&served_text("x.rs", wrapped));
    assert_eq!(hits.len(), 1, "one served word, one comment: {hits:?}");
    let tested = "fn f() {}\n#[cfg(test)]\nmod tests { const S: &str = \"paler\"; }\n";
    assert!(lightness_words(&served_text("x.rs", tested)).is_empty());
    let html = "<!-- deeper -->\n<p>Lighter squares</p>";
    assert_eq!(lightness_words(&served_text("x.html", html)).len(), 1);
}
