//! A page may hand someone a shell command, but not as the answer.
//!
//! # The defect
//!
//! Several surfaces met a non-technical reader with a command line as the
//! whole of their advice:
//!
//! * The one-click **Restart** button, when it could not act, said only
//!   "Restart it from a shell: `sudo systemctl restart birdnet-behavior`".
//! * The **Analytics Engine** tile on the health page said "Start with
//!   `--analytics-db` to enable" — a start-up flag, on the page written for
//!   the person who owns the station rather than the one who compiles it.
//! * The behavioural status table told the reader to set
//!   `BIRDNET_BUNDLED_EXTENSION_FILE` at build time or vendor a copy under
//!   `crates/birdnet-behavioral/vendor/` — a path inside the source
//!   repository, shown in a web page.
//! * The **Add a microphone** form's only hint for "Device id" was "run
//!   `arecord -l` to list devices".
//!
//! None of these is something the owner of a garden microphone can do, and
//! each was the first and only thing the surface said.
//!
//! # What is guarded, and what is deliberately not
//!
//! A command may still appear — this product is self-hosted and the person who
//! set it up does read these screens. What it may not do is *lead*: the
//! sentence a reader meets first has to be one they can act on, with the
//! command after it and marked for whoever can run it.
//!
//! Checked only against **rendered markup**. The JSON siblings in
//! `routes::timeseries::helpers`, `routes::timeseries::stubs` and
//! `routes::analytics` keep their flags and are exempt by construction: an API
//! consumer is exactly the reader who can act on `--features analytics`.

use std::path::{Path, PathBuf};

/// Commands and build knobs that are useless to the station's owner.
const OPERATOR_ONLY: [&str; 8] = [
    "sudo systemctl",
    "journalctl",
    "arecord -l",
    "pw-cli",
    "--features analytics",
    "--analytics-db",
    "--refresh-extension",
    "BIRDNET_BUNDLED_EXTENSION_FILE",
];

/// Files this check does not read, each for a stated reason.
///
/// The first two render for a reader the page itself names as the operator: a
/// log-trace card under an "Operator" eyebrow is not pretending to be for
/// anyone else. The last three emit **JSON**, whose consumer is exactly the
/// reader who can act on `--features analytics`.
///
/// Named rather than inferred. An earlier draft tried to tell markup from JSON
/// by looking for `class=` on the same line, and a multi-line Rust string
/// whose `class="ctl-warn"` sat on the line above slipped straight through —
/// the check passed against the very text it was written to catch.
const EXEMPT: [&str; 5] = [
    "src/routes/pages/detection_detail.rs",
    "src/diagnostics.rs",
    "src/routes/timeseries/helpers.rs",
    "src/routes/timeseries/stubs.rs",
    "src/routes/analytics.rs",
];

fn rendered_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack = vec![root.join("src"), root.join("templates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs" | "html")
            ) {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if !EXEMPT.iter().any(|s| rel_str == *s) {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// Markup the browser shows, rather than a Rust or HTML comment about it.
fn is_rendered(line: &str) -> bool {
    let t = line.trim_start();
    !t.starts_with("//") && !t.starts_with("///") && !t.starts_with("<!--") && !t.starts_with('*')
}

/// How many lines above a command may carry its hand-off.
const HANDOFF_WINDOW: usize = 4;

/// The command is introduced rather than offered bare: the prose around it
/// hands the reader to whoever can run it.
///
/// The window is **joined before matching**, not searched line by line. Where
/// the markup breaks is arbitrary — `admin_audio_sources.html` wraps mid-phrase
/// between "whoever" and "set the station up" — and two earlier drafts of this
/// check, one scanning a single line and one scanning lines independently,
/// both reported the corrected copy as an offender. A check that depends on
/// where a line happens to wrap is not checking the text. A third draft still
/// missed it, because a Rust string continued across lines ends each one with
/// a backslash that landed mid-phrase.
fn is_handed_off(lines: &[&str], at: usize) -> bool {
    let from = at.saturating_sub(HANDOFF_WINDOW);
    // A Rust string continued across lines ends each one with a `\`, which
    // would otherwise land between two words and break the phrase in half.
    let window = lines[from..=at]
        .iter()
        .map(|l| l.trim_end().trim_end_matches('\\'))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let squashed = window.split_whitespace().collect::<Vec<_>>().join(" ");
    squashed.contains("whoever set the station up")
        || squashed.contains("if you have a terminal")
        || squashed.contains("for whoever set the station up")
}

#[test]
fn a_command_line_is_never_a_surfaces_first_answer() {
    let mut offenders = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in rendered_files() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !is_rendered(line) {
                continue;
            }
            for needle in OPERATOR_ONLY {
                if line.contains(needle) && !is_handed_off(&lines, i) {
                    offenders.push(format!(
                        "{}:{} [{needle}] {}",
                        path.strip_prefix(root).unwrap_or(&path).display(),
                        i + 1,
                        line.trim().chars().take(120).collect::<String>()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these rendered surfaces answer a reader with a command line and no \
         way in for someone who does not have a terminal. Lead with what they \
         can do and hand the command off:\n  {}",
        offenders.join("\n  ")
    );
}

/// The counterpart. Deleting every command would satisfy the check above and
/// strip the one reader who *can* fix the station of the instructions that fix
/// it. They stay; they just do not lead.
#[test]
fn the_commands_are_still_there_for_the_reader_who_can_run_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let service = std::fs::read_to_string(root.join("src/routes/admin/system_controls/service.rs"))
        .expect("service.rs");
    assert!(
        service.contains("sudo systemctl restart birdnet-behavior"),
        "the restart command must survive for whoever has a shell"
    );
    assert!(
        service.contains("whoever set"),
        "and be introduced by a sentence the station's owner can act on"
    );

    let audio = std::fs::read_to_string(root.join("templates/admin_audio_sources.html"))
        .expect("admin_audio_sources.html");
    assert!(
        audio.contains("arecord -l"),
        "the device-listing command must survive"
    );
    assert!(
        audio.contains("plughw:1,0"),
        "and the hint must first tell the reader the answer that usually works"
    );
}
