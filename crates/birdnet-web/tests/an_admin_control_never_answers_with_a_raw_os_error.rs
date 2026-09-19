//! The controls that delete or replace a station's records must not answer in
//! the vocabulary of a C library.
//!
//! # The defect
//!
//! Restore, clear-detections, clear-clips and check-for-updates all
//! interpolated their error straight into the page:
//!
//! ```text
//! Internal error: No space left on device (os error 28)
//! Failed to clear data: <DbError>
//! Network error: <reqwest::Error>
//! ```
//!
//! That is the single sentence a person reads after the most consequential
//! action in the product, and it tells them nothing they can act on. The crate
//! already has `routes::log_internal` for exactly this — the detail belongs in
//! the log, where it is useful — and these handlers did not use it.
//!
//! One message is deliberately *not* reduced to a reassurance.
//! `restore_archive_into` documents that "Every refusal before step 5 leaves
//! the live files untouched; a failure inside step 5 names the member", and
//! the caller holds only a `String` and cannot tell which happened. So the
//! restore failure says the backup is not applied and the station's state
//! needs checking, and keeps the detail — rather than claiming "nothing was
//! changed", which would be a lie for precisely the failure that matters most.
//!
//! # What is guarded
//!
//! Mechanically, over the sources: no user-facing error string on these
//! surfaces opens with a bare technical preamble. And by rendering: the
//! restore's own failure message says what state the station is in.

use std::path::{Path, PathBuf};

/// The control surfaces whose text a non-technical operator reads.
const CONTROL_FILES: [&str; 4] = [
    "src/routes/admin/system_controls/backup.rs",
    "src/routes/admin/system_controls/data.rs",
    "src/routes/admin/system_controls/update.rs",
    "src/routes/admin/images.rs",
];

/// Openers that name a layer rather than an outcome. Each was in the shipped
/// text; none says anything a reader can act on.
const BARE_PREAMBLES: [&str; 4] = [
    "Internal error:",
    "Network error:",
    "Parse error:",
    "Failed to clear data:",
];

fn control_sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    CONTROL_FILES
        .iter()
        .map(|rel| {
            let path = root.join(rel);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
            (path, text)
        })
        .collect()
}

/// The line is prose the browser will show, rather than a comment about it.
fn is_rendered(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.starts_with("//") && !trimmed.starts_with("///")
}

#[test]
fn no_control_hands_the_operator_a_bare_technical_preamble() {
    let mut offenders = Vec::new();
    for (path, text) in control_sources() {
        for (i, line) in text.lines().enumerate() {
            if !is_rendered(line) {
                continue;
            }
            for preamble in BARE_PREAMBLES {
                if line.contains(preamble) {
                    offenders.push(format!("{}:{} {}", path.display(), i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these controls answer a person with the name of a layer. Put the \
         detail in the log and say what happened and what to do:\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart. Stripping the detail everywhere would pass the check above
/// and leave the one failure that genuinely needs it — a restore that stopped
/// partway — with nothing to go on.
#[test]
fn the_restore_failure_says_what_state_the_station_is_in() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(CONTROL_FILES[0])).expect("backup.rs");

    assert!(
        text.contains("The restore did not finish"),
        "the restore failure must name the outcome"
    );
    assert!(
        text.contains("check <a href=\"/station\">Station health</a> before trying again"),
        "and send the reader somewhere that can tell them what happened"
    );
    assert!(
        text.contains("The station reported: {}"),
        "and keep the detail, which names the member that failed — this is the \
         one message where dropping it would cost the reader something"
    );
    assert!(
        !text.contains("The restore did not finish, and nothing was changed"),
        "the restore must not claim the station is untouched: \
         `restore_archive_into` documents that a failure inside step 5 leaves \
         some members already placed"
    );
}

/// The same rule for the other destructive pair. Clearing detections is two
/// steps and clearing clips walks a directory file by file, so neither may
/// promise the reader that nothing went.
#[test]
fn the_clear_controls_do_not_promise_nothing_was_removed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(CONTROL_FILES[1])).expect("data.rs");

    let rendered: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rendered.contains("nothing was removed"),
        "a clear that failed part way leaves some of it gone; saying otherwise \
         sends the reader away without checking"
    );
    assert!(
        rendered.contains("Some may already have been removed"),
        "and it has to say so, because that is the state they are in"
    );
}
