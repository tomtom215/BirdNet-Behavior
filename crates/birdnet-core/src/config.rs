//! BirdNET-Pi configuration parser.
//!
//! Parses `/etc/birdnet/birdnet.conf`, an INI-style file without section headers
//! where values may be wrapped in PHP-style double quotes.
//!
//! Equivalent to Python's `PHPConfigParser` in `scripts/utils/helpers.py`.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// Default configuration file path.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/birdnet/birdnet.conf";

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
    InvalidValue { key: String, message: String },
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

impl Config {
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

            // Strip surrounding double quotes (PHP-style config values)
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value)
                .to_string();

            values.insert(key, value);
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
        value.parse::<T>().map_err(|e| ConfigError::InvalidValue {
            key: key.into(),
            message: e.to_string(),
        })
    }

    /// Get a value with a default if the key is missing.
    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
