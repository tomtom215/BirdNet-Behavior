//! A hint that sends the reader somewhere must send them to a control that
//! exists.
//!
//! # The defect
//!
//! `/admin/settings` carried, under a label reading "Audio Channels":
//!
//! > Set per source on Audio & Microphones — each microphone or stream carries
//! > its own channel count, sample rate and gain.
//!
//! None of that was reachable. The add-a-source form offers `device_id`,
//! `kind`, `label`, `rtsp_transport`, `sample_rate` and `scope`; the edit form
//! offers `agc`, `dc_removal`, `device_id`, `eq_chain`, `high_pass`, `kind`,
//! `label`, `pipeline_present`, `quiet_start`, `quiet_end` and
//! `rtsp_keepalive`. There is no channel-layout control and no gain control on
//! either, and `sample_rate` is on the add form only, so it cannot be revisited
//! after a source exists. A grep for a writer of a non-`Mono` `Channels` or a
//! non-zero `gain_db` anywhere outside tests finds none: every source in the
//! product is mono at unity gain.
//!
//! That is not a cosmetic inaccuracy. `admin/audio.rs` warns, in its own
//! summary line, that a stereo pair "averaged to mono … on a spaced pair can
//! cancel signal — pick Left or Right unless the capsules are together" — and
//! the reader it tells to pick has nothing to pick with.
//!
//! # What is guarded
//!
//! The link between the two, in the direction that matters: the prose may
//! promise a per-source gain or channel control **only** when a form actually
//! has one. Adding the control unblocks the wording again, which is the right
//! way round — this is a check on honesty, not a ban on a feature.

use std::path::{Path, PathBuf};

/// Files that can carry an audio-source form control.
fn form_bearing_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        root.join("templates/admin_audio_sources.html"),
        root.join("src/routes/admin/audio.rs"),
    ]
}

/// Does any audio-source form expose an input with this `name`?
///
/// Both the Rust-embedded markup (`name=\"x\"` inside an escaped string
/// literal) and the plain template (`name="x"`) are covered.
fn a_form_field_exists(name: &str) -> bool {
    let plain = format!("name=\"{name}\"");
    let escaped = format!("name=\\\"{name}\\\"");
    form_bearing_files().iter().any(|path| {
        std::fs::read_to_string(path)
            .is_ok_and(|text| text.contains(&plain) || text.contains(&escaped))
    })
}

/// The section's own source, so the assertions below quote what ships.
fn audio_section() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/admin/settings/render/audio.rs"),
    )
    .expect("the audio settings section must be readable")
}

/// The hint that follows the "Audio Channels" label.
///
/// Scoped deliberately. A first draft scanned the whole file for the word
/// "gain" and went red against the *corrected* copy, because the clip-loudness
/// hint two controls away says normalisation "applies one gain" — a true
/// sentence about a different thing. A check that cannot tell a promise from a
/// description is not a check.
fn audio_channels_hint(section: &str) -> &str {
    let label = "<label>Audio Channels</label>";
    let start = section
        .find(label)
        .map(|i| i + label.len())
        .expect("the Audio Channels label must still be there");
    let rest = &section[start..];
    let end = rest.find("</p>").expect("its hint must close");
    &rest[..end]
}

#[test]
fn the_audio_hint_does_not_offer_a_control_the_forms_lack() {
    let section = audio_section();
    let hint = audio_channels_hint(&section);

    for (word, field) in [("gain", "gain_db"), ("channel count", "channels")] {
        if a_form_field_exists(field) {
            continue;
        }
        assert!(
            !hint.contains(word),
            "the Audio Channels hint tells the reader about \"{word}\", but no \
             audio-source form has a `{field}` input, so there is nowhere to set \
             it. Either add the control or stop promising it. Hint reads: {hint}"
        );
    }
}

/// The counterpart, in the other direction: a control the forms *do* have must
/// stay findable from the settings page, or the pointer is useless in the way
/// that is harder to notice.
#[test]
fn the_audio_hint_still_points_at_the_page_that_holds_the_controls() {
    let section = audio_section();
    let hint = audio_channels_hint(&section);

    assert!(
        a_form_field_exists("sample_rate"),
        "precondition: the add-a-source form still carries a sample rate"
    );
    assert!(
        hint.contains("/admin/audio"),
        "the Audio settings section must still link to the page that carries \
         the per-source controls"
    );
}
