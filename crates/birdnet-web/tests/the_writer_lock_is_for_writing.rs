//! A database call in a handler goes to a read connection unless it writes, and
//! every exception is listed with its reason.
//!
//! `AppState::with_db` takes the one connection every writer shares, the capture
//! daemon's detection inserts included. A page that read through it waited behind
//! a detection being stored and made the detection wait behind the page; sixteen
//! visitors to a public feed held all of it for 81 s. 223 call sites took it; 117
//! of them only read and now use `with_read_db` (the pool of read-only
//! connections). What is left are the writes, and
//! `writer_lock_sites.txt` is their ledger. The `DuckDB` analytics store has the
//! same split (`with_analytics` for writes, `with_analytics_read` and
//! `with_timeseries_read` for reads, on two further connections) and its own
//! ledger, `analytics_writer_sites.txt`.
//!
//! Two checks, each with its counterpart in the self-tests below:
//!
//! 1. Every `with_db(` in shipped code is in the ledger (by file and enclosing
//!    function, with a count) and every ledger line still exists. A new call to the
//!    writer is therefore a line someone reviews, with the reason it writes beside
//!    it, and a deleted one cannot leave a stale excuse behind.
//! 2. No closure given to `with_read_db` contains a write: an `INSERT`, `UPDATE`,
//!    `DELETE`, `CREATE`, `DROP`, `ALTER`, `VACUUM` or `REINDEX` in SQL text, a
//!    `.execute(`, `.execute_batch(` or transaction, or an `ensure_…(` call (the
//!    `CREATE TABLE IF NOT EXISTS` helpers). The pooled connections are read-only,
//!    so such a write fails; the case this exists for is the failure that is
//!    swallowed (`.ok()`, `let _ =`), which no response shows. This is a direct
//!    scan of the closure, not of what it calls; `every_route_answers_through_a_read_only_connection.rs`
//!    covers what it calls, for the routes it reaches.

use std::collections::BTreeMap;
use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The source before its test module, with `//` comment lines blanked (offsets
/// are kept). Test modules are gated `#[cfg(test)]` or, for the analytics ones,
/// `#[cfg(all(test, feature = "analytics"))]`.
fn shipped(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            break;
        }
        if t.starts_with("//") {
            out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        } else {
            out.push_str(line);
        }
    }
    out
}

fn enclosing_fn(text: &str, at: usize) -> String {
    let head = &text[..at];
    let mut best = String::from("<none>");
    let mut from = 0;
    while let Some(i) = head[from..].find("fn ") {
        let start = from + i;
        from = start + 3;
        let boundary = start == 0
            || !head.as_bytes()[start - 1].is_ascii_alphanumeric()
                && head.as_bytes()[start - 1] != b'_';
        if !boundary {
            continue;
        }
        let name: String = head[start + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            best = name;
        }
    }
    best
}

/// The text of the parenthesised argument starting at `open` (the index of
/// `(`), skipping strings, raw strings, char literals and comments.
fn argument(text: &str, open: usize) -> &str {
    let b = text.as_bytes();
    let mut depth = 0_i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'r' if (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
                && matches!(b.get(i + 1), Some(b'"' | b'#')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while b.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) == Some(&b'"') {
                    let close: Vec<u8> = std::iter::once(b'"')
                        .chain(std::iter::repeat_n(b'#', hashes))
                        .collect();
                    j += 1;
                    while j + close.len() <= b.len() && b[j..j + close.len()] != close[..] {
                        j += 1;
                    }
                    i = j + close.len();
                    continue;
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                // A char literal ('x', '\n'), not a lifetime ('a).
                if b.get(i + 2) == Some(&b'\'') {
                    i += 3;
                    continue;
                }
                if b.get(i + 1) == Some(&b'\\') && b.get(i + 3) == Some(&b'\'') {
                    i += 4;
                    continue;
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open..=i];
                }
            }
            _ => {}
        }
        i += 1;
    }
    &text[open..]
}

/// One database, its writer methods and its reader methods.
struct Surface {
    name: &'static str,
    /// Methods that take the single writer connection.
    writers: &'static [&'static str],
    /// Methods that take a read connection.
    readers: &'static [&'static str],
    /// The ledger of the writer sites, relative to this crate.
    ledger: &'static str,
    /// Calls that mean a write when they appear in a reader's closure.
    write_calls: &'static [&'static str],
    /// Fewer writer sites than this means the scan is not reading the code.
    min_sites: usize,
}

