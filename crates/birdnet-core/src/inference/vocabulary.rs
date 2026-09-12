//! Aligning the metadata model's species vocabulary to the classifier's.
//!
//! The two models do not score the same species list, and they do not agree on
//! every name. Measured on the pair the installer pins — BirdNET+ Geomodel
//! V3.0.2 Global 12K labels (`sha256 c15818db…87c784`, 12 012 rows) against
//! BirdNET+ V3.0-preview3 Global 11K labels (`sha256 8124b0ea…21f8f0`,
//! 11 560 rows) — **1 679 of the geomodel's rows have no exact scientific-name
//! counterpart in the classifier**.
//!
//! Most of those are species the classifier simply cannot emit, and dropping
//! them is right. But 62 of them are the *same taxon under a reclassified
//! genus*: the geomodel says *Leuconotopicus villosus* where the classifier
//! says *Dryobates villosus*, and both mean Hairy Woodpecker. Matching on the
//! scientific name alone, those 62 — 41 of them birds, among them Hairy
//! Woodpecker, Red-cockaded Woodpecker, White-headed Woodpecker and Evening
//! Grosbeak — never enter the passing set, so the classifier's own name for
//! them is never admitted. They are *permanently undetectable* at any station
//! running the occurrence filter, and nothing says so.
//!
//! # How a name is matched
//!
//! [`Alignment::build`] resolves each metadata row to a classifier row, in
//! this order, and records which rule fired:
//!
//! 1. **Scientific name**, trimmed and compared case-insensitively.
//! 2. **Common name and specific epithet together.** The common name is
//!    normalised to its letters (`Fruit-Dove`, `Fruit Dove` and `fruitdove`
//!    are one name) and must be unambiguous on both sides; the specific
//!    epithet must then agree, exactly or modulo Latin gender agreement
//!    (*gymnocerca* / *gymnocercus*).
//!
//! The epithet is the guard, not decoration. On the pinned pair three rows
//! match by common name alone and disagree on the epithet, and two of the
//! three are plain wrong: the classifier's label file calls *Lama glama* (the
//! llama) "Guanaco", and *Scapteriscus borellii* answers to "Southern Mole
//! Cricket" for a geomodel row that is *Gryllotalpa australis*. Admitting
//! either would let the geomodel's opinion about one species decide another's.
//! The third, *Physeter macrocephalus* against *Physeter catodon*, is a real
//! synonym the guard costs us; it is not a bird, and the trade is one lost
//! whale against two wrong admissions.
//!
//! # What is deliberately not here
//!
//! **A shipped alias table.** The plan this module replaces was to vendor
//! `OpenFauna`'s `aliases.json`, the table `tphakala/birdnet-go` embeds. Two
//! things rule it out, and the second on its own would be enough. It is CC
//! BY-SA 4.0, and this project is CC BY-NC-SA 4.0 — `ShareAlike` does not permit
//! adding the `NonCommercial` restriction. And it does not work: applied to the
//! pinned pair, **all 237 of its entries recover exactly 0 of the 1 679
//! unmatched rows**, because its reclassifications (*Accipiter* → *Tachyspiza*
//! and the like) are ones both of our files already agree on. An operator who
//! wants it can still install it; see `SPECIES_ALIASES_PATH`.

use std::collections::HashMap;

use crate::inference::labels::LabelSet;

/// Which rule matched a metadata species to a classifier species.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The two files spell the scientific name the same way (ignoring case).
    Scientific,
    /// The two files disagree on the genus but agree on the common name and
    /// the specific epithet.
    CommonName,
    /// An operator-supplied alias file maps the metadata name onto a name the
    /// classifier carries.
    Alias,
}

impl MatchKind {
    /// A short word for logs and the doctor's output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scientific => "scientific name",
            Self::CommonName => "common name + epithet",
            Self::Alias => "operator alias",
        }
    }
}

/// What [`Alignment::build`] found, for the log line and the doctor.
///
/// Every field is a count of *metadata* rows except `classifier_unreachable`,
/// which counts the other direction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlignmentStats {
    /// Rows in the metadata model's label file.
    pub metadata_species: usize,
    /// Rows in the classifier's label file.
    pub classifier_species: usize,
    /// Matched because both files spell the scientific name the same way.
    pub by_scientific_name: usize,
    /// Matched on the common name and specific epithet after the scientific
    /// names disagreed.
    pub by_common_name: usize,
    /// Matched through the operator's alias file.
    pub by_alias: usize,
    /// Metadata rows no rule could place. These name species the classifier
    /// cannot emit, so they are correctly dropped — but the count is the only
    /// thing that would show a *wrongly* dropped one, so it is reported.
    pub unmatched: usize,
    /// Classifier species no metadata row resolves to. The occurrence filter
    /// can never admit one of these while it is running, because the geomodel
    /// has no opinion to offer about them.
    pub classifier_unreachable: usize,
    /// Up to [`Self::SAMPLE`] of the names behind `by_common_name`, so an
    /// operator can see what the looser rule actually did.
    pub recovered_sample: Vec<String>,
}

