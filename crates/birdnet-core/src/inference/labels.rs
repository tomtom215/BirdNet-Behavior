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
    /// Load labels from a text file.
    ///
    /// Each line should contain `Scientific_Common` (underscore-separated).
    /// Empty lines and lines starting with `#` are skipped.
    ///
    /// # Errors
    ///
    /// Returns `LabelError` if the file cannot be read or has invalid format.
    pub fn load(path: &Path) -> Result<Self, LabelError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse labels from a string.
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
