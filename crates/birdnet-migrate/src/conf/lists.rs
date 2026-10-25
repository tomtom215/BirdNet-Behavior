//! BirdNET-Pi's species list files, and its `apprise.txt`.
//!
//! BirdNET-Pi keeps three lists as text files in its install directory:
//! `include_species_list.txt`, `exclude_species_list.txt` and
//! `whitelist_species_list.txt` (and a fourth, `confirmed_species_list.txt`,
//! that nothing here uses). A line is the label the species picker writes,
//! `Scientific name_Common name`, and BirdNET-Pi reads a line as
//! `line.strip().split('_')[0]` (`utils/analysis.py: loadCustomSpeciesList`):
//! the scientific name, and only the scientific name.
//!
//! What the three lists do there is what the lists of the same names do here
//! (`species_include`, `species_exclude`, `species_whitelist`): include =
//! record only these when it is not empty, exclude = never record these,
//! whitelist = exempt from the occurrence filter and from nothing else.

/// Which of BirdNET-Pi's lists a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// `include_species_list.txt`: record only these.
    Include,
    /// `exclude_species_list.txt`: never record these.
    Exclude,
    /// `whitelist_species_list.txt`: let these through the occurrence filter.
    Whitelist,
}

impl ListKind {
    /// The file BirdNET-Pi keeps this list in.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Include => "include_species_list.txt",
            Self::Exclude => "exclude_species_list.txt",
            Self::Whitelist => "whitelist_species_list.txt",
        }
    }

    /// The settings key this station keeps the same list in.
    #[must_use]
    pub const fn setting(self) -> &'static str {
        match self {
            Self::Include => "species_include",
            Self::Exclude => "species_exclude",
            Self::Whitelist => "species_whitelist",
        }
    }

    /// What the list does, for the preview.
    #[must_use]
    pub const fn meaning(self) -> &'static str {
        match self {
            Self::Include => "record only these species",
            Self::Exclude => "never record these species",
            Self::Whitelist => "let these species through the occurrence filter",
        }
    }

    /// Every list.
    pub const ALL: [Self; 3] = [Self::Include, Self::Exclude, Self::Whitelist];

    /// The list a file name is, if it is one of the three.
    #[must_use]
    pub fn from_file_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.file_name() == name)
    }
}

/// A BirdNET-Pi species list file, read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpeciesList {
    /// Scientific names, once each, in file order.
    pub species: Vec<String>,
    /// Blank lines. BirdNET-Pi reads one as an entry that matches nothing, so
    /// an *include* list holding only a blank line made it record nothing.
    pub blank_lines: usize,
    /// Lines whose first part is not a scientific name (`Genus species`), and
    /// which therefore matched nothing in BirdNET-Pi either. Not imported.
    pub unusable: Vec<String>,
    /// Names that appeared more than once.
    pub repeated: usize,
}

impl SpeciesList {
    /// Read a list file as BirdNET-Pi does.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                out.blank_lines += 1;
                continue;
            }
            let name = line.split('_').next().unwrap_or("").trim();
            if !looks_scientific(name) {
                out.unusable.push(line.to_owned());
                continue;
            }
            if seen.insert(name.to_ascii_lowercase()) {
                out.species.push(name.to_owned());
            } else {
                out.repeated += 1;
            }
        }
        out
    }
}

/// Whether `name` has the shape of a scientific name: a capitalised genus and
/// a lower-case species, optionally a subspecies, letters and hyphens only.
fn looks_scientific(name: &str) -> bool {
    let words: Vec<&str> = name.split(' ').collect();
    if !(2..=3).contains(&words.len()) {
        return false;
    }
    let letters = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
    let genus_ok = words[0]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        && letters(words[0]);
    genus_ok
        && words[1..]
            .iter()
            .all(|w| letters(w) && w.chars().all(|c| !c.is_ascii_uppercase()))
}

/// Add `incoming` species to `existing`, keeping what is there.
///
/// `existing` is the comma-separated list as the settings table stores it. A
/// name already present under any spelling of case is not added again, and
/// nothing already there is removed or reordered. Returns the new value and
/// how many names were new.
#[must_use]
pub fn merge_into(existing: &str, incoming: &[String]) -> (String, usize) {
    let mut names: Vec<String> = existing
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let mut present: std::collections::HashSet<String> =
        names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut added = 0;
    for name in incoming {
        if present.insert(name.to_ascii_lowercase()) {
            names.push(name.clone());
            added += 1;
        }
    }
    (names.join(", "), added)
}

