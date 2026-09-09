//! Every `birdnet.conf` key the workspace reads is in
//! `birdnet_core::config::known_keys::KNOWN_CONFIG_KEYS`, and every key there
//! is read by something (LC-7).
//!
//! The runtime warns about a key in the file that nothing reads, and the
//! warning is only as good as the list: a reader added without an entry makes
//! the station call a real setting a typo, and an entry that outlives its
//! reader lets a dead key sit in a config file for years looking honoured.
//! Both directions are drift, and this reads the source rather than trusting
//! either side.
//!
//! The reads are found by shape. A config value is asked for through one of a
//! small set of call forms — `config.get("KEY")`, `get_or`, `get_parsed`,
//! `require`, the offsite helpers `setting(config, "KEY")` and friends, the
//! settings bridge `Wiring::Bridged("KEY")`, the dynamic-threshold closures —
//! and the shape list is pinned by the reverse direction: a known key read
//! through a shape this scan does not recognise shows up as "known but never
//! read", which is the signal to teach the scan the new shape rather than to
//! delete the key.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use birdnet_core::config::known_keys::KNOWN_CONFIG_KEYS;

/// Every `.rs` file under the workspace's production source trees.
fn production_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut roots = vec![root.join("src")];
    for entry in fs::read_dir(root.join("crates")).expect("crates/") {
        let dir = entry.expect("entry").path();
        if dir.join("src").is_dir() {
            roots.push(dir.join("src"));
        }
    }
    let mut files = Vec::new();
    for r in roots {
        walk(&r, &mut files);
    }
    files.sort();
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_none_or(|n| n != "tests.rs")
        {
            out.push(path);
        }
    }
}

