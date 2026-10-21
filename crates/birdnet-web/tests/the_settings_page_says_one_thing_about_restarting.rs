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
//! # What the page says now
//!
//! The sentence "Settings are applied when the station next starts" was itself
//! the defect's last form: it was false of the species lists and the
//! spectrogram options, which are in force the moment they are saved. The page
//! now says *which* settings wait. Every field carries an "Applies immediately"
//! or "Applies after restart" badge, a save names how many of the saved values
//! wait and lists them, a change still waiting is shown on the next visit, and
//! the restart is a button on the banner rather than a link to somewhere else.
//! The two Save footers say one sentence about all of it.
//!
//! # What is guarded
//!
//! One sentence, everywhere it is said, and a way to act on it. The old
//! unconditional sentence must stay gone (it would now contradict the badges).
//! The counterpart holds each way of acting to a control that exists —
//! telling someone to restart is worse than useless if the button or link goes
//! nowhere they can do it.

use std::path::Path;

/// The one sentence, as both Save footers carry it. Held here so a change has
/// to be made in the test as well as in the two places that say it.
const NOTE: &str = "Each setting says when it applies. The ones marked <em>after restart</em> wait until you restart the station.";

/// What the footers used to say of every setting, which is no longer true of
/// all of them.
const UNCONDITIONAL: &str = "Settings are applied when the station next starts";

fn source(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel} must be readable: {e}"))
}

/// `text` with every run of whitespace made one space, so the footers may wrap
/// the sentence across lines as the second renderer does.
fn flattened(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn every_place_that_mentions_the_restart_says_the_same_thing() {
    let render = flattened(&source("src/routes/admin/settings/render/mod.rs"));
    let handler = source("src/routes/admin/settings/handler.rs");

    // Both Save footers — the page has two renderers.
    assert_eq!(
        render.matches(NOTE).count(),
        2,
        "both Save-button footers must carry the sentence"
    );
    // The post-save banner does not repeat the footer; it reports what this
    // save did, in the same terms the footer and the badges use: the ones that
    // "need a restart" and the ones "in force already".
    assert!(
        handler.contains("need a restart") && handler.contains("in force already"),
        "the post-save banner must say which saved settings wait for a restart \
         and which are in force, in the footer's terms"
    );

    // Prose only: the handler's own comment quotes the old sentence to say why
    // it went, and that is not the page saying it.
    let render_raw = source("src/routes/admin/settings/render/mod.rs");
    for stale in [
        UNCONDITIONAL,
        "Most settings require a restart",
        "Changes apply on next restart.",
    ] {
        for (name, text) in [("render/mod.rs", &render_raw), ("handler.rs", &handler)] {
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
    let handler = source("src/routes/admin/settings/handler.rs");
    let badges = source("src/routes/admin/settings/render/badges.rs");
    let script = source("static/setup-controls.js");
    let system = source("src/routes/admin/system.rs");
    let controls = source("src/routes/admin/system_controls/mod.rs");

    // The banner after a save and the one on a later visit both carry a button
    // marked for the script, and the script handles that mark.
    for (name, text) in [("handler.rs", &handler), ("badges.rs", &badges)] {
        assert!(
            text.contains("data-restart-now>Restart the station now</button>"),
            "{name}: the banner must carry a restart button, not just state the fact"
        );
    }
    assert!(
        script.contains("[data-restart-now]"),
        "precondition: setup-controls.js must handle the button the banners carry"
    );
    // The toast's action says what it is for, not "Open system", and leads to a
    // path the router answers.
    assert!(
        handler.contains(r#""/admin/system/restart", "Restart"#),
        "and the toast's action must say what it is for, not \"Open system\""
    );
    assert!(
        controls.contains(r#""/admin/system/restart""#),
        "precondition: the toast's destination must be a route"
    );
    assert!(
        system.contains("Restart the service?"),
        "precondition: /admin/system must actually carry a restart control"
    );
}
