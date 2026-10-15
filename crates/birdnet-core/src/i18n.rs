//! Species common names in the station's own language.
//!
//! A detection is stored under the classifier's names: `Com_Name` and
//! `Sci_Name` are how the database, the species lists, the thresholds and the
//! alert rules say which bird they mean. This module never touches them. It
//! answers a different question, *what should a person be shown for this
//! species*, and the answer is looked up at the moment a name is drawn.
//!
//! That is a deliberate departure from BirdNET-Pi and birdnet-go, which load a
//! translated label file and so store the translated name. Doing that here
//! would have made the language a property of the data: switch from English to
//! German and last year's *Eurasian Blackbird* and this year's *Amsel* become
//! two species in every list that groups by name, and an exclusion typed as
//! "Dog" stops matching anything, because the species filter drops an entry
//! that names nothing in the (now German) label set. With the stored name left
//! alone, the language is a setting, not a migration.
//!
//! # Where the names come from
//!
//! `data/l18n/species_names.tsv`, compiled into the binary: the 35 non-English
//! tables of BirdNET-Pi's `model/l18n/` (Nachtzuster's fork, commit `88985a3`),
//! reduced to the cells that are genuine translations. `data/l18n/README.md`
//! says whose work they are, under what licence, and what the table does not
//! contain; `scripts/build-species-names.py` rebuilds it, and
//! `tests/species_names_table.rs` holds it to the numbers in `SOURCES.tsv`.
//!
//! Coverage differs a great deal by language: 98% of the table's species have a
//! Swedish name and 9% have a Korean one. A species with no name in the chosen
//! language keeps the classifier's own, so a station set to Korean shows mostly
//! English. [`NameLocalizer::coverage`] counts it against the installed
//! classifier's label set so that nobody has to guess.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::inference::labels::LabelSet;

/// The bundled table: a header row (`sci_name`, then one column per language
/// except English), then one row per species with at least one translation.
/// An empty cell means the language has no name of its own for the species.
const TABLE: &str = include_str!("../data/l18n/species_names.tsv");

/// The code of the language the classifier's own names are in. It has no
/// column in the table: choosing it means "show what is stored".
pub const ENGLISH: &str = "en";

/// A language species common names can be shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    /// The code a station is configured with (`DATABASE_LANG`): an ISO 639-1
    /// code, or `zh_CN` / `zh_TW`, which BirdNET-Pi spells that way.
    pub code: &'static str,
    /// The language's name in English.
    pub english_name: &'static str,
    /// The language's name in itself, for a person looking for theirs.
    pub native_name: &'static str,
}

/// Every language a station can choose, in code order.
pub const LANGUAGES: &[Language] = &[
    Language {
        code: "af",
        english_name: "Afrikaans",
        native_name: "Afrikaans",
    },
    Language {
        code: "ar",
        english_name: "Arabic",
        native_name: "العربية",
    },
    Language {
        code: "ca",
        english_name: "Catalan",
        native_name: "Català",
    },
    Language {
        code: "cs",
        english_name: "Czech",
        native_name: "Čeština",
    },
    Language {
        code: "da",
        english_name: "Danish",
        native_name: "Dansk",
    },
    Language {
        code: "de",
        english_name: "German",
        native_name: "Deutsch",
    },
    Language {
        code: "en",
        english_name: "English",
        native_name: "English",
    },
    Language {
        code: "es",
        english_name: "Spanish",
        native_name: "Español",
    },
    Language {
        code: "et",
        english_name: "Estonian",
        native_name: "Eesti",
    },
    Language {
        code: "fi",
        english_name: "Finnish",
        native_name: "Suomi",
    },
    Language {
        code: "fr",
        english_name: "French",
        native_name: "Français",
    },
    Language {
        code: "hr",
        english_name: "Croatian",
        native_name: "Hrvatski",
    },
    Language {
        code: "hu",
        english_name: "Hungarian",
        native_name: "Magyar",
    },
    Language {
        code: "id",
        english_name: "Indonesian",
        native_name: "Bahasa Indonesia",
    },
    Language {
        code: "is",
        english_name: "Icelandic",
        native_name: "Íslenska",
    },
    Language {
        code: "it",
        english_name: "Italian",
        native_name: "Italiano",
    },
    Language {
        code: "ja",
        english_name: "Japanese",
        native_name: "日本語",
    },
    Language {
        code: "ko",
        english_name: "Korean",
        native_name: "한국어",
    },
    Language {
        code: "lt",
        english_name: "Lithuanian",
        native_name: "Lietuvių",
    },
    Language {
        code: "lv",
        english_name: "Latvian",
        native_name: "Latviešu",
    },
    Language {
        code: "nl",
        english_name: "Dutch",
        native_name: "Nederlands",
    },
    Language {
        code: "no",
        english_name: "Norwegian",
        native_name: "Norsk",
    },
    Language {
        code: "pl",
        english_name: "Polish",
        native_name: "Polski",
    },
    Language {
        code: "pt",
        english_name: "Portuguese",
        native_name: "Português",
    },
    Language {
        code: "ro",
        english_name: "Romanian",
        native_name: "Română",
    },
    Language {
        code: "ru",
        english_name: "Russian",
        native_name: "Русский",
    },
    Language {
        code: "sk",
        english_name: "Slovak",
        native_name: "Slovenčina",
    },
    Language {
        code: "sl",
        english_name: "Slovenian",
        native_name: "Slovenščina",
    },
    Language {
        code: "sr",
        english_name: "Serbian",
        native_name: "Српски",
    },
    Language {
        code: "sv",
        english_name: "Swedish",
        native_name: "Svenska",
    },
    Language {
        code: "th",
        english_name: "Thai",
        native_name: "ไทย",
    },
    Language {
        code: "tr",
        english_name: "Turkish",
        native_name: "Türkçe",
    },
    Language {
        code: "uk",
        english_name: "Ukrainian",
        native_name: "Українська",
    },
    Language {
        code: "vi",
        english_name: "Vietnamese",
        native_name: "Tiếng Việt",
    },
    Language {
        code: "zh_CN",
        english_name: "Chinese (Simplified)",
        native_name: "简体中文",
    },
    Language {
        code: "zh_TW",
        english_name: "Chinese (Traditional)",
        native_name: "繁體中文",
    },
];