const SURFACES: &[Surface] = &[
    Surface {
        name: "SQLite",
        writers: &["with_db("],
        readers: &["with_read_db("],
        ledger: "tests/writer_lock_sites.txt",
        write_calls: &[
            ".execute(",
            ".execute_batch(",
            ".transaction(",
            ".unchecked_transaction(",
        ],
        min_sites: 40,
    },
    Surface {
        name: "DuckDB analytics",
        writers: &["with_analytics(", "with_timeseries("],
        readers: &["with_analytics_read(", "with_timeseries_read("],
        ledger: "tests/analytics_writer_sites.txt",
        write_calls: &[
            ".execute(",
            ".execute_batch(",
            ".insert_",
            ".delete_",
            ".set_",
            ".clear_",
            ".sync_",
            ".resync",
        ],
        min_sites: 5,
    },
];

/// `(file-relative key, enclosing fn)` for each call of a `writers` method in
/// shipped code.
fn writer_sites(file: &str, source: &str, writers: &[&str]) -> Vec<(String, String)> {
    let text = shipped(source);
    let mut out = Vec::new();
    for method in writers {
        let mut from = 0;
        while let Some(i) = text[from..].find(method) {
            let at = from + i;
            from = at + method.len();
            let before = text[..at].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue; // with_ingest_db( etc. are other methods
            }
            out.push((file.to_owned(), enclosing_fn(&text, at)));
        }
    }
    out
}

/// Whether `word` appears in `text` as a whole word (so `JOB_BACKUP_VACUUM`, a
/// constant's name, is not a `VACUUM` statement).
fn has_word(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    while let Some(i) = text[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        if (start == 0 || !ident(b[start - 1])) && (end >= b.len() || !ident(b[end])) {
            return true;
        }
        from = start + 1;
    }
    false
}

const WRITE_SQL: &[&str] = &[
    "INSERT",
    "UPDATE",
    "DELETE",
    "REPLACE INTO",
    "CREATE",
    "DROP",
    "ALTER",
    "VACUUM",
    "REINDEX",
];

/// Writes inside the closure given to each `readers` method in shipped code.
fn writes_on_readers(file: &str, source: &str, surface: &Surface) -> Vec<String> {
    let text = shipped(source);
    let mut out = Vec::new();
    for method in surface.readers {
        let mut from = 0;
        while let Some(i) = text[from..].find(method) {
            let at = from + i;
            from = at + method.len();
            let arg = argument(&text, at + method.len() - 1);
            let mut hits: Vec<&str> = WRITE_SQL
                .iter()
                .copied()
                .filter(|w| has_word(arg, w))
                .chain(
                    surface
                        .write_calls
                        .iter()
                        .copied()
                        .filter(|w| arg.contains(w)),
                )
                .collect();
            // `ensure_x(` helpers run CREATE TABLE IF NOT EXISTS.
            if let Some(e) = arg.find("ensure_")
                && arg[e..].find('(').is_some_and(|p| {
                    arg[e..e + p]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
            {
                hits.push("ensure_…(");
            }
            if !hits.is_empty() {
                out.push(format!(
                    "{file}: {} gives {method} a closure containing {hits:?}",
                    enclosing_fn(&text, at)
                ));
            }
        }
    }
    out
}

fn parse_ledger(text: &str) -> BTreeMap<(String, String), usize> {
    let mut out = BTreeMap::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (key, rest) = line.split_once("  ").expect("`file :: fn  count  reason`");
        let (file, name) = key.split_once(" :: ").expect("`file :: fn`");
        let count: usize = rest
            .split_whitespace()
            .next()
            .and_then(|c| c.parse().ok())
            .expect("a count after the function");
        out.insert((file.trim().to_owned(), name.trim().to_owned()), count);
    }
    out
}

#[test]
fn every_writer_site_is_listed_and_every_listing_exists() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 100, "found only {} source files", files.len());

    let mut problems = Vec::new();
    for surface in SURFACES {
        let mut found: BTreeMap<(String, String), usize> = BTreeMap::new();
        for path in &files {
            let rel = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let source = std::fs::read_to_string(path).expect("readable");
            for key in writer_sites(&rel, &source, surface.writers) {
                *found.entry(key).or_default() += 1;
            }
            problems.extend(writes_on_readers(&rel, &source, surface));
        }
        let ledger = parse_ledger(
            &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(surface.ledger))
                .unwrap_or_else(|e| panic!("{}: {e}", surface.ledger)),
        );
        let methods = surface.writers.join(" / ");
        for (key, n) in &found {
            match ledger.get(key) {
                None => problems.push(format!(
                    "{} :: {} takes the {} writer ({n}x) and is not in {}. If it only reads, use the \
                     read method; if it writes, list it with the reason.",
                    key.0, key.1, surface.name, surface.ledger
                )),
                Some(listed) if listed != n => problems.push(format!(
                    "{} :: {} takes the {} writer {n}x but {} says {listed}",
                    key.0, key.1, surface.name, surface.ledger
                )),
                Some(_) => {}
            }
        }
        for key in ledger.keys().filter(|k| !found.contains_key(*k)) {
            problems.push(format!(
                "{} lists {} :: {}, which no longer calls {methods}; remove the line",
                surface.ledger, key.0, key.1
            ));
        }
        assert!(
            found.values().sum::<usize>() >= surface.min_sites,
            "the {} scan found only {} writer sites; is it reading the right text?",
            surface.name,
            found.values().sum::<usize>()
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

const fn sqlite() -> &'static Surface {
    &SURFACES[0]
}
const fn duckdb() -> &'static Surface {
    &SURFACES[1]
}

