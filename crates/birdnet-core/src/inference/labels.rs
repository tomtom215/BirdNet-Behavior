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
    ///   `sci_name` and `com_name` columns (BirdNET+ V3.0 / Zenodo format).
    ///
    /// The format is detected from the first non-blank line: if it contains a
    /// comma and the word `sci_name`, CSV mode is used; otherwise txt mode.
    ///
    /// # Errors
    ///
    /// Returns `LabelError` if the file cannot be read or has invalid format.
    pub fn load(path: &Path) -> Result<Self, LabelError> {
        let content = std::fs::read_to_string(path)?;
        let first = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        if first.contains(',') && first.to_lowercase().contains("sci_name") {
            Self::parse_csv(&content)
        } else {
            Self::parse(&content)
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
        let mut lines = content.lines();

        // Find and parse the header row.
        let header_line = lines
            .find(|l| !l.trim().is_empty())
            .ok_or_else(|| LabelError::Format("CSV file is empty".into()))?;

        let headers: Vec<&str> = header_line.split(',').map(str::trim).collect();

        let sci_col = headers
            .iter()
            .position(|h| *h == "sci_name")
            .ok_or_else(|| LabelError::Format("CSV missing 'sci_name' column".into()))?;

        let com_col = headers
            .iter()
            .position(|h| *h == "com_name")
            .ok_or_else(|| LabelError::Format("CSV missing 'com_name' column".into()))?;

        let mut labels = Vec::new();

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let fields: Vec<&str> = line.split(',').collect();
            let sci = fields
                .get(sci_col)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    LabelError::Format(format!("missing sci_name in row: {line}"))
                })?;
            let com = fields
                .get(com_col)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    LabelError::Format(format!("missing com_name in row: {line}"))
                })?;

            labels.push(SpeciesLabel {
                index: labels.len(),
                scientific_name: sci.to_string(),
                common_name: com.to_string(),
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
            })
            .collect();
        Self { labels }
    }

    /// Get a label by index.
    pub fn get(&self, index: usize) -> Option<&SpeciesLabel> {
        self.labels.get(index)
    }

    /// Number of labels (species count).
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Whether the label set is empty.
    pub fn is_empty(&self) -> bool {
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
    fn parse_csv_v3_format() {
        let csv = "idx,id,sci_name,com_name,class,order\n\
                   0,abc,Turdus merula,Eurasian Blackbird,Aves,Passeriformes\n\
                   1,def,Erithacus rubecula,European Robin,Aves,Passeriformes\n";
        let labels = LabelSet::parse_csv(csv).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels.get(0).unwrap().scientific_name, "Turdus merula");
        assert_eq!(labels.get(0).unwrap().common_name, "Eurasian Blackbird");
        assert_eq!(labels.get(1).unwrap().scientific_name, "Erithacus rubecula");
        assert_eq!(labels.get(1).unwrap().common_name, "European Robin");
    }

    #[test]
    fn load_auto_detects_csv() {
        let csv = "idx,id,sci_name,com_name,class,order\n\
                   0,abc,Turdus merula,Eurasian Blackbird,Aves,Passeriformes\n";
        let labels = LabelSet::parse_csv(csv).unwrap();
        assert_eq!(labels.len(), 1);
    }

    #[test]
    fn parse_csv_missing_sci_name_column_errors() {
        let csv = "idx,com_name\n0,Eurasian Blackbird\n";
        assert!(LabelSet::parse_csv(csv).is_err());
    }

    #[test]
    fn parse_csv_missing_com_name_column_errors() {
        let csv = "idx,sci_name\n0,Turdus merula\n";
        assert!(LabelSet::parse_csv(csv).is_err());
    }

    #[test]
    fn parse_csv_columns_in_any_order() {
        // com_name before sci_name — column detection must use header positions
        let csv = "com_name,sci_name\nEurasian Blackbird,Turdus merula\n";
        let labels = LabelSet::parse_csv(csv).unwrap();
        assert_eq!(labels.get(0).unwrap().scientific_name, "Turdus merula");
        assert_eq!(labels.get(0).unwrap().common_name, "Eurasian Blackbird");
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