/// The language for a configured value, or `None` if it names none we have.
///
/// Case and `-` versus `_` do not matter (`zh-cn`, `ZH_CN`), and a regional tag
/// resolves to its base language when that is one we have (`de_DE`, `pt-BR`,
/// `en_GB`). Chinese has no base language here: its two scripts are two tables
/// (`zh_CN`, `zh_TW`), so a bare `zh` or a tag like `zh_HK` is not guessed into
/// one of them.
#[must_use]
pub fn find_language(raw: &str) -> Option<&'static Language> {
    let wanted = raw.trim().replace('-', "_");
    let exact = |code: &str| LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code));
    if let Some(language) = exact(&wanted) {
        return Some(language);
    }
    let base = wanted.split('_').next().filter(|b| !b.is_empty())?;
    exact(base)
}

/// Species name translations for a single language.
#[derive(Debug, Clone)]
pub struct LanguagePack {
    language: &'static Language,
    /// Map from scientific name to localized common name.
    translations: HashMap<String, String>,
}

impl LanguagePack {
    /// The bundled names for a language.
    ///
    /// English yields an empty pack: its names are the classifier's, which is
    /// what a station set to English shows.
    ///
    /// # Errors
    ///
    /// [`I18nError::UnsupportedLanguage`] if `code` is not one of [`LANGUAGES`]
    /// (matched as [`find_language`] matches).
    pub fn bundled(code: &str) -> Result<Self, I18nError> {
        let language =
            find_language(code).ok_or_else(|| I18nError::UnsupportedLanguage(code.to_owned()))?;
        let mut lines = TABLE.lines();
        let header = lines.next().ok_or_else(|| {
            I18nError::Parse("the bundled species-name table is empty".to_owned())
        })?;
        let mut translations = HashMap::new();
        if language.code != ENGLISH {
            let column = header
                .split('\t')
                .position(|c| c == language.code)
                .ok_or_else(|| {
                    I18nError::Parse(format!(
                        "the bundled species-name table has no {} column",
                        language.code
                    ))
                })?;
            for line in lines {
                let mut cells = line.split('\t');
                let (Some(scientific), Some(name)) = (cells.next(), cells.nth(column - 1)) else {
                    continue;
                };
                if !name.is_empty() {
                    translations.insert(scientific.to_owned(), name.to_owned());
                }
            }
        }
        tracing::debug!(
            lang = language.code,
            species_count = translations.len(),
            "loaded bundled species names"
        );
        Ok(Self {
            language,
            translations,
        })
    }

    /// Load names from an operator-supplied `BirdNET` labels file.
    ///
    /// Searches the given directory for, in order:
    /// 1. `{labels_dir}/{lang_code}_labels.txt`
    /// 2. `{labels_dir}/labels_l18n/{lang_code}_labels.txt`
    ///
    /// # File format
    ///
    /// One line per species: `"Scientific Name_Common Name"` (underscore-separated).
    /// For example: `"Turdus merula_Amsel"` (German).
    ///
    /// # Errors
    ///
    /// [`I18nError::UnsupportedLanguage`] if the code is not one of
    /// [`LANGUAGES`]; [`I18nError::FileNotFound`] if no labels file is found;
    /// [`I18nError::Parse`] if it holds no entry.
    pub fn load(lang_code: &str, labels_dir: &Path) -> Result<Self, I18nError> {
        let language = find_language(lang_code)
            .ok_or_else(|| I18nError::UnsupportedLanguage(lang_code.to_owned()))?;

        let filename = format!("{}_labels.txt", language.code);

        // Try direct path first, then labels_l18n subdirectory
        let candidates = [
            labels_dir.join(&filename),
            labels_dir.join("labels_l18n").join(&filename),
        ];

        let file_path = candidates
            .iter()
            .find(|p| p.is_file())
            .ok_or_else(|| I18nError::FileNotFound(filename.clone()))?;

        let content = std::fs::read_to_string(file_path)
            .map_err(|e| I18nError::FileNotFound(format!("{}: {e}", file_path.display())))?;

        let translations = parse_labels(&content)?;

        tracing::debug!(
            lang = language.code,
            species_count = translations.len(),
            path = %file_path.display(),
            "loaded language pack"
        );

        Ok(Self {
            language,
            translations,
        })
    }

