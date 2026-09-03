//! One-shot bootstrap that rotates the seed admin row's password hash
//! when the operator first sets `CADDY_PWD`, so the cookie wire flip can
//! verify against a real argon2id hash instead of the empty seed.
//!
//! Runs once per process start, immediately after `AppState` is built
//! (i.e. after migrations have created the row). Behaviour:
//!
//! `CADDY_PWD` is resolved from the **loaded config first, then the
//! environment** — the same order `crate::doctor::config::check_admin_exposure`
//! uses, so the diagnostic and the thing it diagnoses cannot disagree.
//!
//! That ordering is not cosmetic. The bare-metal installer generates an admin
//! password on a fresh non-loopback install and writes it to
//! `/etc/birdnet/birdnet.conf`; the unit it installs sets no `EnvironmentFile`,
//! so nothing ever put `CADDY_PWD` in the environment. Reading only the
//! environment therefore skipped the bootstrap on every such station: the seed
//! admin kept its legacy hash, `auth_middleware::admin_password_configured`
//! returned false, and `/admin` was served to anyone on the network — while
//! `--doctor`, reading the config, reported the panel protected. Measured on a
//! Raspberry Pi 4: `CADDY_PWD` present in the config, `/admin/settings` 200
//! unauthenticated, doctor exit 0.
//!
//! * `CADDY_PWD` unset in both → no-op. The basic-auth path was already letting
//!   everyone through; the cookie path inherits that contract via its
//!   "no admin password configured → bypass" branch in
//!   `crate::auth_middleware`.
//! * `CADDY_PWD` set, seed admin's stored hash empty / legacy
//!   (`PLAINTEXT-PLACEHOLDER:` from #89's scaffolding) → hash the env
//!   value and write it.
//! * `CADDY_PWD` set, stored hash is real argon2id but does NOT verify
//!   `CADDY_PWD` → operator rotated the env var; rotate the stored hash
//!   to match so basic-auth and cookie-auth stay in sync.
//! * `CADDY_PWD` set, stored hash verifies cleanly → no-op.
//!
//! The bootstrap never panics: any error here logs and the start path
//! continues — the basic-auth fallback (still wired in #89's surface)
//! keeps the station reachable.

use birdnet_core::config::Config;
use birdnet_db::accounts::{self, AccountsError, UserStore};
use birdnet_web::state::AppState;

/// Resolve the configured admin password: file config first, then the
/// environment. An empty value at either level counts as unset, so a blank
/// `CADDY_PWD=` line in the config cannot mask a real environment value.
///
/// `doctor::config::check_admin_exposure` calls **this same function** rather
/// than reimplementing the precedence. The previous arrangement was two copies
/// of the rule plus a comment asserting they matched; they did not, and the
/// station shipped an open `/admin` that its own diagnostic called protected.
/// Sharing the resolver makes "these cannot disagree" true by construction.
///
/// Takes the environment value as a parameter so the precedence is testable
/// without mutating the process environment, which is `unsafe` in edition 2024
/// while this crate forbids `unsafe_code`.
pub fn resolve_admin_password(config: Option<&Config>, env: Option<String>) -> Option<String> {
    config
        .and_then(|c| c.get("CADDY_PWD"))
        .map(str::to_owned)
        .filter(|pwd| !pwd.is_empty())
        .or_else(|| env.filter(|pwd| !pwd.is_empty()))
}

/// Resolve the station's API token: file config first, then the environment —
/// the same precedence, and for the same reason, as
/// [`resolve_admin_password`].
///
/// # Why not a settings row
///
/// Because this project already decided credentials do not live there:
/// [`purge_legacy_credential_settings`] *deletes* plaintext credential rows a
/// previous build's settings form could write, and the dashboard that renders
/// settings is unauthenticated on a default station (`O-4`). A token in
/// `settings` would be a credential published on a public page.
///
/// # Why the environment value is a parameter
///
/// Identical to `resolve_admin_password`: `std::env::set_var` is `unsafe` in
/// edition 2024 and this crate forbids `unsafe_code`, so a test cannot set the
/// variable. Passing it in is what makes the precedence testable at all.
///
/// Returns the raw string; validating its length is
/// [`birdnet_web::api_token::ApiToken::new`]'s job, so the rule lives with the
/// type that enforces it rather than being restated here.
#[must_use]
pub fn resolve_api_token(config: Option<&Config>, env: Option<String>) -> Option<String> {
    config
        .and_then(|c| c.get(birdnet_web::api_token::API_TOKEN_KEY))
        .map(str::to_owned)
        .filter(|t| !t.is_empty())
        .or_else(|| env.filter(|t| !t.is_empty()))
}

