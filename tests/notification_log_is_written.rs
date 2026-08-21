//! `notification_log` must have a writer that runs on a real station.
//!
//! # What this is defending
//!
//! Through 0.14.0 the table had none. `birdnet_db::notifications::log_notification`
//! was implemented, documented and unit-tested, and the only caller anywhere in
//! the tree was `crates/birdnet-web/examples/screenshot_server.rs` — the fixture
//! that seeds the documentation screenshots. Meanwhile three surfaces read it:
//!
//!   * `routes/pages/notification_center.rs` — an entire page
//!   * `routes/admin/notifications.rs`
//!   * `routes/pages/homes/station_tabs.rs` — the Station home's recent tab
//!
//! So every real station showed three empty screens, permanently, while
//! `docs/book/images/notifications.png` showed them full. Nothing failed. No
//! test noticed, because every test that touched the table wrote to it itself.
//!
//! That is the shape of the bug rather than the bug: a mechanism built
//! end-to-end and never connected to the thing that was supposed to drive it.
//! A behavioural test cannot catch it — it would seed the row too. What catches
//! it is asking whether production code *calls* the writer at all, which is what
//! this does.
//!
//! # Why it reads source rather than running anything
//!
//! The alternative is standing up the whole detection pipeline — an ONNX model,
//! an audio file, a tokio runtime, four notification clients — to observe one
//! INSERT. This gate is cheap enough to keep, and it fails for exactly the
//! reason the defect existed.

use std::path::{Path, PathBuf};

/// Production Rust: the binary and the library crates. Deliberately **not**
/// `examples/` (where the only 0.14.0 caller lived), `tests/`, or `benches/`.
fn production_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    collect(&root.join("src"), &mut out);
    for crate_dir in std::fs::read_dir(root.join("crates"))
        .expect("crates/ is readable")
        .flatten()
    {
        collect(&crate_dir.path().join("src"), &mut out);
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Everything in a file that is not inside a `#[cfg(test)]` module.
///
/// Without this the gate passes on `notifications.rs`'s own unit tests, which is
/// how the table came to have six callers and no writer.
fn non_test_source(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = String::new();
    let mut in_test = false;
    let mut depth: i32 = 0;
    let mut opened = false;
    for line in text.lines() {
        if !in_test && line.trim_start().starts_with("#[cfg(test)]") {
            in_test = true;
            depth = 0;
            opened = false;
            continue;
        }
        if in_test {
            depth += i32::try_from(line.matches('{').count()).unwrap_or(0);
            depth -= i32::try_from(line.matches('}').count()).unwrap_or(0);
            if line.contains('{') {
                opened = true;
            }
            if opened && depth <= 0 {
                in_test = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn something_in_production_writes_the_notification_log() {
    let has_caller = production_sources()
        .into_iter()
        .filter(|p| {
            // The definition itself does not count as a caller.
            !p.ends_with("notifications.rs") || !p.to_string_lossy().contains("birdnet-db")
        })
        .any(|p| non_test_source(&p).contains("log_notification("));

    assert!(
        has_caller,
        "nothing in production writes `notification_log`, but the Notification \
         Center, /admin/notifications and the Station home all read it — they \
         will be empty on every real station"
    );
}

/// The counterpart, so the gate above is a discrimination rather than a spelling
/// check: the surfaces it is defending must actually still read the table. If
/// they are ever removed, this gate should be removed with them rather than
/// quietly guarding nothing.
#[test]
fn the_surfaces_this_defends_still_read_the_notification_log() {
    let readers = [
        "crates/birdnet-web/src/routes/pages/notification_center.rs",
        "crates/birdnet-web/src/routes/admin/notifications.rs",
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for reader in readers {
        let text = std::fs::read_to_string(root.join(reader))
            .unwrap_or_else(|e| panic!("{reader} is readable: {e}"));
        assert!(
            text.contains("recent_notifications")
                || text.contains("notifications_by_channel")
                || text.contains("notification_stats"),
            "{reader} no longer reads notification_log — if the feature is gone, \
             delete this gate with it"
        );
    }
}

/// Every channel the processor dispatches on must record what happened to it.
///
/// Four channels fan out per detection (apprise, birdweather, email, mqtt) and
/// each was silent. Naming them here means adding a fifth without a log row
/// fails rather than shipping another permanently-blank column in the
/// Notification Center's per-channel breakdown.
#[test]
fn every_dispatched_channel_records_an_outcome() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = non_test_source(&root.join("src/daemon/processor.rs"));
    for channel in ["apprise", "birdweather", "email", "mqtt"] {
        assert!(
            src.contains(&format!("\"{channel}\",")),
            "the {channel} dispatch does not record a notification_log outcome"
        );
    }
}
