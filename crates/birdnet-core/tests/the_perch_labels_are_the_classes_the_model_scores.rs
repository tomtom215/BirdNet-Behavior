//! The Perch v2 label file this project installs names the classes the model
//! scores, in the order it scores them.
//!
//! Perch v2's ONNX conversion ships no labels, and labels are positional: a file
//! that is one class out of step is not an error but a station naming every bird
//! after its neighbour in the list. `scripts/build-perch-labels.py` writes the
//! embedded file from Google's published list; this holds the file to what the
//! script promises, and to the one thing reading the names cannot show, which is
//! that class 10 397 really is *Pica pica*.
//!
//! That position was found, not assumed: the real Perch v2 model, given the
//! repository's 30-second magpie recording (`tests/testdata/Pica_pica_30s.wav`),
//! put its largest output on class 10 397 in four of the windows it was shown
//! (a logit of 13.3 to 13.5, the next class at 6.8 or below), and the list's
//! entry at that position is *Pica pica*. A class list shifted by one would have
//! named the neighbour. The check needs the 409 MB model, so it is stated here
//! and run by hand, not in CI; what CI can hold is the file, below.

use std::collections::HashSet;
use std::fmt::Write as _;

use birdnet_core::inference::labels::{
    BIRDNET_V24_ENGLISH_LABELS, LabelSet, PERCH_V2_LABELS, PERCH_V2_LABELS_SHA256,
};
use sha2::{Digest as _, Sha256};

/// The embedded file is the one the catalogue pins: editing the data without
/// regenerating and re-pinning it (or the reverse) fails here, not in the field
/// where `--install-model` would refuse its own digest.
#[test]
fn the_embedded_file_is_the_one_whose_digest_is_pinned() {
    let mut digest = String::new();
    for byte in &Sha256::digest(PERCH_V2_LABELS.as_bytes()) {
        write!(digest, "{byte:02x}").expect("writing to a String");
    }
    assert_eq!(digest, PERCH_V2_LABELS_SHA256);
}

#[test]
fn it_loads_as_the_14795_classes_the_model_outputs_each_once() {
    let set = LabelSet::load_from_str(PERCH_V2_LABELS).expect("parses as a station label CSV");
    assert_eq!(
        set.len(),
        14_795,
        "Perch v2's classifier head has 14 795 outputs"
    );

    let mut seen = HashSet::new();
    for i in 0..set.len() {
        let label = set.get(i).expect("index in range");
        assert!(
            seen.insert(label.scientific_name.clone()),
            "{} appears twice: a class named twice is one the model cannot be read for",
            label.scientific_name
        );
    }
}

/// Google's list carries no common names; the file gets them from the station's
/// own V2.4 names and from the IOC list, and holds back any that two classes would
/// share. What is held here is what the README states, and that the same bird has
/// one name whichever classifier heard it.
#[test]
fn common_names_agree_with_v24_where_it_has_one_and_are_never_shared() {
    let perch = LabelSet::load_from_str(PERCH_V2_LABELS).expect("parses");
    let v24 = LabelSet::load_from_str(BIRDNET_V24_ENGLISH_LABELS).expect("parses");
    let v24_name: std::collections::HashMap<String, String> = (0..v24.len())
        .filter_map(|i| v24.get(i))
        .map(|l| (l.scientific_name.clone(), l.common_name.clone()))
        .collect();

    let mut named = 0_usize;
    let mut users: std::collections::HashMap<String, Vec<String>> = Default::default();
    for i in 0..perch.len() {
        let label = perch.get(i).expect("index in range");
        if let Some(english) = v24_name.get(&label.scientific_name) {
            assert_eq!(
                &label.common_name, english,
                "{}: the same species must carry the name V2.4 gives it",
                label.scientific_name
            );
        }
        if label.common_name != label.scientific_name {
            named += 1;
            users
                .entry(label.common_name.clone())
                .or_default()
                .push(label.scientific_name.clone());
        }
    }
    assert_eq!(
        named, 10_178,
        "6 261 from V2.4 (of its 6 266 matches, five are names V2.4 itself gives as the class name: \
         Dog, Engine, Fireworks, Siren, Gryllus assimilis) and 3 917 from IOC, as the README states"
    );
    let shared: Vec<_> = users.iter().filter(|(_, v)| v.len() > 1).collect();
    assert!(
        shared.is_empty(),
        "an English name given to two classes would merge two species wherever the station groups by common name: {shared:?}"
    );
}