/// Build the station's [`ApiToken`], complaining loudly if one was configured
/// and refused.
///
/// A token that is too short leaves the mutating API **off**, because enabling
/// it weakly is the failure an operator cannot see. The warning names the knob
/// and the floor.
///
/// [`ApiToken`]: birdnet_web::api_token::ApiToken
#[must_use]
pub fn build_api_token(config: Option<&Config>) -> Option<birdnet_web::api_token::ApiToken> {
    use birdnet_web::api_token::{API_TOKEN_KEY, ApiToken};

    let raw = resolve_api_token(config, std::env::var(API_TOKEN_KEY).ok())?;
    match ApiToken::new(&raw) {
        Ok(token) => {
            tracing::info!("the mutating /api/v2 endpoints are enabled: {API_TOKEN_KEY} is set");
            Some(token)
        }
        Err(e) => {
            tracing::warn!(
                "{e}; the mutating /api/v2 endpoints stay disabled. Generate one with \
                 `openssl rand -base64 48`"
            );
            None
        }
    }
}

/// Run the one-shot admin-row password bootstrap. Idempotent across
/// restarts.
pub fn bootstrap_admin_password(state: &AppState, config: Option<&Config>) {
    let Some(env_pwd) = resolve_admin_password(config, std::env::var("CADDY_PWD").ok()) else {
        tracing::debug!(
            "CADDY_PWD unset or empty in both config and environment; bootstrap skipped"
        );
        return;
    };

    let outcome = state.with_db(|conn| -> Result<BootstrapOutcome, AccountsError> {
        let admin = conn.find_user_by_name("admin")?;
        if accounts::is_legacy_password_hash(&admin.pwd_argon2) {
            let hash = accounts::hash_password(&env_pwd)?;
            conn.set_password(admin.id, &hash)?;
            return Ok(BootstrapOutcome::RotatedLegacy);
        }
        let verifies = accounts::verify_password(&admin.pwd_argon2, &env_pwd).unwrap_or(false);
        if !verifies {
            let hash = accounts::hash_password(&env_pwd)?;
            conn.set_password(admin.id, &hash)?;
            return Ok(BootstrapOutcome::RotatedAfterEnvChange);
        }
        Ok(BootstrapOutcome::AlreadyConsistent)
    });

    match outcome {
        Ok(BootstrapOutcome::RotatedLegacy) => {
            tracing::info!("admin password hash initialised from CADDY_PWD");
        }
        Ok(BootstrapOutcome::RotatedAfterEnvChange) => {
            tracing::info!("admin password hash rotated to match updated CADDY_PWD");
        }
        Ok(BootstrapOutcome::AlreadyConsistent) => {
            tracing::debug!("admin password hash already up-to-date");
        }
        Err(e) => tracing::warn!(
            error = %e,
            "admin password bootstrap failed; basic-auth path stays usable"
        ),
    }
}

/// Settings rows an earlier build's "Web Authentication" form could write.
///
/// Neither was ever read by anything: the admin credential lives as an Argon2id
/// hash in the accounts table, seeded from `CADDY_PWD`. The form nevertheless
/// stored whatever was typed as a **plaintext** `settings` row and rendered it
/// back into the page HTML on every later load. The inputs are gone, but a
/// station upgraded from a build that had them still carries the row.
const LEGACY_CREDENTIAL_SETTINGS: &[&str] = &["auth_password", "auth_username"];

