//! The configuration that decided what a run recorded (RES-11).
//!
//! `analysis_runs` has columns for the model files and for five numbers
//! (confidence, sensitivity, overlap, species-frequency threshold, position).
//! What else decided which windows became rows — each classifier's own
//! threshold, the occurrence filter and its week rule, channel analysis, the
//! privacy and noise filters, corroboration, the station profile — is recorded
//! here, once per run, as one canonical JSON object in
//! `analysis_runs.config_json`, with its SHA-256 in `config_sha256`.
//!
//! # Canonical form
//!
//! A flat object: keys are dotted names (`filter.privacy.threshold`,
//! `classifier.<id>.threshold`), sorted bytewise; values are strings, numbers,
//! booleans or `null`. A number is written in the shortest form that reads back
//! as the same `f64`, and an `f32` setting is first widened through its own
//! shortest decimal form, so `0.7_f32` is recorded as `0.7`, not
//! `0.699999988079071`. The same settings give the same bytes, so two runs with
//! equal digests were configured alike, and a digest that differs says which
//! run to diff.
//!
//! # What is never recorded
//!
//! Only what a caller sets explicitly is written, so a secret (an API key, a
//! stream URL with credentials) cannot arrive here by enumerating a
//! configuration. [`RunConfig::set_str`] refuses a key whose name says it is a
//! secret, as a second line.
//!
//! A run registered before this was recorded has `NULL` in both columns, and a
//! reader must say "not recorded", not "default".

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rusqlite::{Connection, params};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::sqlite::connection::DbError;

/// The schema of the object, recorded in it as `schema`. Bumped when a key
/// changes meaning; adding a key does not bump it.
pub const RUN_CONFIG_SCHEMA: &str = "run-config-1";

/// Words in a key that mark it as a secret, which is never recorded.
const SECRET_WORDS: [&str; 6] = ["password", "secret", "token", "api_key", "apikey", "url"];

/// A run's deciding configuration, built key by key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunConfig {
    entries: BTreeMap<String, Value>,
}

