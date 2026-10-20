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
use birdnet_db::accounts::{self, AccountsError, SessionStore as _, UserStore};
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

/// Name of the file, beside the database, that records which `CADDY_PWD` was
/// last applied to the admin account.
///
/// It holds an Argon2id hash, never the password. Without it a restart cannot
/// tell "the operator edited `CADDY_PWD`" (apply it) from "the operator changed
/// the password on the Accounts page and `CADDY_PWD` is just the old value still
/// sitting in the config file" (keep theirs): both look like a stored hash that
/// does not verify `CADDY_PWD`. The second used to be undone at every restart,
/// which quietly put back a password that may be the one that leaked.
const APPLIED_MARKER_FILE: &str = "admin-password.applied";

fn applied_marker_path(state: &AppState) -> std::path::PathBuf {
    state.db_path().with_file_name(APPLIED_MARKER_FILE)
}

/// Whether `CADDY_PWD` is the one this station last applied.
fn caddy_pwd_is_the_one_last_applied(marker: &std::path::Path, env_pwd: &str) -> bool {
    std::fs::read_to_string(marker)
        .is_ok_and(|hash| accounts::verify_password(hash.trim(), env_pwd).unwrap_or(false))
}

/// Record that `env_pwd` is the `CADDY_PWD` now applied. Best effort: with no
/// marker a later restart falls back to the old rule (the config value wins),
/// which is the safe direction for an operator who has locked themselves out.
fn record_applied(marker: &std::path::Path, env_pwd: &str) {
    let write = || -> std::io::Result<()> {
        let hash =
            accounts::hash_password(env_pwd).map_err(|e| std::io::Error::other(e.to_string()))?;
        let part = birdnet_core::atomic_file::part_path(marker);
        std::fs::write(&part, hash)?;
        birdnet_core::atomic_file::commit(&part, marker)
    };
    if let Err(e) = write() {
        tracing::warn!(error = %e, path = %marker.display(), "could not record which CADDY_PWD was applied");
    }
}

/// Whether the admin account has no password yet: the state in which the
/// station's admin pages are open.
///
/// A database error reads as "not open": a warning that the station is open,
/// printed because a read failed, would be a claim the station cannot support.
fn admin_is_open(state: &AppState) -> bool {
    state
        .with_db(|conn| conn.find_user_by_name("admin"))
        .is_ok_and(|admin| accounts::is_legacy_password_hash(&admin.pwd_argon2))
}

