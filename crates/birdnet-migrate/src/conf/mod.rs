//! Moving a BirdNET-Pi `birdnet.conf` onto this station.
//!
//! BirdNET-Pi keeps its settings in one shell-sourced file, `birdnet.conf`,
//! whose keys are not this station's. A copy of the file used to be read
//! as-is, and forty of its sixty default keys were reported as "not a setting
//! this station reads". This module is the other way to do it: a table of
//! what each BirdNET-Pi key becomes ([`RULES`], derived from how BirdNET-Pi's
//! own code uses the key, not from its name), a parser for the file
//! ([`ConfFile`]), and a planner ([`plan`]) that says, key by key, what would
//! be set, what changes on the way, and what has no equivalent and why.
//!
//! # What this module does not do
//!
//! It writes nothing. [`Plan::assignments`] is a list of `(setting, value)`
//! pairs for the caller to save **through the settings form's own path**
//! (`birdnet-web`), which validates them: this crate neither knows the
//! ranges nor opens the destination database. The caller supplies a
//! validator ([`PlanInput::check`]) so that a value the settings page would
//! refuse is refused here too, in the preview, rather than after "Apply".
//!
//! # Secrets
//!
//! `BIRDWEATHER_ID`, the stream addresses (which may carry a camera's user
//! name and password), the Flickr key, `CADDY_PWD`, the heartbeat address and
//! the other credentials are marked [`Rule::secret`]. A secret is never in a
//! `Debug` print, never in a [`Row::shown`], and never in a log line: the
//! preview shows its length and nothing else.

mod table;

pub mod lists;

use std::collections::{BTreeMap, HashMap};
use std::fmt;

pub use table::{Convert, RULES, Rule, Target, mapping_table_markdown};

/// One `KEY=value` line of a `birdnet.conf`.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfEntry {
    /// The key, exactly as written.
    pub key: String,
    /// The value, with its surrounding quotes taken off.
    pub value: String,
    /// The 1-based line it was on.
    pub line: usize,
}

impl fmt::Debug for ConfEntry {
    /// Prints no value for a key that holds a credential. A `{:?}` of a parsed
    /// file in a log line or a failed assertion must not carry one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("ConfEntry");
        d.field("key", &self.key);
        if is_secret_key(&self.key) {
            d.field("value", &"<redacted>");
        } else {
            d.field("value", &self.value);
        }
        d.field("line", &self.line).finish()
    }
}

/// A parsed `birdnet.conf`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfFile {
    /// The settings, in file order. A key that appears twice appears once, at
    /// the position and with the value of its **last** line, which is what
    /// `source` gives a shell script.
    pub entries: Vec<ConfEntry>,
    /// Keys that appeared on more than one line.
    pub repeated: Vec<String>,
    /// Lines that were neither blank, a comment nor `KEY=value`.
    pub unreadable_lines: usize,
}

impl ConfFile {
    /// Read the text of a `birdnet.conf`.
    ///
    /// BirdNET-Pi reads the same file three ways — `source` in its shell
    /// scripts, `configparser` in Python and `parse_ini_string` in PHP — and
    /// the file it writes is built to satisfy all three: `KEY=value` per line,
    /// values optionally in double quotes, whole-line `#` comments. This reads
    /// that, plus what `source` would also accept and the file has been seen
    /// with: an `export ` prefix, single quotes, spaces around `=`, a byte
    /// order mark, Windows line endings.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut entries: Vec<ConfEntry> = Vec::new();
        let mut repeated = Vec::new();
        let mut unreadable_lines = 0_usize;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let line = line.strip_prefix("export ").map_or(line, str::trim_start);
            let Some((key, value)) = line.split_once('=') else {
                unreadable_lines += 1;
                continue;
            };
            let key = key.trim();
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                unreadable_lines += 1;
                continue;
            }
            let entry = ConfEntry {
                key: key.to_owned(),
                value: unquote(value.trim()),
                line: index + 1,
            };
            if let Some(at) = entries.iter().position(|e| e.key == entry.key) {
                if !repeated.contains(&entry.key) {
                    repeated.push(entry.key.clone());
                }
                entries.remove(at);
            }
            entries.push(entry);
        }
        Self {
            entries,
            repeated,
            unreadable_lines,
        }
    }

    /// The value of `key`, if the file sets it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.value.as_str())
    }
}

