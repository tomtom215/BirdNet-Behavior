//! Does the V2.4 range filter let through the species BirdNET-Pi's does?
//!
//! BirdNET-Pi runs the V2.4 `MData` model on `[lat, lon, week]` and keeps the
//! species it scores at or above `SF_THRESH`. This loads the ONNX conversion of
//! the same model into the shipping `SpeciesFilter`, asks it the same twelve
//! questions BirdNET-Pi's own code was asked (`tools/parity`, golden
//! `species_list_e.json`: two places x six weeks, including the weeks 49-53
//! BirdNET-Pi passes straight through from the ISO calendar) and compares the
//! size of every answer and a hash of its sorted species names.
//!
//! The weeks matter: the model reads anything outside 1-48 as "no seasonal
//! information", and returns one year-round list that is about twice as long as
//! any real week's. The filter must pass the week it is given through untouched
//! so that is the list it returns; clamping or wrapping a 49 to a 48 would
//! quietly change which birds a December station records.
//!
//! Needs the v2 range-filter ONNX (`birdnet-v24-range-v2` in the model
//! catalogue) and the V2.4 labels; skipped unless `BIRDNET_TEST_V24_MDATA` and
//! `BIRDNET_TEST_V24_LABELS` name them. CI sets `BIRDNET_REQUIRE_V24_MODEL=1`
//! once both are fetched and verified.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig};
use sha2::{Digest, Sha256};

const GOLDEN: &str = "tools/parity/golden/species_list_e.json";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[test]
fn v24_range_filter_lists_match_birdnet_pi() {
    let Some((_, labels_path)) = common::v24_model_paths_with_mdata() else {
        return;
    };
    let mdata = PathBuf::from(
        std::env::var("BIRDNET_TEST_V24_MDATA").expect("checked by v24_model_paths_with_mdata"),
    );

    let labels = LabelSet::load(&labels_path).expect("labels");
    let mut filter = SpeciesFilter::load_with_vocabulary(
        &mdata,
        None,
        &labels,
        &HashMap::new(),
        SpeciesFilterConfig {
            sf_thresh: 0.03,
            ..SpeciesFilterConfig::default()
        },
    )
    .expect("the v2 range filter loads as a matched pair for the V2.4 labels");

    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let entries: Vec<&serde_json::Value> = golden["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter(|e| e["version"] == 2)
        .collect();
    assert_eq!(entries.len(), 12, "the golden has twelve v2 questions");

    for e in entries {
        let (lat, lon) = (e["lat"].as_f64().unwrap(), e["lon"].as_f64().unwrap());
        let week = u32::try_from(e["week"].as_u64().unwrap()).unwrap();
        let allowed = filter
            .filter_species(Some((lat, lon)), week, &labels)
            .expect("filter");

        let mut names: Vec<&str> = allowed.iter().map(String::as_str).collect();
        names.sort_unstable();
        let digest = hex(&Sha256::digest(names.join("\n").as_bytes()));

        assert_eq!(
            allowed.len(),
            usize::try_from(e["count"].as_u64().unwrap()).unwrap(),
            "species count at ({lat}, {lon}) week {week}"
        );
        assert_eq!(
            digest,
            e["sorted_names_sha256"].as_str().unwrap(),
            "species list at ({lat}, {lon}) week {week}"
        );
    }
}