/// Widen an `f32` setting through its shortest decimal form, so the value
/// recorded is the one an operator typed.
fn widen(v: f32) -> Value {
    if !v.is_finite() {
        return Value::Null;
    }
    format!("{v}")
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

impl RunConfig {
    /// An empty record carrying only its schema.
    #[must_use]
    pub fn new() -> Self {
        let mut c = Self::default();
        c.entries
            .insert("schema".to_owned(), Value::String(RUN_CONFIG_SCHEMA.into()));
        c
    }

    /// Record a number. A non-finite value is recorded as `null`.
    #[must_use]
    pub fn set_f64(mut self, key: &str, v: f64) -> Self {
        let value = serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number);
        self.entries.insert(key.to_owned(), value);
        self
    }

    /// Record an `f32` setting as the decimal it was given as.
    #[must_use]
    pub fn set_f32(mut self, key: &str, v: f32) -> Self {
        self.entries.insert(key.to_owned(), widen(v));
        self
    }

    /// Record an optional number; `None` is recorded as `null` ("not set").
    #[must_use]
    pub fn set_opt_f64(self, key: &str, v: Option<f64>) -> Self {
        match v {
            Some(v) => self.set_f64(key, v),
            None => self.set_null(key),
        }
    }

    /// Record an integer.
    #[must_use]
    pub fn set_i64(mut self, key: &str, v: i64) -> Self {
        self.entries.insert(key.to_owned(), Value::from(v));
        self
    }

    /// Record a flag.
    #[must_use]
    pub fn set_bool(mut self, key: &str, v: bool) -> Self {
        self.entries.insert(key.to_owned(), Value::Bool(v));
        self
    }

    /// Record `null`: the setting exists and is unset.
    #[must_use]
    pub fn set_null(mut self, key: &str) -> Self {
        self.entries.insert(key.to_owned(), Value::Null);
        self
    }

    /// Record a string. A key whose name marks a secret is not recorded (see
    /// the module documentation); [`Self::is_secret_key`] says which.
    #[must_use]
    pub fn set_str(mut self, key: &str, v: &str) -> Self {
        if !Self::is_secret_key(key) {
            self.entries
                .insert(key.to_owned(), Value::String(v.to_owned()));
        }
        self
    }

    /// Whether a key's name marks it as a secret.
    #[must_use]
    pub fn is_secret_key(key: &str) -> bool {
        let k = key.to_ascii_lowercase();
        SECRET_WORDS.iter().any(|w| k.contains(w))
    }

    /// `other`'s entries laid over these; `other` wins where both set a key.
    #[must_use]
    pub fn merged(mut self, other: Self) -> Self {
        self.entries.extend(other.entries);
        self
    }

    /// The recorded value of `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    /// The canonical JSON text.
    #[must_use]
    pub fn canonical_json(&self) -> String {
        // A `BTreeMap` serialises in key order whatever `serde_json`'s map
        // features are, and every value is a scalar.
        serde_json::to_string(&self.entries).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Lowercase hex SHA-256 of [`Self::canonical_json`].
    #[must_use]
    pub fn sha256(&self) -> String {
        sha256_hex(self.canonical_json().as_bytes())
    }
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Record `config` on run `run_id`. Write-once: a run that already has a record
/// keeps it, and `Ok(false)` says so; a run that does not exist is `Ok(false)`
/// too.
///
/// # Errors
///
/// Returns `DbError` on update failure.
pub fn record_run_config(
    conn: &Connection,
    run_id: i64,
    config: &RunConfig,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE analysis_runs SET config_json = ?2, config_sha256 = ?3
          WHERE id = ?1 AND config_json IS NULL",
        params![run_id, config.canonical_json(), config.sha256()],
    )?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;
    use crate::sqlite::queries::analysis_runs::{analysis_run, fixture_run, insert_analysis_run};

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn a_run_reads_back_the_configuration_that_decided_it() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let id = insert_analysis_run(&conn, &fixture_run(SHA, "m")).unwrap();
        let before = analysis_run(&conn, id).unwrap().unwrap();
        assert_eq!(before.config_json, None, "not recorded is not a default");

        let config = RunConfig::new()
            .set_f32("classifier.perch.threshold", 0.7)
            .set_str("filter.occurrence.week", "birdnet-48")
            .set_bool("filter.privacy.enabled", true);
        assert!(record_run_config(&conn, id, &config).unwrap());
        let after = analysis_run(&conn, id).unwrap().unwrap();
        let json = after.config_json.unwrap();
        assert!(
            json.contains("\"classifier.perch.threshold\":0.7"),
            "{json}"
        );
        assert_eq!(after.config_sha256.unwrap(), sha256_hex(json.as_bytes()));

        // Write-once: a second record of the same run does not replace it.
        let other = RunConfig::new().set_f64("x", 1.0);
        assert!(!record_run_config(&conn, id, &other).unwrap());
        assert!(
            analysis_run(&conn, id)
                .unwrap()
                .unwrap()
                .config_json
                .unwrap()
                .contains("perch")
        );
    }

    #[test]
    fn the_same_settings_in_any_order_give_the_same_bytes() {
        let a = RunConfig::new().set_f64("b", 0.5).set_i64("a", 3);
        let b = RunConfig::new().set_i64("a", 3).set_f64("b", 0.5);
        assert_eq!(a.canonical_json(), b.canonical_json());
        assert_eq!(a.sha256(), b.sha256());
        let c = RunConfig::new().set_i64("a", 3).set_f64("b", 0.25);
        assert_ne!(
            a.sha256(),
            c.sha256(),
            "a changed setting changes the digest"
        );
    }

    #[test]
    fn a_secret_is_never_recorded_and_a_setting_is() {
        let c = RunConfig::new()
            .set_str("birdweather.token", "abc")
            .set_str("rtsp.url", "rtsp://u:p@h")
            .set_str("profile", "research");
        let json = c.canonical_json();
        assert!(!json.contains("abc") && !json.contains("u:p"), "{json}");
        assert!(json.contains("\"profile\":\"research\""), "{json}");
    }
}
