//! Reading a credential out of a file instead of an environment variable.
//!
//! Docker and Kubernetes both mount secrets as files — `/run/secrets/<name>`,
//! or a projected volume — precisely so a credential is not in the process
//! environment, where `docker inspect`, `/proc/<pid>/environ`, a crash dump and
//! anything that logs its own environment can all reach it. The convention both
//! ecosystems settled on is a second variable named `<VAR>_FILE` holding the
//! path.
//!
//! This station had no way to accept that: every outbound credential it reads —
//! the notification URLs, which carry a bot token *inside* the URL, the
//! `BirdWeather` token, the MQTT password, the heartbeat URL whose path is the
//! secret — could only be given as a value.
//!
//! # Scope, and why it is the environment only
//!
//! `BIRDNET_<KEY>_FILE` is read from the process environment. It is deliberately
//! *not* a `birdnet.conf` key: the deployment this exists for supplies its
//! configuration as environment variables, and a station editing `birdnet.conf`
//! by hand can already put the secret in the file it is editing. Keeping it out
//! of the config namespace also keeps it out of the settings table — see below.
//!
//! # What it must not do
//!
//! A value resolved from a file must **not** be copied into the `settings`
//! table by the first-run seed. That table is in the database, the database is
//! in every backup, and an operator who mounted a secret as a file did so to
//! keep it out of exactly those places. [`SecretFiles::resolved`] names the
//! keys the caller must exclude from seeding, and the binary's seed step reads
//! it.

use std::collections::BTreeSet;

/// The configuration keys whose value may be supplied as the contents of a file
/// named by the `BIRDNET_<KEY>_FILE` environment variable.
///
/// Every entry is a credential that leaves this station: a token, a password,
/// or a URL whose path *is* the token. A key is here because knowing it is
/// enough to post as this station, not because it is merely private.
///
/// Two deliberate absences:
///
/// * `APPRISE_CONFIG` — its file form, `APPRISE_CONFIG_FILE`, already exists
///   and means something else entirely (the path of an Apprise configuration
///   file, which Apprise itself reads). Adding it here would give one variable
///   name two meanings.
/// * The SMTP password — it lives in the `settings` table under
///   `email_smtp_pass`, set from the admin UI, and is never read from the
///   environment or the config file, so there is nothing here to indirect.
pub const FILE_INDIRECT_KEYS: &[&str] = &[
    "APPRISE_URL",
    "BIRDWEATHER_TOKEN",
    "HEARTBEAT_URL",
    "MQTT_PASSWORD",
    "NOTIFY_URLS",
];

/// The environment variable naming the file for `key`.
#[must_use]
pub fn file_env_name(key: &str) -> String {
    format!("BIRDNET_{key}_FILE")
}

/// Every `BIRDNET_*_FILE` variable this station reads, for the environment
/// checker that reports variables nothing reads.
#[must_use]
pub fn file_env_names() -> Vec<String> {
    FILE_INDIRECT_KEYS
        .iter()
        .map(|key| file_env_name(key))
        .collect()
}

/// What happened to one `BIRDNET_<KEY>_FILE` variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretFileOutcome {
    /// The file was read and its contents became the value of `key`.
    Loaded {
        /// The configuration key that was set.
        key: String,
        /// The file it was read from. A path, never the value.
        path: String,
    },
    /// The file could not be read. The key keeps whatever value it had, which
    /// is usually none — so this is a station that will start with the feature
    /// silently off unless it is reported.
    Unreadable {
        /// The configuration key that was not set.
        key: String,
        /// The path that could not be read.
        path: String,
        /// The operating system's reason.
        reason: String,
    },
    /// The file was read and found empty. Treated as unreadable rather than as
    /// "configured to nothing": an empty secret is never what was meant, and a
    /// truncated or not-yet-populated mount is a real failure mode.
    Empty {
        /// The configuration key that was not set.
        key: String,
        /// The path that held nothing.
        path: String,
    },
    /// Both the direct value and the file were supplied. The direct value
    /// stands — it is what the rest of startup would have used anyway, and
    /// silently preferring the file would change the behaviour of a station
    /// that added the file by mistake.
    Ignored {
        /// The configuration key that already had a value.
        key: String,
        /// The path that was not read.
        path: String,
    },
}