/// What an `apprise.txt` holds, without its secrets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppriseSummary {
    /// How many service addresses it has.
    pub destinations: usize,
    /// The schemes they use (`discord`, `tgram`, `mailto`…), once each, in
    /// file order. The scheme names a service; nothing after the `://` is read
    /// into this summary, because that is where the token is.
    pub schemes: Vec<String>,
}

impl AppriseSummary {
    /// Count the addresses in an Apprise configuration (text or YAML).
    #[must_use]
    pub fn of(text: &str) -> Self {
        let mut out = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut rest = line;
            while let Some(at) = rest.find("://") {
                let before = &rest[..at];
                let scheme: String = before
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if !scheme.is_empty() {
                    out.destinations += 1;
                    let scheme = scheme.to_ascii_lowercase();
                    if !out.schemes.contains(&scheme) {
                        out.schemes.push(scheme);
                    }
                }
                rest = &rest[at + 3..];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_read_as_birdnet_pi_reads_it() {
        let list = SpeciesList::parse(
            "Turdus merula_Eurasian Blackbird\nPica pica_Eurasian Magpie\n  Parus major_Great Tit  \nParus major_Great Tit\n",
        );
        assert_eq!(list.species, ["Turdus merula", "Pica pica", "Parus major"]);
        assert_eq!(list.repeated, 1);
        assert!(list.unusable.is_empty());
    }

    #[test]
    fn a_name_without_the_common_name_still_counts() {
        let list = SpeciesList::parse("Turdus merula\n");
        assert_eq!(list.species, ["Turdus merula"]);
    }

    #[test]
    fn a_blank_line_is_counted_and_a_common_name_is_not_a_species() {
        let list = SpeciesList::parse("\nEurasian Blackbird\nCorvus corax_Common Raven\n");
        assert_eq!(list.blank_lines, 1);
        assert_eq!(list.unusable, ["Eurasian Blackbird"]);
        assert_eq!(list.species, ["Corvus corax"]);
    }

    #[test]
    fn a_subspecies_and_a_hyphenated_epithet_are_names() {
        let list = SpeciesList::parse(
            "Corvus corone cornix_Hooded Crow\nPoecile atricapillus_Black-capped Chickadee\nGallinago hardwickii-x_odd\n",
        );
        assert_eq!(list.species.len(), 3, "{:?}", list.species);
    }

    #[test]
    fn merging_keeps_what_is_there_and_adds_only_what_is_new() {
        let (merged, added) = merge_into(
            "European Robin, pica pica",
            &["Pica pica".to_owned(), "Turdus merula".to_owned()],
        );
        assert_eq!(merged, "European Robin, pica pica, Turdus merula");
        assert_eq!(added, 1);
        let (merged, added) = merge_into("", &["Pica pica".to_owned()]);
        assert_eq!((merged.as_str(), added), ("Pica pica", 1));
    }

    #[test]
    fn an_apprise_file_summary_has_schemes_and_no_secrets() {
        let text = "# my services\ndiscord://1234/abcdSECRET\nmailto://user:pass@example.com\nhome=tgram://bot/chat\nurls:\n  - ntfy://topic\n";
        let summary = AppriseSummary::of(text);
        assert_eq!(summary.destinations, 4);
        assert_eq!(summary.schemes, ["discord", "mailto", "tgram", "ntfy"]);
        let printed = format!("{summary:?}");
        assert!(
            !printed.contains("SECRET") && !printed.contains("pass"),
            "{printed}"
        );
    }

    /// A list the size of the size cap is read in linear time: the duplicate
    /// check was a scan of everything read so far.
    #[test]
    fn a_long_list_is_read_in_linear_time() {
        use std::fmt::Write as _;
        let word = |mut n: usize| {
            let mut w = String::new();
            for _ in 0..5 {
                w.push(char::from(b'a' + u8::try_from(n % 26).unwrap()));
                n /= 26;
            }
            w
        };
        let mut text = String::new();
        for i in 0..40_000 {
            let _ = writeln!(text, "Genus {}", word(i));
        }
        let started = std::time::Instant::now();
        let list = SpeciesList::parse(&text);
        let incoming = list.species.clone();
        let (_, added) = merge_into("", &incoming);
        assert_eq!(list.species.len(), 40_000);
        assert_eq!(added, 40_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
    }
}
