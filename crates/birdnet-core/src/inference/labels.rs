//! Species label loading and lookup.
//!
//! `BirdNET` models output logits indexed by species. This module loads
//! label files (one `Scientific_Common` pair per line) and provides
//! bidirectional lookup.

use std::fmt;
use std::path::Path;

/// A species label entry.
#[derive(Debug, Clone)]
pub struct SpeciesLabel {
    /// Zero-based index in the model output.
    pub index: usize,
    /// Scientific name (e.g., "Turdus merula").
    pub scientific_name: String,
    /// Common name (e.g., "Eurasian Blackbird").
    pub common_name: String,
    /// Taxonomic class from the V3 CSV's `class` column (e.g. `"Aves"`,
    /// `"Insecta"`), or `None` for the V2.4 text format, which has no such
    /// column.
    ///
    /// The Global 11K model is not a bird-only classifier — it carries insects
    /// and amphibians too, and for some of them the `com_name` column simply
    /// repeats the scientific name. An operator seeing *Tettigonia
    /// viridissima* in a feed of blue tits and great tits has no way to tell
    /// whether that is a bird they have never heard of, or a bush-cricket. The
    /// CSV answers that in a column the parser used to drop on the floor;
    /// keeping it lets a caller label or filter non-birds rather than guess.
    pub class: Option<String>,
}

/// A collection of species labels.
#[derive(Debug, Clone)]
pub struct LabelSet {
    labels: Vec<SpeciesLabel>,
}

/// Errors during label loading.
#[derive(Debug)]
pub enum LabelError {
    /// File I/O error.
    Io(std::io::Error),
    /// Invalid label file format.
    Format(String),
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "label I/O error: {e}"),
            Self::Format(msg) => write!(f, "label format error: {msg}"),
        }
    }
}

impl std::error::Error for LabelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(_) => None,
        }
    }
}

impl From<std::io::Error> for LabelError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl LabelSet {
    /// Load labels from a file, auto-detecting the format.
    ///
    /// Two formats are supported:
    ///
    /// - **V2.4 txt**: one `Scientific name_Common name` entry per line.
    /// - **V3.0 CSV**: comma-separated with a header row containing at least
    ///   `sci_name` and `com_name` columns (`BirdNET+` V3.0 / Zenodo format).
    ///
    /// The format is detected from the first non-blank line: if it contains a
    /// comma and the word `sci_name`, CSV mode is used; otherwise txt mode.
    ///
    /// # Errors
    ///
    /// Returns `LabelError` if the file cannot be read or has invalid format.
    pub fn load(path: &Path) -> Result<Self, LabelError> {
        let content = std::fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }

    /// Parse labels from a string, auto-detecting the format.
    ///
    /// Same format detection as [`Self::load`] but accepts an in-memory string.
    /// Useful for testing and embedded label data.
    ///
    /// # Errors
    ///
    /// Returns `LabelError::Format` if no labels can be parsed.
    pub fn load_from_str(content: &str) -> Result<Self, LabelError> {
        // Strip BOM for detection.
        let check = content.trim_start_matches('\u{feff}');
        let first = check.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        // CSV if the header contains sci_name with any delimiter.
        if (first.contains(',') || first.contains(';')) && first.to_lowercase().contains("sci_name")
        {
            Self::parse_csv(content)
        } else {
            Self::parse(content)
        }
    }

    /// Parse labels from a V2.4-style text file.
    ///
    /// Each line should contain `Scientific_Common` (underscore-separated).
    /// Empty lines and lines starting with `#` are skipped.
    ///
    /// # Errors
    ///
    /// Returns `LabelError::Format` if any line cannot be parsed.
    pub fn parse(content: &str) -> Result<Self, LabelError> {
        let mut labels = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // BirdNET label format: "Scientific name_Common name"
            let Some((sci, com)) = line.split_once('_') else {
                return Err(LabelError::Format(format!(
                    "expected 'Scientific_Common', got: {line}"
                )));
            };

            labels.push(SpeciesLabel {
                index: labels.len(),
                scientific_name: sci.to_string(),
                common_name: com.to_string(),
                // The V2.4 text format carries no taxonomy.
                class: None,
            });
        }

        if labels.is_empty() {
            return Err(LabelError::Format("no labels found".into()));
        }

