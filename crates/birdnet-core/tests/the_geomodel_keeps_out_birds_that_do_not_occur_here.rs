//! The geomodel keeps out birds that do not occur where the station is.
//!
//! A Raspberry Pi in Europe, on 0.17.0, recorded a Great Horned Owl and a
//! Dickcissel — two North American species. Either its occurrence filter was
//! not running, or it was running and letting them through. Nothing tested the
//! second: every geomodel test checked that the model loads, or that its week
//! input is right, and none asked the model whether a given bird belongs at a
//! given place.
//!
//! This asks the real geomodel (v3.0.2, the one the installer fetches) with the
//! real classifier vocabulary, through `load_with_vocabulary` and
//! `filter_species` — the calls the daemon makes — about the two species from
//! that station and a magpie, at a place each occurs and a place it does not.
//!
//! Model-gated: needs `BIRDNET_TEST_LABELS` (the classifier labels CSV),
//! `BIRDNET_TEST_GEOMODEL` and `BIRDNET_TEST_GEOMODEL_LABELS`. With
//! `BIRDNET_REQUIRE_MODEL=1` (CI, once the files are fetched) a missing file
//! fails instead of skipping.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig};

const MAGPIE: &str = "Pica pica";
const DICKCISSEL: &str = "Spiza americana";
const GREAT_HORNED_OWL: &str = "Bubo virginianus";

/// Berlin, and central Kansas: each is home to the birds the other is not.
const BERLIN: (f64, f64) = (52.52, 13.405);
const KANSAS: (f64, f64) = (38.5, -98.0);

/// Week 22 of the model's 48-week year: early June, when the Dickcissel, a
/// summer visitor, is on its breeding grounds. The owl and the magpie are
/// resident all year.
const JUNE: u32 = 22;

/// The three files, or `None` to skip — a panic when CI promised them.
fn files() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let required = std::env::var("BIRDNET_REQUIRE_MODEL").is_ok_and(|v| !v.is_empty() && v != "0");
    let paths: Vec<Option<PathBuf>> = [
        "BIRDNET_TEST_LABELS",
        "BIRDNET_TEST_GEOMODEL",
        "BIRDNET_TEST_GEOMODEL_LABELS",
    ]
    .iter()
    .map(|k| {
        std::env::var(k)
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
    })
    .collect();
    if let [Some(labels), Some(geo), Some(geo_labels)] = paths.as_slice() {
        return Some((labels.clone(), geo.clone(), geo_labels.clone()));
    }
    assert!(
        !required,
        "BIRDNET_REQUIRE_MODEL is set, but BIRDNET_TEST_LABELS / BIRDNET_TEST_GEOMODEL / \
         BIRDNET_TEST_GEOMODEL_LABELS do not all name files, so the occurrence filter \
         was NOT checked"
    );
    eprintln!("SKIP: set BIRDNET_TEST_LABELS, BIRDNET_TEST_GEOMODEL, BIRDNET_TEST_GEOMODEL_LABELS");
    None
}

/// The species the filter passes at `place` in June, plus the vocabulary size.
fn allowed_at(place: (f64, f64)) -> Option<(HashSet<String>, usize)> {
    let (labels, geo, geo_labels) = files()?;
    let classifier = LabelSet::load(&labels).expect("classifier labels load");
    let meta = LabelSet::load(&geo_labels).expect("geomodel labels load");
    let mut filter = SpeciesFilter::load_with_vocabulary(
        &geo,
        Some(meta),
        &classifier,
        &HashMap::new(),
        SpeciesFilterConfig::default(),
    )
    .expect("geomodel loads against the classifier vocabulary");
    let allowed = filter
        .filter_species(Some(place), JUNE, &classifier)
        .expect("geomodel runs");
    Some((allowed, classifier.len()))
}

#[test]
fn a_european_station_keeps_out_the_north_american_birds_and_keeps_the_magpie() {
    let Some((allowed, total)) = allowed_at(BERLIN) else {
        return;
    };
    // A filter that passed everything, or nothing, would make the assertions
    // below say nothing about occurrence.
    assert!(
        !allowed.is_empty() && allowed.len() < total,
        "the filter passed {} of {total} species in Berlin — it is not filtering",
        allowed.len()
    );
    assert!(
        allowed.contains(MAGPIE),
        "the magpie, resident in Berlin, was filtered out"
    );
    for bird in [DICKCISSEL, GREAT_HORNED_OWL] {
        assert!(
            !allowed.contains(bird),
            "{bird}, a North American species, passes the filter in Berlin"
        );
    }
}

#[test]
fn counterpart_a_north_american_station_keeps_them_and_keeps_out_the_magpie() {
    let Some((allowed, _)) = allowed_at(KANSAS) else {
        return;
    };
    for bird in [DICKCISSEL, GREAT_HORNED_OWL] {
        assert!(
            allowed.contains(bird),
            "{bird} was filtered out in Kansas in June"
        );
    }
    assert!(
        !allowed.contains(MAGPIE),
        "the Eurasian Magpie passes the filter in Kansas"
    );
}

