//! One species under two scientific names (audit MMP-09).
//!
//! The merge of several classifiers' verdicts used to key on the scientific
//! name as a string, so a bird that BirdNET+ V3.0 calls *Anthus lutescens* and
//! Perch v2 calls *Anthus chii* was two species to the station: two rows from
//! one chunk, each with `model_agreement` 1, and each one's "Who heard it"
//! card showing the other classifier as having no class for it.
//!
//! # The data
//!
//! `docs/data/taxonomy/v3-names-not-in-perch.tsv` reads every V3.0 class whose
//! name is not a Perch class against four public taxonomies and records the
//! relation. The bundled table ([`BUNDLED_SAME_SPECIES`]) is the part of it that
//! is a *synonym*: relation `same_species_other_name`, resting on two or more
//! independent sources — 101 pairs. Lumps (`perch_lumps_it`), splits and the
//! uncertain rows are not here: two species merged into one class is not two
//! names for one species, and treating it as one would let one classifier's
//! opinion about a species stand for another's. The 20 renames resting on a
//! single source are left out for the same reason the manual gives
//! (`docs/book/reference/taxonomy-overlap.md`).
//!
//! # Which name a detection carries
//!
//! The merge keys a pair as one species, and the detection it keeps is named
//! by [`SynonymTable::station_name`]: the **primary classifier's** spelling
//! where the primary has either name (the station names its species as its
//! primary does, as it already does for common names, MMP-10), else the
//! pair's second name, so the choice never depends on which classifier won a
//! chunk. Detections already stored under the other name are not rewritten.
//!
//! # A classifier that distinguishes the two names
//!
//! A label set that carries both names of a pair treats them as two species,
//! and merging them would undo its distinction. [`SynonymTable::restricted_to`]
//! drops every such pair for the classifiers a station actually loaded.

use std::collections::HashMap;

use crate::inference::labels::LabelSet;

/// The bundled synonym pairs: `name<TAB>name<TAB>sources` per line, `#`
/// comments. See the module documentation for what is in it and why.
pub const BUNDLED_SAME_SPECIES: &str = include_str!("../../data/taxonomy/same-species.tsv");

/// Pairs of scientific names that name one species.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SynonymTable {
    /// Each name of a pair, to the other name.
    other: HashMap<String, String>,
    /// Each name of a pair, to the pair's key: its second name.
    key: HashMap<String, String>,
}

impl SynonymTable {
    /// A table with no pairs: every name is its own species.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The bundled table ([`BUNDLED_SAME_SPECIES`]).
    #[must_use]
    pub fn bundled() -> Self {
        Self::parse(BUNDLED_SAME_SPECIES)
    }