impl AlignmentStats {
    /// How many recovered names `recovered_sample` keeps.
    pub const SAMPLE: usize = 8;

    /// Metadata rows that resolved to a classifier species, by any rule.
    #[must_use]
    pub const fn matched(&self) -> usize {
        self.by_scientific_name + self.by_common_name + self.by_alias
    }
}

/// A resolved map from metadata output index to the classifier's own spelling.
///
/// Built once at load. It used to be a linear scan of the classifier's labels
/// per passing species per inference — with the pinned pair that is up to
/// 12 012 × 11 560 lowercasing string comparisons behind one cache miss.
#[derive(Debug, Clone)]
pub struct Alignment {
    /// Indexed by metadata output index. `None` where nothing matched.
    ///
    /// The stored string is the **classifier's** spelling, not the metadata
    /// file's, because it is what a detection will carry and what the passing
    /// set is tested against downstream.
    to_classifier: Vec<Option<Box<str>>>,
    stats: AlignmentStats,
}

impl Alignment {
    /// Align `metadata`'s vocabulary onto `classifier`'s.
    ///
    /// `aliases` maps a metadata scientific name to the name the classifier
    /// uses, lowercased on both sides by the caller
    /// ([`parse_alias_file`] does this). Pass an empty map for none.
    #[must_use]
    pub fn build(
        metadata: &LabelSet,
        classifier: &LabelSet,
        aliases: &HashMap<String, String>,
    ) -> Self {
        let by_scientific: HashMap<String, usize> = classifier
            .iter()
            .map(|l| (l.scientific_name.trim().to_lowercase(), l.index))
            .collect();

        // A normalised common name that names more than one classifier row
        // cannot identify a species, so it is struck out rather than resolved
        // to whichever row came first. 19 of the pinned classifier's common
        // names collide once normalised.
        let mut by_common: HashMap<String, Option<usize>> = HashMap::new();
        for label in classifier.iter() {
            let key = normalise_common(&label.common_name);
            if key.is_empty() {
                continue;
            }
            by_common
                .entry(key)
                .and_modify(|slot| *slot = None)
                .or_insert(Some(label.index));
        }

        let mut to_classifier: Vec<Option<Box<str>>> = vec![None; metadata.len()];
        let mut kinds: Vec<Option<MatchKind>> = vec![None; metadata.len()];
        // Which classifier index each metadata row claimed, so a second claim
        // on the same species can be refused.
        let mut claimed_by: HashMap<usize, usize> = HashMap::new();

        // Pass 1: the scientific name, and the operator's aliases, which are
        // an explicit instruction and so rank with it.
        for label in metadata.iter() {
            let name = label.scientific_name.trim().to_lowercase();
            if let Some(&index) = by_scientific.get(&name) {
                claim(
                    &mut to_classifier,
                    &mut kinds,
                    &mut claimed_by,
                    classifier,
                    label.index,
                    index,
                    MatchKind::Scientific,
                );
            } else if let Some(&index) = aliases.get(&name).and_then(|a| by_scientific.get(a)) {
                claim(
                    &mut to_classifier,
                    &mut kinds,
                    &mut claimed_by,
                    classifier,
                    label.index,
                    index,
                    MatchKind::Alias,
                );
            }
        }

        // Pass 2: common name and epithet, over what pass 1 left. It runs
        // second so that an exact scientific-name match always wins the
        // classifier row it names — a looser rule must never displace it.
        for label in metadata.iter() {
            if to_classifier[label.index].is_some() {
                continue;
            }
            let Some(Some(index)) = by_common
                .get(&normalise_common(&label.common_name))
                .copied()
            else {
                continue;
            };
            // The looser rule may only *recover* species the stricter one
            // could not reach. A classifier species already matched by name
            // needs no heuristic help, and a second route to it can add a
            // false admission but can never save a lost bird.
            if claimed_by.contains_key(&index) {
                continue;
            }
            let Some(candidate) = classifier.get(index) else {
                continue;
            };
            if !epithets_agree(&label.scientific_name, &candidate.scientific_name) {
                continue;
            }
            claim(
                &mut to_classifier,
                &mut kinds,
                &mut claimed_by,
                classifier,
                label.index,
                index,
                MatchKind::CommonName,
            );
        }

        let mut stats = AlignmentStats {
            metadata_species: metadata.len(),
            classifier_species: classifier.len(),
            classifier_unreachable: classifier.len().saturating_sub(claimed_by.len()),
            ..AlignmentStats::default()
        };
        for (i, kind) in kinds.iter().enumerate() {
            match kind {
                Some(MatchKind::Scientific) => stats.by_scientific_name += 1,
                Some(MatchKind::Alias) => stats.by_alias += 1,
                Some(MatchKind::CommonName) => {
                    stats.by_common_name += 1;
                    if stats.recovered_sample.len() < AlignmentStats::SAMPLE
                        && let (Some(m), Some(c)) = (metadata.get(i), to_classifier[i].as_ref())
                    {
                        stats
                            .recovered_sample
                            .push(format!("{} = {c}", m.scientific_name));
                    }
                }
                None => stats.unmatched += 1,
            }
        }

        Self {
            to_classifier,
            stats,
        }
    }