/// A name is only ever added to a species. The 200 classes that name a kind of
/// sound keep the class name, and so does a frog: nothing is invented.
#[test]
fn sound_classes_keep_their_own_name_and_the_magpie_has_its_english_one() {
    let set = LabelSet::load_from_str(PERCH_V2_LABELS).expect("parses");
    let find = |name: &str| {
        (0..set.len())
            .filter_map(|i| set.get(i))
            .find(|l| l.scientific_name == name)
            .unwrap_or_else(|| panic!("{name} is a class"))
    };
    for sound in ["Engine", "Vehicle", "Animal", "Wild_animals"] {
        assert_eq!(find(sound).common_name, sound);
    }
    assert_eq!(find("Abavorana luctuosa").common_name, "Abavorana luctuosa");
    assert_eq!(find("Pica pica").common_name, "Eurasian Magpie");
}

/// The order, at the positions that are known.
#[test]
fn the_classes_are_in_the_models_order_at_the_positions_that_are_known() {
    let set = LabelSet::load_from_str(PERCH_V2_LABELS).expect("parses");
    let at = |i: usize| set.get(i).expect("index in range").scientific_name.clone();
    assert_eq!(
        at(0),
        "Abavorana luctuosa",
        "the first class of Google's list"
    );
    assert_eq!(
        at(10_397),
        "Pica pica",
        "where the real model puts a magpie (see the module documentation)"
    );
    assert_eq!(
        at(14_794),
        "Zvenella yunnana",
        "the last class of Google's list"
    );
}

/// The file's line one is a header and its other lines are classes, with Unix
/// line ends and no byte-order mark: Google's list has Windows line ends, which
/// a `\r` left in a name would carry into every species name stored.
#[test]
fn it_is_clean_text_with_a_header_and_no_carriage_returns() {
    assert!(PERCH_V2_LABELS.starts_with("sci_name,com_name\n"));
    assert!(!PERCH_V2_LABELS.contains('\r'));
    assert!(!PERCH_V2_LABELS.starts_with('\u{feff}'));
    assert_eq!(
        PERCH_V2_LABELS.lines().count(),
        14_796,
        "header and 14 795 classes"
    );
}

/// Not every class is a species: 200 name a kind of sound (FSD50K), and the
/// manual and the catalogue say so. Hold the number the documentation states.
#[test]
fn exactly_two_hundred_classes_are_not_latin_binomials() {
    let set = LabelSet::load_from_str(PERCH_V2_LABELS).expect("parses");
    let is_binomial = |s: &str| {
        let mut words = s.split(' ');
        let (Some(genus), Some(species)) = (words.next(), words.next()) else {
            return false;
        };
        genus.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && genus
                .chars()
                .skip(1)
                .all(|c| c.is_ascii_lowercase() || c == '-')
            && species
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
            && species.chars().all(|c| c.is_ascii_lowercase() || c == '-')
    };
    let non_species = (0..set.len())
        .filter_map(|i| set.get(i))
        .filter(|l| !is_binomial(&l.scientific_name))
        .count();
    assert_eq!(non_species, 200);
    for sound in ["Engine", "Vehicle", "Animal", "Wild_animals"] {
        assert!(
            (0..set.len())
                .filter_map(|i| set.get(i))
                .any(|l| l.scientific_name == sound),
            "{sound} is a class, which is why a quiet window can be \"detected\" as one"
        );
    }
}