/// The source with every `#[cfg(test)]`-attributed item removed by brace
/// depth (a `mod tests { … }` block or a one-line `mod tests;`), and with
/// comment lines dropped so prose about a read is not a read.
fn production_half(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if line.trim_start().starts_with("#[cfg(test)]") {
            let mut depth: i32 = 0;
            let mut seen_open = false;
            for l in lines.by_ref() {
                depth += i32::try_from(l.matches('{').count()).unwrap_or(0);
                depth -= i32::try_from(l.matches('}').count()).unwrap_or(0);
                if l.contains('{') {
                    seen_open = true;
                }
                if l.trim_start().starts_with('#') && !seen_open {
                    continue;
                }
                if (seen_open && depth <= 0) || (!seen_open && l.trim_end().ends_with(';')) {
                    break;
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The call shapes through which a `birdnet.conf` key is read. Each is the
/// text immediately before the opening quote of the key.
const READ_SHAPES: &[&str] = &[
    ".get(\"",
    ".get_or(\"",
    ".require(\"",
    "get(config, \"",
    "setting(config, \"",
    "config_path(config, \"",
    "parsed_or(config, \"",
    "require(config, \"",
    "Wiring::Bridged(\"",
    "flag(\"",
    "number(\"",
    "check_unit_range(config, \"",
    "check_bounded(config, \"",
    "check_positive_int(config, \"",
    // The API token is read through a named constant.
    "pub const API_TOKEN_KEY: &str = \"",
];

/// Keys read in `src`, by shape. `get_parsed::<T>("KEY")` is matched
/// separately because the turbofish sits between the name and the quote.
fn keys_read(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut take = |rest: &str| {
        if let Some(end) = rest.find('"') {
            let key = &rest[..end];
            if is_key_shaped(key) {
                out.insert(key.to_owned());
            }
        }
    };
    for shape in READ_SHAPES {
        for (idx, _) in src.match_indices(shape) {
            take(&src[idx + shape.len()..]);
        }
    }
    for (idx, _) in src.match_indices(".get_parsed") {
        let rest = &src[idx..];
        if let Some(open) = rest.find("(\"") {
            take(&rest[open + 2..]);
        }
    }
    // A local closure `let get = |key| config.get(key)` is called as a bare
    // `get("KEY")`: the shape with no receiver, which the dotted form above
    // does not match.
    for (idx, _) in src.match_indices("get(\"") {
        let before = src[..idx].chars().next_back();
        if before.is_none_or(|c| !(c == '.' || c.is_alphanumeric() || c == '_')) {
            take(&src[idx + "get(\"".len()..]);
        }
    }
    out
}

/// `UPPER_SNAKE`, at least three characters: the shape every config key has,
/// which keeps a `.get("some-header")` on another map out of the set.
fn is_key_shaped(key: &str) -> bool {
    key.len() >= 3
        && key.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Offsite keys are read through `setting(config, key)` with the key held in
/// a local, so the offsite module is scanned for its quoted keys in any
/// position — every uppercase literal in that file is a config key.
fn offsite_keys(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = src;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        let key = &after[..close];
        if is_key_shaped(key) && key.starts_with("OFFSITE_") {
            out.insert(key.to_owned());
        }
        rest = &after[close + 1..];
    }
    out
}

#[test]
fn every_config_key_read_anywhere_is_known_and_every_known_key_is_read() {
    let mut read: BTreeSet<String> = BTreeSet::new();
    let mut where_read: Vec<(String, String)> = Vec::new();
    for path in production_sources() {
        let src = fs::read_to_string(&path).expect("read");
        let production = production_half(&src);
        let production = production.as_str();
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut keys = keys_read(production);
        if rel.ends_with("helpers/offsite.rs") {
            keys.extend(offsite_keys(production));
        }
        for k in keys {
            where_read.push((k.clone(), rel.clone()));
            read.insert(k);
        }
    }
    // A synthetic finding name the validator uses that is not a file key.
    read.remove("AUDIO_SOURCE");

    let known: BTreeSet<String> = KNOWN_CONFIG_KEYS.iter().map(|k| (*k).to_owned()).collect();

    let unknown_reads: Vec<String> = read
        .difference(&known)
        .map(|k| {
            let at: Vec<&str> = where_read
                .iter()
                .filter(|(key, _)| key == k)
                .map(|(_, f)| f.as_str())
                .collect();
            format!("{k} (read in {})", at.join(", "))
        })
        .collect();
    let dead_entries: Vec<&String> = known.difference(&read).collect();

    assert!(
        read.len() >= 80,
        "the scan found only {} keys; it is no longer reading the workspace and \
         must be fixed, not deleted",
        read.len()
    );
    assert!(
        unknown_reads.is_empty(),
        "these birdnet.conf keys are read by the code and missing from \
         KNOWN_CONFIG_KEYS, so a station would call them typos:\n  {}",
        unknown_reads.join("\n  ")
    );
    assert!(
        dead_entries.is_empty(),
        "KNOWN_CONFIG_KEYS lists keys nothing reads (or reads through a shape \
         this scan does not know — add the shape to READ_SHAPES if so):\n  {}",
        dead_entries
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn the_scan_recognises_the_read_shapes() {
    let src = r#"
        let a = config.get("ALPHA_ONE");
        let b = cfg.get_or("BETA_TWO", "x");
        let c = config.get_parsed::<f32>("GAMMA_THREE");
        let d = setting(config, "DELTA_FOUR");
        let e = headers.get("content-type");
        let get = |k: &str| config.get(k);
        let f = get("ZETA_SIX");
        (
            "ui_key",
            Wiring::Bridged("EPSILON_FIVE"),
        )
    "#;
    let keys = keys_read(src);
    for k in [
        "ALPHA_ONE",
        "BETA_TWO",
        "GAMMA_THREE",
        "DELTA_FOUR",
        "EPSILON_FIVE",
        "ZETA_SIX",
    ] {
        assert!(keys.contains(k), "{k} not found in {keys:?}");
    }
    assert!(!keys.contains("content-type"));

    let with_tests = "fn a() { config.get(\"REAL_ONE\"); }\n#[cfg(test)]\nmod early;\nfn b() { config.get(\"REAL_TWO\"); }\n#[cfg(test)]\nmod tests {\n    fn t() { config.get(\"TEST_ONLY\"); }\n}\n";
    let keys = keys_read(&production_half(with_tests));
    assert!(
        keys.contains("REAL_ONE") && keys.contains("REAL_TWO"),
        "{keys:?}"
    );
    assert!(
        !keys.contains("TEST_ONLY"),
        "a read inside a test module is not a production read: {keys:?}"
    );
}
