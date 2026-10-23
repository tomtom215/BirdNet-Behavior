//! The vendored three.js is the build the documentation says it is, it is small, and nothing
//! loads it before somebody asks for the 3D terrain.
//!
//! `static/vendor/three/three.module.min.js` is 538 kB of somebody else's code that the station
//! serves from its own binary. What would go wrong without a gate:
//!
//! * the file is rebuilt (a class added to `tools/vendor-three/entry.js`, a version bumped) and
//!   the hash, the licence, the notice or the URL the terrain imports it from are not, so the
//!   year-long immutable cache hands out the old file under the new page, or a licence text
//!   describes a version that is no longer shipped;
//! * the bundle grows by a whole library's worth, because the entry file imported `*` once,
//!   and nobody notices a 134 kB download turning into 600 kB;
//! * a script on every page starts importing it, and a Pi's browser pays for the 3D view on a
//!   page that never shows it.
//!
//! `tools/vendor-three/build.sh --check` is the other half (it needs the network and the npm
//! registry; this needs neither): it rebuilds the file from the pinned inputs and compares.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Gzipped bytes the bundle may take. It is 134 086 with `gzip -9` today; the budget leaves
/// room for one more class (a few hundred bytes) and nothing like a second renderer.
const BUDGET_GZIP_BYTES: usize = 136_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn text(rel: &str) -> String {
    String::from_utf8(read(rel)).unwrap_or_else(|e| panic!("{rel} is not UTF-8: {e}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

const BUNDLE: &str = "static/vendor/three/three.module.min.js";

/// The value in the README's table row that starts with `label`, between backticks.
fn readme_value(readme: &str, label: &str) -> String {
    let row = readme
        .lines()
        .find(|l| l.starts_with(&format!("| {label}")))
        .unwrap_or_else(|| panic!("README has no row for {label}"));
    // The value is the row's second cell (the label may itself hold backticks).
    let cell = row
        .split('|')
        .nth(2)
        .unwrap_or_else(|| panic!("row {label} has no value cell"))
        .trim();
    assert!(
        cell.starts_with('`') && cell.ends_with('`'),
        "row {label}: value not in backticks: {cell}"
    );
    cell.trim_matches('`').to_owned()
}

#[test]
fn the_bundle_is_the_file_the_readme_hashes() {
    let readme = text("static/vendor/three/README.md");
    let want = readme_value(&readme, "`three.module.min.js` SHA-256");
    assert_eq!(
        sha256_hex(&read(BUNDLE)),
        want,
        "three.module.min.js is not the file static/vendor/three/README.md describes: after a \
         rebuild, update the README's hash, THREE_HASH in static/terrain.js and the size row"
    );
}

#[test]
fn the_terrain_asks_for_the_file_by_its_hash() {
    let js = text("static/terrain.js");
    let sha = sha256_hex(&read(BUNDLE));
    let line = js
        .lines()
        .find(|l| l.starts_with("const THREE_HASH = "))
        .expect("terrain.js names THREE_HASH");
    assert!(
        line.contains(&format!("\"{}\"", &sha[..8])),
        "THREE_HASH must be the first eight digits of {sha}: {line}. The file is served for a \
         year with `immutable`; a URL that does not change when the file does would keep the old one."
    );
    assert!(
        js.contains("./vendor/three/three.module.min.js?v=${THREE_HASH}"),
        "terrain.js imports the library from the URL the server routes, with the hash"
    );
}

/// The budget the bundle is held to, in the number a download costs.
#[test]
fn the_bundle_stays_small() {
    use std::io::Write as _;
    let bytes = read(BUNDLE);
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&bytes).expect("gzip");
    let gz = enc.finish().expect("gzip").len();
    eprintln!(
        "three.module.min.js: {} bytes, {gz} gzipped (budget {BUDGET_GZIP_BYTES})",
        bytes.len()
    );
    assert!(
        gz <= BUDGET_GZIP_BYTES,
        "three.module.min.js is {gz} bytes gzipped, over the {BUDGET_GZIP_BYTES} budget. Did \
         tools/vendor-three/entry.js start importing more than static/terrain.js uses?"
    );
    // A bundle this small cannot be the library whole (the WebGPU build alone is 2.3 MB).
    assert!(bytes.len() < 700_000, "{} bytes", bytes.len());
}

/// Strict CSP: no `unsafe-eval`, so the bundle must not need it, and a browser that has
/// loaded it must not be able to fetch anything else.
#[test]
fn the_bundle_needs_neither_eval_nor_the_network() {
    let js = text(BUNDLE);
    for banned in [
        "new Function",
        "eval(",
        "importScripts",
        "XMLHttpRequest",
        "fetch(",
    ] {
        assert!(!js.contains(banned), "the bundle contains {banned:?}");
    }
    assert!(
        js.starts_with("var ") && js.contains("=\"186\";"),
        "the bundle should start with the library's REVISION (186)"
    );
}

#[test]
fn the_version_is_the_same_everywhere_it_is_written() {
    let pin: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("../../tools/vendor-three/package.json")).expect("package.json"),
    )
    .expect("json");
    let version = pin["dependencies"]["three"]
        .as_str()
        .expect("three is pinned");
    assert!(
        !version.starts_with(['^', '~']) && version.split('.').count() == 3,
        "pinned exactly, not by range: {version}"
    );
    let readme = text("static/vendor/three/README.md");
    assert!(
        readme.contains(&format!("three.js {version}")),
        "README names another version than {version}"
    );
    let notice = std::fs::read_to_string(root().join("../../NOTICE")).expect("NOTICE");
    assert!(
        notice.contains(&format!("three.js {version}")),
        "NOTICE names another version than {version}"
    );
    let licence = std::fs::read(root().join(format!("../../third_party/three-{version}/LICENSE")))
        .unwrap_or_else(|e| panic!("third_party/three-{version}/LICENSE: {e}"));
    assert_eq!(
        licence,
        read("static/vendor/three/LICENSE"),
        "the two licence texts differ"
    );
    assert!(String::from_utf8_lossy(&licence).starts_with("The MIT License"));
    let all =
        std::fs::read_to_string(root().join("../../THIRD_PARTY_LICENSES.txt")).expect("licences");
    assert!(
        all.contains(&format!("three.js {version} (")),
        "THIRD_PARTY_LICENSES.txt has no section for three.js {version}: python3 scripts/gen-third-party-licenses.py"
    );
    // The revision the bundle says it is is the pinned version's minor number.
    let minor = version.split('.').nth(1).expect("minor");
    assert!(
        text(BUNDLE).contains(&format!("=\"{minor}\";")),
        "bundle revision != {minor}"
    );
}

