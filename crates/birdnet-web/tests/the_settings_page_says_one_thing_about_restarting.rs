//! The settings page told its reader about the restart in three places, in
//! three different ways, one of which was silence.
//!
//! # The defect
//!
//! Saving a setting produced all of these at once:
//!
//! * a permanent footer under the Save button: *"Most settings require a
//!   restart to take effect."*
//! * an inline banner after the save: *"Changes apply on next restart."* —
//!   unconditional, contradicting the "Most" above it;
//! * a toast, the most prominent of the three: *"Settings saved (N values
//!   updated)."* — no mention of the restart at all, with an action button
//!   reading "Open system →", which says where to go and not why.
//!
//! Nothing anywhere named which settings were in the unnamed "most", so the
//! hedge cost the reader the one thing it was hedging about: whether their
//! change had taken effect. And the only actionable element of the three was
//! the button whose label did not say what it was for.
//!
//! # What is guarded
//!
//! One sentence, everywhere it is said, and a way to act on it. The
//! counterpart holds the action's destination to a control that exists —
//! telling someone to restart is worse than useless if the link goes nowhere
//! they can do it.

use std::path::Path;

/// The one sentence. Held here so a change has to be made in the test as well
/// as in the three places that say it.
const NOTE: &str = "Settings are applied when the station next starts";

fn source(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel} must be readable: {e}"))
}

#[test]
fn every_place_that_mentions_the_restart_says_the_same_thing() {
    let render = source("src/routes/admin/settings/render/mod.rs");
    let handler = source("src/routes/admin/settings/handler.rs");

    // Both Save footers — the page has two renderers — plus the post-save
    // banner.
    assert_eq!(
        render.matches(NOTE).count(),
        2,
        "both Save-button footers must carry the sentence"
    );
    assert!(
        handler.contains(NOTE),
        "the post-save banner must carry the same sentence, not its own"
    );

    for stale in [
        "Most settings require a restart",
        "Changes apply on next restart.",
    ] {
        for (name, text) in [("render/mod.rs", &render), ("handler.rs", &handler)] {
            let in_prose = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| l.contains(stale));
            assert!(
                !in_prose,
                "{name} still says \"{stale}\" alongside the sentence the other \
                 places use"
            );
        }
    }
}

/// The counterpart. A note telling someone to restart, whose only action is a
/// link, is only as good as the link.
#[test]
fn the_restart_the_note_offers_is_somewhere_a_reader_can_restart() {
    let render = source("src/routes/admin/settings/render/mod.rs");
    let handler = source("src/routes/admin/settings/handler.rs");
    let system = source("src/routes/admin/system.rs");

    assert!(
        render.contains(r#"<a href="/admin/system">Restart</a>"#),
        "the footer's note must offer a link, not just state the fact"
    );
    assert!(
        handler.contains(r#""/admin/system", "Restart"#),
        "and the toast's action must say what it is for, not \"Open system\""
    );
    assert!(
        system.contains("Restart the service?"),
        "precondition: /admin/system must actually carry a restart control"
    );
}
