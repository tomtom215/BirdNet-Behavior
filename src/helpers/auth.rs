//! One-shot bootstrap that rotates the seed admin row's password hash
//! when the operator first sets `CADDY_PWD`, so the cookie wire flip can
//! verify against a real argon2id hash instead of the empty seed.
//!
//! Runs once per process start, immediately after `AppState` is built
//! (i.e. after migrations have created the row). Behaviour:
//!
//! * `CADDY_PWD` unset → no-op. The basic-auth path was already letting
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

use birdnet_db::accounts::{self, AccountsError, UserStore};
use birdnet_web::state::AppState;

/// Run the one-shot admin-row password bootstrap. Idempotent across
/// restarts.
pub fn bootstrap_admin_password(state: &AppState) {
    let Ok(env_pwd) = std::env::var("CADDY_PWD") else {
        tracing::debug!("CADDY_PWD unset; admin password bootstrap skipped");
        return;
    };
    if env_pwd.is_empty() {
        tracing::debug!("CADDY_PWD empty; admin password bootstrap skipped");
        return;
    }

    let outcome = state.with_db(|conn| -> Result<BootstrapOutcome, AccountsError> {
        let admin = conn.find_user_by_name("admin")?;
        if accounts::is_legacy_password_hash(&admin.pwd_argon2) {
            let hash = accounts::hash_password(&env_pwd)?;
            conn.set_password(admin.id, &hash)?;
            return Ok(BootstrapOutcome::RotatedLegacy);
        }
        let verifies = accounts::verify_password(&admin.pwd_argon2, &env_pwd)
            .unwrap_or(false);
        if !verifies {
            let hash = accounts::hash_password(&env_pwd)?;
            conn.set_password(admin.id, &hash)?;
            return Ok(BootstrapOutcome::RotatedAfterEnvChange);
        }
        Ok(BootstrapOutcome::AlreadyConsistent)
    });

    match outcome {
        Ok(BootstrapOutcome::RotatedLegacy) => tracing::info!(
            "admin password hash initialised from CADDY_PWD"
        ),
        Ok(BootstrapOutcome::RotatedAfterEnvChange) => tracing::info!(
            "admin password hash rotated to match updated CADDY_PWD"
        ),
        Ok(BootstrapOutcome::AlreadyConsistent) => tracing::debug!(
            "admin password hash already up-to-date"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "admin password bootstrap failed; basic-auth path stays usable"
        ),
    }
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