    /// The bundled names for a language, with an operator's labels file laid
    /// over them where the directory has one.
    ///
    /// A directory that holds no file for the language is the ordinary case and
    /// changes nothing. One whose file cannot be used is reported through
    /// `on_override_error` and skipped, so a typo in an optional override never
    /// takes the bundled names away.
    ///
    /// # Errors
    ///
    /// As [`LanguagePack::bundled`].
    pub fn bundled_with_overrides(
        code: &str,
        labels_dir: Option<&Path>,
        on_override_error: impl FnOnce(&I18nError),
    ) -> Result<Self, I18nError> {
        let mut pack = Self::bundled(code)?;
        if let Some(dir) = labels_dir {
            match Self::load(code, dir) {
                Ok(over) => pack.translations.extend(over.translations),
                Err(I18nError::FileNotFound(_)) => {}
                Err(e) => on_override_error(&e),
            }
        }
        Ok(pack)
    }

    /// Create a language pack from an in-memory map (for testing).
    #[cfg(test)]
    fn from_map(lang_code: &str, translations: HashMap<String, String>) -> Self {
        Self {
            language: find_language(lang_code).expect("a language of the registry"),
            translations,
        }
    }

    /// Get localized name for a species, falling back to the original name.
    pub fn translate<'a>(&'a self, sci_name: &'a str) -> &'a str {
        self.translations
            .get(sci_name)
            .map_or(sci_name, String::as_str)
    }

    /// The language this pack is for.
    pub const fn language(&self) -> &'static Language {
        self.language
    }

    /// Get the language code.
    pub const fn lang_code(&self) -> &'static str {
        self.language.code
    }

    /// Get the number of translated species.
    pub fn species_count(&self) -> usize {
        self.translations.len()
    }
}

/// How much of a classifier's label set a language has names for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    /// Labels the language has a name for.
    pub named: usize,
    /// Labels in the set.
    pub of: usize,
}

/// Resolves the name stored for a species to the name to show.
///
/// Built once at startup from a [`LanguagePack`] and the classifier's label set,
/// then shared read-only. See the [module](self) for why the stored name is left
/// alone.
#[derive(Debug, Clone)]
pub struct NameLocalizer {
    language: &'static Language,
    by_scientific: HashMap<String, String>,
    /// Stored common name to translation, for the places that hold only the
    /// name. A stored name two species share, with different translations, is
    /// absent: showing either would mislabel the other.
    by_stored: HashMap<String, String>,
    /// Stored names two species share with different translations; kept so that
    /// names added later cannot quietly re-introduce one.
    ambiguous: HashSet<String>,
    /// How many of the classifier's species each language has a name for, from
    /// the bundled table alone, so a station can see what another language
    /// would give it before choosing it. Empty without a label set.
    named_by_language: HashMap<&'static str, usize>,
    /// How many of the classifier's species the *active* pack names, which can
    /// exceed the table's count for the language when an operator's labels file
    /// added names.
    active_named: usize,
    /// Species in the classifier's label set; 0 without one.
    label_count: usize,
}

impl NameLocalizer {
    /// Shows every name as stored, and knows nothing about a classifier. What a
    /// station uses until it has been told otherwise.
    ///
    /// # Panics
    ///
    /// Never in a build that passes its tests: English is a row of the
    /// registry, and `english_shows_every_name_as_stored` constructs this on
    /// every run.
    #[must_use]
    pub fn english() -> Self {
        Self {
            language: find_language(ENGLISH).expect("English is in the registry"),
            by_scientific: HashMap::new(),
            by_stored: HashMap::new(),
            ambiguous: HashSet::new(),
            named_by_language: HashMap::new(),
            active_named: 0,
            label_count: 0,
        }
    }

    /// Names in the pack's language, for the classifier whose species are in
    /// `labels` (or for none, which leaves [`Self::coverage`] unanswerable and
    /// resolves by scientific name only).
    #[must_use]
    pub fn new(pack: LanguagePack, labels: Option<&LabelSet>) -> Self {
        let active_named = labels.map_or(0, |set| {
            set.iter()
                .filter(|l| pack.translations.contains_key(&l.scientific_name))
                .count()
        });
        let mut localizer = Self {
            language: pack.language,
            by_scientific: pack.translations,
            by_stored: HashMap::new(),
            ambiguous: HashSet::new(),
            named_by_language: labels.map(bundled_names_per_language).unwrap_or_default(),
            active_named,
            label_count: labels.map_or(0, |l| l.iter().count()),
        };
        localizer.index_stored(
            labels
                .into_iter()
                .flat_map(LabelSet::iter)
                .map(|l| (l.scientific_name.as_str(), l.common_name.as_str())),
        );
        localizer
    }

