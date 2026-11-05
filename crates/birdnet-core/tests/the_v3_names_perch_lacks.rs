//! `docs/data/taxonomy/v3-names-not-in-perch.tsv` states what holds between the
//! BirdNET+ V3.0 preview 3 classes and Perch v2's, name by name. This holds the
//! part of it that can be checked without V3's label file, which is not in this
//! repository.
//!
//! The table is the 644 V3 classes whose scientific name is not, as a string, a
//! Perch class. Whether each is a rename, a lump, a split or simply absent is a
//! judgement made source by source (`docs/book/reference/taxonomy-overlap.md`
//! says how); what is mechanical is checked here: no listed name is a Perch
//! class (or the list would be wrong about its own definition), and every
//! counterpart the table names is one, so a typo in a counterpart cannot pass
//! as evidence.

use std::collections::{BTreeMap, HashSet};

use birdnet_core::inference::labels::{LabelSet, PERCH_V2_LABELS};

const TABLE: &str = include_str!("../../../docs/data/taxonomy/v3-names-not-in-perch.tsv");

fn perch_classes() -> HashSet<String> {
    let set = LabelSet::load_from_str(PERCH_V2_LABELS).expect("the Perch labels parse");
    (0..set.len())
        .filter_map(|i| set.get(i))
        .map(|l| l.scientific_name.clone())
        .collect()
}

fn rows() -> Vec<Vec<&'static str>> {
    let mut lines = TABLE.lines();
    let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    assert_eq!(
        header,
        [
            "v3_sci_name",
            "v3_class",
            "relation",
            "confidence",
            "perch_counterparts",
            "independent_sources",
            "agreeing_sources",
            "record_ids"
        ]
    );
    lines
        .map(|l| {
            let cells: Vec<&str> = l.split('\t').collect();
            assert_eq!(cells.len(), header.len(), "{l}");
            cells
        })
        .collect()
}

#[test]
fn it_lists_644_names_in_the_six_relations_the_manual_counts() {
    let rows = rows();
    assert_eq!(rows.len(), 644);
    let mut by_relation: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &rows {
        *by_relation.entry(r[2]).or_default() += 1;
    }
    assert_eq!(
        by_relation,
        BTreeMap::from([
            ("absent_from_perch", 369),
            ("perch_lumps_it", 129),
            ("perch_splits_it", 8),
            ("same_species_other_name", 121),
            ("subspecies_or_form_only", 6),
            ("uncertain", 11),
        ])
    );
    let names: HashSet<_> = rows.iter().map(|r| r[0]).collect();
    assert_eq!(names.len(), 644, "no name is listed twice");
}

#[test]
fn no_listed_name_is_a_perch_class_and_every_counterpart_is() {
    let perch = perch_classes();
    for r in rows() {
        assert!(
            !perch.contains(r[0]),
            "{} is a Perch class, so it is not one of the names Perch lacks",
            r[0]
        );
        for counterpart in r[4].split("; ").filter(|c| !c.is_empty()) {
            assert!(
                perch.contains(counterpart),
                "{}: the counterpart {counterpart:?} is not a Perch class",
                r[0]
            );
        }
        match r[2] {
            "absent_from_perch" => assert!(r[4].is_empty(), "{}: absent, so no counterpart", r[0]),
            "uncertain" => {}
            other => assert!(
                !r[4].is_empty(),
                "{}: {other} must name a counterpart",
                r[0]
            ),
        }
    }
}
