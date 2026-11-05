//! `audio::input_level` reads what `amixer` and `pactl` print, and grades it.
//!
//! The fixtures in `tests/data/input_level/` are either captured from real
//! PulseAudio and PipeWire servers, or published by other people; none is from a
//! sound card (see the README there). Where the logic needs a case no captured
//! sample has (a hardware-controlled source), the test builds it from a captured
//! sample by changing the one thing that matters, and says so.

use birdnet_core::audio::input_level::{
    Advice, Backend, CAP_HARDWARE_DB, CAP_PERCENT, InputLevel, ReadError, assess,
    command_to_reach_cap, default_source, parse_amixer_scontents, parse_pactl_source,
};

const PA16: &str = include_str!("data/input_level/pactl_list_sources_pulseaudio_16.1.json");
const PIPEWIRE: &str =
    include_str!("data/input_level/pactl_list_sources_pipewire_pulse_1.0.5.json");
const PA16_INFO: &str = include_str!("data/input_level/pactl_info_pulseaudio_16.1.txt");
const EMULATED: &str = include_str!("data/input_level/amixer_scontents_emulated_pulse_plugin.txt");
const USB: &str = include_str!("data/input_level/amixer_scontents_published_usb_codec.txt");
const HDA: &str = include_str!("data/input_level/amixer_scontents_published_intel_hda.txt");

fn near(a: Option<f32>, b: f32) -> bool {
    a.is_some_and(|a| (a - b).abs() < 0.005)
}

// ----- PulseAudio and PipeWire (pactl --format=json) -----------------------------------------

/// The captured `virt_mic` has a left channel at 40% and a right at 60%. The level is
/// the loudest channel's, with the dB beside that channel.
#[test]
fn a_pulse_source_reads_as_its_loudest_channel() {
    let level = parse_pactl_source(PA16, "virt_mic").expect("virt_mic is in the listing");
    assert_eq!(level.backend, Backend::Pulse);
    assert_eq!(level.percent, 60);
    assert!(near(level.db, -13.31), "{:?}", level.db);
    assert!(near(level.base_db, 0.0));
    assert!(
        near(level.hardware_db, -13.31),
        "no offset from a base of 0 dB"
    );
    assert_eq!(level.muted, Some(false));
    assert!(
        !level.hardware_controlled,
        "a null source has no hardware volume"
    );
    assert_eq!(assess(&level), Advice::Ok);
}

/// A source with no decibel volume prints an empty `db`: the level is the percentage alone.
#[test]
fn a_source_without_decibel_volume_has_no_db() {
    let level = parse_pactl_source(PA16, "vsrc").expect("vsrc is in the listing");
    assert_eq!(level.percent, 60);
    assert_eq!(level.db, None);
    assert_eq!(level.hardware_db, None);
    assert_eq!(assess(&level), Advice::Ok);
}

/// A software volume at 100% is unity gain, not the hardware's maximum.
#[test]
fn a_software_source_at_100_percent_is_not_at_a_maximum() {
    let level = parse_pactl_source(PA16, "virt_stereo").expect("listed");
    assert_eq!(level.percent, 100);
    assert_eq!(assess(&level), Advice::Ok);
}

/// The captured `pipewire-pulse` source is at 150% and muted: muted wins, and without the mute
/// 150% of a software volume is a digital boost.
#[test]
fn a_muted_source_is_muted_and_an_unmuted_boost_is_above_the_cap() {
    let level = parse_pactl_source(PIPEWIRE, "virt_mic").expect("listed");
    assert_eq!(level.percent, 150);
    assert!(near(level.db, 10.57));
    assert_eq!(level.muted, Some(true));
    assert_eq!(assess(&level), Advice::Muted);

    let unmuted = PIPEWIRE.replacen("\"mute\":true", "\"mute\":false", 1);
    assert_ne!(unmuted, PIPEWIRE, "the fixture has a mute to flip");
    let level = parse_pactl_source(&unmuted, "virt_mic").expect("listed");
    assert_eq!(level.muted, Some(false));
    assert_eq!(
        assess(&level),
        Advice::AboveCap,
        "a software gain above 100% amplifies the signal"
    );
}

