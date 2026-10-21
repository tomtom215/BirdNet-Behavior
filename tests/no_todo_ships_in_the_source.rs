//! No `TODO`, `FIXME`, `todo!()` or `unimplemented!()` ships.
//!
//! A marker in shipped code is a promise nobody is keeping: the six
//! `TODO(O-nn-followup)` comments this gate was written for pointed at work that
//! had been done (the credential store moved into the `users` table, the capture
//! daemon reads `audio_sources`) or at work that was never going to be done in the
//! place the comment said (a help link on every admin page). A reader of the
//! comment could not tell which. Work that is real and open lives in
//! `docs/TECH_DEBT.md`, with the evidence for why it is open; the code says what it
//! does, not what it will one day do.
//!
//! The scan covers every crate's `src/`, the binary's `src/`, and the web UI's
//! templates and scripts. It is a text scan, not a parse, on purpose: a marker is
//! as much a problem in a comment or a string as in a macro.

use std::path::{Path, PathBuf};

/// What counts as a marker. Built from pieces so that this file does not contain
/// them itself.
fn markers() -> Vec<String> {
    vec![
        ["TO", "DO("].concat(),
        ["TO", "DO:"].concat(),
        ["FIX", "ME"].concat(),
        ["to", "do!("].concat(),
        ["unimple", "mented!("].concat(),
    ]
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.map(|e| e.expect("entry").path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "rs" | "html" | "js" | "mjs" | "css"))
            // Vendored, minified code is not ours to edit.
            && !p.to_string_lossy().ends_with(".min.js")
        {
            out.push(p);
        }
    }
}

fn sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    let crates = root.join("crates");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
        .expect("crates")
        .map(|e| e.expect("entry").path())
        .collect();
    dirs.sort();
    for d in dirs {
        walk(&d.join("src"), &mut files);
        walk(&d.join("static"), &mut files);
        walk(&d.join("templates"), &mut files);
    }
    files
}

/// Lines in `text` that carry a marker, as `(line number, line)`.
fn hits(text: &str) -> Vec<(usize, String)> {
    let markers = markers();
    text.lines()
        .enumerate()
        .filter(|(_, l)| markers.iter().any(|m| l.contains(m.as_str())))
        .map(|(i, l)| (i + 1, l.trim().to_owned()))
        .collect()
}

#[test]
fn no_marker_of_unfinished_work_ships() {
    let files = sources();
    // The walk found the code: a scan of nothing is clean.
    assert!(
        files.len() > 400,
        "scanned only {} files; the walk has stopped finding the tree",
        files.len()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        for (n, line) in hits(&text) {
            found.push(format!(
                "{}:{n}  {line}",
                f.strip_prefix(root).unwrap_or(f).display()
            ));
        }
    }
    assert!(
        found.is_empty(),
        "{} marker(s) of unfinished work in shipped code. Do the work, delete the \
         comment, or put the debt in docs/TECH_DEBT.md with its evidence:\n{}",
        found.len(),
        found.join("\n")
    );
}

/// The counterpart: the scan sees each kind of marker, and passes ordinary text,
/// including words that only look like them.
#[test]
fn the_scan_sees_each_marker_and_passes_ordinary_text() {
    let todo = ["TO", "DO"].concat();
    let fixme = ["FIX", "ME"].concat();
    let text = format!(
        "// {todo}(O-1-followup) later\n// {todo}: soon\n// {fixme}\nfn f() {{ {}() }}\nfn g() {{ {}() }}\n",
        ["to", "do!"].concat(),
        ["unimple", "mented!"].concat(),
    );
    assert_eq!(hits(&text).len(), 5, "{text}");
    for fine in [
        "// the mastodon was here",
        "let x = \"XXX\";",
        "// unimplementedness is not a macro",
        "// a to-do list on the page",
    ] {
        assert!(hits(fine).is_empty(), "{fine}");
    }
}
