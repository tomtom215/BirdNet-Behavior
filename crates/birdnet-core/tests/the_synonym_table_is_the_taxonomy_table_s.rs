//! The bundled synonym table (`crates/birdnet-core/data/taxonomy/same-species.tsv`)
//! is a derived copy of part of `docs/data/taxonomy/v3-names-not-in-perch.tsv`:
//! the rows whose relation is `same_species_other_name` and that rest on two or
//! more independent sources (MMP-09). This holds the copy to its source, so an
//! edit to the reviewed table cannot leave the merge reading an older one.

use std::collections::BTreeSet;

use birdnet_core::inference::labels::{LabelSet, PERCH_V2_LABELS};
use birdnet_core::inference::synonyms::{BUNDLED_SAME_SPECIES, SynonymTable};

const TABLE: &str = include_str!("../../../docs/data/taxonomy/v3-names-not-in-perch.tsv");

fn from_the_taxonomy_table() -> BTreeSet<(String, String, String)> {
    TABLE
        .lines()
        .skip(1)
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|c| c[2] == "same_species_other_name")
        .filter(|c| c[5].parse::<u32>().is_ok_and(|n| n >= 2))
        .map(|c| (c[0].to_owned(), c[4].to_owned(), c[5].to_owned()))
        .collect()
}

fn bundled_rows() -> BTreeSet<(String, String, String)> {
    BUNDLED_SAME_SPECIES
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            assert_eq!(c.len(), 3, "{l}");
            (c[0].to_owned(), c[1].to_owned(), c[2].to_owned())
        })
        .collect()
}

#[test]
fn the_bundled_pairs_are_the_well_supported_renames_of_the_taxonomy_table() {
    let expected = from_the_taxonomy_table();
    assert_eq!(expected.len(), 101, "precondition: the table's count");
    assert_eq!(bundled_rows(), expected);
}

#[test]
fn every_pair_is_parsed_and_each_second_name_is_a_perch_class() {
    let table = SynonymTable::bundled();
    assert_eq!(table.len(), bundled_rows().len(), "no row was skipped");
    let perch = LabelSet::load_from_str(PERCH_V2_LABELS).expect("the Perch labels parse");
    let perch: BTreeSet<&str> = perch.iter().map(|l| l.scientific_name.as_str()).collect();
    for (a, b, _) in bundled_rows() {
        assert!(perch.contains(b.as_str()), "{b} is not a Perch class");
        assert!(!perch.contains(a.as_str()), "{a} is a Perch class already");
        assert_eq!(table.merge_key(&a), table.merge_key(&b));
    }
}
