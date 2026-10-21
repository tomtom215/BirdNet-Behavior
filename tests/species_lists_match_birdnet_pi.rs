//! Do the include, exclude and whitelist lists keep the species BirdNET-Pi's keep?
//!
//! `tools/parity/species_lists_ref.py` ran BirdNET-Pi's own `run_analysis()` over
//! twenty-seven scenarios (`golden/species_lists.json`): real list files under a
//! throw-away `$HOME`, its own v2 range model, eight species each scored 0.99 in
//! one window. This asks the same questions of this project: the file text is
//! read by the migration importer (`SpeciesList::parse`, which is how a
//! BirdNET-Pi station's lists arrive here) and handed to the shipping
//! `SpeciesFilter` with the ONNX range model, and the species that survive are
//! compared with BirdNET-Pi's.
//!
//! Where the two differ, the difference is *listed here by name* with what each
//! side does, so that it is a decision somebody can read, and so that neither
//! side can start doing something else without this test saying so. Every
//! scenario not in `KNOWN_DIFFERENCES` must agree exactly.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig};
use birdnet_migrate::conf::lists::SpeciesList;

const GOLDEN: &str = "tools/parity/golden/species_lists.json";

/// Scenarios on which this project deliberately does not do what BirdNET-Pi
/// does, with what each does. `(name, BirdNET-Pi keeps, this project keeps)`;
/// the species are those of the golden's universe, in its order.
///
/// * **`include_*` with an entry that names nothing usable.** BirdNET-Pi treats
///   a non-empty include list as a wall: whatever its entries match is let
///   through and everything else is not, so a list whose only entry is a typo,
///   a common name, a lower-case name or a blank line makes it record
///   *nothing*, silently. The importer drops entries that are not shaped like a
///   scientific name (and counts them for the operator's preview), the filter
///   ignores an include list that resolves to no species, and a station keeps
///   recording. This is the one place a migrated station records *more* than
///   the station it came from; the import preview names every dropped line.
/// * **`sf_thresh_1_*`.** BirdNET-Pi switches the occurrence filter off when the
///   model returns no species at all (`len(predicted_species_list) != 0`);
///   here an empty list from a working model is an empty allow-list. Only a
///   threshold at the top of its range gets there (the model's probabilities
///   are below 1), and it is the opposite reading of the same number.
const KNOWN_DIFFERENCES: &[(&str, &[&str], &[&str])] = &[
    (
        "include_common_name_only",
        &[],
        &[
            "Pica pica",
            "Parus major",
            "Turdus merula",
            "Erithacus rubecula",
        ],
    ),
    (
        "include_typo",
        &[],
        &[
            "Pica pica",
            "Parus major",
            "Turdus merula",
            "Erithacus rubecula",
        ],
    ),
    (
        "include_blank_line_only",
        &[],
        &[
            "Pica pica",
            "Parus major",
            "Turdus merula",
            "Erithacus rubecula",
        ],
    ),
    (
        "include_lowercase",
        &[],
        &[
            "Pica pica",
            "Parus major",
            "Turdus merula",
            "Erithacus rubecula",
        ],
    ),
    (
        "sf_thresh_1_empty_list_allows_all",
        &[
            "Pica pica",
            "Parus major",
            "Turdus merula",
            "Erithacus rubecula",
            "Bias musicus",
            "Casuarius casuarius",
            "Pitta sordida",
            "Menura novaehollandiae",
        ],
        &[],
    ),
    (
        "sf_thresh_1_exclude_one",
        &[
            "Parus major",
            "Turdus merula",
            "Erithacus rubecula",
            "Bias musicus",
            "Casuarius casuarius",
            "Pitta sordida",
            "Menura novaehollandiae",
        ],
        &[],
    ),
];