    /// Read `name<TAB>name[<TAB>…]` lines. Blank lines and `#` comments are
    /// skipped; so is a line with fewer than two names, a self-pair, or a name
    /// already in another pair (a chain of renames is not a pair, and which
    /// link would win is not something to decide silently).
    #[must_use]
    pub fn parse(content: &str) -> Self {
        let mut table = Self::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cells = line.split('\t').map(str::trim);
            let (Some(a), Some(b)) = (cells.next(), cells.next()) else {
                continue;
            };
            if a.is_empty() || b.is_empty() || a == b {
                continue;
            }
            if table.other.contains_key(a) || table.other.contains_key(b) {
                continue;
            }
            table.other.insert(a.to_owned(), b.to_owned());
            table.other.insert(b.to_owned(), a.to_owned());
            table.key.insert(a.to_owned(), b.to_owned());
            table.key.insert(b.to_owned(), b.to_owned());
        }
        table
    }

    /// The same table without the pairs that some label set in `vocabularies`
    /// carries both names of (it distinguishes them), and without the pairs no
    /// label set carries either name of (they can never meet).
    #[must_use]
    pub fn restricted_to(self, vocabularies: &[&LabelSet]) -> Self {
        let carries = |name: &str| {
            vocabularies
                .iter()
                .any(|v| v.iter().any(|l| l.scientific_name == name))
        };
        let distinguishes = |a: &str, b: &str| {
            vocabularies.iter().any(|v| {
                v.iter().any(|l| l.scientific_name == a) && v.iter().any(|l| l.scientific_name == b)
            })
        };
        let mut kept = Self::default();
        for (a, b) in &self.key {
            if a == b {
                continue;
            }
            if distinguishes(a, b) || !(carries(a) || carries(b)) {
                continue;
            }
            kept.other.insert(a.clone(), b.clone());
            kept.other.insert(b.clone(), a.clone());
            kept.key.insert(a.clone(), b.clone());
            kept.key.insert(b.clone(), b.clone());
        }
        kept
    }

    /// How many pairs the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.other.len() / 2
    }

    /// Whether the table holds no pairs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.other.is_empty()
    }

    /// The other name of `name`'s pair, if it is in one.
    #[must_use]
    pub fn other_name(&self, name: &str) -> Option<&str> {
        self.other.get(name).map(String::as_str)
    }

    /// What the merge keys `name` by: its pair's key, or the name itself.
    /// Two names of one pair have one key; any other two names have two.
    #[must_use]
    pub fn merge_key<'a>(&'a self, name: &'a str) -> &'a str {
        self.key.get(name).map_or(name, String::as_str)
    }

    /// The name a detection of `name`'s species is stored under: `name` when
    /// it is in no pair; otherwise whichever of the pair `primary_has`, the
    /// first of the two when it has both, and the pair's key when it has
    /// neither.
    #[must_use]
    pub fn station_name<'a>(
        &'a self,
        name: &'a str,
        primary_has: impl Fn(&str) -> bool,
    ) -> &'a str {
        let Some(other) = self.other_name(name) else {
            return name;
        };
        if primary_has(name) {
            name
        } else if primary_has(other) {
            other
        } else {
            self.merge_key(name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(names: &[&str]) -> LabelSet {
        LabelSet::from_entries(
            names
                .iter()
                .map(|n| ((*n).to_owned(), format!("Common {n}")))
                .collect(),
        )
    }

    #[test]
    fn the_bundled_table_holds_the_101_well_supported_renames() {
        let t = SynonymTable::bundled();
        assert_eq!(t.len(), 101);
        assert_eq!(t.other_name("Anthus lutescens"), Some("Anthus chii"));
        assert_eq!(t.other_name("Anthus chii"), Some("Anthus lutescens"));
        assert_eq!(
            t.merge_key("Anthus lutescens"),
            t.merge_key("Anthus chii"),
            "a pair is one species to the merge"
        );
        // A name in no pair is its own key, and two such names stay apart.
        assert_eq!(t.merge_key("Turdus merula"), "Turdus merula");
        assert_ne!(t.merge_key("Turdus merula"), t.merge_key("Anthus chii"));
    }

    #[test]
    fn a_lump_is_not_a_synonym() {
        // Acanthis cabaret is lumped into A. flammea by Perch: two species to
        // V3.0, one to Perch. Not a pair.
        let t = SynonymTable::bundled();
        assert_eq!(t.other_name("Acanthis cabaret"), None);
        assert_eq!(t.other_name("Acanthis flammea"), None);
    }

    #[test]
    fn a_single_source_rename_is_not_applied() {
        // Aglaothorax ovata / ovatus rests on one source in the table.
        let t = SynonymTable::bundled();
        assert_eq!(t.other_name("Aglaothorax ovata"), None);
    }

    #[test]
    fn a_classifier_that_carries_both_names_keeps_them_apart() {
        let t = SynonymTable::parse("A a\tB b\nC c\tD d\n");
        let both = labels(&["A a", "B b", "C c"]);
        let other = labels(&["D d"]);
        let kept = t.restricted_to(&[&both, &other]);
        assert_eq!(
            kept.other_name("A a"),
            None,
            "one label set distinguishes them"
        );
        assert_eq!(kept.other_name("C c"), Some("D d"), "the counterpart stays");
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn a_pair_no_classifier_carries_is_dropped() {
        let t = SynonymTable::parse("A a\tB b\n").restricted_to(&[&labels(&["X x"])]);
        assert!(t.is_empty());
    }

    #[test]
    fn the_station_name_is_the_primary_s_spelling() {
        let t = SynonymTable::parse("Old genus\tNew genus\n");
        let primary_old = |n: &str| n == "Old genus";
        let primary_new = |n: &str| n == "New genus";
        let primary_none = |_: &str| false;
        for name in ["Old genus", "New genus"] {
            assert_eq!(t.station_name(name, primary_old), "Old genus");
            assert_eq!(t.station_name(name, primary_new), "New genus");
            assert_eq!(
                t.station_name(name, primary_none),
                "New genus",
                "with neither, the pair's key, whoever won"
            );
        }
        assert_eq!(t.station_name("Unpaired", primary_none), "Unpaired");
    }

    #[test]
    fn a_chain_or_a_self_pair_is_skipped() {
        let t = SynonymTable::parse("A\tA\nA\tB\nB\tC\n# c\n\nlonely\n");
        assert_eq!(t.len(), 1);
        assert_eq!(t.other_name("A"), Some("B"));
        assert_eq!(t.other_name("C"), None, "B is already paired");
    }
}