    /// Also resolve the names that are actually stored in the station's
    /// database, given as `(scientific name, stored common name)` pairs.
    ///
    /// The classifier's label file is not the only source of stored names: a
    /// BirdNET-Pi import carries that station's names, and a station that has
    /// changed model keeps rows named by the old one. Without them a page that
    /// holds only the stored name (a species count, a companion list) could not
    /// be translated for exactly those rows.
    #[must_use]
    pub fn with_observed_names<'a>(
        mut self,
        observed: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        self.index_stored(observed);
        self
    }

    /// Add `stored name → translation` for each `(scientific, stored)` pair the
    /// language has a name for. A stored name two species share, with different
    /// translations, is dropped and stays dropped.
    fn index_stored<'a>(&mut self, pairs: impl IntoIterator<Item = (&'a str, &'a str)>) {
        for (scientific, stored) in pairs {
            let Some(name) = self.by_scientific.get(scientific) else {
                continue;
            };
            if stored == scientific || self.ambiguous.contains(stored) {
                continue;
            }
            match self.by_stored.get(stored) {
                Some(seen) if seen != name => {
                    self.by_stored.remove(stored);
                    self.ambiguous.insert(stored.to_owned());
                }
                Some(_) => {}
                None => {
                    self.by_stored.insert(stored.to_owned(), name.clone());
                }
            }
        }
    }

    /// The language names are shown in.
    #[must_use]
    pub const fn language(&self) -> &'static Language {
        self.language
    }

    /// `true` when every name is shown as stored.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.by_scientific.is_empty()
    }

    /// How many of the classifier's species have a name in the language being
    /// shown, operator-supplied names included. `None` when no label file was
    /// available to count against, or the language is English.
    #[must_use]
    pub fn coverage(&self) -> Option<Coverage> {
        if self.label_count == 0 || self.is_identity() {
            return None;
        }
        Some(Coverage {
            named: self.active_named,
            of: self.label_count,
        })
    }

    /// The same count for any bundled language, from the table alone, so the
    /// Settings page can show what each choice would give this station.
    /// `None` for English, for an unknown code, and without a label file.
    #[must_use]
    pub fn coverage_of(&self, code: &str) -> Option<Coverage> {
        let language = find_language(code)?;
        if self.label_count == 0 || language.code == ENGLISH {
            return None;
        }
        let named = if language.code == self.language.code {
            self.active_named
        } else {
            self.named_by_language
                .get(language.code)
                .copied()
                .unwrap_or(0)
        };
        Some(Coverage {
            named,
            of: self.label_count,
        })
    }

    /// The name to show for a species, given its scientific name and the common
    /// name stored with it. The stored name when the language has none.
    #[must_use]
    pub fn name<'a>(&'a self, scientific: &str, stored: &'a str) -> &'a str {
        self.by_scientific
            .get(scientific)
            .or_else(|| self.by_stored.get(stored))
            .map_or(stored, String::as_str)
    }

    /// The name to show where only the stored common name is at hand.
    #[must_use]
    pub fn name_of_stored<'a>(&'a self, stored: &'a str) -> &'a str {
        self.by_stored.get(stored).map_or(stored, String::as_str)
    }

    /// The scientific name of the one species whose name in this language is
    /// exactly `shown`, ignoring case. `None` when none is, and when more than
    /// one is: two species can share a name in a language (68 pairs in the German
    /// table), and a typed name that could mean either must not pick one.
    #[must_use]
    pub fn scientific_name_for(&self, shown: &str) -> Option<&str> {
        let shown = shown.trim().to_lowercase();
        if shown.is_empty() {
            return None;
        }
        let mut hits = self
            .by_scientific
            .iter()
            .filter(|(_, name)| name.to_lowercase() == shown)
            .map(|(sci, _)| sci.as_str());
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    }

    /// The scientific names of the species whose name *in this language*
    /// contains `needle`, ignoring case, in scientific-name order.
    ///
    /// What lets a German station find *Kohlmeise* by typing "kohl": the stored
    /// names are English, so a search that only looks at them finds nothing.
    /// Empty for English, whose names are the stored ones and are searched as
    /// they always were, and for an empty `needle`.
    #[must_use]
    pub fn scientific_names_matching(&self, needle: &str) -> Vec<&str> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<&str> = self
            .by_scientific
            .iter()
            .filter(|(_, name)| name.to_lowercase().contains(&needle))
            .map(|(sci, _)| sci.as_str())
            .collect();
        found.sort_unstable();
        found
    }
}

/// For each language in the bundled table, how many of `labels`' species it has
/// a name for. One pass over the table.
fn bundled_names_per_language(labels: &LabelSet) -> HashMap<&'static str, usize> {
    let species: HashSet<&str> = labels.iter().map(|l| l.scientific_name.as_str()).collect();
    let mut lines = TABLE.lines();
    let codes: Vec<&'static str> = lines
        .next()
        .map(|h| h.split('\t').skip(1).collect())
        .unwrap_or_default();
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for line in lines {
        let mut cells = line.split('\t');
        if !cells.next().is_some_and(|sci| species.contains(sci)) {
            continue;
        }
        for (code, cell) in codes.iter().zip(cells) {
            if !cell.is_empty() {
                *counts.entry(code).or_default() += 1;
            }
        }
    }
    counts
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from the i18n subsystem.
#[derive(Debug)]
pub enum I18nError {
    /// Labels file not found at expected path.
    FileNotFound(String),
    /// Error parsing a labels file.
    Parse(String),
    /// The requested language code is not in `LANGUAGES`.
    UnsupportedLanguage(String),
}

impl fmt::Display for I18nError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileNotFound(path) => write!(f, "labels file not found: {path}"),
            Self::Parse(msg) => write!(f, "labels parse error: {msg}"),
            Self::UnsupportedLanguage(code) => write!(f, "unsupported language: {code}"),
        }
    }
}

impl std::error::Error for I18nError {}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a `BirdNET` labels file into a map of scientific name to common name.
///
/// Each line is `"Scientific Name_Common Name"`. Lines that are empty, start with
/// `#`, or lack an underscore separator are skipped.
fn parse_labels(content: &str) -> Result<HashMap<String, String>, I18nError> {
    let mut map = HashMap::new();
    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on the first underscore that separates scientific name from common name.
        // Scientific names are always two words (genus + species), so find the underscore
        // after the scientific name portion.
        if let Some(sep_pos) = find_label_separator(line) {
            let sci_name = line[..sep_pos].trim();
            let common_name = line[sep_pos + 1..].trim();
            if !sci_name.is_empty() && !common_name.is_empty() {
                map.insert(sci_name.to_owned(), common_name.to_owned());
            }
        } else {
            tracing::trace!(
                line_num = line_num + 1,
                line,
                "skipping label line without separator"
            );
        }
    }
    if map.is_empty() {
        return Err(I18nError::Parse(
            "no valid entries found in labels file".to_owned(),
        ));
    }
    Ok(map)
}

