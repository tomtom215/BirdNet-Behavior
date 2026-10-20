//! The V2.4 label file this project installs names birds as BirdNET-Pi does.
//!
//! The birdnet-go label file V2.4 is paired with names 460 of its 6 522 classes
//! differently from the English names BirdNET-Pi stores for the same classes
//! (`Pica pica` is "Common Magpie" there and "Eurasian Magpie" in BirdNET-Pi). A
//! station that imports a BirdNET-Pi history and then records with V2.4 would
//! hold one species under two names, and every query that groups by common name
//! would split it. `scripts/build-v24-labels.py` writes the embedded file; this
//! holds the file to what the script promises and to the order the model
//! outputs scores in, which no amount of reading the names can show.

use std::collections::HashSet;
use std::fmt::Write as _;

use birdnet_core::inference::labels::{
    BIRDNET_V24_ENGLISH_LABELS, BIRDNET_V24_ENGLISH_LABELS_SHA256, LabelSet,
};
use sha2::{Digest as _, Sha256};

/// The embedded file is the one the catalogue pins: editing the data without
/// regenerating and re-pinning it (or the reverse) fails here, not in the
/// field where `--install-model` would refuse its own digest.
#[test]
fn the_embedded_file_is_the_one_whose_digest_is_pinned() {
    let mut digest = String::new();
    for byte in &Sha256::digest(BIRDNET_V24_ENGLISH_LABELS.as_bytes()) {
        write!(digest, "{byte:02x}").expect("writing to a String");
    }
    assert_eq!(digest, BIRDNET_V24_ENGLISH_LABELS_SHA256);
}

#[test]
fn it_has_every_class_once_and_names_the_birds_birdnet_pi_names_them() {
    let set = LabelSet::load_from_str(BIRDNET_V24_ENGLISH_LABELS).expect("parses as V2.4 labels");
    assert_eq!(set.len(), 6522, "the V2.4 model has 6 522 outputs");

    let mut seen = HashSet::new();
    for i in 0..set.len() {
        let label = set.get(i).expect("index in range");
        assert!(
            seen.insert(label.scientific_name.clone()),
            "{} appears twice: a class named twice is one the model cannot be read for",
            label.scientific_name
        );
        assert!(
            !label.common_name.trim().is_empty(),
            "class {i} has no name"
        );
    }

    let name_of = |sci: &str| -> String {
        (0..set.len())
            .filter_map(|i| set.get(i))
            .find(|l| l.scientific_name == sci)
            .map_or_else(|| panic!("{sci} is not a class"), |l| l.common_name.clone())
    };
    // BirdNET-Pi's own test asserts the magpie; the other three are classes
    // whose names the birdnet-go file spells differently.
    assert_eq!(name_of("Pica pica"), "Eurasian Magpie");
    assert_eq!(name_of("Accipiter gentilis"), "Northern Goshawk");
    assert_eq!(name_of("Accipiter poliogaster"), "Gray-bellied Hawk");
    assert_eq!(name_of("Aegypius monachus"), "Cinereous Vulture");
    // The model's non-species classes keep the names the privacy and noise
    // filters look for: a rename here would silently switch them off.
    assert!(
        (0..set.len())
            .filter_map(|i| set.get(i))
            .filter(|l| birdnet_core::detection::privacy::names_a_human(
                &l.scientific_name,
                &l.common_name
            ))
            .count()
            >= 3,
        "the three human classes must still be recognised as human"
    );
}

/// The model outputs scores by position, so the only thing that makes these
/// names *the model's* names is that the classes are in the same order as the
/// label file the ONNX model is paired with. Compared line for line when that
/// file is at hand (CI fetches it for the V2.4 tests, and sets the require flag
/// once it has verified it, so a missing file there is a failure, not a skip).
#[test]
fn the_class_order_is_the_order_of_the_file_the_model_is_paired_with() {
    let Ok(path) = std::env::var("BIRDNET_TEST_V24_LABELS") else {
        assert!(
            std::env::var("BIRDNET_REQUIRE_V24_MODEL").is_err(),
            "BIRDNET_REQUIRE_V24_MODEL is set but BIRDNET_TEST_V24_LABELS is not"
        );
        eprintln!("SKIP: BIRDNET_TEST_V24_LABELS unset");
        return;
    };
    let theirs = std::fs::read_to_string(&path).expect("read the V2.4 label file");
    let ours: Vec<&str> = BIRDNET_V24_ENGLISH_LABELS.lines().collect();
    let theirs: Vec<&str> = theirs.lines().collect();
    assert_eq!(ours.len(), theirs.len(), "class count");
    let mut renamed = 0;
    for (n, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        let (sci_a, com_a) = a.split_once('_').unwrap_or((a, ""));
        let (sci_b, com_b) = b.split_once('_').unwrap_or((b, ""));
        assert_eq!(
            sci_a,
            sci_b,
            "line {}: the classes are in a different order",
            n + 1
        );
        if com_a != com_b {
            renamed += 1;
        }
    }
    assert!(
        renamed > 0,
        "the two files name every class alike, so this file would change nothing"
    );
}