#[test]
fn the_scan_flags_the_shapes_it_exists_for() {
    let writer = "pub fn show(state: &AppState) {\n    state.with_db(|conn| read(conn));\n}\n";
    assert_eq!(
        writer_sites("a.rs", writer, sqlite().writers),
        vec![("a.rs".to_owned(), "show".to_owned())]
    );
    let swallowed = "fn page(s: &AppState) {\n    s.with_read_db(|conn| {\n        let _ = conn.execute(\"x\", []);\n    });\n}\n";
    assert_eq!(writes_on_readers("a.rs", swallowed, sqlite()).len(), 1);
    let sql =
        "fn page(s: &AppState) {\n    s.with_read_db(|c| c.prepare(\"UPDATE t SET a=1\"));\n}\n";
    assert_eq!(writes_on_readers("a.rs", sql, sqlite()).len(), 1);
    let keyword = "fn page(s: &AppState) {\n    s.with_read_db(|c| c.query_row(\"SELECT 1 -- INSERT later\", [], |r| r.get(0)));\n}\n";
    assert_eq!(
        writes_on_readers("a.rs", keyword, sqlite()).len(),
        1,
        "a keyword inside SQL text is a hit, by design: the scan is direct and conservative"
    );
    let ensure = "fn page(s: &AppState) {\n    s.with_read_db(|c| { ensure_settings_table(c).ok(); 1 });\n}\n";
    assert_eq!(writes_on_readers("a.rs", ensure, sqlite()).len(), 1);
    // DuckDB: the writer methods, and a mirror call on a reader.
    let analytics = "fn mirror(s: &AppState) {\n    s.with_analytics(|adb| adb.insert_detection(&d));\n    s.with_timeseries(|t| t.x());\n}\n";
    assert_eq!(writer_sites("a.rs", analytics, duckdb().writers).len(), 2);
    let on_reader = "fn page(s: &AppState) {\n    s.with_analytics_read(|adb| adb.delete_detection(a, b));\n}\n";
    assert_eq!(writes_on_readers("a.rs", on_reader, duckdb()).len(), 1);
    let ts = "fn page(s: &AppState) {\n    s.with_timeseries_read(|t| t.conn().execute_batch(\"x\"));\n}\n";
    assert_eq!(writes_on_readers("a.rs", ts, duckdb()).len(), 1);
}

#[test]
fn the_scan_leaves_alone_what_it_must() {
    let constant =
        "fn page(s: &AppState) {\n    s.with_read_db(|c| job(c, JOB_BACKUP_VACUUM));\n}\n";
    assert!(
        writes_on_readers("a.rs", constant, sqlite()).is_empty(),
        "a constant whose name contains VACUUM is not a VACUUM"
    );
    let reads = "fn page(s: &AppState) {\n    s.with_read_db(|c| c.query_row(\"SELECT COUNT(*) FROM detections\", [], |r| r.get(0)));\n}\n";
    assert!(writes_on_readers("a.rs", reads, sqlite()).is_empty());
    let other = "fn f(s: &AppState) { s.with_ingest_db(|c| c); }\n";
    assert!(
        writer_sites("a.rs", other, sqlite().writers).is_empty(),
        "with_ingest_db is not with_db"
    );
    let read_only = "fn f(s: &AppState) { s.with_analytics_read(|a| a.sessionize(&p)); }\n";
    assert!(
        writer_sites("a.rs", read_only, duckdb().writers).is_empty(),
        "with_analytics_read is not with_analytics"
    );
    assert!(writes_on_readers("a.rs", read_only, duckdb()).is_empty());
    let in_comment = "// s.with_db(|c| c) is how this used to be written\nfn f() {}\n";
    assert!(writer_sites("a.rs", in_comment, sqlite().writers).is_empty());
    let in_tests = "fn f() {}\n#[cfg(all(test, feature = \"analytics\"))]\nmod tests { fn t(s: &S) { s.with_db(|c| c); } }\n";
    assert!(writer_sites("a.rs", in_tests, sqlite().writers).is_empty());
    let ledger = parse_ledger("# c\n\nroutes/x.rs :: save  2  POST /x\n");
    assert_eq!(
        ledger.get(&("routes/x.rs".to_owned(), "save".to_owned())),
        Some(&2)
    );
}
