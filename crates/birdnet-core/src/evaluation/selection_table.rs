//! Raven-style selection tables: the annotations an evaluation is scored
//! against.
//!
//! A selection table is tab-separated text with a header row. The columns read
//! here are `Begin Time (s)`, `End Time (s)`, one column naming the species
//! (`Species` by default; any header can be named), and, when present,
//! `Begin File` or `Begin Path` saying which recording a selection is in. A
//! table without either file column is one recording's table, and the caller
//! names the recording. Every other column (`Selection`, `View`, `Channel`,
//! frequencies, notes) is carried by Raven and ignored here.
//!
//! Raven writes each selection once per view (`Waveform 1`, `Spectrogram 1`);
//! rows that repeat a `Selection` number in one recording are read once.

use std::collections::HashSet;
use std::fmt;

/// One annotated sound.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// The recording, by file stem (no directory, no extension).
    pub recording: String,
    /// Seconds into the recording where the sound begins.
    pub begin_secs: f64,
    /// Seconds into the recording where the sound ends.
    pub end_secs: f64,
    /// The species, as the annotator wrote it (matched to detections exactly,
    /// after trimming).
    pub species: String,
}

/// Why a selection table could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// The table has no header row.
    Empty,
    /// A column the reader needs is not in the header.
    MissingColumn(String),
    /// A row's cell could not be read; 1-based line number and why.
    BadRow(usize, String),
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the selection table is empty"),
            Self::MissingColumn(c) => write!(f, "the selection table has no `{c}` column"),
            Self::BadRow(line, why) => write!(f, "line {line}: {why}"),
        }
    }
}

impl std::error::Error for SelectionError {}

/// A path or file name reduced to its stem: no directory, no extension.
#[must_use]
pub fn recording_stem(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rsplit_once('.')
        .map_or(base, |(stem, _)| stem)
        .trim()
        .to_owned()
}

/// Read one selection table.
///
/// `species_column` names the header holding the species. `recording` names
/// the recording for a table with no `Begin File`/`Begin Path` column, and is
/// ignored otherwise. A row with an empty species cell is skipped (Raven
/// leaves unlabelled selections blank); a row whose times do not parse, or
/// whose end precedes its begin, is an error, because a silently dropped
/// annotation would raise the recall it should have counted against.
///
/// # Errors
///
/// [`SelectionError`] as above.
pub fn parse(
    content: &str,
    species_column: &str,
    recording: Option<&str>,
) -> Result<Vec<Annotation>, SelectionError> {
    let mut lines = content
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (_, header) = lines.next().ok_or(SelectionError::Empty)?;
    let header: Vec<&str> = header.split('\t').map(str::trim).collect();
    let col = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
    let need = |name: &str| col(name).ok_or_else(|| SelectionError::MissingColumn(name.to_owned()));
    let begin = need("Begin Time (s)")?;
    let end = need("End Time (s)")?;
    let species = need(species_column)?;
    let file = col("Begin File").or_else(|| col("Begin Path"));
    let selection = col("Selection");
    if file.is_none() && recording.is_none() {
        return Err(SelectionError::MissingColumn("Begin File".to_owned()));
    }

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    for (i, line) in lines {
        let cells: Vec<&str> = line.split('\t').map(str::trim).collect();
        let cell = |c: usize| cells.get(c).copied().unwrap_or("");
        let name = cell(species);
        if name.is_empty() {
            continue;
        }
        let rec = file.map_or_else(
            || recording.map_or_else(String::new, recording_stem),
            |c| recording_stem(cell(c)),
        );
        if let Some(s) = selection
            && !seen.insert((rec.clone(), cell(s).to_owned()))
        {
            continue;
        }
        let time = |c: usize, what: &str| {
            cell(c)
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| SelectionError::BadRow(i + 1, format!("{what} `{}`", cell(c))))
        };
        let (b, e) = (time(begin, "begin time")?, time(end, "end time")?);
        if e < b {
            return Err(SelectionError::BadRow(
                i + 1,
                format!("ends at {e} before it begins at {b}"),
            ));
        }
        out.push(Annotation {
            recording: rec,
            begin_secs: b,
            end_secs: e,
            species: name.to_owned(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "Selection\tView\tChannel\tBegin Time (s)\tEnd Time (s)\tLow Freq (Hz)\tHigh Freq (Hz)\tBegin File\tSpecies\n\
1\tWaveform 1\t1\t1.20\t2.40\t2000\t6000\tsite1_0600.wav\tTurdus merula\n\
1\tSpectrogram 1\t1\t1.20\t2.40\t2000\t6000\tsite1_0600.wav\tTurdus merula\n\
2\tSpectrogram 1\t1\t7.0\t9.5\t3000\t8000\tsite1_0600.wav\tErithacus rubecula\n\
3\tSpectrogram 1\t1\t10.0\t11.0\t3000\t8000\tsite1_0600.wav\t\n";

    #[test]
    fn a_raven_table_reads_one_annotation_per_labelled_selection() {
        let got = parse(TABLE, "Species", None).unwrap();
        assert_eq!(
            got.len(),
            2,
            "views repeat a selection; a blank one is unlabelled"
        );
        assert_eq!(got[0].recording, "site1_0600");
        assert_eq!(got[0].species, "Turdus merula");
        assert!((got[0].begin_secs - 1.2).abs() < 1e-9);
        assert!((got[1].end_secs - 9.5).abs() < 1e-9);
    }

    #[test]
    fn a_table_without_a_file_column_needs_the_recording_named() {
        let t = "Begin Time (s)\tEnd Time (s)\tAnnotation\n0\t1\tPica pica\n";
        assert_eq!(
            parse(t, "Annotation", None),
            Err(SelectionError::MissingColumn("Begin File".into()))
        );
        let got = parse(t, "Annotation", Some("/data/rec 7.flac")).unwrap();
        assert_eq!(got[0].recording, "rec 7");
    }

    #[test]
    fn a_bad_time_is_an_error_not_a_dropped_annotation() {
        let t = "Begin Time (s)\tEnd Time (s)\tSpecies\tBegin File\n2\t1\tPica pica\ta.wav\n";
        assert!(matches!(
            parse(t, "Species", None),
            Err(SelectionError::BadRow(2, _))
        ));
        let t = "Begin Time (s)\tEnd Time (s)\tSpecies\tBegin File\nx\t1\tPica pica\ta.wav\n";
        assert!(matches!(
            parse(t, "Species", None),
            Err(SelectionError::BadRow(2, _))
        ));
    }

    #[test]
    fn a_missing_species_column_is_named() {
        assert_eq!(
            parse(TABLE, "Label", None),
            Err(SelectionError::MissingColumn("Label".into()))
        );
    }
}
