//! Resolving credentials mounted as files (`BIRDNET_<KEY>_FILE`) at startup.
//!
//! The mechanism and its reasoning live in
//! [`birdnet_core::config::secret_file`]; this is the binary's half — the real
//! environment, the real filesystem, and the log lines an operator reads when a
//! mount is wrong.
//!
//! Reporting is the whole point of the log lines. A secret file that cannot be
//! read leaves the feature it configures silently off: no notifications, no
//! BirdWeather uploads, no heartbeat — and a station that is quiet because it
//! was told to be looks exactly like a station that is quiet because a mount
//! path had a typo.

use birdnet_core::config::Config;
use birdnet_core::config::secret_file::{SecretFileOutcome, SecretFiles, read_secret_file};

/// Resolve every `BIRDNET_<KEY>_FILE` variable into the configuration, logging
/// what happened to each.
///
/// Takes and returns the configuration because a station with no config file at
/// all may still mount secrets: in that case an empty [`Config`] is created to
/// hold them, and the rest of startup reads it exactly as it would a parsed
/// file.
#[must_use]
pub fn resolve_secret_files(config: Option<Config>) -> (Option<Config>, SecretFiles) {
    let env = |name: &str| std::env::var(name).ok();
    // Nothing to do at all unless at least one of the variables is set, so the
    // overwhelmingly common case does not manufacture a Config.
    let any_set = birdnet_core::config::secret_file::file_env_names()
        .iter()
        .any(|name| env(name).is_some_and(|v| !v.trim().is_empty()));
    if !any_set {
        return (config, SecretFiles::default());
    }

    let mut config = config.unwrap_or_else(Config::empty);
    let report = config.resolve_secret_files(&env, &read_secret_file);
    for outcome in &report.outcomes {
        match outcome {
            SecretFileOutcome::Loaded { key, path } => {
                // The path, never the value. This line exists so an operator
                // can confirm the mount took, and a log that quoted the secret
                // would defeat the whole feature.
                tracing::info!(key, path, "read a credential from a mounted file");
            }
            SecretFileOutcome::Unreadable { key, path, reason } => {
                tracing::error!(
                    key,
                    path,
                    reason,
                    "a credential file could not be read; the feature it configures will be \
                     off until this is fixed"
                );
            }
            SecretFileOutcome::Empty { key, path } => {
                tracing::error!(
                    key,
                    path,
                    "a credential file is empty; the feature it configures will be off. An \
                     empty secret is never what was meant — check the mount is populated"
                );
            }
            SecretFileOutcome::Ignored { key, path } => {
                tracing::warn!(
                    key,
                    path,
                    "both the value and a credential file were supplied; the value wins and \
                     the file was not read. Unset one of them"
                );
            }
        }
    }
    (Some(config), report)
}
