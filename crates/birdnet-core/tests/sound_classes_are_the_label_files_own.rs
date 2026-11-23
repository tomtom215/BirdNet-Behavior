//! MMP-04: the non-taxon list is held to the label files compiled into the
//! binary, so a label file that changes cannot leave a sound class stored as a
//! species, or a species dropped as a sound.

use birdnet_core::detection::sound_class::{SOUND_CLASSES, is_sound_class};
use birdnet_core::inference::labels::{BIRDNET_V24_ENGLISH_LABELS, LabelSet, PERCH_V2_LABELS};

fn sounds(set: &LabelSet) -> Vec<String> {
    let mut out: Vec<String> = set
        .iter()
        .filter(|l| is_sound_class(&l.scientific_name, &l.common_name))
        .map(|l| l.scientific_name.clone())
        .collect();
    out.sort();
    out
}

#[test]
fn every_perch_row_without_a_binomial_and_homo_sapiens_is_a_sound_and_no_species_is() {
    let set = LabelSet::load_from_str(PERCH_V2_LABELS).expect("Perch labels parse");
    let mut expected: Vec<String> = set
        .iter()
        .filter(|l| !l.scientific_name.contains(' ') || l.scientific_name == "Homo sapiens")
        .map(|l| l.scientific_name.clone())
        .collect();
    expected.sort();
    assert_eq!(expected.len(), 199, "198 sound classes and Homo sapiens");
    assert_eq!(sounds(&set), expected);
}

#[test]
fn v24_s_sounds_are_its_eight_named_classes_and_three_human_ones() {
    let set = LabelSet::load_from_str(BIRDNET_V24_ENGLISH_LABELS).expect("V2.4 labels parse");
    assert_eq!(
        sounds(&set),
        [
            "Dog",
            "Engine",
            "Environmental",
            "Fireworks",
            "Gun",
            "Human non-vocal",
            "Human vocal",
            "Human whistle",
            "Noise",
            "Power tools",
            "Siren",
        ]
    );
}

/// Every entry names a row of one of the two files: a misspelt entry would
/// match nothing and look like coverage.
#[test]
fn every_listed_class_is_a_row_of_a_bundled_label_file() {
    let perch = LabelSet::load_from_str(PERCH_V2_LABELS).expect("Perch labels parse");
    let v24 = LabelSet::load_from_str(BIRDNET_V24_ENGLISH_LABELS).expect("V2.4 labels parse");
    let missing: Vec<&str> = SOUND_CLASSES
        .iter()
        .copied()
        .filter(|c| {
            perch.find_by_scientific_name(c).is_none() && v24.find_by_scientific_name(c).is_none()
        })
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
}