/// The case no captured sample has: a source with a hardware volume control, built from the captured
/// JSON by adding the `HW_VOLUME_CTRL` flag and a base volume of -12 dB (the shape of a USB
/// microphone behind PulseAudio, where 100% is the hardware's maximum).
fn hardware_source(percent: u32, db: &str) -> String {
    use serde_json::{Value, json};
    let mut sources: Value = serde_json::from_str(PA16).expect("the captured listing is JSON");
    let virt = sources
        .as_array_mut()
        .and_then(|all| all.iter_mut().find(|s| s["name"] == "virt_mic"))
        .expect("virt_mic is in the captured listing");
    virt["flags"] = json!(["HW_VOLUME_CTRL", "DECIBEL_VOLUME", "LATENCY"]);
    virt["base_volume"] = json!({"value": 26214, "value_percent": "40%", "db": "-12.00 dB"});
    for channel in ["front-left", "front-right"] {
        virt["volume"][channel] =
            json!({"value": 1, "value_percent": format!("{percent}%"), "db": db});
    }
    sources.to_string()
}

#[test]
fn a_hardware_source_is_graded_against_the_cap_and_the_maximum() {
    // Precondition: the construction changed what it was meant to.
    let built = hardware_source(100, "0.00 dB");
    let level = parse_pactl_source(&built, "virt_mic").expect("listed");
    assert!(level.hardware_controlled, "the flag was added");
    assert!(near(level.base_db, -12.0));
    assert_eq!(level.percent, 100);
    assert!(
        near(level.hardware_db, 12.0),
        "0 dB reported over a base of -12 dB is +12 dB of hardware gain"
    );

    assert_eq!(
        assess(&level),
        Advice::AtMaximum,
        "100% of a hardware range"
    );
    let at = |p, db| {
        let l = parse_pactl_source(&hardware_source(p, db), "virt_mic").expect("listed");
        assess(&l)
    };
    assert_eq!(
        at(80, "-5.85 dB"),
        Advice::AboveCap,
        "80% is over {CAP_PERCENT}%"
    );
    assert_eq!(
        at(75, "-7.50 dB"),
        Advice::Ok,
        "75% is at the cap, not over it"
    );
    assert_eq!(at(60, "-13.31 dB"), Advice::Ok);
}

/// The dB cap binds before the percentage when the hardware range is large.
#[test]
fn the_db_cap_can_bind_before_the_percentage() {
    let mut level =
        parse_pactl_source(&hardware_source(70, "-9.30 dB"), "virt_mic").expect("listed");
    level.base_db = Some(-30.0);
    level.hardware_db = Some(-9.30 - -30.0);
    assert!(
        level.hardware_db.is_some_and(|d| d > CAP_HARDWARE_DB),
        "+20.7 dB of hardware gain"
    );
    assert_eq!(
        level.percent, 70,
        "under the {CAP_PERCENT}% cap, so only the dB rule can flag it"
    );
    assert_eq!(assess(&level), Advice::AboveCap);
    assert_eq!(
        command_to_reach_cap(&level, "virt_mic").as_deref(),
        Some("pactl set-source-volume virt_mic 63%"),
        "10^((18 - 30) / 60) is 63%"
    );
}

#[test]
fn the_command_to_reach_the_cap_is_75_percent_when_the_range_is_small() {
    let level = parse_pactl_source(&hardware_source(100, "0.00 dB"), "virt_mic").expect("listed");
    assert_eq!(
        command_to_reach_cap(&level, "virt_mic").as_deref(),
        Some("pactl set-source-volume virt_mic 75%")
    );
    // Nothing to write for a software volume, or for a source already at the cap.
    let software = parse_pactl_source(PA16, "virt_stereo").expect("listed");
    assert_eq!(command_to_reach_cap(&software, "virt_stereo"), None);
    let low = parse_pactl_source(&hardware_source(60, "-13.31 dB"), "virt_mic").expect("listed");
    assert_eq!(command_to_reach_cap(&low, "virt_mic"), None);
}

