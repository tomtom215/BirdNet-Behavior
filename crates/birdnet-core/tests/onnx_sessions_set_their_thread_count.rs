//! Every ONNX Runtime session the station builds sets its thread count.
//!
//! Left at ONNX Runtime's default (0), the intra-op pool is sized to the
//! machine and each thread is pinned to a core with `sched_setaffinity`. The
//! systemd unit denies that syscall (`SystemCallFilter=~@resources`), so the
//! kernel kills the service with SIGSYS the moment such a session is built —
//! and again on every restart. The geomodel session was built that way; 0.17.0's
//! installer enabled the geomodel, and an in-place update left a station that
//! never came back up. The classifier session always set its count, which is
//! why nothing caught it: the same filter, one call site apart.
//!
//! `installer/test/upgrade-e2e.sh` catches the behaviour under a real unit;
//! this catches the shape in `cargo test`, without systemd.

use std::path::{Path, PathBuf};

/// How far past `Session::builder(…)` the builder chain may run before its
/// `commit_from_*`.
const WINDOW: usize = 2_000;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The file with `//` comments removed, so prose that names the builder is
/// not mistaken for a call, and the chain is matched across line breaks.
fn code_of(src: &str) -> String {
    src.lines()
        .map(|l| l.find("//").map_or(l, |i| &l[..i]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every Rust file under `root/src` and `root/crates/*/src`.
fn sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let Ok(crates) = std::fs::read_dir(root.join("crates")) else {
        return Vec::new();
    };
    for krate in crates.flatten() {
        rust_files(&krate.path().join("src"), &mut files);
    }
    files
}

/// `(file, builder chain)` for every `Session::builder(…)` call under `root`.
///
/// Matched with its argument list open: from `ort` 2.0.0-rc.14 the builder
/// takes the process's `Environment`, `Session::builder(env)`, and a needle
/// that still read `Session::builder()` found no call at all.
fn builders(root: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for file in sources(root) {
        let code = code_of(&std::fs::read_to_string(&file).unwrap_or_default());
        let mut from = 0;
        while let Some(at) = code[from..].find("Session::builder(") {
            let start = from + at;
            let tail = &code[start..code.len().min(start + WINDOW)];
            let chain = tail
                .find("commit_from")
                .map_or(tail, |end| &tail[..end])
                .to_owned();
            found.push((file.clone(), chain));
            from = start + 1;
        }
    }
    found
}

#[test]
fn every_onnx_session_sets_its_intra_op_thread_count() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let found = builders(&root);

    // The scan has to have seen the two sessions the station runs, or a green
    // result says nothing.
    for expected in ["inference/model.rs", "inference/species_filter.rs"] {
        assert!(
            found.iter().any(|(f, _)| f.ends_with(expected)),
            "found no Session::builder(…) in {expected}; the scan is not reading the sources"
        );
    }

    let unpinned: Vec<String> = found
        .iter()
        .filter(|(_, chain)| !chain.contains("with_intra_threads"))
        .map(|(f, _)| f.display().to_string())
        .collect();
    assert!(
        unpinned.is_empty(),
        "ONNX sessions built without with_intra_threads (ORT's default pins its \
         threads with sched_setaffinity, which the systemd unit kills the service for): \
         {unpinned:#?}"
    );
}

/// The environment-wide counterpart. `ort` 2.0.0-rc.14 can give the whole
/// environment one thread pool (`with_global_thread_pool`), which sessions then
/// share instead of their own — so a count set on each session would no longer
/// be the one that runs, and the pool's defaults are ORT's. None is used; this
/// keeps it that way until someone has checked what such a pool does under the
/// unit's syscall filter.
#[test]
fn no_environment_wide_thread_pool() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = sources(&root);
    assert!(
        files.len() > 100,
        "the scan read only {} files",
        files.len()
    );
    let users: Vec<String> = files
        .iter()
        .filter(|f| {
            code_of(&std::fs::read_to_string(f).unwrap_or_default())
                .contains("with_global_thread_pool")
        })
        .map(|f| f.display().to_string())
        .collect();
    assert!(
        users.is_empty(),
        "an environment-wide ONNX thread pool: {users:#?}"
    );
}