/// A value with one layer of matching quotes taken off.
///
/// BirdNET-Pi's Python side strips every leading and trailing `"`; the shell
/// takes one layer, single or double. One layer is what a value written by
/// `config.php` (`KEY="value"`) has.
fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

/// Whether `key` holds a credential or an address that is one.
///
/// The set of [`Rule::secret`] keys plus the `Debug` and display paths that
/// must not print their values.
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    RULES.iter().any(|r| r.pi_key == key && r.secret)
}

/// What the import will do with one BirdNET-Pi key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Set as it is.
    Imported,
    /// Set, with the value changed on the way (a unit, a name, a rule).
    Converted,
    /// Carried over, or not, but there is something to read before relying on it.
    Attention,
    /// This station reads it only from `birdnet.conf`, not from the browser;
    /// the line to put there is in the result.
    ConfigFile,
    /// Has no equivalent here, and the row says why.
    NotCarried,
    /// Nothing to import: blank, a placeholder, or already what it should be.
    Left,
    /// The value is one this station does not accept; nothing is set from it.
    Refused,
}

impl Verdict {
    /// A short label for the preview.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Imported => "Will be set",
            Self::Converted => "Will be set, converted",
            Self::Attention => "Look at this",
            Self::ConfigFile => "Goes in birdnet.conf",
            Self::NotCarried => "Not carried over",
            Self::Left => "Nothing to import",
            Self::Refused => "Not accepted here",
        }
    }
}

/// One setting the import would write.
#[derive(Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The settings-table key (`confidence_threshold`).
    pub setting: &'static str,
    /// The value to store.
    pub value: String,
    /// Whether the value is a credential, for display.
    pub secret: bool,
}

impl fmt::Debug for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Assignment");
        d.field("setting", &self.setting);
        if self.secret {
            d.field("value", &"<redacted>");
        } else {
            d.field("value", &self.value);
        }
        d.finish()
    }
}

/// A line this station reads only from `birdnet.conf`.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigLine {
    /// The key to put in `birdnet.conf`.
    pub key: &'static str,
    /// Its value.
    pub value: String,
    /// Whether the value is a credential.
    pub secret: bool,
}

impl fmt::Debug for ConfigLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("ConfigLine");
        d.field("key", &self.key);
        if self.secret {
            d.field("value", &"<redacted>");
        } else {
            d.field("value", &self.value);
        }
        d.finish()
    }
}

/// One row of the preview: a BirdNET-Pi key and what happens to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The BirdNET-Pi key as written in the file.
    pub pi_key: String,
    /// Its value, for display: a secret is shown as its length.
    pub shown: String,
    /// What the import does with it.
    pub verdict: Verdict,
    /// What it becomes: `setting = value` pairs, with a secret's value hidden.
    pub becomes: Vec<(String, String)>,
    /// What this station has now for the first of those, if anything, for
    /// display (a secret hidden).
    pub was: Option<String>,
    /// Why, in a sentence a person can act on. Empty when the row is
    /// self-explanatory.
    pub why: String,
}

/// Everything the planner needs.
pub struct PlanInput<'a> {
    /// The parsed file.
    pub conf: &'a ConfFile,
    /// This station's settings now, by key, so a row can say what it replaces.
    pub current: &'a HashMap<String, String>,
    /// Whether this station would accept `value` for the setting `key`:
    /// `None` when it would, otherwise what is wrong with it, in words for the
    /// person who owns the file. The caller passes the settings form's own
    /// validation here, so the preview and the Apply button cannot disagree.
    pub check: &'a dyn Fn(&str, &str) -> Option<String>,
}

// Not derived: `current` holds this station's settings, secrets among them, and
// a derived `Debug` would print them.
impl std::fmt::Debug for PlanInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanInput")
            .field("current_settings", &self.current.len())
            .finish_non_exhaustive()
    }
}

/// What importing a `birdnet.conf` would do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// One row per key in the file, in file order.
    pub rows: Vec<Row>,
    /// The settings to write, once each. A setting two keys would set (the
    /// three notification switches become one) appears once.
    pub assignments: Vec<Assignment>,
    /// The lines for `birdnet.conf` that cannot be set from the browser.
    pub config_lines: Vec<ConfigLine>,
    /// Keys that appeared more than once in the file.
    pub repeated_keys: Vec<String>,
    /// Lines that could not be read at all.
    pub unreadable_lines: usize,
}