impl SecretFileOutcome {
    /// The configuration key this outcome is about.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Loaded { key, .. }
            | Self::Unreadable { key, .. }
            | Self::Empty { key, .. }
            | Self::Ignored { key, .. } => key,
        }
    }

    /// Whether this outcome means a credential the operator asked for is not
    /// in effect.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        matches!(self, Self::Unreadable { .. } | Self::Empty { .. })
    }
}

/// The result of resolving every `BIRDNET_<KEY>_FILE` variable.
#[derive(Debug, Clone, Default)]
pub struct SecretFiles {
    /// One entry per `BIRDNET_<KEY>_FILE` variable that was set, in key order.
    pub outcomes: Vec<SecretFileOutcome>,
}

impl SecretFiles {
    /// The configuration keys whose value came from a file.
    ///
    /// The first-run settings seed must skip these: copying them into the
    /// `settings` table would put the credential in the database, and so in
    /// every backup of it, which is the thing a mounted secret exists to avoid.
    #[must_use]
    pub fn resolved(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter_map(|o| match o {
                SecretFileOutcome::Loaded { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Whether any file the operator named could not be used.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.outcomes.iter().any(SecretFileOutcome::is_failure)
    }
}

impl super::Config {
    /// Resolve `BIRDNET_<KEY>_FILE` variables into configuration values.
    ///
    /// `env` looks a variable up; `read` reads a file. Both are injected so the
    /// behaviour is testable without mutating the process environment, which
    /// no test can do safely while other tests are running in the same process.
    ///
    /// A value already present and non-blank is left alone, and the file is
    /// reported as [`SecretFileOutcome::Ignored`] rather than read.
    pub fn resolve_secret_files(
        &mut self,
        env: &impl Fn(&str) -> Option<String>,
        read: &impl Fn(&str) -> Result<String, String>,
    ) -> SecretFiles {
        let mut outcomes = Vec::new();
        // Sorted and deduplicated, so the report reads the same on every start
        // and a key listed twice cannot be resolved twice.
        let keys: BTreeSet<&str> = FILE_INDIRECT_KEYS.iter().copied().collect();
        for key in keys {
            let Some(path) = env(&file_env_name(key)).map(|p| p.trim().to_owned()) else {
                continue;
            };
            if path.is_empty() {
                // `docker compose` interpolates an unset variable to the empty
                // string, so a blank path is "not configured", not a request to
                // read the file named "".
                continue;
            }
            let already_set = self.get(key).is_some_and(|v| !v.trim().is_empty());
            if already_set {
                outcomes.push(SecretFileOutcome::Ignored {
                    key: key.to_owned(),
                    path,
                });
                continue;
            }
            match read(&path) {
                Ok(contents) => {
                    // Trim the surrounding whitespace a secret file almost
                    // always ends with, and nothing inside it: `NOTIFY_URLS`
                    // may legitimately be several lines.
                    let value = contents.trim();
                    if value.is_empty() {
                        outcomes.push(SecretFileOutcome::Empty {
                            key: key.to_owned(),
                            path,
                        });
                    } else {
                        self.set(key, value);
                        outcomes.push(SecretFileOutcome::Loaded {
                            key: key.to_owned(),
                            path,
                        });
                    }
                }
                Err(reason) => outcomes.push(SecretFileOutcome::Unreadable {
                    key: key.to_owned(),
                    path,
                    reason,
                }),
            }
        }
        SecretFiles { outcomes }
    }
}

/// Read a file from disk, returning the OS reason as a string.
///
/// The default `read` for [`Config::resolve_secret_files`]. Separate so the
/// caller injects it and the tests do not touch the filesystem.
///
/// # Errors
///
/// Returns the operating system's message when the file cannot be read.
pub fn read_secret_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{FILE_INDIRECT_KEYS, SecretFileOutcome, file_env_name, file_env_names};
    use crate::config::Config;
    use std::collections::HashMap;

    /// An environment built from pairs, so no test touches the process's own.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    /// A filesystem built from pairs, likewise.
    fn files_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Result<String, String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |path: &str| {
            map.get(path)
                .cloned()
                .ok_or_else(|| "No such file or directory (os error 2)".to_owned())
        }
    }

    #[test]
    fn a_mounted_file_becomes_the_value() {
        let mut config = Config::empty();
        let report = config.resolve_secret_files(
            &env_of(&[("BIRDNET_NOTIFY_URLS_FILE", "/run/secrets/urls")]),
            &files_of(&[("/run/secrets/urls", "ntfy://ntfy.sh/garden\n")]),
        );
        assert_eq!(config.get("NOTIFY_URLS"), Some("ntfy://ntfy.sh/garden"));
        assert_eq!(report.resolved(), vec!["NOTIFY_URLS"]);
        assert!(!report.has_failures());
    }

    /// A secret file all but always ends with a newline, and several of these
    /// values are URLs that a trailing `\n` would corrupt — a bot token with a
    /// newline glued to it fails authentication at the far end, which is a much
    /// harder failure to diagnose than an empty one.
    #[test]
    fn surrounding_whitespace_is_trimmed_and_inner_newlines_are_kept() {
        let mut config = Config::empty();
        config.resolve_secret_files(
            &env_of(&[("BIRDNET_NOTIFY_URLS_FILE", "/s")]),
            &files_of(&[("/s", "\n  ntfy://a/one\ntgram://b/two  \n\n")]),
        );
        assert_eq!(
            config.get("NOTIFY_URLS"),
            Some("ntfy://a/one\ntgram://b/two"),
            "the two URLs must survive as two lines"
        );
    }

    /// The direct value wins, and the file is reported rather than read.
    ///
    /// Silently preferring the file would change the behaviour of a station
    /// that added the mount by mistake, and the rest of startup resolves the
    /// direct value first anyway — a resolver that claimed otherwise would be
    /// lying about what the station is using.
    #[test]
    fn a_value_already_set_wins_and_the_file_is_reported() {
        let mut config = Config::empty();
        config.set("BIRDWEATHER_TOKEN", "from-the-environment");
        let report = config.resolve_secret_files(
            &env_of(&[("BIRDNET_BIRDWEATHER_TOKEN_FILE", "/s")]),
            &files_of(&[("/s", "from-the-file")]),
        );
        assert_eq!(
            config.get("BIRDWEATHER_TOKEN"),
            Some("from-the-environment")
        );
        assert!(matches!(
            report.outcomes.as_slice(),
            [SecretFileOutcome::Ignored { key, .. }] if key == "BIRDWEATHER_TOKEN"
        ));
        assert!(
            report.resolved().is_empty(),
            "a value that did not come from the file must not be excluded from the \
             settings seed on its account"
        );
    }

    /// A blank value is "not configured" and does not block the file.
    ///
    /// Every surface that can supply these produces a blank rather than an
    /// absent value when the operator declines the feature — `docker compose`
    /// interpolates `${BIRDNET_NOTIFY_URLS:-}` whether or not it is set — so
    /// treating a blank as "already set" would make the file useless in exactly
    /// the deployment it exists for.
    #[test]
    fn a_blank_value_does_not_block_the_file() {
        let mut config = Config::empty();
        config.set("NOTIFY_URLS", "   ");
        config.resolve_secret_files(
            &env_of(&[("BIRDNET_NOTIFY_URLS_FILE", "/s")]),
            &files_of(&[("/s", "ntfy://a/one")]),
        );
        assert_eq!(config.get("NOTIFY_URLS"), Some("ntfy://a/one"));
    }

    /// A blank *path* is likewise "not configured" — the same interpolation
    /// produces one for the `_FILE` variable too.
    #[test]
    fn a_blank_path_is_not_a_request_to_read_it() {
        let mut config = Config::empty();
        let report = config.resolve_secret_files(
            &env_of(&[("BIRDNET_NOTIFY_URLS_FILE", "  ")]),
            &files_of(&[]),
        );
        assert!(report.outcomes.is_empty(), "{:?}", report.outcomes);
        assert_eq!(config.get("NOTIFY_URLS"), None);
    }

    /// An unreadable file is a reported failure, not a silent one.
    #[test]
    fn an_unreadable_file_is_a_failure() {
        let mut config = Config::empty();
        let report = config.resolve_secret_files(
            &env_of(&[("BIRDNET_MQTT_PASSWORD_FILE", "/run/secrets/typo")]),
            &files_of(&[("/run/secrets/mqtt", "hunter2")]),
        );
        assert!(config.get("MQTT_PASSWORD").is_none());
        assert!(report.has_failures());
        assert!(matches!(
            report.outcomes.as_slice(),
            [SecretFileOutcome::Unreadable { key, .. }] if key == "MQTT_PASSWORD"
        ));
    }

    /// So is an empty one. A projected secret volume that has not been
    /// populated yet reads as a zero-length file, and accepting that as "the
    /// password is the empty string" would send the station off to authenticate
    /// with nothing and report the far end's rejection instead of the real
    /// fault.
    #[test]
    fn an_empty_file_is_a_failure_not_an_empty_credential() {
        let mut config = Config::empty();
        let report = config.resolve_secret_files(
            &env_of(&[("BIRDNET_MQTT_PASSWORD_FILE", "/s")]),
            &files_of(&[("/s", "\n  \n")]),
        );
        assert!(config.get("MQTT_PASSWORD").is_none());
        assert!(report.has_failures());
        assert!(matches!(
            report.outcomes.as_slice(),
            [SecretFileOutcome::Empty { key, .. }] if key == "MQTT_PASSWORD"
        ));
    }

    /// Every key that can be indirected is a key some reader actually asks the
    /// config for — otherwise the file would be read into a value nothing uses.
    #[test]
    fn every_indirect_key_is_a_known_config_key() {
        use crate::config::known_keys::KNOWN_CONFIG_KEYS;

        for key in FILE_INDIRECT_KEYS {
            assert!(
                KNOWN_CONFIG_KEYS.contains(key),
                "{key} can be supplied as a file but is not a config key anything reads"
            );
        }
    }

    /// `APPRISE_CONFIG` must never join the list: `APPRISE_CONFIG_FILE` already
    /// exists and means the path of an Apprise *configuration* file, which
    /// Apprise itself reads. One variable name, two meanings, is how an
    /// operator's config file gets read as a credential.
    #[test]
    fn no_indirect_key_collides_with_an_existing_config_key() {
        use crate::config::known_keys::KNOWN_CONFIG_KEYS;

        for key in FILE_INDIRECT_KEYS {
            let derived = format!("{key}_FILE");
            assert!(
                !KNOWN_CONFIG_KEYS.contains(&derived.as_str()),
                "{derived} is already a config key with its own meaning, so {key} cannot \
                 take that name for its file indirection"
            );
        }
    }

    /// The environment names are derived from the keys, in both directions.
    #[test]
    fn the_env_names_are_the_keys() {
        let names = file_env_names();
        assert_eq!(names.len(), FILE_INDIRECT_KEYS.len());
        for key in FILE_INDIRECT_KEYS {
            assert!(names.contains(&file_env_name(key)));
        }
        assert!(names.iter().all(|n| n.starts_with("BIRDNET_")));
        assert!(names.iter().all(|n| n.ends_with("_FILE")));
    }
}
