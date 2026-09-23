//! BirdNET-Pi configuration parser.
//!
//! Parses `/etc/birdnet/birdnet.conf`, an INI-style file without section headers
//! where values may be wrapped in PHP-style double quotes.
//!
//! Equivalent to Python's `PHPConfigParser` in `scripts/utils/helpers.py`.
//!
//! The [`validate`] submodule provides range and shape checks for the parsed
//! values so misconfiguration surfaces at startup instead of at first use.
//!
//! The [`secret_file`] submodule accepts a credential as the contents of a
//! file named by `BIRDNET_<KEY>_FILE`, which is how Docker and Kubernetes hand
//! a secret to a process without putting it in the environment.

pub mod known_keys;
pub mod locale;
pub mod redact;
pub mod secret_file;
pub mod validate;

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// Default configuration file path.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/birdnet/birdnet.conf";

/// Default minimum-confidence threshold for recording a detection.
///
/// The single source of truth shared by the detection daemon (the value it
/// enforces when no `CONFIDENCE` is configured), the admin settings form (the
/// value it displays) and the first-run onboarding wizard (the preset it
/// pre-selects), so they can never drift apart — a drift that previously let
/// the daemon record at 0.25 while the UI advertised 0.70.
///
/// Set slightly above BirdNET-Pi's 0.70 so an out-of-the-box station produces a
/// realistic log rather than one padded with marginal IDs, while staying well
/// clear of the over-filtered range where quiet and distant birds stop being
/// recorded at all. Operators who want either extreme change it in one place:
/// Settings → Detection, or the wizard's Accuracy step.
pub const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0.75;

/// Default detection sensitivity, matching BirdNET-Pi's `SENSITIVITY` default.
///
/// Like [`DEFAULT_CONFIDENCE_THRESHOLD`], this is shared by the daemon and the
/// settings form so they cannot drift. Sensitivity is a pre-sigmoid scale factor
/// that only affects the V2.4 model path; the bundled BirdNET V3.0 model emits
/// calibrated probabilities and ignores it, so this default is a no-op there and
/// exists mainly for familiarity and for operators who swap in a V2.4 model.
pub const DEFAULT_SENSITIVITY: f32 = 1.25;

/// Parsed BirdNET-Pi configuration.
#[derive(Debug, Clone)]
pub struct Config {
    values: HashMap<String, String>,
}

/// Configuration parsing errors.
#[derive(Debug)]
pub enum ConfigError {
    /// Config file not found.
    NotFound(String),
    /// Permission denied reading config.
    Permission(String),
    /// Config file has invalid syntax.
    Parse(String),
    /// Required key missing.
    MissingKey(String),
    /// Value cannot be parsed as expected type.
    InvalidValue {
        /// Configuration key whose value was invalid.
        key: String,
        /// Human-readable description of why the value was rejected.
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "config not found: {path}"),
            Self::Permission(path) => write!(f, "permission denied: {path}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::MissingKey(key) => write!(f, "missing required key: {key}"),
            Self::InvalidValue { key, message } => {
                write!(f, "invalid value for '{key}': {message}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// A key present in the file that no reader asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKey {
    /// The key as written.
    pub key: String,
    /// The known key it is closest to, when one is close enough to suggest.
    pub did_you_mean: Option<&'static str>,
}

impl fmt::Display for UnknownKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not a setting this station reads", self.key)?;
        if let Some(meant) = self.did_you_mean {
            write!(f, "; did you mean {meant}?")?;
        }
        Ok(())
    }
}

impl Config {
    /// Create an empty configuration with no entries.
    ///
    /// Used when no config file exists on disk but settings still need to be
    /// supplied at runtime (e.g. overlaid from the admin settings database on a
    /// fresh install that was configured entirely through the web UI).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Insert or overwrite a single key/value pair.
    ///
    /// This is how runtime overrides (from the admin settings database) are
    /// layered on top of the file-based config: the caller maps a UI setting to
    /// its config key and calls `set`, and the new value wins over anything
    /// parsed from `/etc/birdnet/birdnet.conf`.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }

