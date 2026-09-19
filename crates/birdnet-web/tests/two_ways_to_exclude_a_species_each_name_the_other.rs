//! The two "exclude this bird" lists must each say what the other one does.
//!
//! # The defect
//!
//! The settings page carries two controls that both stop a species reaching
//! the reader, in different sections, with very different consequences:
//!
//! * **Species Filters → excluded species** stops the bird being *recorded*.
//!   No clip, no row in the reader's records, nothing to go back to.
//! * **Notifications → never notify for these** stops the *alert*. The bird is
//!   still heard, still saved, still in the life list.
//!
//! Each hint was accurate on its own — "These species will never be saved or
//! notified" and "Species excluded from all notifications (dual-filter with
//! notify-only list above)" — and neither mentioned that the other existed.
//! Someone who wants "stop pinging me about House Sparrows" and someone who
//! wants "stop counting House Sparrows" are one click apart, and the one who
//! picks wrong loses records permanently without being told they had a choice.
//!
//! # What is guarded
//!
//! Each hint names the consequence that distinguishes it, and points at its
//! neighbour. Both halves matter: a hint that described itself perfectly and
//! said nothing about the other control is exactly what shipped.

use std::path::Path;

fn section(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel} must be readable: {e}"))
}

#[test]
fn the_recording_filter_says_the_records_are_lost_and_points_at_the_alert_one() {
    let s = section("src/routes/admin/settings/render/species.rs");
    assert!(
        s.contains("not recorded at all"),
        "the recording filter must say that nothing is kept — that is the \
         consequence the reader cannot undo"
    );
    assert!(
        s.contains("Never notify for these species"),
        "and it must name the other control, so the reader who only wanted the \
         alerts stopped can find it"
    );
}

#[test]
fn the_alert_filter_says_the_records_are_kept_and_points_at_the_recording_one() {
    let s = section("src/routes/admin/settings/render/notifications.rs");
    assert!(
        s.contains("still recorded and still appear in your"),
        "the alert filter must say the sightings survive"
    );
    assert!(
        s.contains("Never record these"),
        "and it must name the other control"
    );
}

/// The counterpart. The labels the hints point at have to be the labels that
/// are actually on screen, or the cross-reference sends the reader looking for
/// a control that does not exist under that name.
#[test]
fn each_hint_points_at_a_label_that_is_really_there() {
    let species = section("src/routes/admin/settings/render/species.rs");
    let notifications = section("src/routes/admin/settings/render/notifications.rs");

    assert!(
        species.contains(">Never record these"),
        "Species Filters must carry the label Notifications sends readers to"
    );
    // Prefix, not an exact element: both labels carry a parenthetical about
    // comma separation, and an exact match reported the correct label as
    // missing.
    assert!(
        notifications.contains(">Never notify for these species"),
        "Notifications must carry the label Species Filters sends readers to"
    );
}