/// Find the position of the underscore that separates the scientific name from the
/// common name in a `BirdNET` label line.
///
/// `BirdNET` label format: `"Genus species_Common Name"`.
/// We look for the first underscore that follows the "Genus species" pattern (i.e.,
/// after at least one space and one more word).
fn find_label_separator(line: &str) -> Option<usize> {
    // Find the first space (between genus and species)
    let first_space = line.find(' ')?;
    // Find the underscore after the species name
    line[first_space..].find('_').map(|pos| first_space + pos)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::io::Write as _;

    /// A label set from `(scientific, common)` pairs, in the V2.4 text format.
    fn labels(pairs: &[(&str, &str)]) -> LabelSet {
        let mut text = String::new();
        for (sci, com) in pairs {
            writeln!(text, "{sci}_{com}").expect("writing to a String cannot fail");
        }
        LabelSet::load_from_str(&text).expect("a valid label set")
    }

    fn pack(code: &str, pairs: &[(&str, &str)]) -> LanguagePack {
        LanguagePack::from_map(
            code,
            pairs
                .iter()
                .map(|(s, n)| ((*s).to_owned(), (*n).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn registry_is_the_thirty_six_birdnet_pi_languages_in_code_order() {
        assert_eq!(LANGUAGES.len(), 36);
        let codes: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(codes, sorted, "code order, no duplicates");
        assert!(LANGUAGES.iter().all(|l| !l.native_name.is_empty()));
    }

    #[test]
    fn find_language_by_code_ignoring_case_and_separator() {
        assert_eq!(find_language("de").unwrap().code, "de");
        assert_eq!(find_language(" DE ").unwrap().code, "de");
        assert_eq!(find_language("zh_CN").unwrap().code, "zh_CN");
        assert_eq!(find_language("zh-tw").unwrap().code, "zh_TW");
        assert_eq!(find_language("EN").unwrap().code, "en");
    }

    #[test]
    fn find_language_resolves_a_regional_tag_to_its_base_language() {
        assert_eq!(find_language("de_DE").unwrap().code, "de");
        assert_eq!(find_language("pt-BR").unwrap().code, "pt");
        assert_eq!(find_language("en_GB").unwrap().code, "en");
    }

    #[test]
    fn find_language_does_not_guess_a_chinese_script() {
        assert!(find_language("zh").is_none());
        assert!(find_language("zh_HK").is_none());
        assert!(find_language("zh-Hans").is_none());
    }

    #[test]
    fn find_language_rejects_what_is_not_a_language_we_have() {
        for raw in ["", " ", "xx", "eng", "_", "-de", "deu", "nb_NO"] {
            assert!(find_language(raw).is_none(), "{raw:?}");
        }
    }

    #[test]
    fn bundled_german_has_the_names_a_german_birder_expects() {
        let de = LanguagePack::bundled("de").unwrap();
        assert_eq!(de.translate("Turdus merula"), "Amsel");
        assert_eq!(de.translate("Parus major"), "Kohlmeise");
        assert_eq!(de.translate("Pica pica"), "Elster");
        assert_eq!(de.lang_code(), "de");
    }

    #[test]
    fn bundled_names_are_in_the_script_of_their_language() {
        for (code, sci, name) in [
            ("ja", "Pica pica", "ユーラシアカササギ"),
            ("ru", "Turdus merula", "Чёрный дрозд"),
            ("ko", "Pica pica", "까치"),
            ("fr", "Parus major", "Mésange charbonnière"),
        ] {
            let pack = LanguagePack::bundled(code).unwrap();
            assert_eq!(pack.translate(sci), name, "{code}");
        }
    }

    #[test]
    fn bundled_english_is_empty_because_the_stored_names_are_english() {
        let en = LanguagePack::bundled("en").unwrap();
        assert_eq!(en.species_count(), 0);
        assert_eq!(en.translate("Turdus merula"), "Turdus merula");
    }

    #[test]
    fn bundled_rejects_an_unknown_language() {
        assert!(matches!(
            LanguagePack::bundled("xx"),
            Err(I18nError::UnsupportedLanguage(_))
        ));
    }

    #[test]
    fn bundled_with_overrides_lays_a_labels_file_over_the_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(tmp.path().join("de_labels.txt")).unwrap();
        writeln!(f, "Turdus merula_Schwarzdrossel").unwrap();
        writeln!(f, "Zzz zzz_Eigener Name").unwrap();

        let pack = LanguagePack::bundled_with_overrides("de", Some(tmp.path()), |e| panic!("{e}"))
            .unwrap();
        assert_eq!(
            pack.translate("Turdus merula"),
            "Schwarzdrossel",
            "the override wins"
        );
        assert_eq!(pack.translate("Zzz zzz"), "Eigener Name", "and may add");
        assert_eq!(
            pack.translate("Parus major"),
            "Kohlmeise",
            "the rest is the bundle"
        );
    }

    #[test]
    fn a_directory_without_a_file_for_the_language_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = LanguagePack::bundled_with_overrides("de", Some(tmp.path()), |e| panic!("{e}"))
            .unwrap();
        assert_eq!(pack.translate("Turdus merula"), "Amsel");
    }

    #[test]
    fn an_unusable_override_is_reported_and_leaves_the_bundle_intact() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("de_labels.txt"), "# nothing usable\n").unwrap();
        let mut reported = Vec::new();
        let pack = LanguagePack::bundled_with_overrides("de", Some(tmp.path()), |e| {
            reported.push(e.to_string());
        })
        .unwrap();
        assert_eq!(reported.len(), 1);
        assert_eq!(pack.translate("Turdus merula"), "Amsel");
    }

    #[test]
    fn localizer_prefers_the_scientific_name_and_falls_back_to_the_stored_one() {
        let set = labels(&[
            ("Turdus merula", "Common Blackbird"),
            ("Parus major", "Great Tit"),
        ]);
        let loc = NameLocalizer::new(pack("de", &[("Turdus merula", "Amsel")]), Some(&set));
        assert_eq!(loc.name("Turdus merula", "Common Blackbird"), "Amsel");
        // No German name: what is stored.
        assert_eq!(loc.name("Parus major", "Great Tit"), "Great Tit");
        // A species the labels never heard of, stored under an older name.
        assert_eq!(loc.name("Unknown sp", "Whatever"), "Whatever");
        assert!(!loc.is_identity());
    }

    #[test]
    fn the_scientific_name_outranks_a_stored_name_that_now_belongs_to_another_species() {
        // A row recorded as `Cus dus` under a name the label set has since given
        // to `Aus bus`: the scientific name says which bird it is.
        let set = labels(&[("Aus bus", "Old Name"), ("Cus dus", "Other Name")]);
        let loc = NameLocalizer::new(
            pack("de", &[("Aus bus", "Eins"), ("Cus dus", "Zwei")]),
            Some(&set),
        );
        assert_eq!(loc.name("Cus dus", "Old Name"), "Zwei");
        // With a scientific name the pack does not know, the stored name still
        // resolves, through the label set.
        assert_eq!(loc.name("Unknown sp", "Old Name"), "Eins");
    }

    #[test]
    fn localizer_resolves_a_bare_stored_name_through_the_label_set() {
        let set = labels(&[("Turdus merula", "Common Blackbird")]);
        let loc = NameLocalizer::new(pack("de", &[("Turdus merula", "Amsel")]), Some(&set));
        assert_eq!(loc.name_of_stored("Common Blackbird"), "Amsel");
        assert_eq!(loc.name_of_stored("Great Tit"), "Great Tit");
    }

    #[test]
    fn a_stored_name_two_species_share_is_not_guessed_by_name_alone() {
        let set = labels(&[
            ("Aus bus", "Shared Name"),
            ("Cus dus", "Shared Name"),
            ("Eus fus", "Lone Name"),
        ]);
        let loc = NameLocalizer::new(
            pack(
                "de",
                &[
                    ("Aus bus", "Eins"),
                    ("Cus dus", "Zwei"),
                    ("Eus fus", "Drei"),
                ],
            ),
            Some(&set),
        );
        // By name alone it is ambiguous, so it is left as stored...
        assert_eq!(loc.name_of_stored("Shared Name"), "Shared Name");
        // ...but each species is still right when its scientific name is known.
        assert_eq!(loc.name("Aus bus", "Shared Name"), "Eins");
        assert_eq!(loc.name("Cus dus", "Shared Name"), "Zwei");
        // And the unambiguous neighbour is unaffected.
        assert_eq!(loc.name_of_stored("Lone Name"), "Drei");
    }

    #[test]
    fn names_stored_by_an_import_or_an_old_model_resolve_once_they_are_observed() {
        // The label set the station runs now names the bird "Common Blackbird";
        // rows imported from a BirdNET-Pi station say "Eurasian Blackbird".
        let set = labels(&[("Turdus merula", "Common Blackbird")]);
        let de = || pack("de", &[("Turdus merula", "Amsel"), ("Aus bus", "Eins")]);
        let before = NameLocalizer::new(de(), Some(&set));
        assert_eq!(
            before.name_of_stored("Eurasian Blackbird"),
            "Eurasian Blackbird"
        );

        let after = NameLocalizer::new(de(), Some(&set)).with_observed_names([
            ("Turdus merula", "Eurasian Blackbird"),
            // A species the language has no name for adds nothing.
            ("Zzz zzz", "Mystery Bird"),
            // A row whose stored name is just the scientific name adds nothing.
            ("Aus bus", "Aus bus"),
        ]);
        assert_eq!(after.name_of_stored("Eurasian Blackbird"), "Amsel");
        assert_eq!(
            after.name_of_stored("Common Blackbird"),
            "Amsel",
            "the label set still counts"
        );
        assert_eq!(after.name_of_stored("Mystery Bird"), "Mystery Bird");
        assert_eq!(after.name_of_stored("Aus bus"), "Aus bus");
    }

    #[test]
    fn an_observed_name_two_species_share_stays_unresolved_even_if_seen_again() {
        let loc = NameLocalizer::new(
            pack("de", &[("Aus bus", "Eins"), ("Cus dus", "Zwei")]),
            None,
        )
        .with_observed_names([
            ("Aus bus", "Shared"),
            ("Cus dus", "Shared"),
            // Seen again: the second sighting must not bring the name back.
            ("Aus bus", "Shared"),
        ]);
        assert_eq!(loc.name_of_stored("Shared"), "Shared");
        assert_eq!(
            loc.name("Aus bus", "Shared"),
            "Eins",
            "by scientific name it is exact"
        );
    }

    #[test]
    fn two_species_sharing_a_stored_name_and_a_translation_stay_resolvable() {
        let set = labels(&[("Aus bus", "Shared Name"), ("Cus dus", "Shared Name")]);
        let loc = NameLocalizer::new(
            pack("de", &[("Aus bus", "Gleich"), ("Cus dus", "Gleich")]),
            Some(&set),
        );
        assert_eq!(loc.name_of_stored("Shared Name"), "Gleich");
    }

    #[test]
    fn a_term_finds_the_species_whose_name_in_the_language_contains_it() {
        let loc = NameLocalizer::new(LanguagePack::bundled("de").unwrap(), None);
        // A substring, so the great tit and the three other `…kohlmeise`.
        let found = loc.scientific_names_matching("kohlmeise");
        assert_eq!(
            found,
            [
                "Parus cinereus",
                "Parus major",
                "Parus minor",
                "Parus monticolus"
            ]
        );
        // Case does not matter, and a fragment finds every species it is part of.
        let tits = loc.scientific_names_matching("MEISE");
        assert!(tits.contains(&"Parus major") && tits.contains(&"Cyanistes caeruleus"));
        assert!(
            tits.len() > 5,
            "{} species have Meise in the name",
            tits.len()
        );
        // Sorted, so the answer does not depend on hash order.
        assert!(tits.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_typed_name_resolves_to_its_one_species_and_never_guesses_between_two() {
        let set = labels(&[("Aus bus", "One"), ("Cus dus", "Two"), ("Eus fus", "Three")]);
        let loc = NameLocalizer::new(
            pack(
                "de",
                &[
                    ("Aus bus", "Amsel"),
                    ("Cus dus", "Gleich"),
                    ("Eus fus", "gleich"),
                ],
            ),
            Some(&set),
        );
        assert_eq!(loc.scientific_name_for("amsel"), Some("Aus bus"));
        assert_eq!(loc.scientific_name_for("  AMSEL "), Some("Aus bus"));
        // Two species share it (case aside): not either.
        assert_eq!(loc.scientific_name_for("Gleich"), None);
        // A fragment is not a name; empty is not a name.
        assert_eq!(loc.scientific_name_for("Ams"), None);
        assert_eq!(loc.scientific_name_for(""), None);
        assert_eq!(NameLocalizer::english().scientific_name_for("Amsel"), None);
    }

    #[test]
    fn a_term_finds_nothing_in_english_or_when_empty_or_unmatched() {
        assert!(
            NameLocalizer::english()
                .scientific_names_matching("tit")
                .is_empty()
        );
        let de = NameLocalizer::new(LanguagePack::bundled("de").unwrap(), None);
        assert!(de.scientific_names_matching("").is_empty());
        assert!(de.scientific_names_matching("   ").is_empty());
        assert!(de.scientific_names_matching("zzzzqq").is_empty());
    }

    #[test]
    fn english_shows_every_name_as_stored() {
        let loc = NameLocalizer::english();
        assert!(loc.is_identity());
        assert_eq!(
            loc.name("Turdus merula", "Common Blackbird"),
            "Common Blackbird"
        );
        assert_eq!(loc.name_of_stored("Great Tit"), "Great Tit");
        assert_eq!(loc.language().code, "en");
    }

    #[test]
    fn coverage_counts_labels_with_a_name_not_names_in_the_pack() {
        let set = labels(&[
            ("Turdus merula", "Common Blackbird"),
            ("Parus major", "Great Tit"),
            ("Pica pica", "Eurasian Magpie"),
        ]);
        // The pack knows a species the classifier does not report: it must not count.
        let loc = NameLocalizer::new(
            pack("de", &[("Turdus merula", "Amsel"), ("Not reported", "X")]),
            Some(&set),
        );
        assert_eq!(loc.coverage(), Some(Coverage { named: 1, of: 3 }));
    }

    #[test]
    fn coverage_is_unanswerable_for_english_and_without_a_label_set() {
        let set = labels(&[("Turdus merula", "Common Blackbird")]);
        let english = NameLocalizer::new(LanguagePack::bundled("en").unwrap(), Some(&set));
        assert_eq!(english.coverage(), None);
        assert_eq!(english.coverage_of("en"), None);

        let no_labels = NameLocalizer::new(LanguagePack::bundled("de").unwrap(), None);
        assert!(
            !no_labels.is_identity(),
            "names still resolve by scientific name"
        );
        assert_eq!(no_labels.coverage(), None);
        assert_eq!(no_labels.coverage_of("fr"), None);
        assert_eq!(no_labels.name("Turdus merula", "Common Blackbird"), "Amsel");
    }

    #[test]
    fn a_station_in_english_can_see_what_another_language_would_give_it() {
        // Two species both German and Korean tables name, and one only German does
        // (`Acanthisitta chloris`: Grünstummelschwanz in German, nothing in Korean).
        let set = labels(&[
            ("Turdus merula", "Common Blackbird"),
            ("Acanthisitta chloris", "Rifleman"),
            ("Not in the table", "Whatever"),
        ]);
        let loc = NameLocalizer::new(LanguagePack::bundled("en").unwrap(), Some(&set));
        assert_eq!(loc.coverage_of("de"), Some(Coverage { named: 2, of: 3 }));
        assert_eq!(loc.coverage_of("ko"), Some(Coverage { named: 1, of: 3 }));
        assert_eq!(loc.coverage_of("xx"), None);
        // Regional spelling resolves like everywhere else.
        assert_eq!(loc.coverage_of("de_AT"), loc.coverage_of("de"));
    }

    #[test]
    fn the_active_language_counts_names_an_override_added() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("de_labels.txt"), "Zzz zzz_Eigener Name\n").unwrap();
        let set = labels(&[("Turdus merula", "Common Blackbird"), ("Zzz zzz", "Mine")]);
        let pack = LanguagePack::bundled_with_overrides("de", Some(tmp.path()), |e| panic!("{e}"))
            .unwrap();
        let loc = NameLocalizer::new(pack, Some(&set));
        // The bundle names Turdus merula; the operator's file names Zzz zzz.
        assert_eq!(loc.coverage(), Some(Coverage { named: 2, of: 2 }));
        assert_eq!(loc.coverage_of("de"), loc.coverage());
        // Another language is counted from the table alone.
        assert_eq!(loc.coverage_of("fr"), Some(Coverage { named: 1, of: 2 }));
    }

    #[test]
    fn parse_labels_basic() {
        let content = "Turdus merula_Eurasian Blackbird\nParus major_Great Tit\n";
        let map = parse_labels(content).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("Turdus merula").unwrap(), "Eurasian Blackbird");
        assert_eq!(map.get("Parus major").unwrap(), "Great Tit");
    }

    #[test]
    fn parse_labels_german() {
        let content = "Turdus merula_Amsel\nParus major_Kohlmeise\n";
        let map = parse_labels(content).unwrap();
        assert_eq!(map.get("Turdus merula").unwrap(), "Amsel");
        assert_eq!(map.get("Parus major").unwrap(), "Kohlmeise");
    }

    #[test]
    fn parse_labels_skips_comments_and_blanks() {
        let content = "# Header comment\n\nTurdus merula_Blackbird\n\n# Another comment\n";
        let map = parse_labels(content).unwrap();
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_labels_empty_returns_error() {
        let content = "# Only comments\n\n";
        let result = parse_labels(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_labels_common_name_with_underscore() {
        // Some common names might have underscores; we split on the first one after
        // the scientific name (which contains a space).
        let content = "Turdus merula_Eurasian_Blackbird\n";
        let map = parse_labels(content).unwrap();
        assert_eq!(map.get("Turdus merula").unwrap(), "Eurasian_Blackbird");
    }

    #[test]
    fn language_pack_translate_found() {
        let pack = pack("de", &[("Turdus merula", "Amsel")]);
        assert_eq!(pack.translate("Turdus merula"), "Amsel");
    }

    #[test]
    fn language_pack_translate_fallback() {
        let pack = pack("de", &[]);
        assert_eq!(pack.translate("Unknown species"), "Unknown species");
    }

    #[test]
    fn language_pack_accessors() {
        let pack = pack("fr", &[("A b", "Name")]);
        assert_eq!(pack.lang_code(), "fr");
        assert_eq!(pack.language().english_name, "French");
        assert_eq!(pack.species_count(), 1);
    }

    #[test]
    fn language_pack_load_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let labels_path = tmp.path().join("en_labels.txt");
        let mut f = std::fs::File::create(&labels_path).unwrap();
        writeln!(f, "Turdus merula_Eurasian Blackbird").unwrap();
        writeln!(f, "Parus major_Great Tit").unwrap();

        let pack = LanguagePack::load("en", tmp.path()).unwrap();
        assert_eq!(pack.lang_code(), "en");
        assert_eq!(pack.species_count(), 2);
        assert_eq!(pack.translate("Turdus merula"), "Eurasian Blackbird");
    }

    #[test]
    fn language_pack_load_from_subdirectory() {
        let tmp = tempfile::tempdir().unwrap();
        let subdir = tmp.path().join("labels_l18n");
        std::fs::create_dir(&subdir).unwrap();
        let labels_path = subdir.join("de_labels.txt");
        let mut f = std::fs::File::create(&labels_path).unwrap();
        writeln!(f, "Turdus merula_Amsel").unwrap();

        let pack = LanguagePack::load("de", tmp.path()).unwrap();
        assert_eq!(pack.translate("Turdus merula"), "Amsel");
    }

    #[test]
    fn language_pack_load_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        let result = LanguagePack::load("xx", tmp.path());
        assert!(matches!(result, Err(I18nError::UnsupportedLanguage(_))));
    }

    #[test]
    fn language_pack_load_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let result = LanguagePack::load("en", tmp.path());
        assert!(matches!(result, Err(I18nError::FileNotFound(_))));
    }

    #[test]
    fn find_label_separator_basic() {
        assert_eq!(find_label_separator("Turdus merula_Blackbird"), Some(13));
    }

    #[test]
    fn find_label_separator_no_space() {
        // No space means no valid scientific name
        assert_eq!(find_label_separator("Turdus_Blackbird"), None);
    }

    #[test]
    fn find_label_separator_no_underscore() {
        assert_eq!(find_label_separator("Turdus merula Blackbird"), None);
    }

    #[test]
    fn i18n_error_display() {
        let err = I18nError::FileNotFound("test.txt".to_owned());
        assert!(err.to_string().contains("test.txt"));

        let err = I18nError::Parse("bad format".to_owned());
        assert!(err.to_string().contains("bad format"));

        let err = I18nError::UnsupportedLanguage("xx".to_owned());
        assert!(err.to_string().contains("xx"));
    }
}
