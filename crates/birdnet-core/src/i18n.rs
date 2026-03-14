//! Localization framework for species common names.
//!
//! BirdNET supports 36 languages for species common names. This module loads
//! and manages language packs from BirdNET label files, providing translation
//! from scientific names to localized common names.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// Supported languages with their codes.
pub const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("ar", "Arabic"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("de", "German"),
    ("en", "English"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("hr", "Croatian"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("is", "Icelandic"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("nl", "Dutch"),
    ("no", "Norwegian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sr", "Serbian"),
    ("sv", "Swedish"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
    ("zh_CN", "Chinese (Simplified)"),
    ("zh_TW", "Chinese (Traditional)"),
];

/// Species name translations for a single language.
#[derive(Debug)]
pub struct LanguagePack {
    lang_code: String,
    /// Map from scientific name to localized common name.
    translations: HashMap<String, String>,
}

impl LanguagePack {
    /// Load a language pack from a BirdNET labels file.
    ///
    /// Searches for the labels file in the given directory, trying:
    /// 1. `{labels_dir}/{lang_code}_labels.txt`
    /// 2. `{labels_dir}/labels_l18n/{lang_code}_labels.txt`
    ///
    /// # File format
    ///
    /// One line per species: `"Scientific Name_Common Name"` (underscore-separated).
    /// For example: `"Turdus merula_Amsel"` (German) or `"Turdus merula_Eurasian Blackbird"` (English).
    ///
    /// # Errors
    ///
    /// Returns `I18nError::UnsupportedLanguage` if the language code is not in
    /// `SUPPORTED_LANGUAGES`, or `I18nError::FileNotFound` if no labels file is found.
    pub fn load(lang_code: &str, labels_dir: &Path) -> Result<Self, I18nError> {
        if !is_supported_language(lang_code) {
            return Err(I18nError::UnsupportedLanguage(lang_code.to_owned()));
        }

        let filename = format!("{lang_code}_labels.txt");

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
            lang = lang_code,
            species_count = translations.len(),
            path = %file_path.display(),
            "loaded language pack"
        );

        Ok(Self {
            lang_code: lang_code.to_owned(),
            translations,
        })
    }

    /// Create a language pack from an in-memory map (for testing).
    #[cfg(test)]
    fn from_map(lang_code: &str, translations: HashMap<String, String>) -> Self {
        Self {
            lang_code: lang_code.to_owned(),
            translations,
        }
    }

    /// Get localized name for a species, falling back to the original name.
    pub fn translate<'a>(&'a self, sci_name: &'a str) -> &'a str {
        self.translations
            .get(sci_name)
            .map_or(sci_name, String::as_str)
    }

    /// Get the language code.
    pub fn lang_code(&self) -> &str {
        &self.lang_code
    }

    /// Get the number of translated species.
    pub fn species_count(&self) -> usize {
        self.translations.len()
    }
}

/// Manager for multiple language packs.
#[derive(Debug)]
pub struct I18nManager {
    packs: HashMap<String, LanguagePack>,
    default_lang: String,
}

impl I18nManager {
    /// Create a new i18n manager with the given default language.
    pub fn new(default_lang: &str) -> Self {
        Self {
            packs: HashMap::new(),
            default_lang: default_lang.to_owned(),
        }
    }

    /// Load a language pack from the labels directory.
    ///
    /// # Errors
    ///
    /// Returns an `I18nError` if the language is unsupported or the file cannot be loaded.
    pub fn load_language(&mut self, lang_code: &str, labels_dir: &Path) -> Result<(), I18nError> {
        let pack = LanguagePack::load(lang_code, labels_dir)?;
        self.packs.insert(lang_code.to_owned(), pack);
        Ok(())
    }

    /// Translate a scientific name to a localized common name.
    ///
    /// Uses `lang_code` if provided, otherwise falls back to the default language.
    /// If no translation is found, returns the scientific name unchanged.
    pub fn translate<'a>(&'a self, sci_name: &'a str, lang_code: Option<&str>) -> &'a str {
        let code = lang_code.unwrap_or(&self.default_lang);
        self.packs
            .get(code)
            .map_or(sci_name, |pack| pack.translate(sci_name))
    }

    /// List available (loaded) languages as `(code, display_name)` pairs.
    pub fn available_languages(&self) -> Vec<(&str, &str)> {
        let mut langs: Vec<(&str, &str)> = self
            .packs
            .keys()
            .filter_map(|code| {
                SUPPORTED_LANGUAGES
                    .iter()
                    .find(|(c, _)| *c == code.as_str())
                    .map(|(c, name)| (*c, *name))
            })
            .collect();
        langs.sort_by_key(|(code, _)| *code);
        langs
    }

    /// Get the default language code.
    pub fn default_lang(&self) -> &str {
        &self.default_lang
    }

    /// Check whether any language packs are loaded.
    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }
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
    /// The requested language code is not in `SUPPORTED_LANGUAGES`.
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

/// Check whether a language code is in the supported list.
fn is_supported_language(code: &str) -> bool {
    SUPPORTED_LANGUAGES.iter().any(|(c, _)| *c == code)
}

/// Parse a BirdNET labels file into a map of scientific name to common name.
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
/// common name in a BirdNET label line.
///
/// BirdNET label format: `"Genus species_Common Name"`.
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
    use std::io::Write as _;

    #[test]
    fn supported_languages_count() {
        assert_eq!(SUPPORTED_LANGUAGES.len(), 36);
    }

    #[test]
    fn is_supported_language_en() {
        assert!(is_supported_language("en"));
        assert!(is_supported_language("de"));
        assert!(is_supported_language("zh_CN"));
        assert!(!is_supported_language("xx"));
        assert!(!is_supported_language(""));
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
        let mut translations = HashMap::new();
        translations.insert("Turdus merula".to_owned(), "Amsel".to_owned());
        let pack = LanguagePack::from_map("de", translations);
        assert_eq!(pack.translate("Turdus merula"), "Amsel");
    }

    #[test]
    fn language_pack_translate_fallback() {
        let pack = LanguagePack::from_map("de", HashMap::new());
        assert_eq!(pack.translate("Unknown species"), "Unknown species");
    }

    #[test]
    fn language_pack_accessors() {
        let mut translations = HashMap::new();
        translations.insert("A b".to_owned(), "Name".to_owned());
        let pack = LanguagePack::from_map("fr", translations);
        assert_eq!(pack.lang_code(), "fr");
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
    fn i18n_manager_translate() {
        let tmp = tempfile::tempdir().unwrap();
        let labels_path = tmp.path().join("en_labels.txt");
        let mut f = std::fs::File::create(&labels_path).unwrap();
        writeln!(f, "Turdus merula_Eurasian Blackbird").unwrap();

        let de_path = tmp.path().join("de_labels.txt");
        let mut f2 = std::fs::File::create(&de_path).unwrap();
        writeln!(f2, "Turdus merula_Amsel").unwrap();

        let mut mgr = I18nManager::new("en");
        mgr.load_language("en", tmp.path()).unwrap();
        mgr.load_language("de", tmp.path()).unwrap();

        // Default language
        assert_eq!(mgr.translate("Turdus merula", None), "Eurasian Blackbird");
        // Explicit language
        assert_eq!(mgr.translate("Turdus merula", Some("de")), "Amsel");
        // Unknown species falls back
        assert_eq!(mgr.translate("Unknown sp", None), "Unknown sp");
        // Unknown language falls back
        assert_eq!(mgr.translate("Turdus merula", Some("xx")), "Turdus merula");
    }

    #[test]
    fn i18n_manager_available_languages() {
        let tmp = tempfile::tempdir().unwrap();
        for code in &["en", "de"] {
            let path = tmp.path().join(format!("{code}_labels.txt"));
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "Turdus merula_Name").unwrap();
        }

        let mut mgr = I18nManager::new("en");
        mgr.load_language("en", tmp.path()).unwrap();
        mgr.load_language("de", tmp.path()).unwrap();

        let langs = mgr.available_languages();
        assert_eq!(langs.len(), 2);
        assert_eq!(langs[0], ("de", "German"));
        assert_eq!(langs[1], ("en", "English"));
    }

    #[test]
    fn i18n_manager_default_lang() {
        let mgr = I18nManager::new("fr");
        assert_eq!(mgr.default_lang(), "fr");
        assert!(mgr.is_empty());
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