impl Plan {
    /// How many rows have the given verdict.
    #[must_use]
    pub fn count(&self, verdict: Verdict) -> usize {
        self.rows.iter().filter(|r| r.verdict == verdict).count()
    }

    /// The value the plan would set for `setting`, if it sets it.
    #[must_use]
    pub fn value_of(&self, setting: &str) -> Option<&str> {
        self.assignments
            .iter()
            .find(|a| a.setting == setting)
            .map(|a| a.value.as_str())
    }
}

/// Decide, for every key in the file, what the import does with it.
///
/// Pure: reads nothing and writes nothing. The same input gives the same plan,
/// which is what lets the preview and the Apply button agree.
#[must_use]
pub fn plan(input: &PlanInput<'_>) -> Plan {
    let mut out = Plan {
        repeated_keys: input.conf.repeated.clone(),
        unreadable_lines: input.conf.unreadable_lines,
        ..Plan::default()
    };
    // Settings in the order they were first assigned, last value winning.
    let mut assigned: BTreeMap<&'static str, usize> = BTreeMap::new();

    for entry in &input.conf.entries {
        let rule = RULES.iter().find(|r| r.pi_key == entry.key);
        let decision = table::decide(entry, rule, input.conf, input.current, input.check);
        for a in &decision.assignments {
            if let Some(&at) = assigned.get(a.setting) {
                out.assignments[at].value.clone_from(&a.value);
            } else {
                assigned.insert(a.setting, out.assignments.len());
                out.assignments.push(a.clone());
            }
        }
        for line in &decision.config_lines {
            if !out.config_lines.iter().any(|l| l.key == line.key) {
                out.config_lines.push(line.clone());
            }
        }
        out.rows.push(decision.row);
    }

    // Behaviour BirdNET-Pi has that this station would otherwise change by
    // default: pinned explicitly, and shown as rows of their own.
    let implied = table::implied(input.conf, input.current, input.check, &mut out);
    out.rows.extend(implied);
    out
}

/// What a decision produced, before it is folded into the [`Plan`].
#[derive(Debug)]
pub(crate) struct Decision {
    pub(crate) row: Row,
    pub(crate) assignments: Vec<Assignment>,
    pub(crate) config_lines: Vec<ConfigLine>,
}

/// Shorten a secret for display: its length, never a character of it.
#[must_use]
pub fn hide(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!("(hidden, {} characters)", value.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_read_the_way_the_shell_reads_it() {
        let conf = ConfFile::parse(
            "# a comment\n\nLATITUDE=52.0\nSITE_NAME=\"My Garden\"\nexport OVERLAP = 0.5\nREC_CARD='plughw:1,0'\r\nBROKEN\n=nokey\n",
        );
        assert_eq!(conf.get("LATITUDE"), Some("52.0"));
        assert_eq!(conf.get("SITE_NAME"), Some("My Garden"));
        assert_eq!(conf.get("OVERLAP"), Some("0.5"));
        assert_eq!(conf.get("REC_CARD"), Some("plughw:1,0"));
        assert_eq!(conf.unreadable_lines, 2, "BROKEN and =nokey");
    }

    #[test]
    fn the_last_line_wins_as_it_does_for_source() {
        let conf = ConfFile::parse("CONFIDENCE=0.5\nSENSITIVITY=1\nCONFIDENCE=0.9\n");
        assert_eq!(conf.get("CONFIDENCE"), Some("0.9"));
        assert_eq!(conf.repeated, vec!["CONFIDENCE".to_owned()]);
        assert_eq!(conf.entries.len(), 2);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_first_key() {
        let conf = ConfFile::parse("\u{feff}LATITUDE=1.5\n");
        assert_eq!(conf.get("LATITUDE"), Some("1.5"));
    }

    #[test]
    fn a_secret_never_reaches_a_debug_print() {
        let conf = ConfFile::parse("BIRDWEATHER_ID=abc123token\nLATITUDE=52.0\n");
        let printed = format!("{conf:?}");
        assert!(!printed.contains("abc123token"), "{printed}");
        assert!(printed.contains("52.0"), "ordinary values still print");
        assert_eq!(hide("abc123token"), "(hidden, 11 characters)");
    }
}
