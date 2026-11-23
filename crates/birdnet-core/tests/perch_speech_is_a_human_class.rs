//! MMP-05: the privacy filter must recognise Perch v2's speech classes.
//!
//! The human-sound rule reads a classifier's human classes from its labels
//! through [`names_a_human`]. Perch v2's vocabulary carries `AudioSet`'s sound
//! classes, whose speech rows (`Speech`, `Conversation`, …) do not contain the
//! word "human", so a source routed to Perch alone had a privacy filter that
//! could only fire on two of them. These tests hold the matcher to the label
//! files compiled into the binary.

use birdnet_core::detection::privacy::names_a_human;
use birdnet_core::inference::labels::{BIRDNET_V24_ENGLISH_LABELS, LabelSet, PERCH_V2_LABELS};

/// The speech set the audit named (`docs/audit-2026-11/02-multi-model-pipeline.md`,
/// MMP-05): each is a conversation-bearing class in Perch's labels.
const SPEECH: &[&str] = &[
    "Speech",
    "Conversation",
    "Male_speech_and_man_speaking",
    "Female_speech_and_woman_speaking",
    "Child_speech_and_kid_speaking",
    "Whispering",
    "Shout",
    "Yell",
    "Laughter",
    "Singing",
    "Homo sapiens",
];

fn perch() -> LabelSet {
    LabelSet::load_from_str(PERCH_V2_LABELS).expect("the Perch labels parse")
}

fn humans(set: &LabelSet) -> Vec<String> {
    let mut out: Vec<String> = set
        .iter()
        .filter(|l| names_a_human(&l.scientific_name, &l.common_name))
        .map(|l| l.scientific_name.clone())
        .collect();
    out.sort();
    out
}

#[test]
fn every_perch_speech_class_is_a_human_class() {
    let set = perch();
    let missed: Vec<&str> = SPEECH
        .iter()
        .copied()
        .filter(|name| {
            let label = set
                .find_by_scientific_name(name)
                .unwrap_or_else(|| panic!("`{name}` is a row of perch-v2.csv"));
            !names_a_human(&label.scientific_name, &label.common_name)
        })
        .collect();
    assert!(missed.is_empty(), "not read as human: {missed:?}");
}

/// The counterpart: the rule is a list, not "everything Perch is unsure of".
/// Exactly the `AudioSet` "Human sounds" classes and *Homo sapiens* are human;
/// no bird, frog or other sound class is.
#[test]
fn perch_s_human_classes_are_exactly_audioset_s_human_sounds_and_homo_sapiens() {
    // Every Perch class under "Human sounds" in the AudioSet ontology
    // (github.com/audioset/ontology, ontology.json, read 2026-11-23), and *Homo sapiens*.
    let expected = [
        "Applause",
        "Breathing",
        "Burping_and_eructation",
        "Chatter",
        "Cheering",
        "Chewing_and_mastication",
        "Child_speech_and_kid_speaking",
        "Chuckle_and_chortle",
        "Clapping",
        "Conversation",
        "Cough",
        "Crowd",
        "Crying_and_sobbing",
        "Fart",
        "Female_singing",
        "Female_speech_and_woman_speaking",
        "Finger_snapping",
        "Gasp",
        "Giggle",
        "Hands",
        "Homo sapiens",
        "Human_group_actions",
        "Human_voice",
        "Laughter",
        "Male_singing",
        "Male_speech_and_man_speaking",
        "Respiratory_sounds",
        "Run",
        "Screaming",
        "Shout",
        "Sigh",
        "Singing",
        "Sneeze",
        "Speech",
        "Speech_synthesizer",
        "Walk_and_footsteps",
        "Whispering",
        "Yell",
    ];
    assert_eq!(humans(&perch()), expected);
}

/// The other counterpart: V2.4's human classes are its three `Human …` rows
/// and nothing else, as before.
#[test]
fn v24_s_human_classes_are_unchanged() {
    let set = LabelSet::load_from_str(BIRDNET_V24_ENGLISH_LABELS).expect("V2.4 labels parse");
    assert_eq!(
        humans(&set),
        ["Human non-vocal", "Human vocal", "Human whistle"]
    );
}