/// Run the one-shot admin-row password bootstrap. Idempotent across
/// restarts.
pub fn bootstrap_admin_password(state: &AppState, config: Option<&Config>) {
    let Some(env_pwd) = resolve_admin_password(config, std::env::var("CADDY_PWD").ok()) else {
        tracing::debug!(
            "CADDY_PWD unset or empty in both config and environment; bootstrap skipped"
        );
        if admin_is_open(state) {
            // The one line a container's owner reads: `docker compose up` with a
            // hand-made `.env` publishes the port on every interface and sets no
            // password, so until somebody finishes the first-run wizard, anyone
            // who can reach the port chooses the admin password.
            tracing::warn!(
                "no admin password is set: /admin and the first-run setup are open to anyone \
                 who can reach this station, and whoever finishes the setup first chooses the \
                 password. Open the station in a browser and finish the setup now, or set \
                 CADDY_PWD"
            );
        }
        return;
    };

    let marker = applied_marker_path(state);
    let env_unchanged = caddy_pwd_is_the_one_last_applied(&marker, &env_pwd);

    let outcome = state.with_db(|conn| -> Result<BootstrapOutcome, AccountsError> {
        let admin = conn.find_user_by_name("admin")?;
        if accounts::is_legacy_password_hash(&admin.pwd_argon2) {
            let hash = accounts::hash_password(&env_pwd)?;
            conn.set_password(admin.id, &hash)?;
            // The station had no password until now, so every session it holds
            // came from the open window, where any `POST /login` is admin.
            conn.revoke_others(admin.id, "")?;
            return Ok(BootstrapOutcome::RotatedLegacy);
        }
        let verifies = accounts::verify_password(&admin.pwd_argon2, &env_pwd).unwrap_or(false);
        if !verifies && env_unchanged {
            // `CADDY_PWD` is what this station applied last time and the stored
            // hash is something else: the password was changed since, on the
            // Accounts page. That change stands.
            return Ok(BootstrapOutcome::KeptAccountsPagePassword);
        }
        if !verifies {
            let hash = accounts::hash_password(&env_pwd)?;
            conn.set_password(admin.id, &hash)?;
            // The signing secret no longer rotates with the password (it is
            // persisted beside the database, DD-15), so a rotated `CADDY_PWD`
            // signs the old sessions out here instead — every one of them:
            // nobody is signed in on a process that is still starting.
            let revoked = conn.revoke_others(admin.id, "")?;
            return Ok(BootstrapOutcome::RotatedAfterEnvChange { revoked });
        }
        Ok(BootstrapOutcome::AlreadyConsistent)
    });

    // Whatever was applied (or found to match) is what the marker should name,
    // except when the Accounts-page password was kept: then it keeps naming the
    // `CADDY_PWD` that was applied before, so the next restart decides the same.
    if matches!(
        outcome,
        Ok(BootstrapOutcome::RotatedLegacy
            | BootstrapOutcome::RotatedAfterEnvChange { .. }
            | BootstrapOutcome::AlreadyConsistent)
    ) && !env_unchanged
    {
        record_applied(&marker, &env_pwd);
    }

    match outcome {
        Ok(BootstrapOutcome::KeptAccountsPagePassword) => {
            tracing::warn!(
                "the admin password was changed on the Accounts page since CADDY_PWD was last \
                 applied, so that password stands and CADDY_PWD is ignored. Remove CADDY_PWD \
                 from the configuration, or set it to a new value to replace the admin \
                 password with it"
            );
        }
        Ok(BootstrapOutcome::RotatedLegacy) => {
            tracing::info!("admin password hash initialised from CADDY_PWD");
        }
        Ok(BootstrapOutcome::RotatedAfterEnvChange { revoked }) => {
            // A warning, not a note: it replaces the stored password. For a
            // station with no record of the last applied `CADDY_PWD` (one that
            // started before this was recorded) it may be replacing a password
            // changed on the Accounts page, which the person who changed it
            // will not expect and cannot otherwise see.
            tracing::warn!(
                revoked,
                "admin password replaced by CADDY_PWD at startup (it did not match the stored \
                 hash, and CADDY_PWD is not the one last applied), and existing sessions were \
                 signed out"
            );
        }
        Ok(BootstrapOutcome::AlreadyConsistent) => {
            tracing::debug!("admin password hash already up-to-date");
        }
        Err(e) => {
            // Reported by `?strict=1` too (DD-19): a station that could not
            // write its own admin credential is one whose database refused a
            // write, and that was only ever a warning in the journal.
            state.set_admin_bootstrap_failed(true);
            tracing::warn!(
                error = %e,
                "admin password bootstrap failed; basic-auth path stays usable"
            );
        }
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
    RotatedAfterEnvChange {
        /// Sessions signed out by the rotation.
        revoked: usize,
    },
    /// Stored hash already verifies cleanly — no write needed.
    AlreadyConsistent,
    /// The stored hash does not verify `CADDY_PWD`, but `CADDY_PWD` is the one
    /// this station applied last time: the password was changed on the Accounts
    /// page since, and stays.
    KeptAccountsPagePassword,
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

    /// A station that had no password until `CADDY_PWD` appeared in its config
    /// may hold sessions minted in the open window, where any `POST /login`
    /// is admin. The first boot with a password must end all of them.
    #[test]
    fn a_first_password_from_the_config_ends_the_open_windows_sessions() {
        use birdnet_db::accounts::SessionStore as _;
        let (_d, state) = fixture();
        state
            .with_db(|conn| -> Result<(), AccountsError> {
                let admin = conn.find_user_by_name("admin")?;
                conn.create_session("open-window", admin.id, "2999-01-01 00:00:00", None, None)
                    .map(drop)
            })
            .expect("a session from the open window");

        bootstrap_admin_password(&state, Some(&config_with("CADDY_PWD", "now-it-has-one")));

        assert!(
            state
                .with_db(|conn| conn.find_active_session("open-window"))
                .is_err(),
            "the open window's session survived the station acquiring a password"
        );
        assert!(!accounts::is_legacy_password_hash(&admin_hash(&state)));
    }

    /// The start-up warning is printed for a station with no password and for
    /// no other: a fresh database is open, and one whose owner has chosen a
    /// password (the wizard, the Accounts page, `CADDY_PWD`) is not.
    #[test]
    fn a_station_is_open_until_it_has_a_password() {
        let (_d, state) = fixture();
        assert!(admin_is_open(&state), "a fresh station has no password");
        change_on_the_accounts_page(&state, &pw());
        assert!(!admin_is_open(&state), "a chosen password closes it");
    }

    /// A password made per call, because the `CodeQL` query on hard-coded
    /// cryptographic values follows a literal into any parameter called
    /// `password` (CLAUDE.md); `format!` over runtime values is not one.
    ///
    /// It takes no argument on purpose. An earlier `pw(tag: &str)` carried the
    /// caller's literal through `format!("{tag}-…")`, so every `pw("…")` call
    /// site was the alert. Each call returns a different password; the
    /// variable's name says which one it is.
    fn pw() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!(
            "pw-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn verifies(state: &AppState, plaintext: &str) -> bool {
        accounts::verify_password(&admin_hash(state), plaintext).unwrap_or(false)
    }

    /// Change the admin password the way the Accounts page does.
    fn change_on_the_accounts_page(state: &AppState, plaintext: &str) {
        state
            .with_db(|conn| -> Result<(), AccountsError> {
                let admin = conn.find_user_by_name("admin")?;
                let hash = accounts::hash_password(plaintext)?;
                conn.set_password(admin.id, &hash)
            })
            .expect("change the password");
    }

    /// The reason the applied marker exists. `CADDY_PWD` sits in plaintext in
    /// the config file; when it leaks, the first thing an owner does is change
    /// the password on the Accounts page. Restarting must not put the leaked
    /// one back.
    #[test]
    fn a_password_changed_on_the_accounts_page_survives_a_restart() {
        let (_d, state) = fixture();
        let from_config = pw();
        let chosen = pw();
        let cfg = config_with("CADDY_PWD", &from_config);

        bootstrap_admin_password(&state, Some(&cfg));
        assert!(
            verifies(&state, &from_config),
            "first start applies CADDY_PWD"
        );

        change_on_the_accounts_page(&state, &chosen);
        bootstrap_admin_password(&state, Some(&cfg));

        assert!(
            verifies(&state, &chosen),
            "the Accounts-page password was replaced at restart"
        );
        assert!(
            !verifies(&state, &from_config),
            "the CADDY_PWD that was set aside works again"
        );
    }

    /// The counterpart, and the recovery path: editing `CADDY_PWD` to a new value
    /// is an instruction, and replaces whatever the Accounts page set, so a
    /// forgotten password can always be reset from the config file.
    #[test]
    fn editing_caddy_pwd_replaces_a_password_set_on_the_accounts_page() {
        let (_d, state) = fixture();
        let first = pw();
        let chosen = pw();
        let second = pw();

        bootstrap_admin_password(&state, Some(&config_with("CADDY_PWD", &first)));
        change_on_the_accounts_page(&state, &chosen);
        bootstrap_admin_password(&state, Some(&config_with("CADDY_PWD", &second)));

        assert!(verifies(&state, &second), "a new CADDY_PWD must apply");
        assert!(!verifies(&state, &chosen));
        // And it is now the one on record: another restart changes nothing.
        bootstrap_admin_password(&state, Some(&config_with("CADDY_PWD", &second)));
        assert!(verifies(&state, &second));
    }

    /// A station that started before the marker existed has no record of what
    /// was last applied. It keeps the rule it always had: the config wins, and
    /// the marker is written so that the next restart can tell the difference.
    #[test]
    fn a_station_with_no_record_keeps_the_old_rule_and_starts_one() {
        let (_d, state) = fixture();
        let from_config = pw();
        let stored = pw();
        change_on_the_accounts_page(&state, &stored);
        assert!(!applied_marker_path(&state).exists());

        bootstrap_admin_password(&state, Some(&config_with("CADDY_PWD", &from_config)));

        assert!(verifies(&state, &from_config));
        assert!(
            applied_marker_path(&state).exists(),
            "the marker is written"
        );
        let on_disk = std::fs::read_to_string(applied_marker_path(&state)).expect("marker");
        assert!(
            !on_disk.contains(&from_config),
            "the marker must hold a hash, never the password"
        );
    }

    #[test]
    fn rotates_empty_seed_hash() {
        let (_d, state) = fixture();
        assert_eq!(admin_hash(&state), "");
        // Run the bootstrap path directly with a known plaintext, bypassing
        // the env var read (which is not safe to mutate under
        // `unsafe_code = deny`). This mirrors what `bootstrap_admin_password`
        // does once it has resolved CADDY_PWD.
        let plaintext = pw();
        let result = state.with_db(|conn| -> Result<(), AccountsError> {
            let admin = conn.find_user_by_name("admin")?;
            assert!(accounts::is_legacy_password_hash(&admin.pwd_argon2));
            let hash = accounts::hash_password(&plaintext)?;
            conn.set_password(admin.id, &hash)
        });
        result.expect("rotate");
        let h = admin_hash(&state);
        assert!(!h.is_empty());
        assert!(!accounts::is_legacy_password_hash(&h));
        assert!(accounts::verify_password(&h, &plaintext).unwrap());
    }

    #[test]
    fn legacy_plaintext_seed_recognised_as_legacy() {
        let (_d, state) = fixture();
        // Simulate a viewer account that was created pre-flip via the
        // PLAINTEXT-PLACEHOLDER scaffolding in #89.
        let legacy_password = pw();
        let placeholder = format!("PLAINTEXT-PLACEHOLDER:{legacy_password}");
        let _ = state.with_db(|conn| -> Result<(), AccountsError> {
            conn.create_user(
                "viewer1",
                &placeholder,
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
        assert!(accounts::verify_password(&viewer_hash, &legacy_password).unwrap());
    }
}
