//! No child process is waited on without a deadline (PR-8).
//!
//! `std::process` has no bounded wait, so one child that never exits — `df`
//! on a dead network mount, `ffmpeg` on a device that stopped answering —
//! held whichever thread waited on it for ever. When that thread was the
//! event processor the detection channel filled, the heartbeat stopped and
//! the watchdog restarted the station with no line saying why.
//! `birdnet_core::process::run_with_timeout` is the one bounded wait, and this
//! gate keeps every spawn in the workspace going through it.
//!
//! The scan is of source text, not of behaviour: it reads every `.rs` file
//! under `src/` and `crates/*/src/`, drops `#[cfg(test)]` modules by brace
//! depth, and rejects a synchronous wait on a `std::process::Command` that is
//! not the helper itself. Long-lived children (`arecord`, the capture `ffmpeg`,
//! the livestream) are spawned and supervised, not waited on, and are not what
//! this is about; a `tokio::process` wait must sit inside `tokio::time::timeout`.

use std::fs;
use std::path::{Path, PathBuf};

/// The one file allowed to call `Command::output`, `status` or `wait`.
const HELPER: &str = "crates/birdnet-core/src/process.rs";

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
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The source with every `#[cfg(test)]`-attributed item removed, by brace
/// depth from the attribute to the matching close.
fn without_test_items(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            // Skip attribute lines up to and including the item; then skip the
            // item's body by brace depth, which for a `mod tests {` block is
            // the whole block and for a `fn` or `use` line is that line.
            let mut depth: i32 = 0;
            let mut seen_open = false;
            for l in lines.by_ref() {
                depth += i32::try_from(l.matches('{').count()).unwrap_or(0);
                depth -= i32::try_from(l.matches('}').count()).unwrap_or(0);
                if l.contains('{') {
                    seen_open = true;
                }
                if l.trim_start().starts_with('#') && !seen_open {
                    continue; // a further attribute on the same item
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

/// A synchronous wait on a `std::process` child in `src`, if any: the first
/// offending line, with its number.
fn first_unbounded_wait(src: &str) -> Option<(usize, String)> {
    let uses_std_command = src.contains("std::process::Command")
        || (src.contains("use std::process::") && imports_command_from_std_process(src));
    if !uses_std_command {
        return None;
    }
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        // A `wait` within three lines of a `kill` is the reap after a kill —
        // the supervised capture child's stop and drop — and is bounded by the
        // kill, not by the child.
        let after_kill = lines[i.saturating_sub(3)..i]
            .iter()
            .any(|l| l.contains(".kill("));
        if t.contains(".wait()") && after_kill {
            continue;
        }
        // `.output()`, `.status()` and `.wait_with_output()` are the waits
        // `Command` and `Child` offer; `.status()` alone is also an HTTP
        // response's, so it counts only in the shape a command chain has —
        // a bare method call line, or one that follows `Command::new(` or a
        // `cmd`/`child` receiver.
        let is_wait = t.contains(".output()")
            || t.contains(".wait_with_output()")
            || t.contains(".wait()")
            || (t.contains(".status()")
                && (t.starts_with(".status()")
                    || t.contains("Command::new(")
                    || t.contains("cmd.status()")
                    || t.contains("child.status()")
                    || t.contains("command.status()")));
        if is_wait {
            return Some((i + 1, (*line).to_owned()));
        }
    }
    None
}

/// Whether a `use std::process::{…}` line brings `Command` into scope.
fn imports_command_from_std_process(src: &str) -> bool {
    src.lines().any(|l| {
        let t = l.trim();
        t.starts_with("use std::process::")
            && (t.contains("Command") || t.ends_with("::*;"))
            && !t.contains("Stdio;")
    })
}

/// A `tokio::process` wait outside `tokio::time::timeout`, if any.
fn first_unbounded_async_wait(src: &str) -> Option<(usize, String)> {
    if !src.contains("tokio::process::Command") {
        return None;
    }
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        let is_wait = t.contains(".wait_with_output()")
            || t.contains(".output().await")
            || t.contains(".status().await")
            || t.contains(".wait().await");
        if is_wait && !t.contains("timeout(") {
            return Some((i + 1, line.to_owned()));
        }
    }
    None
}

#[test]
fn every_synchronous_wait_on_a_child_process_goes_through_the_helper() {
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for path in production_sources() {
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // `tests.rs` is a `#[cfg(test)] mod tests;` split into its own file;
        // the attribute is on the declaration in the parent, not in the file.
        if rel == HELPER || path.file_name().is_some_and(|n| n == "tests.rs") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read");
        let production = without_test_items(&src);
        if production.contains("Command::new(") {
            scanned += 1;
        }
        if let Some((line, text)) = first_unbounded_wait(&production) {
            offenders.push(format!("{rel}:{line}: {}", text.trim()));
        }
        if let Some((line, text)) = first_unbounded_async_wait(&production) {
            offenders.push(format!("{rel}:{line}: {}", text.trim()));
        }
    }
    assert!(
        scanned >= 15,
        "only {scanned} production files spawn a child; the scan is no longer \
         reading the workspace and must be fixed, not deleted"
    );
    assert!(
        offenders.is_empty(),
        "a child process is waited on with no deadline; route the wait through \
         `birdnet_core::process::run_with_timeout` (or `tokio::time::timeout` \
         for a tokio child):\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_scan_sees_a_wait_and_ignores_a_test_module() {
    // The gate above is only as good as its parser: pin that it flags the
    // shapes the codebase actually used, and that a test module is dropped.
    let flagged =
        "use std::process::Command;\nfn f() {\n    let o = Command::new(\"df\").output();\n}\n";
    assert!(first_unbounded_wait(flagged).is_some());
    let chained = "fn f() -> bool {\n    std::process::Command::new(\"x\")\n        .status()\n        .is_ok()\n}\n";
    assert!(first_unbounded_wait(chained).is_some());
    let http = "use std::process::Command;\nfn f(resp: reqwest::blocking::Response) {\n    let s = resp.status();\n    Command::new(\"x\").spawn();\n}\n";
    assert!(
        first_unbounded_wait(http).is_none(),
        "an HTTP status must not be mistaken for a wait"
    );
    let in_tests = "use std::process::Command;\n#[cfg(test)]\nmod tests {\n    fn f() {\n        Command::new(\"x\").output();\n    }\n}\n";
    assert!(first_unbounded_wait(&without_test_items(in_tests)).is_none());
    let reap = "use std::process::Command;\nfn f(child: &mut std::process::Child) {\n    let _ = child.kill();\n    let _ = child.wait();\n}\n";
    assert!(
        first_unbounded_wait(reap).is_none(),
        "a wait after a kill is bounded by the kill"
    );
    let stdio_only = "use std::process::Stdio;\nlet c = tokio::process::Command::new(\"x\").stdout(Stdio::piped());\nlet o = tokio::time::timeout(d, child.wait_with_output()).await;\n";
    assert!(
        first_unbounded_wait(stdio_only).is_none(),
        "`use std::process::Stdio` does not make a tokio child a std one"
    );
    let async_wait =
        "let c = tokio::process::Command::new(\"x\");\nlet o = child.wait_with_output().await;\n";
    assert!(first_unbounded_async_wait(async_wait).is_some());
    let bounded = "let c = tokio::process::Command::new(\"x\");\nlet o = tokio::time::timeout(d, child.wait_with_output()).await;\n";
    assert!(first_unbounded_async_wait(bounded).is_none());
}