#[test]
fn unreadable_listings_say_why() {
    assert_eq!(
        parse_pactl_source("not json", "virt_mic"),
        Err(ReadError::NotJson)
    );
    assert_eq!(
        parse_pactl_source("{}", "virt_mic"),
        Err(ReadError::NotJson),
        "an object is not a listing"
    );
    assert_eq!(
        parse_pactl_source(PA16, "nope"),
        Err(ReadError::NoSuchSource("nope".into()))
    );
    assert_eq!(
        parse_pactl_source(r#"[{"name":"x","mute":false}]"#, "x"),
        Err(ReadError::NoVolume("x".into()))
    );
}

#[test]
fn the_default_source_is_read_from_pactl_info() {
    assert_eq!(default_source(PA16_INFO).as_deref(), Some("virt_mic"));
    assert_eq!(default_source("Default Source: \n"), None);
    assert_eq!(default_source("Server Name: x\n"), None);
}

// ----- ALSA (amixer scontents) ---------------------------------------------------------------

fn only(text: &str) -> InputLevel {
    let mut levels = parse_amixer_scontents(text);
    assert_eq!(levels.len(), 1, "{levels:?}");
    levels.remove(0)
}

/// The published USB codec: `Mic` has playback and capture on one line, `Speaker` and `Auto Gain
/// Control` are playback-only and are not capture-volume controls.
#[test]
fn the_published_usb_codec_has_one_capture_control_at_half_range() {
    let level = only(USB);
    assert_eq!(level.control, "Mic");
    assert_eq!(level.backend, Backend::Alsa);
    assert_eq!(level.percent, 50);
    assert!(near(level.db, 11.9));
    assert_eq!(level.raw, Some((8, 0, 16)));
    assert_eq!(level.muted, Some(false));
    assert_eq!(
        assess(&level),
        Advice::Ok,
        "50% and +11.9 dB are under the cap"
    );
}

#[test]
fn at_the_top_of_the_range_a_control_is_at_its_maximum_and_the_command_names_a_raw_step() {
    let top = USB.replace("Capture 8 [50%] [11.90dB]", "Capture 16 [100%] [23.81dB]");
    assert_ne!(top, USB);
    let level = only(&top);
    assert_eq!((level.percent, level.raw), (100, Some((16, 0, 16))));
    assert_eq!(assess(&level), Advice::AtMaximum);
    assert_eq!(
        command_to_reach_cap(&level, "1").as_deref(),
        Some("amixer -c 1 -- sset 'Mic' capture 12"),
        "75% of 0..16 is step 12, about +17.9 dB on this codec"
    );
    let high = USB.replace("Capture 8 [50%] [11.90dB]", "Capture 14 [88%] [20.83dB]");
    assert_eq!(assess(&only(&high)), Advice::AboveCap);
}

/// The cap is "over 75%" and "over +18 dB", on either rule alone.
#[test]
fn an_alsa_control_is_over_the_cap_by_percentage_or_by_db() {
    let at = |percent: u32, db: &str| {
        let text = USB.replace(
            "Capture 8 [50%] [11.90dB]",
            &format!("Capture 12 [{percent}%] [{db}dB]"),
        );
        assess(&only(&text))
    };
    assert_eq!(at(75, "17.86"), Advice::Ok, "exactly 75% and under 18 dB");
    assert_eq!(
        at(76, "17.86"),
        Advice::AboveCap,
        "one point over the percentage cap"
    );
    assert_eq!(
        at(60, "18.00"),
        Advice::Ok,
        "exactly +18 dB is the cap, not over it"
    );
    assert_eq!(
        at(60, "18.10"),
        Advice::AboveCap,
        "under the percentage but over the dB cap"
    );
}

/// A control counts as a capture volume by its capabilities (`cvolume`), not by a line that happens to
/// say `Capture`. Built from the published HDA listing: a playback-only control given a capture line.
#[test]
fn a_control_without_the_capture_volume_capability_is_left_out() {
    let text = "Simple mixer control 'Line',0\n  Capabilities: pvolume pswitch\n  Limits: Capture 0 - 31\n  Front Left: Capture 5 [16%] [-8.00dB] [on]\n";
    assert!(parse_amixer_scontents(text).is_empty());
    let with = text.replace("pvolume pswitch", "cvolume cswitch");
    assert_eq!(
        only(&with).control,
        "Line",
        "the same lines count once the capability is there"
    );
}

/// A stereo control with one channel off still carries sound: it is muted only when every channel is.
#[test]
fn a_control_is_muted_only_when_every_channel_is_off() {
    let text = "Simple mixer control 'Capture',0\n  Capabilities: cvolume cswitch\n  Limits: Capture 0 - 31\n  Front Left: Capture 5 [16%] [-8.00dB] [on]\n  Front Right: Capture 5 [16%] [-8.00dB] [off]\n";
    assert_eq!(only(text).muted, Some(false));
    assert_eq!(only(&text.replace("[on]", "[off]")).muted, Some(true));
}

#[test]
fn a_switched_off_capture_is_muted_and_beats_the_level() {
    let off = USB.replace(
        "Capture 8 [50%] [11.90dB] [on]",
        "Capture 8 [50%] [11.90dB] [off]",
    );
    assert_ne!(off, USB);
    let level = only(&off);
    assert_eq!(level.muted, Some(true));
    assert_eq!(assess(&level), Advice::Muted);
}

/// The published Intel HDA codec: `Mic Boost` is an enumerated control, not a capture volume, and is
/// left out; `Capture` is at its minimum.
#[test]
fn an_enumerated_boost_is_not_a_capture_volume() {
    let level = only(HDA);
    assert_eq!(level.control, "Capture");
    assert_eq!(level.percent, 0);
    assert!(near(level.db, -16.5));
    assert_eq!(level.raw, Some((0, 0, 31)));
    assert_eq!(assess(&level), Advice::Ok);
}

/// Real `amixer` formatting over an emulated mixer: no dB at all, a range of 65536.
#[test]
fn an_emulated_mixer_has_no_db_and_a_huge_range() {
    let level = only(EMULATED);
    assert_eq!(level.control, "Capture");
    assert_eq!((level.percent, level.db), (100, None));
    assert_eq!(level.raw, Some((65536, 0, 65536)));
}

#[test]
fn alsas_minus_infinity_is_no_db() {
    let text = "Simple mixer control 'Capture',0\n  Capabilities: cvolume cswitch\n  Limits: Capture 0 - 31\n  Front Left: Capture 0 [0%] [-99999.99dB] [on]\n";
    let level = only(text);
    assert_eq!(level.db, None);
    assert_eq!(level.percent, 0);
}

/// Several capture controls (`Mic` and `Capture` both) are all returned: the signal passes
/// through each.
#[test]
fn every_capture_control_is_read() {
    let both = format!("{USB}{HDA}");
    let names: Vec<_> = parse_amixer_scontents(&both)
        .into_iter()
        .map(|l| l.control)
        .collect();
    assert_eq!(names, ["Mic", "Capture"]);
}

#[test]
fn a_control_name_with_a_quote_gets_no_command() {
    let text = USB
        .replace("'Mic',0", "'Mic's',0")
        .replace("Capture 8 [50%] [11.90dB]", "Capture 16 [100%] [23.81dB]");
    let level = only(&text);
    assert_eq!(level.control, "Mic's");
    assert_eq!(
        command_to_reach_cap(&level, "1"),
        None,
        "a quote cannot be written safely into a shell command"
    );
    assert_eq!(
        command_to_reach_cap(
            &only(&USB.replace("Capture 8 [50%] [11.90dB]", "Capture 16 [100%] [23.81dB]")),
            "1;rm"
        ),
        None,
        "nor can a card name"
    );
}

#[test]
fn text_that_is_not_a_listing_has_no_controls() {
    assert!(parse_amixer_scontents("").is_empty());
    assert!(
        parse_amixer_scontents("amixer: Mixer attach hw:CARD=nosuch error: No such device\n")
            .is_empty()
    );
}
