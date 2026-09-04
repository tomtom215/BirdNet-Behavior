//! The station's restart decision is wired once, at startup, and nowhere else.
//!
//! # What went wrong
//!
//! `request_restart` used to read `INVOCATION_ID`/`JOURNAL_STREAM` itself, on
//! every request. That made the endpoint's behaviour a property of whatever
//! environment the calling process inherited — and a GitHub Actions runner
//! sets `INVOCATION_ID`. So in CI the restart endpoint took the *signalling*
//! branch, and `crates/birdnet-web/tests/the_api_can_change_the_station.rs`
//! had the test binary send itself `SIGTERM` 400 ms later. It surfaced as
//!
//! ```text
//! ---- a_restart_says_so_when_nothing_would_bring_the_station_back stdout ----
//! panicked at crates/birdnet-web/tests/the_api_can_change_the_station.rs:665:5:
//! this test assumes it is not running under systemd
//! ```
//!
//! only because that test happened to assert the environment before calling.
//! `every_documented_route_is_mounted`, in the same binary, `POST`s every
//! route in the table with a valid token — including `/api/v2/control/restart`
//! — and had no such guard.
//!
//! # Why a source scan
//!
//! The fix is that the answer is decided once, by `app.rs`, and carried on
//! `AppState`. Two halves of that can each be broken silently:
//!
//! * **Nothing reads the environment.** Delete the `with_supervised_by_systemd`
//!   call from `app.rs` and every station refuses every restart, for ever, with
//!   a plausible message. No test fails: the whole test suite builds states that
//!   are — correctly — not supervised, and asserts exactly that.
//! * **Something reads it again.** A later handler that goes back to
//!   `std::env::var("INVOCATION_ID")` re-creates the original defect, and the
//!   test that would catch it is the one that gets killed.
//!
//! Neither is visible to a behavioural test, so this is 2.5's lesson again: a
//! rule expressed only as scattered call sites cannot be checked.

use std::path::{Path, PathBuf};

/// The two variables that say "systemd started this process".
const SYSTEMD_VARS: [&str; 2] = ["INVOCATION_ID", "JOURNAL_STREAM"];

/// The file holding the single function allowed to read them.
const THE_ONE_READER: &str = "crates/birdnet-web/src/routes/admin/system_controls/service.rs";

/// The function inside that file, and nothing else in it.
const THE_ONE_FUNCTION: &str = "pub fn supervised_by_systemd(";

/// The line range of `THE_ONE_FUNCTION`'s body, as 1-based inclusive bounds.
///
/// Ends at the first line that is exactly `}` — a top-level item's closing
/// brace, since everything nested is indented. Returns `None` if the function
/// is gone, which the caller treats as a failure rather than a free pass: a
/// scanner that silently finds nothing is the failure mode this whole file
/// exists to prevent.
fn the_one_function_body(text: &str) -> Option<(usize, usize)> {
    let start = text.lines().position(|l| l.starts_with(THE_ONE_FUNCTION))? + 1;
    let end = text
        .lines()
        .skip(start)
        .position(|l| l == "}")
        .map(|off| start + off + 1)?;
    Some((start, end))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, recursively, skipping `target`.
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

/// The application records the answer on the state it serves.
#[test]
fn the_startup_wiring_decides_whether_a_restart_would_do_anything() {
    let app = std::fs::read_to_string(repo_root().join("src/app.rs")).expect("src/app.rs");
    assert!(
        app.contains(".with_supervised_by_systemd("),
        "src/app.rs no longer records whether systemd is supervising this process. \
         Every station now refuses every restart with \"not running under systemd\", \
         and no behavioural test can see it: the suite's states are all — correctly \
         — unsupervised, and assert exactly that."
    );
    assert!(
        app.contains("supervised_by_systemd()"),
        "src/app.rs passes something other than the environment probe to \
         with_supervised_by_systemd"
    );
}

/// Nothing else in the workspace decides it for itself.
#[test]
fn only_one_place_asks_the_environment_whether_we_are_under_systemd() {
    let mut files = Vec::new();
    rust_files(&repo_root(), &mut files);
    assert!(files.len() > 100, "found only {} sources", files.len());

    let allowed_file = repo_root().join(THE_ONE_READER);
    let allowed_text = std::fs::read_to_string(&allowed_file).expect(THE_ONE_READER);
    let allowed_span = the_one_function_body(&allowed_text).unwrap_or_else(|| {
        panic!("{THE_ONE_READER} no longer defines `{THE_ONE_FUNCTION}`; retarget this gate")
    });

    let mut offenders: Vec<String> = Vec::new();
    let mut inside_the_one_function = 0_usize;

    for path in &files {
        if path.ends_with(file!()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let is_allowed_file = *path == allowed_file;
        for (n, line) in text.lines().enumerate() {
            // A `var`/`var_os` lookup, not a mention in prose or a doc comment.
            let is_lookup = line.contains("env::var") || line.contains("env::var_os");
            if !(is_lookup && SYSTEMD_VARS.iter().any(|v| line.contains(v))) {
                continue;
            }
            let lineno = n + 1;
            // The exemption is the *function*, not the file it lives in. The
            // original defect was `request_restart` reading the environment —
            // in this very file, a few lines below the exempt one — so a
            // file-level exemption would have let it straight back in.
            if is_allowed_file && (allowed_span.0..=allowed_span.1).contains(&lineno) {
                inside_the_one_function += 1;
                continue;
            }
            offenders.push(format!(
                "{}:{}: {}",
                path.strip_prefix(repo_root()).unwrap_or(path).display(),
                lineno,
                line.trim()
            ));
        }
    }

    // The counterpart, and the answer to "why does this pass?": the scanner
    // finds the one legitimate read. Without this, a matcher that found
    // nothing at all — a renamed variable, a reshaped call — would report a
    // clean tree for ever.
    assert!(
        inside_the_one_function > 0,
        "the scanner found no environment read inside `{THE_ONE_FUNCTION}` at all, so it \
         is not looking for the right thing and its clean result means nothing"
    );

    assert!(
        offenders.is_empty(),
        "{} places read the systemd environment for themselves, re-creating the defect \
         where the restart endpoint behaved differently under a test binary that \
         inherited INVOCATION_ID (a GitHub Actions runner sets it) and had the test \
         process SIGTERM itself. The answer is decided once, by \
         `supervised_by_systemd()` in {THE_ONE_READER}, and carried on AppState:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