/// Delete any plaintext credential rows left behind by an earlier build.
///
/// Idempotent, and a no-op on a station that never used the removed form. Runs
/// alongside the password bootstrap so an upgrade clears the stored plaintext
/// without the operator having to know it was ever there. Returns the number of
/// rows removed.
pub fn purge_legacy_credential_settings(state: &AppState) -> usize {
    use birdnet_db::settings::{delete, ensure_settings_table};

    let removed = state.with_db(|conn| {
        // A brand-new database may not have the table yet; that just means
        // there is nothing to purge.
        if ensure_settings_table(conn).is_err() {
            return 0;
        }
        LEGACY_CREDENTIAL_SETTINGS
            .iter()
            .filter(|key| delete(conn, key).unwrap_or(false))
            .count()
    });

    if removed > 0 {
        tracing::warn!(
            count = removed,
            "removed plaintext credential rows written by a previous build's settings form; \
             the admin password is set through CADDY_PWD and was never read from these"
        );
    }
    removed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapOutcome {
    /// Stored hash was empty or `PLAINTEXT-PLACEHOLDER:` — set the real one.
    RotatedLegacy,
    /// Stored hash was real argon2id but didn't verify `CADDY_PWD`
    /// (operator rotated the env) — refreshed to match.
    RotatedAfterEnvChange,
    /// Stored hash already verifies cleanly — no write needed.
    AlreadyConsistent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::sqlite;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("birds.db");
        let _conn = sqlite::open_or_create(&db_path).expect("open db");
        let state = AppState::new(db_path).expect("state");
        (dir, state)
    }

    fn admin_hash(state: &AppState) -> String {
        state
            .with_db(|conn| conn.find_user_by_name("admin"))
            .expect("admin row")
            .pwd_argon2
    }

    fn config_with(key: &str, value: &str) -> Config {
        let mut c = Config::empty();
        c.set(key, value);
        c
    }

    /// The regression this fix exists for: the bare-metal installer writes
    /// `CADDY_PWD` to the config and the unit sets no `EnvironmentFile`, so an
    /// environment-only read skipped the bootstrap and left `/admin` open while
    /// `--doctor` (which reads the config) called it protected.
    #[test]
    fn config_password_is_used_when_the_environment_is_empty() {
        let cfg = config_with("CADDY_PWD", "from-the-config-file");
        assert_eq!(
            resolve_admin_password(Some(&cfg), None).as_deref(),
            Some("from-the-config-file")
        );
    }

    #[test]
    fn config_wins_over_the_environment() {
        // Same precedence as doctor::config::check_admin_exposure, so the
        // diagnostic and the runtime can never disagree.
        let cfg = config_with("CADDY_PWD", "from-config");
        assert_eq!(
            resolve_admin_password(Some(&cfg), Some("from-env".to_owned())).as_deref(),
            Some("from-config")
        );
    }

    #[test]
    fn environment_is_used_when_the_config_has_no_entry() {
        // The Docker path: no config file, CADDY_PWD supplied as an env var.
        assert_eq!(
            resolve_admin_password(None, Some("from-env".to_owned())).as_deref(),
            Some("from-env")
        );
        let cfg = Config::empty();
        assert_eq!(
            resolve_admin_password(Some(&cfg), Some("from-env".to_owned())).as_deref(),
            Some("from-env")
        );
    }

    #[test]
    fn empty_values_are_treated_as_unset() {
        assert_eq!(resolve_admin_password(None, None), None);
        assert_eq!(resolve_admin_password(None, Some(String::new())), None);
        let cfg = config_with("CADDY_PWD", "");
        // An empty config entry must not mask a real environment value.
        assert_eq!(
            resolve_admin_password(Some(&cfg), Some("from-env".to_owned())).as_deref(),
            Some("from-env")
        );
    }

    #[test]
    fn purges_plaintext_credential_rows_left_by_the_old_form() {
        use birdnet_db::settings::{SettingsCategory, ensure_settings_table, get, set};

        let (_d, state) = fixture();
        state.with_db(|conn| {
            ensure_settings_table(conn).unwrap();
            set(conn, "auth_password", "hunter2", SettingsCategory::System).unwrap();
            set(conn, "auth_username", "birdnet", SettingsCategory::System).unwrap();
            // An unrelated row must survive the purge.
            set(conn, "site_name", "Backyard", SettingsCategory::System).unwrap();
        });

        assert_eq!(purge_legacy_credential_settings(&state), 2);
        state.with_db(|conn| {
            assert!(get(conn, "auth_password").is_err(), "plaintext row remains");
            assert!(get(conn, "auth_username").is_err());
            assert_eq!(get(conn, "site_name").unwrap(), "Backyard");
        });
    }

    #[test]
    fn purge_is_idempotent_and_silent_on_a_clean_station() {
        let (_d, state) = fixture();
        assert_eq!(purge_legacy_credential_settings(&state), 0);
        assert_eq!(purge_legacy_credential_settings(&state), 0);
    }

    #[test]
    fn rotates_empty_seed_hash() {
        let (_d, state) = fixture();
        assert_eq!(admin_hash(&state), "");
        // Run the bootstrap path directly with a known plaintext, bypassing
        // the env var read (which is not safe to mutate under
        // `unsafe_code = deny`). This mirrors what `bootstrap_admin_password`
        // does once it has resolved CADDY_PWD.
        let plaintext = "test-bootstrap-password";
        let result = state.with_db(|conn| -> Result<(), AccountsError> {
            let admin = conn.find_user_by_name("admin")?;
            assert!(accounts::is_legacy_password_hash(&admin.pwd_argon2));
            let hash = accounts::hash_password(plaintext)?;
            conn.set_password(admin.id, &hash)
        });
        result.expect("rotate");
        let h = admin_hash(&state);
        assert!(!h.is_empty());
        assert!(!accounts::is_legacy_password_hash(&h));
        assert!(accounts::verify_password(&h, plaintext).unwrap());
    }

    #[test]
    fn legacy_plaintext_seed_recognised_as_legacy() {
        let (_d, state) = fixture();
        // Simulate a viewer account that was created pre-flip via the
        // PLAINTEXT-PLACEHOLDER scaffolding in #89.
        let _ = state.with_db(|conn| -> Result<(), AccountsError> {
            conn.create_user(
                "viewer1",
                "PLAINTEXT-PLACEHOLDER:hunter2",
                birdnet_db::accounts::Role::Viewer,
                None,
            )?;
            Ok(())
        });
        let viewer_hash = state
            .with_db(|conn| conn.find_user_by_name("viewer1"))
            .unwrap()
            .pwd_argon2;
        assert!(accounts::is_legacy_password_hash(&viewer_hash));
        assert!(accounts::verify_password(&viewer_hash, "hunter2").unwrap());
    }
}