/// What an operator is shown when they ask why a bird is not being heard is the
/// decision the station makes, not a second opinion.
///
/// `occurrence_scores` and `filter_species` share one model run, and this holds
/// the two answers equal on the real geomodel: the species marked admitted are
/// exactly the species the filter passes, the list is ordered by probability,
/// and the three birds above sit where the filter put them with a number that
/// agrees with the threshold. The counterpart for a filter with no model is
/// `None`, not an empty list that reads as "nothing occurs here".
#[test]
fn the_scores_an_operator_is_shown_are_the_decision_the_station_makes() {
    let Some((labels, geo, geo_labels)) = files() else {
        return;
    };
    let classifier = LabelSet::load(&labels).expect("classifier labels load");
    let meta = LabelSet::load(&geo_labels).expect("geomodel labels load");
    let config = SpeciesFilterConfig::default();
    let threshold = config.sf_thresh;
    let mut filter =
        SpeciesFilter::load_with_vocabulary(&geo, Some(meta), &classifier, &HashMap::new(), config)
            .expect("geomodel loads");

    let passed = filter
        .filter_species(Some(BERLIN), JUNE, &classifier)
        .expect("filter runs");
    let scores = filter
        .occurrence_scores(BERLIN, JUNE, &classifier)
        .expect("scores run")
        .expect("a model is loaded");

    let admitted: HashSet<String> = scores
        .iter()
        .filter(|s| s.admitted)
        .map(|s| s.scientific_name.clone())
        .collect();
    assert_eq!(
        admitted, passed,
        "the explorer and the filter disagree about what Berlin admits in June"
    );
    assert!(
        scores
            .windows(2)
            .all(|w| w[0].probability >= w[1].probability),
        "highest probability first"
    );

    let find = |sci: &str| {
        scores
            .iter()
            .find(|s| s.scientific_name == sci)
            .unwrap_or_else(|| panic!("{sci} is not scored"))
    };
    let magpie = find(MAGPIE);
    assert!(
        magpie.admitted && magpie.probability >= threshold,
        "{magpie:?}"
    );
    assert_eq!(magpie.common_name, "Eurasian Magpie");
    for bird in [DICKCISSEL, GREAT_HORNED_OWL] {
        let s = find(bird);
        assert!(
            !s.admitted && s.probability < threshold,
            "{bird} must be below the threshold in Berlin: {s:?}"
        );
    }

    // No model: nothing is filtered, so there is nothing to explain.
    let mut passthrough = SpeciesFilter::new_passthrough(SpeciesFilterConfig::default());
    assert!(
        passthrough
            .occurrence_scores(BERLIN, JUNE, &classifier)
            .expect("no model is not an error")
            .is_none()
    );
}

/// The same agreement for the filter BirdNET-Pi users have: the V2.4 range
/// filter, indexed like the V2.4 labels with no label file of its own, so the
/// explorer takes the other branch (no alignment, index straight to label).
#[test]
fn the_v24_range_filter_scores_agree_with_the_filter_it_explains() {
    let required =
        std::env::var("BIRDNET_REQUIRE_V24_MODEL").is_ok_and(|v| !v.is_empty() && v != "0");
    let (Some(labels), Some(mdata)) = (
        std::env::var_os("BIRDNET_TEST_V24_LABELS").map(PathBuf::from),
        std::env::var_os("BIRDNET_TEST_V24_MDATA").map(PathBuf::from),
    ) else {
        assert!(
            !required,
            "BIRDNET_REQUIRE_V24_MODEL is set but BIRDNET_TEST_V24_LABELS / \
             BIRDNET_TEST_V24_MDATA are not: the V2.4 range filter was NOT checked"
        );
        eprintln!("SKIP: set BIRDNET_TEST_V24_LABELS and BIRDNET_TEST_V24_MDATA");
        return;
    };
    let classifier = LabelSet::load(&labels).expect("V2.4 labels load");
    let mut filter = SpeciesFilter::load_with_vocabulary(
        &mdata,
        None,
        &classifier,
        &HashMap::new(),
        SpeciesFilterConfig::default(),
    )
    .expect("the V2.4 range filter loads against the V2.4 labels");

    let passed = filter
        .filter_species(Some(BERLIN), JUNE, &classifier)
        .expect("filter runs");
    let scores = filter
        .occurrence_scores(BERLIN, JUNE, &classifier)
        .expect("scores run")
        .expect("a model is loaded");
    let admitted: HashSet<String> = scores
        .iter()
        .filter(|s| s.admitted)
        .map(|s| s.scientific_name.clone())
        .collect();
    assert_eq!(admitted, passed);
    assert_eq!(scores.len(), classifier.len(), "one score per V2.4 class");
    assert!(passed.contains(MAGPIE) && !passed.contains(DICKCISSEL));
}