    /// The classifier's own name for the species at metadata output `index`,
    /// or `None` when nothing in the classifier matches it.
    #[must_use]
    pub fn classifier_name(&self, index: usize) -> Option<&str> {
        self.to_classifier.get(index)?.as_deref()
    }

    /// What the alignment found.
    #[must_use]
    pub const fn stats(&self) -> &AlignmentStats {
        &self.stats
    }
}

/// Record one metadata row's resolution to a classifier row.
fn claim(
    to_classifier: &mut [Option<Box<str>>],
    kinds: &mut [Option<MatchKind>],
    claimed_by: &mut HashMap<usize, usize>,
    classifier: &LabelSet,
    metadata_index: usize,
    classifier_index: usize,
    kind: MatchKind,
) {
    let Some(label) = classifier.get(classifier_index) else {
        return;
    };
    to_classifier[metadata_index] = Some(label.scientific_name.as_str().into());
    kinds[metadata_index] = Some(kind);
    claimed_by.entry(classifier_index).or_insert(metadata_index);
}

/// A common name reduced to its letters, lowercased.
///
/// The two files punctuate differently and neither is wrong:
/// `Black-chinned Fruit-Dove` against `Black-chinned Fruit Dove`,
/// `Eastern Dwarf Tree Frog` against `Eastern Dwarf Treefrog`. Eight of the 62
/// recovered rows on the pinned pair differ only this way.
fn normalise_common(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The specific epithet: the second whitespace-separated word, lowercased.
///
/// `None` for a name that has no second word. The classifier's label file has
/// 55 such rows — family-level labels like `Acrididae` and genus-only ones
/// like `Alouatta` — and they must not match each other on a missing epithet.
fn epithet(scientific: &str) -> Option<String> {
    scientific.split_whitespace().nth(1).map(str::to_lowercase)
}

/// Whether two scientific names share a specific epithet.
///
/// Exactly, or after stripping the Latin gender-agreement ending: an epithet
/// agrees in gender with its genus, so moving a species to a genus of another
/// gender rewrites it — *Lycalopex gymnocerca* and *Lycalopex gymnocercus*,
/// *Emblema modestum* and *Emblema modesta*.
fn epithets_agree(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (epithet(a), epithet(b)) else {
        return false;
    };
    a == b || gender_stem(&a) == gender_stem(&b)
}

/// An epithet with its gender-agreement ending removed.
///
/// The length floor keeps the rule off short epithets, where stripping two
/// letters from a five-letter word collapses names that are not variants of
/// each other.
fn gender_stem(epithet: &str) -> &str {
    const ENDINGS: [&str; 5] = ["us", "um", "is", "a", "e"];
    for ending in ENDINGS {
        if epithet.len() > ending.len() + 2
            && let Some(stem) = epithet.strip_suffix(ending)
        {
            return stem;
        }
    }
    if epithet.len() > 3 {
        if let Some(stem) = epithet.strip_suffix("ii") {
            return stem;
        }
        if let Some(stem) = epithet.strip_suffix('i') {
            return stem;
        }
    }
    epithet
}

/// Parse an operator's alias file: `legacy<TAB>canonical` per line.
///
/// Blank lines and `#` comments are skipped, as in the geomodel's own label
/// file. Both names are lowercased, so the map can be looked up with a
/// lowercased scientific name. A line without a tab, an empty column, or a
/// self-alias is skipped rather than fatal: the file is hand-maintained, and
/// one bad line must not take the occurrence filter off a running station.
///
/// The returned count of skipped lines is what the caller logs.
#[must_use]
pub fn parse_alias_file(content: &str) -> (HashMap<String, String>, usize) {
    let mut map = HashMap::new();
    let mut skipped = 0usize;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((legacy, canonical)) = line.split_once('\t') else {
            skipped += 1;
            continue;
        };
        let legacy = legacy.trim().to_lowercase();
        let canonical = canonical.trim().to_lowercase();
        if legacy.is_empty() || canonical.is_empty() || legacy == canonical {
            skipped += 1;
            continue;
        }
        map.insert(legacy, canonical);
    }
    (map, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label set from `(scientific, common)` pairs.
    fn labels(entries: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_entries(
            entries
                .iter()
                .map(|(s, c)| ((*s).to_owned(), (*c).to_owned()))
                .collect(),
        )
    }

    fn no_aliases() -> HashMap<String, String> {
        HashMap::new()
    }

    /// The defect, in the smallest form that shows it.
    ///
    /// *Leuconotopicus villosus* and *Dryobates villosus* are the geomodel's
    /// and the classifier's names for the Hairy Woodpecker, a bird common
    /// enough across North America to be at half the stations that will ever
    /// run this. Matching on the scientific name alone leaves it with no
    /// classifier counterpart, so the geomodel's opinion about it never
    /// reaches the passing set and the station cannot report it at all.
    #[test]
    fn a_reclassified_genus_still_finds_its_classifier_species() {
        let metadata = labels(&[("Leuconotopicus villosus", "Hairy Woodpecker")]);
        let classifier = labels(&[("Dryobates villosus", "Hairy Woodpecker")]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(
            alignment.classifier_name(0),
            Some("Dryobates villosus"),
            "the classifier's own spelling is what a detection will carry, so it \
             is what the passing set must hold"
        );
        assert_eq!(alignment.stats().by_common_name, 1);
        assert_eq!(alignment.stats().unmatched, 0);
    }

    /// The counterpart, and the reason the epithet is checked at all: a shared
    /// common name is not on its own evidence of a shared species.
    ///
    /// Both cases are real, read out of the pinned classifier's own label
    /// file. It calls *Lama glama* — the llama — "Guanaco", which is
    /// *Lama guanicoe*; and it calls *Scapteriscus borellii* "Southern Mole
    /// Cricket", which the geomodel also uses for *Gryllotalpa australis*, a
    /// different genus in a different family.
    #[test]
    fn a_shared_common_name_with_a_different_epithet_is_not_a_match() {
        let metadata = labels(&[
            ("Lama guanicoe", "Guanaco"),
            ("Gryllotalpa australis", "Southern Mole Cricket"),
        ]);
        let classifier = labels(&[
            ("Lama glama", "Guanaco"),
            ("Scapteriscus borellii", "Southern Mole Cricket"),
        ]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), None, "guanaco is not a llama");
        assert_eq!(alignment.classifier_name(1), None);
        assert_eq!(alignment.stats().unmatched, 2);
        assert_eq!(alignment.stats().by_common_name, 0);
    }

    /// Latin epithets agree in gender with their genus, so moving a species
    /// between genera of different gender rewrites the epithet. Both pairs are
    /// from the pinned files.
    #[test]
    fn a_gender_agreement_ending_is_not_a_different_epithet() {
        let metadata = labels(&[
            ("Lycalopex gymnocerca", "Pampas Fox"),
            ("Emblema modestum", "Plum-headed Finch"),
        ]);
        let classifier = labels(&[
            ("Lycalopex gymnocercus", "Pampas Fox"),
            ("Emblema modesta", "Plum-headed Finch"),
        ]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), Some("Lycalopex gymnocercus"));
        assert_eq!(alignment.classifier_name(1), Some("Emblema modesta"));
        assert_eq!(alignment.stats().by_common_name, 2);
    }

    /// The two files punctuate common names differently and neither is wrong.
    #[test]
    fn punctuation_and_case_do_not_stop_a_common_name_matching() {
        let metadata = labels(&[
            ("Ramphiculus jambu", "Jambu Fruit-Dove"),
            ("Drymomantis fallax", "Eastern Dwarf Tree Frog"),
        ]);
        let classifier = labels(&[
            ("Ptilinopus jambu", "Jambu Fruit Dove"),
            ("Litoria fallax", "eastern dwarf treefrog"),
        ]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), Some("Ptilinopus jambu"));
        assert_eq!(alignment.classifier_name(1), Some("Litoria fallax"));
    }

    /// A common name that names two classifier species identifies neither, and
    /// must not resolve to whichever one the iteration reached first.
    #[test]
    fn an_ambiguous_common_name_matches_nothing() {
        let metadata = labels(&[("Genus ambigua", "Little Brown Job")]);
        let classifier = labels(&[
            ("Alpha ambiguus", "Little Brown Job"),
            ("Beta ambiguus", "Little Brown Job"),
        ]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), None);
        assert_eq!(alignment.stats().unmatched, 1);
    }

    /// An exact scientific-name match owns its classifier species. A looser
    /// rule reaching the same species from another metadata row must not
    /// displace it, or the geomodel's probability for one species would decide
    /// another's admission.
    #[test]
    fn a_scientific_name_match_is_never_displaced_by_a_common_name_one() {
        // Ordered so the looser candidate is *first*, which is what would make
        // a single-pass implementation take it.
        let metadata = labels(&[
            ("Gryllotalpa australis", "Southern Mole Cricket"),
            ("Scapteriscus borellii", "Southern Mole Cricket"),
        ]);
        let classifier = labels(&[("Scapteriscus borellii", "Southern Mole Cricket")]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), None);
        assert_eq!(alignment.classifier_name(1), Some("Scapteriscus borellii"));
        assert_eq!(alignment.stats().by_scientific_name, 1);
        assert_eq!(alignment.stats().by_common_name, 0);
    }

    /// The counterpart, where the epithet guard cannot help: two metadata rows
    /// agree on the common name *and* the epithet, and one of them already
    /// matched the classifier species outright. The looser rule must not add a
    /// second route to a species the stricter rule already reached.
    #[test]
    fn a_common_name_match_onto_an_already_matched_species_is_refused() {
        let metadata = labels(&[
            ("Leuconotopicus villosus", "Hairy Woodpecker"),
            ("Dryobates villosus", "Hairy Woodpecker"),
        ]);
        let classifier = labels(&[("Dryobates villosus", "Hairy Woodpecker")]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(1), Some("Dryobates villosus"));
        assert_eq!(
            alignment.classifier_name(0),
            None,
            "the epithets agree here, so nothing but the already-claimed check \
             stands between this and a second route to one species"
        );
        assert_eq!(alignment.stats().by_scientific_name, 1);
        assert_eq!(alignment.stats().by_common_name, 0);
        assert_eq!(alignment.stats().unmatched, 1);
    }

    /// The scientific name still wins first, and its result is the
    /// classifier's spelling rather than the metadata file's — the two differ
    /// in case often enough, and the passing set is compared with exact
    /// string equality downstream.
    #[test]
    fn a_scientific_match_returns_the_classifiers_spelling() {
        let metadata = labels(&[("TURDUS MERULA", "Eurasian Blackbird")]);
        let classifier = labels(&[("Turdus merula", "Eurasian Blackbird")]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), Some("Turdus merula"));
        assert_eq!(alignment.stats().by_scientific_name, 1);
    }

    /// Family- and genus-level labels have no specific epithet. Two of them
    /// must not match each other on the strength of both having none.
    #[test]
    fn labels_without_an_epithet_do_not_match_on_the_common_name_alone() {
        let metadata = labels(&[("Acrididae", "Short-horned Grasshopper")]);
        let classifier = labels(&[("Tettigoniidae", "Short-horned Grasshopper")]);
        let alignment = Alignment::build(&metadata, &classifier, &no_aliases());

        assert_eq!(alignment.classifier_name(0), None);
    }

    /// The operator's own map, for the species the automatic rules cannot
    /// reach — here a pair that shares neither the scientific name nor the
    /// common name.
    #[test]
    fn an_operator_alias_resolves_what_neither_rule_can() {
        let metadata = labels(&[("Streptopelia senegalensis", "Laughing Dove")]);
        let classifier = labels(&[("Spilopelia senegalensis", "Palm Dove")]);

        let unaided = Alignment::build(&metadata, &classifier, &no_aliases());
        assert_eq!(
            unaided.classifier_name(0),
            None,
            "different genus and different common name: nothing automatic can \
             connect these two, which is what the alias file is for"
        );

        let (aliases, skipped) = parse_alias_file(
            "# operator's file\n\
             Streptopelia senegalensis\tSpilopelia senegalensis\n\
             \n\
             not a mapping\n",
        );
        assert_eq!(skipped, 1, "the line without a tab is skipped, not fatal");
        let aided = Alignment::build(&metadata, &classifier, &aliases);
        assert_eq!(
            aided.classifier_name(0),
            Some("Spilopelia senegalensis"),
            "got {:?}",
            aided.stats()
        );
        assert_eq!(aided.stats().by_alias, 1);
    }

    /// An alias file's own degenerate lines: a self-alias is a no-op and an
    /// empty column names nothing. Neither may become a mapping that makes
    /// `classifier_name` return something for a name it should not.
    #[test]
    fn a_degenerate_alias_line_is_skipped_rather_than_stored() {
        let (map, skipped) = parse_alias_file(
            "Turdus merula\tTurdus merula\n\
             \tSpilopelia senegalensis\n\
             Streptopelia senegalensis\t\n\
             Genus one\tGenus two\n",
        );
        assert_eq!(map.len(), 1, "only the real mapping survives: {map:?}");
        assert_eq!(skipped, 3);
        assert_eq!(map.get("genus one").map(String::as_str), Some("genus two"));
    }

    /// The counts are the whole reason this is not silent, so they are
    /// asserted rather than assumed — including both directions of "did not
    /// match", which fail differently and have different remedies.
    #[test]
    fn the_stats_count_every_row_exactly_once() {
        let metadata = labels(&[
            ("Turdus merula", "Eurasian Blackbird"),         // scientific
            ("Leuconotopicus villosus", "Hairy Woodpecker"), // common name
            ("Streptopelia senegalensis", "Laughing Dove"),  // alias
            ("Corvus corax", "Common Raven"),                // unmatched
        ]);
        let classifier = labels(&[
            ("Turdus merula", "Eurasian Blackbird"),
            ("Dryobates villosus", "Hairy Woodpecker"),
            ("Spilopelia senegalensis", "Palm Dove"),
            ("Parus major", "Great Tit"),
            ("Cyanistes caeruleus", "Eurasian Blue Tit"),
        ]);
        let (aliases, _) = parse_alias_file("Streptopelia senegalensis\tSpilopelia senegalensis\n");
        let stats = Alignment::build(&metadata, &classifier, &aliases)
            .stats()
            .clone();

        assert_eq!(stats.metadata_species, 4);
        assert_eq!(stats.classifier_species, 5);
        assert_eq!(stats.by_scientific_name, 1);
        assert_eq!(stats.by_common_name, 1);
        assert_eq!(stats.by_alias, 1);
        assert_eq!(stats.unmatched, 1);
        assert_eq!(stats.matched() + stats.unmatched, stats.metadata_species);
        assert_eq!(
            stats.classifier_unreachable, 2,
            "Parus major and Cyanistes caeruleus have no metadata row, so the \
             occurrence filter can never admit them: {stats:?}"
        );
        assert_eq!(
            stats.recovered_sample,
            vec!["Leuconotopicus villosus = Dryobates villosus".to_owned()],
            "the sample names what the looser rule did, so an operator can \
             judge it"
        );
    }

    /// What the gender stem may and may not collapse.
    ///
    /// *alba* / *albus* are the same adjective in two genders and must agree;
    /// the floor is what stops the rule reaching epithets short enough that
    /// removing an ending leaves nothing distinguishing behind.
    #[test]
    fn the_gender_stem_collapses_endings_and_stops_at_a_three_letter_stem() {
        for (feminine, masculine) in [
            ("alba", "albus"),
            ("gymnocerca", "gymnocercus"),
            ("modesta", "modestum"),
            ("spiloptera", "spilopterus"),
        ] {
            assert_eq!(
                gender_stem(feminine),
                gender_stem(masculine),
                "{feminine} and {masculine} are one epithet in two genders"
            );
        }
        assert_eq!(gender_stem("kirkii"), "kirk");
        assert_eq!(gender_stem("cassini"), "cassin");

        // Below the floor nothing is stripped, so two three-letter epithets
        // stay distinct instead of collapsing to a one- or two-letter stem.
        assert_eq!(gender_stem("ala"), "ala");
        assert_eq!(gender_stem("ova"), "ova");
        assert!(!epithets_agree("Genus ala", "Genus ova"));

        // And the epithet is a veto, not an identifier: it only ever narrows a
        // match the common name already proposed.
        assert!(!epithets_agree("Genus", "Genus"));
        assert!(!epithets_agree("Genus species", "Genus"));
    }
}