fn names(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_lists_keep_what_birdnet_pis_keep_except_where_this_file_says_otherwise() {
    let Some((_, labels_path)) = common::v24_model_paths_with_mdata() else {
        return;
    };
    let mdata = PathBuf::from(std::env::var("BIRDNET_TEST_V24_MDATA").expect("checked"));
    let labels = LabelSet::load(&labels_path).expect("labels");
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let universe = names(&golden["universe"]);
    let place = (
        golden["place"][0].as_f64().unwrap(),
        golden["place"][1].as_f64().unwrap(),
    );
    let week = u32::try_from(golden["iso_week"].as_u64().unwrap()).unwrap();
    let scenarios = golden["scenarios"].as_object().expect("scenarios");
    assert_eq!(scenarios.len(), 27, "the golden holds twenty-seven scenarios");
    let known: BTreeMap<&str, (&[&str], &[&str])> = KNOWN_DIFFERENCES
        .iter()
        .map(|&(n, pi, us)| (n, (pi, us)))
        .collect();

    let mut occurrence: HashMap<u64, std::collections::HashSet<String>> = HashMap::new();
    let mut agreed = 0;
    let mut differed = Vec::new();
    for (name, s) in scenarios {
        let list = |key: &str| -> Vec<String> {
            s[key]
                .as_str()
                .map_or_else(Vec::new, |text| SpeciesList::parse(text).species)
        };
        let mut filter = SpeciesFilter::load_with_vocabulary(
            &mdata,
            None,
            &labels,
            &HashMap::new(),
            SpeciesFilterConfig {
                sf_thresh: s["sf_thresh"].as_f64().unwrap() as f32,
                whitelist: list("whitelist_file").into_iter().collect(),
                include_list: list("include_file"),
                exclude_list: list("exclude_file"),
            },
        )
        .expect("the v2 range filter loads");
        let allowed = filter
            .filter_species(Some(place), week, &labels)
            .expect("filter");
        let ours: Vec<String> = universe
            .iter()
            .filter(|u| allowed.contains(*u))
            .cloned()
            .collect();
        let pi: Vec<String> = names(&s["kept"]);
        // BirdNET-Pi's own occurrence list is part of the golden: the range model must agree about it first
        // (at this scenario's threshold, with no lists), or a difference below would be the range filter's.
        let thr = s["sf_thresh"].as_f64().unwrap();
        let plain = occurrence.entry(thr.to_bits()).or_insert_with(|| {
            let mut f = SpeciesFilter::load_with_vocabulary(
                &mdata,
                None,
                &labels,
                &HashMap::new(),
                SpeciesFilterConfig {
                    sf_thresh: thr as f32,
                    ..SpeciesFilterConfig::default()
                },
            )
            .expect("the v2 range filter loads");
            f.filter_species(Some(place), week, &labels)
                .expect("filter")
        });
        for (species, in_list) in s["occurrence_has"].as_object().unwrap() {
            assert_eq!(
                plain.contains(species),
                in_list.as_bool().unwrap(),
                "{name}: range model, {species}"
            );
        }
        assert_eq!(
            plain.len(),
            usize::try_from(s["occurrence_count"].as_u64().unwrap()).unwrap(),
            "{name}: range model, species count at SF_THRESH {thr}"
        );
        if let Some(&(want_pi, want_ours)) = known.get(name.as_str()) {
            assert_eq!(
                pi, want_pi,
                "{name}: what BirdNET-Pi keeps (the golden changed?)"
            );
            assert_eq!(ours, want_ours, "{name}: what this project keeps");
            assert_ne!(
                pi, ours,
                "{name} is listed as a difference but the two agree: delete it from KNOWN_DIFFERENCES"
            );
            differed.push(name.clone());
        } else {
            assert_eq!(
                ours, pi,
                "{name}: BirdNET-Pi keeps {pi:?}, this project keeps {ours:?}"
            );
            agreed += 1;
        }
    }
    assert_eq!(differed.len(), KNOWN_DIFFERENCES.len(), "{differed:?}");
    assert_eq!(agreed, 27 - KNOWN_DIFFERENCES.len());
}