/// The library is named in one place: the terrain's own script. Not the layout, not another
/// script, not a template; so nothing fetches it on a page that does not show the 3D view.
#[test]
fn nothing_but_the_terrain_loads_it() {
    let mut named = Vec::new();
    for dir in ["templates", "static"] {
        let mut stack = vec![root().join(dir)];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).expect("dir") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    if path.ends_with("vendor") || path.ends_with("fonts") {
                        continue;
                    }
                    stack.push(path);
                } else if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("js" | "html" | "css")
                ) {
                    let body = std::fs::read_to_string(&path).unwrap_or_default();
                    if body.contains("three.module") {
                        named.push(
                            path.strip_prefix(root())
                                .expect("under root")
                                .display()
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    named.sort();
    assert_eq!(
        named,
        ["static/terrain.js"],
        "who else names the library: {named:?}"
    );
    // And terrain.js only asks for it from inside the function that opens the 3D view.
    let js = text("static/terrain.js");
    let at = js
        .find("import(`./vendor/three")
        .expect("the dynamic import");
    let before = &js[..at];
    assert!(
        before
            .rfind("async ensureGL()")
            .is_some_and(|f| !before[f..].contains("\n  }\n")),
        "the import must be inside ensureGL(), which only a click on 3D reaches"
    );
    assert!(
        !js.lines()
            .any(|l| l.trim_start().starts_with("import ") && l.contains("three")),
        "no static import of the library"
    );
}