        Ok(Self { labels })
    }

    /// Parse labels from a V3.0 CSV file.
    ///
    /// Expects a header row with at least `sci_name` and `com_name` columns.
    /// Column order is detected from the header, so extra columns are ignored.
    ///
    /// # Errors
    ///
    /// Returns `LabelError::Format` if the header is missing required columns
    /// or any data row cannot be parsed.
    pub fn parse_csv(content: &str) -> Result<Self, LabelError> {
        // Strip UTF-8 BOM if present.
        let content = content.strip_prefix('\u{feff}').unwrap_or(content);

        let mut lines = content.lines();

        // Find and parse the header row.
        let header_line = lines
            .find(|l| !l.trim().is_empty())
            .ok_or_else(|| LabelError::Format("CSV file is empty".into()))?;

        // Auto-detect delimiter: prefer ';' (used by Zenodo BirdNET+ export),
        // fall back to ',' for standard CSV.
        let delim = if header_line.contains(';') { ';' } else { ',' };

        let headers: Vec<&str> = header_line.split(delim).map(str::trim).collect();

        let sci_col = headers
            .iter()
            .position(|h| *h == "sci_name")
            .ok_or_else(|| LabelError::Format("CSV missing 'sci_name' column".into()))?;

        let com_col = headers
            .iter()
            .position(|h| *h == "com_name")
            .ok_or_else(|| LabelError::Format("CSV missing 'com_name' column".into()))?;

        // Optional — the V3.0 Zenodo export has it, other exports may not.
        let class_col = headers.iter().position(|h| *h == "class");

        let mut labels = Vec::new();

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let fields: Vec<&str> = line.split(delim).collect();
            let sci = fields
                .get(sci_col)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| LabelError::Format(format!("missing sci_name in row: {line}")))?;
            let com = fields
                .get(com_col)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| LabelError::Format(format!("missing com_name in row: {line}")))?;

            // Optional: absent from some exports, and never worth failing a
            // whole 11k-row label file over.
            let class = class_col
                .and_then(|c| fields.get(c))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);

            labels.push(SpeciesLabel {
                index: labels.len(),
                scientific_name: sci.to_string(),
                common_name: com.to_string(),
                class,
            });
        }

        if labels.is_empty() {
            return Err(LabelError::Format("no labels found in CSV".into()));
        }

        Ok(Self { labels })
    }

    /// Create a label set from raw entries (for testing or embedded labels).
    pub fn from_entries(entries: Vec<(String, String)>) -> Self {
        let labels = entries
            .into_iter()
            .enumerate()
            .map(|(index, (scientific_name, common_name))| SpeciesLabel {
                index,
                scientific_name,
                common_name,
                class: None,
            })
            .collect();
        Self { labels }
    }

    /// Get a label by index.
    pub fn get(&self, index: usize) -> Option<&SpeciesLabel> {
        self.labels.get(index)
    }

    /// Number of labels (species count).
    pub const fn len(&self) -> usize {
        self.labels.len()
    }

    /// Whether the label set is empty.
    pub const fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Iterate over all labels.
    pub fn iter(&self) -> impl Iterator<Item = &SpeciesLabel> {
        self.labels.iter()
    }

    /// Find a label by common name (case-insensitive).
    pub fn find_by_common_name(&self, name: &str) -> Option<&SpeciesLabel> {
        let lower = name.to_lowercase();
        self.labels
            .iter()
            .find(|l| l.common_name.to_lowercase() == lower)
    }

    /// Find a label by scientific name (case-insensitive).
    pub fn find_by_scientific_name(&self, name: &str) -> Option<&SpeciesLabel> {
        let lower = name.to_lowercase();
        self.labels
            .iter()
            .find(|l| l.scientific_name.to_lowercase() == lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_labels() {
        let content = "Turdus merula_Eurasian Blackbird\nErithacus rubecula_European Robin\n";
        let labels = LabelSet::parse(content).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels.get(0).unwrap().scientific_name, "Turdus merula");
        assert_eq!(labels.get(0).unwrap().common_name, "Eurasian Blackbird");
        assert_eq!(labels.get(1).unwrap().scientific_name, "Erithacus rubecula");
    }

    #[test]
    fn parse_skips_comments_and_blanks() {
        let content = "# Header\n\nTurdus merula_Eurasian Blackbird\n# comment\n";
        let labels = LabelSet::parse(content).unwrap();
        assert_eq!(labels.len(), 1);
    }

    #[test]
    fn parse_empty_returns_error() {
        let result = LabelSet::parse("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_bad_format_returns_error() {
        let result = LabelSet::parse("no underscore here");
        assert!(result.is_err());
    }

    #[test]
    fn find_by_name() {
        let labels = LabelSet::from_entries(vec![
            ("Turdus merula".into(), "Eurasian Blackbird".into()),
            ("Erithacus rubecula".into(), "European Robin".into()),
        ]);
        assert!(labels.find_by_common_name("european robin").is_some());
        assert!(labels.find_by_scientific_name("Turdus merula").is_some());
        assert!(labels.find_by_common_name("nonexistent").is_none());
    }

    #[test]
    fn parse_csv_v3_semicolon_format() {
        // Real BirdNET+ V3.0 Zenodo format uses semicolons and has a BOM.
        let csv = "\u{feff}idx;id;sci_name;com_name;class;order\n\
                   0;abc;Turdus merula;Eurasian Blackbird;Aves;Passeriformes\n\
                   1;def;Erithacus rubecula;European Robin;Aves;Passeriformes\n";
        let labels = LabelSet::parse_csv(csv).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels.get(0).unwrap().scientific_name, "Turdus merula");
        assert_eq!(labels.get(0).unwrap().common_name, "Eurasian Blackbird");
        assert_eq!(labels.get(1).unwrap().scientific_name, "Erithacus rubecula");
        assert_eq!(labels.get(1).unwrap().common_name, "European Robin");
    }

    #[test]
    fn parse_csv_comma_format_also_works() {
        let csv = "idx,id,sci_name,com_name,class,order\n\
                   0,abc,Turdus merula,Eurasian Blackbird,Aves,Passeriformes\n";
        let labels = LabelSet::parse_csv(csv).unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels.get(0).unwrap().scientific_name, "Turdus merula");
    }

    #[test]
    fn load_auto_detects_semicolon_csv() {
        let csv = "\u{feff}idx;id;sci_name;com_name;class;order\n\
                   0;abc;Turdus merula;Eurasian Blackbird;Aves;Passeriformes\n";
        let labels = LabelSet::load_from_str(csv).unwrap();
        assert_eq!(labels.len(), 1);
    }

    #[test]
    fn parse_csv_missing_sci_name_column_errors() {
        let csv = "idx;com_name\n0;Eurasian Blackbird\n";
        assert!(LabelSet::parse_csv(csv).is_err());
    }

    #[test]
    fn parse_csv_missing_com_name_column_errors() {
        let csv = "idx;sci_name\n0;Turdus merula\n";
        assert!(LabelSet::parse_csv(csv).is_err());
    }

    #[test]
    fn parse_csv_columns_in_any_order() {
        // com_name before sci_name — column detection must use header positions
        let csv = "com_name;sci_name\nEurasian Blackbird;Turdus merula\n";
        let labels = LabelSet::parse_csv(csv).unwrap();
        assert_eq!(labels.get(0).unwrap().scientific_name, "Turdus merula");
        assert_eq!(labels.get(0).unwrap().common_name, "Eurasian Blackbird");
    }

    #[test]
    fn parse_csv_strips_bom() {
        let csv_with_bom = "\u{feff}sci_name;com_name\nPica pica;Eurasian Magpie\n";
        let labels = LabelSet::parse_csv(csv_with_bom).unwrap();
        assert_eq!(labels.get(0).unwrap().scientific_name, "Pica pica");
    }

    #[test]
    fn csv_retains_the_taxonomic_class() {
        // The exact header shipped in BirdNET+ V3.0-preview3 Global 11K, as
        // read off a running station.
        let csv = "idx;id;sci_name;com_name;class;order\n\
                   0;3;Abeillia abeillei;Emerald-chinned Hummingbird;Aves;Apodiformes\n\
                   1;9;Tettigonia viridissima;Tettigonia viridissima;Insecta;Orthoptera\n";
        let labels = LabelSet::parse_csv(csv).unwrap();

        assert_eq!(labels.get(0).unwrap().class.as_deref(), Some("Aves"));

        // The case that prompted this: an 11K-model detection whose common name
        // is just the scientific name back again. Nothing in the label itself
        // says "not a bird" — only the class does.
        let cricket = labels.get(1).unwrap();
        assert_eq!(cricket.common_name, cricket.scientific_name);
        assert_eq!(cricket.class.as_deref(), Some("Insecta"));
    }

    #[test]
    fn class_is_absent_rather_than_fatal_when_the_column_is_missing() {
        // Older/other exports carry no `class`, and the V2.4 text format has no
        // columns at all. Both must still load.
        let csv = LabelSet::parse_csv("sci_name;com_name\nPica pica;Eurasian Magpie\n").unwrap();
        assert!(csv.get(0).unwrap().class.is_none());

        let txt = LabelSet::parse("Turdus merula_Eurasian Blackbird\n").unwrap();
        assert!(txt.get(0).unwrap().class.is_none());
    }

    #[test]
    fn label_indices_are_sequential() {
        let labels = LabelSet::from_entries(vec![
            ("A_species".into(), "Species A".into()),
            ("B_species".into(), "Species B".into()),
            ("C_species".into(), "Species C".into()),
        ]);
        for (i, label) in labels.iter().enumerate() {
            assert_eq!(label.index, i);
        }
    }
}