    /// Load configuration from the default path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the file is missing, unreadable, or malformed.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(Path::new(DEFAULT_CONFIG_PATH))
    }

    /// Load configuration from a specific file path.
    ///
    /// The file format is key=value pairs (one per line), where values may
    /// be wrapped in double quotes (PHP-style). Lines starting with `#` are
    /// comments. Empty lines are ignored.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the file is missing, unreadable, or malformed.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ConfigError::NotFound(path.display().to_string()),
            std::io::ErrorKind::PermissionDenied => {
                ConfigError::Permission(path.display().to_string())
            }
            _ => ConfigError::Parse(e.to_string()),
        })?;

        Self::parse(&content)
    }

    /// Parse configuration from a string.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Parse` if the content is malformed.
    pub fn parse(content: &str) -> Result<Self, ConfigError> {
        let mut values = HashMap::new();

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Skip lines that don't look like assignments
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            let key = key.trim().to_string();
            if key.is_empty() {
                return Err(ConfigError::Parse(format!(
                    "empty key on line {}",
                    line_num + 1
                )));
            }

            values.insert(key, parse_value(value));
        }

        Ok(Self { values })
    }

    /// Get a string value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Get a required string value, returning `ConfigError::MissingKey` if absent.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::MissingKey` if the key is not present.
    pub fn require(&self, key: &str) -> Result<&str, ConfigError> {
        self.get(key)
            .ok_or_else(|| ConfigError::MissingKey(key.into()))
    }

    /// Get a value parsed as the specified type.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::MissingKey` if absent, `ConfigError::InvalidValue` if
    /// the value cannot be parsed.
    pub fn get_parsed<T: std::str::FromStr>(&self, key: &str) -> Result<T, ConfigError>
    where
        T::Err: fmt::Display,
    {
        let value = self.require(key)?;
        value
            .parse::<T>()
            // A decimal comma (`52,52`), which `validate()` accepts through the
            // same `locale::normalize_decimal`. Without this the file passed
            // `--doctor` and the number was then silently not read.
            .or_else(|e| {
                let normalised = locale::normalize_decimal(value);
                if normalised == value.trim() {
                    Err(e)
                } else {
                    normalised.parse::<T>().map_err(|_| e)
                }
            })
            .map_err(|e| ConfigError::InvalidValue {
                key: key.into(),
                message: e.to_string(),
            })
    }

    /// Get a value with a default if the key is missing.
    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
    }

    /// The keys in this configuration that nothing reads, each with the
    /// known key it was most likely meant to be. Sorted by key.
    ///
    /// A typo'd key is parsed, stored and never asked for, so without this a
    /// misspelt setting silently keeps its default; see
    /// [`known_keys::KNOWN_CONFIG_KEYS`].
    #[must_use]
    pub fn unknown_keys(&self) -> Vec<UnknownKey> {
        let mut out: Vec<UnknownKey> = self
            .values
            .keys()
            .filter(|k| {
                !known_keys::is_known(k) && !known_keys::INSTALLER_KEYS.contains(&k.as_str())
            })
            .map(|k| UnknownKey {
                key: k.clone(),
                did_you_mean: known_keys::did_you_mean(k),
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Get all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Number of configuration entries.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the configuration is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// One value from a `KEY=value` line, read the way `bash` — which BirdNET-Pi
/// sources this file with — would, as far as it matters here.
///
/// * `"…"` or `'…'`: the text between the quotes, verbatim, with anything
///   after the closing quote ignored if it is a `# comment`. A `#` inside the
///   quotes is data.
/// * Unquoted: everything up to a `#` that follows whitespace, trimmed. A `#`
///   with no whitespace before it is data (`ntfy://host/topic#frag`), and so
///   are spaces inside the value, which this reader has always kept.
///
/// A value whose quotes do not close, or have more than a comment after them,
/// is read as unquoted text.
fn parse_value(raw: &str) -> String {
    let raw = raw.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = raw.strip_prefix(quote)
            && let Some(end) = inner.find(quote)
        {
            let rest = inner[end + 1..].trim_start();
            if rest.is_empty() || rest.starts_with('#') {
                return inner[..end].to_string();
            }
        }
    }
    let bytes = raw.as_bytes();
    let cut = (1..bytes.len())
        .find(|&i| bytes[i] == b'#' && bytes[i - 1].is_ascii_whitespace())
        .unwrap_or(bytes.len());
    raw[..cut].trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys the installer writes into every config it creates are not
    /// "unknown". `BIRDNET_LISTEN=` (read back by the installer on re-run) and
    /// `CADDY_USER=` (read from the environment by the sign-in form) drew an
    /// unknown-key warning on every start of every fresh install, and kept
    /// `--doctor` from ever exiting 0. A typo is still reported.
    #[test]
    fn the_installers_own_keys_are_not_unknown() {
        let c = Config::parse("BIRDNET_LISTEN=0.0.0.0:8502\nCADDY_USER=admin\nCONFIDENC=0.8\n")
            .expect("parses");
        let unknown: Vec<String> = c.unknown_keys().into_iter().map(|u| u.key).collect();
        assert_eq!(unknown, vec!["CONFIDENC".to_owned()]);
    }

    /// The installer's template documents each key on a commented-out line,
    /// with its range after a `#` on the same line. Uncommenting the key the
    /// obvious way used to keep the comment as part of the value:
    /// `CONFIDENCE=0.75          # 0.0–1.0, …` parsed as the whole tail,
    /// `validate()` rejected it, and the station fell back to its last-good
    /// configuration — or, with none, ran web-only and recorded nothing.
    /// `bash`, which BirdNET-Pi sources this file with, reads `0.75`.
    #[test]
    fn an_inline_comment_is_not_part_of_the_value() {
        let c = Config::parse(
            "CONFIDENCE=0.75          # 0.0–1.0, default 0.75\n\
             SENSITIVITY=1.25\t# V2.4 only\n\
             SITE_NAME=\"Back # garden\"   # quoted: the hash is data\n\
             CADDY_PWD='pa$$ word'  # single quotes, as bash reads them\n\
             APPRISE_URL=ntfy://host/topic#frag\n\
             LABEL=two words  # unquoted, spaces kept\n",
        )
        .expect("parse");
        assert_eq!(c.get("CONFIDENCE"), Some("0.75"));
        assert_eq!(c.get("SENSITIVITY"), Some("1.25"));
        assert_eq!(c.get("SITE_NAME"), Some("Back # garden"));
        assert_eq!(c.get("CADDY_PWD"), Some("pa$$ word"));
        // Counterpart: a `#` with no whitespace before it is data (a URL
        // fragment, a password), exactly as in bash.
        assert_eq!(c.get("APPRISE_URL"), Some("ntfy://host/topic#frag"));
        assert_eq!(c.get("LABEL"), Some("two words"));
    }

    /// `validate()` accepts a decimal comma (`LATITUDE=52,52`, and its own
    /// remediation text suggests one), so the runtime must read one too. It
    /// did not: every consumer used a plain `str::parse`, so a comma latitude
    /// passed `--doctor` and then silently switched off the occurrence filter,
    /// the solar schedule and the BirdWeather location, and `CONFIDENCE=0,9`
    /// quietly ran at the default 0.75.
    #[test]
    fn a_value_validate_accepts_is_a_value_the_runtime_reads() {
        let c = Config::parse("LATITUDE=52,52\nCONFIDENCE=0,9\nSEGMENT=15\n").expect("parse");
        assert!(
            super::validate::validate(&c)
                .iter()
                .all(|f| f.key != "LATITUDE" && f.key != "CONFIDENCE"),
            "validate() must accept the comma form for this gate to mean anything"
        );
        assert_eq!(c.get_parsed::<f64>("LATITUDE").ok(), Some(52.52));
        assert_eq!(c.get_parsed::<f32>("CONFIDENCE").ok(), Some(0.9));
        // Counterpart: integers are untouched, and a comma is not a way to
        // smuggle a fraction into one.
        assert_eq!(c.get_parsed::<u32>("SEGMENT").ok(), Some(15));
        let c = Config::parse("SEGMENT=1,5\n").expect("parse");
        assert!(c.get_parsed::<u32>("SEGMENT").is_err());
    }

    #[test]
    fn parse_basic_config() {
        let content = r#"
# BirdNET-Pi configuration
LATITUDE="42.3601"
LONGITUDE="-71.0589"
CONFIDENCE=0.7
RECORDING_LENGTH=15
MODEL=BirdNET_GLOBAL_6K_V2.4_Model_FP16
"#;
        let config = Config::parse(content).unwrap();
        assert_eq!(config.get("LATITUDE"), Some("42.3601"));
        assert_eq!(config.get("LONGITUDE"), Some("-71.0589"));
        assert_eq!(config.get("CONFIDENCE"), Some("0.7"));
        assert_eq!(config.get("RECORDING_LENGTH"), Some("15"));
        assert_eq!(
            config.get("MODEL"),
            Some("BirdNET_GLOBAL_6K_V2.4_Model_FP16")
        );
    }

    #[test]
    fn strip_php_quotes() {
        let content = "KEY1=\"value with quotes\"\nKEY2=value without quotes\nKEY3=\"\"\n";
        let config = Config::parse(content).unwrap();
        assert_eq!(config.get("KEY1"), Some("value with quotes"));
        assert_eq!(config.get("KEY2"), Some("value without quotes"));
        assert_eq!(config.get("KEY3"), Some(""));
    }

    #[test]
    fn skip_comments_and_empty_lines() {
        let content = "# comment\n\nKEY=value\n  # another comment\n";
        let config = Config::parse(content).unwrap();
        assert_eq!(config.len(), 1);
        assert_eq!(config.get("KEY"), Some("value"));
    }

    #[test]
    fn require_missing_key_returns_error() {
        let config = Config::parse("KEY=value").unwrap();
        assert!(config.require("MISSING").is_err());
    }

    #[test]
    fn get_parsed_integer() {
        let config = Config::parse("PORT=8502").unwrap();
        let port: u16 = config.get_parsed("PORT").unwrap();
        assert_eq!(port, 8502);
    }

    #[test]
    fn get_parsed_float() {
        let config = Config::parse("CONFIDENCE=0.7").unwrap();
        let conf: f64 = config.get_parsed("CONFIDENCE").unwrap();
        assert!((conf - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn get_parsed_invalid_returns_error() {
        let config = Config::parse("PORT=not_a_number").unwrap();
        let result: Result<u16, _> = config.get_parsed("PORT");
        assert!(result.is_err());
    }

    #[test]
    fn load_nonexistent_returns_not_found() {
        let result = Config::load_from(Path::new("/nonexistent/birdnet.conf"));
        assert!(matches!(result, Err(ConfigError::NotFound(_))));
    }

    #[test]
    fn empty_config_has_no_entries() {
        let config = Config::empty();
        assert!(config.is_empty());
        assert_eq!(config.get("CONFIDENCE"), None);
    }

    #[test]
    fn set_inserts_new_key() {
        let mut config = Config::empty();
        config.set("CONFIDENCE", "0.7");
        assert_eq!(config.get("CONFIDENCE"), Some("0.7"));
    }

    #[test]
    fn set_overwrites_existing_key() {
        let mut config = Config::parse("CONFIDENCE=0.25").unwrap();
        config.set("CONFIDENCE", "0.8");
        // The override wins over the value parsed from the file.
        assert_eq!(config.get("CONFIDENCE"), Some("0.8"));
        assert!((config.get_parsed::<f32>("CONFIDENCE").unwrap() - 0.8).abs() < f32::EPSILON);
    }
}
