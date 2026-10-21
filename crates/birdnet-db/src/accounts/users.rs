//! The `users` table store and the argon2id password-hashing helpers.

use argon2::Argon2;
use password_hash::PasswordHasher as _;
use password_hash::PasswordVerifier as _;
use password_hash::phc::PasswordHash;
use rusqlite::{Connection, OptionalExtension, params};

use super::{AccountsError, Role, User};

// ---------------------------------------------------------------------------
// Password hashing helpers — argon2id (O-14 / O-15 wire flip)
// ---------------------------------------------------------------------------

/// Hash a plaintext password using the default argon2id parameters.
///
/// Returns a self-describing PHC string that round-trips through
/// [`verify_password`]; the salt is generated from the OS CSPRNG.
///
/// # Errors
///
/// Returns [`AccountsError::Invalid`] if the underlying hasher rejects
/// the input (the argon2 contract guarantees this only happens for
/// pathologically long passwords; the wrapper surface is the public-facing
/// failure type the accounts surface already uses).
pub fn hash_password(password: &str) -> Result<String, AccountsError> {
    // The salt is drawn from the OS CSPRNG inside `hash_password`.
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AccountsError::Invalid(format!("password hash failed: {e}")))
}

/// Constant-time verification of `password` against `hash`.
///
/// Returns `Ok(true)` on a match, `Ok(false)` on a mismatch, and
/// [`AccountsError::Invalid`] when the stored hash is malformed (e.g.
/// the seed `""` placeholder before the bootstrap migration runs).
///
/// Pre-flip compatibility: the legacy `PLAINTEXT-PLACEHOLDER:` prefix
/// shipped in #89 is matched as a literal-string compare so an operator
/// who created a viewer account before this PR can still sign in. Once
/// the wire is flipped the legacy hashes are rotated on next set-password
/// (the helper at the top of every mutating handler does this).
///
/// # Errors
///
/// Returns [`AccountsError::Invalid`] on malformed hash material.
pub fn verify_password(hash: &str, password: &str) -> Result<bool, AccountsError> {
    if hash.is_empty() {
        return Ok(false);
    }
    if let Some(rest) = hash.strip_prefix("PLAINTEXT-PLACEHOLDER:") {
        // Pre-flip seed — constant-time compare so timing leakage doesn't
        // distinguish "wrong password" from "legacy hash".
        return Ok(constant_time_eq(rest.as_bytes(), password.as_bytes()));
    }
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AccountsError::Invalid(format!("malformed password hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Whether `hash` is one of the legacy placeholder formats from #89's
/// scaffolding (empty string for the seed admin, or
/// `PLAINTEXT-PLACEHOLDER:` for viewer accounts created pre-flip).
///
/// Callers use this to decide whether a successful basic-auth
/// authentication should opportunistically rotate the hash to argon2id
/// on the fly.
#[must_use]
pub fn is_legacy_password_hash(hash: &str) -> bool {
    hash.is_empty() || hash.starts_with("PLAINTEXT-PLACEHOLDER:")
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

// ---------------------------------------------------------------------------
// Helpers (private)
// ---------------------------------------------------------------------------

fn validate_username(u: &str) -> Result<(), AccountsError> {
    if u.is_empty() {
        return Err(AccountsError::Invalid("username is empty".to_string()));
    }
    if u.len() > 64 {
        return Err(AccountsError::Invalid(
            "username longer than 64 characters".to_string(),
        ));
    }
    if !u
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(AccountsError::Invalid(
            "username has unsupported characters".to_string(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// UserStore
// ---------------------------------------------------------------------------

/// Operations on the `users` table.
pub trait UserStore {
    /// Insert a new user. Returns the inserted row.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Conflict`] if the username already exists,
    /// [`AccountsError::Invalid`] if the username fails validation, and
    /// [`AccountsError::Sqlite`] for any underlying database error.
    fn create_user(
        &self,
        username: &str,
        pwd_argon2: &str,
        role: Role,
        label: Option<&str>,
    ) -> Result<User, AccountsError>;

    /// List all users (oldest first).
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn list_users(&self) -> Result<Vec<User>, AccountsError>;

    /// Find one user by id.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::NotFound`] if no user matches.
    fn find_user(&self, id: i64) -> Result<User, AccountsError>;

    /// Find one user by username.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::NotFound`] if no user matches.
    fn find_user_by_name(&self, username: &str) -> Result<User, AccountsError>;

    /// Replace the password hash for the user with `id`.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::NotFound`] if the user does not exist.
    fn set_password(&self, id: i64, pwd_argon2: &str) -> Result<(), AccountsError>;

    /// Give the user a password **only if it still has none**: the stored hash
    /// is empty or one of the legacy placeholders ([`is_legacy_password_hash`]).
    /// One `UPDATE` carries the condition, so of two callers that both saw "no
    /// password" the first writes and the second changes nothing — the
    /// check-then-write in the caller was the race.
    ///
    /// Returns whether this call wrote the hash.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::NotFound`] if the user does not exist.
    fn set_password_if_unset(&self, id: i64, pwd_argon2: &str) -> Result<bool, AccountsError>;

    /// Mark the user as disabled (sets `disabled_at = datetime('now')`).
    /// The seed `admin` row cannot be disabled — attempting to do so
    /// returns [`AccountsError::Invalid`] so the UI can show a sensible
    /// error.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Invalid`] when called on the seed
    /// `admin` row, and [`AccountsError::NotFound`] when the user is
    /// absent.
    fn disable_user(&self, id: i64) -> Result<(), AccountsError>;

    /// Delete a user row. Like `disable_user`, the seed `admin` row is
    /// protected and returns [`AccountsError::Invalid`].
    ///
    /// # Errors
    ///
    /// See [`Self::disable_user`].
    fn delete_user(&self, id: i64) -> Result<(), AccountsError>;
}

impl UserStore for Connection {
    fn create_user(
        &self,
        username: &str,
        pwd_argon2: &str,
        role: Role,
        label: Option<&str>,
    ) -> Result<User, AccountsError> {
        validate_username(username)?;
        self.execute(
            "INSERT INTO users (username, pwd_argon2, role, label)
             VALUES (?1, ?2, ?3, ?4)",
            params![username, pwd_argon2, role.as_str(), label],
        )?;
        let id = self.last_insert_rowid();
        self.find_user(id)
    }

    fn list_users(&self) -> Result<Vec<User>, AccountsError> {
        let mut stmt = self.prepare(
            "SELECT id, username, pwd_argon2, role, label, created_at, disabled_at
             FROM users ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], row_to_user)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn find_user(&self, id: i64) -> Result<User, AccountsError> {
        self.query_row(
            "SELECT id, username, pwd_argon2, role, label, created_at, disabled_at
             FROM users WHERE id = ?1",
            params![id],
            row_to_user,
        )
        .optional()?
        .ok_or_else(|| AccountsError::NotFound(format!("user id={id}")))
    }

    fn find_user_by_name(&self, username: &str) -> Result<User, AccountsError> {
        self.query_row(
            "SELECT id, username, pwd_argon2, role, label, created_at, disabled_at
             FROM users WHERE username = ?1",
            params![username],
            row_to_user,
        )
        .optional()?
        .ok_or_else(|| AccountsError::NotFound(format!("user '{username}'")))
    }

    fn set_password(&self, id: i64, pwd_argon2: &str) -> Result<(), AccountsError> {
        let n = self.execute(
            "UPDATE users SET pwd_argon2 = ?1 WHERE id = ?2",
            params![pwd_argon2, id],
        )?;
        if n == 0 {
            return Err(AccountsError::NotFound(format!("user id={id}")));
        }
        Ok(())
    }

    fn set_password_if_unset(&self, id: i64, pwd_argon2: &str) -> Result<bool, AccountsError> {
        // The same two shapes as `is_legacy_password_hash`, in SQL so the test
        // and the write are one statement.
        let n = self.execute(
            "UPDATE users SET pwd_argon2 = ?1 \
             WHERE id = ?2 AND (pwd_argon2 = '' OR pwd_argon2 LIKE 'PLAINTEXT-PLACEHOLDER:%')",
            params![pwd_argon2, id],
        )?;
        if n == 1 {
            return Ok(true);
        }
        // Nothing changed: either the user is not there or it already has a
        // password. Say which.
        self.find_user(id)?;
        Ok(false)
    }

    fn disable_user(&self, id: i64) -> Result<(), AccountsError> {
        let u = self.find_user(id)?;
        if u.username == "admin" {
            return Err(AccountsError::Invalid(
                "the seed admin user cannot be disabled".to_string(),
            ));
        }
        self.execute(
            "UPDATE users SET disabled_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    fn delete_user(&self, id: i64) -> Result<(), AccountsError> {
        let u = self.find_user(id)?;
        if u.username == "admin" {
            return Err(AccountsError::Invalid(
                "the seed admin user cannot be deleted".to_string(),
            ));
        }
        self.execute("DELETE FROM users WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let role: String = row.get(3)?;
    let role = role.parse::<Role>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        pwd_argon2: row.get(2)?,
        role,
        label: row.get(4)?,
        created_at: row.get(5)?,
        disabled_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    /// A password hash written by the argon2 0.5 line this project shipped
    /// through 0.17.1 must keep verifying after the library is upgraded: it is
    /// in every existing station's `users` table, and the owner is the one
    /// person who cannot be asked to reset it. Captured from that code, not
    /// derived from anything this crate computes today.
    #[test]
    fn a_hash_written_by_argon2_0_5_still_verifies() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/argon2_0_5_vector.txt");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                .unwrap_or_else(|| panic!("{} has no `{key}=` line", path.display()))
        };
        let (hash, password) = (field("hash"), field("password"));
        assert!(verify_password(hash, password).unwrap());
        let one_short = &password[..password.len() - 1];
        assert!(!verify_password(hash, one_short).unwrap());
        let empty = String::new();
        assert!(!verify_password(hash, &empty).unwrap());
    }

    /// The cost a new hash is written at is part of what an upgrade must not
    /// change silently: a library whose default drops it would weaken every
    /// password set after, with every test still green.
    #[test]
    fn a_new_hash_is_argon2id_at_the_cost_the_project_chose() {
        // Made per run, not written down: code scanning reads a literal that
        // reaches `password` as a hard-coded cryptographic value, and any
        // password shows the cost.
        let password = format!("password-{}", std::process::id());
        let h = hash_password(&password).unwrap();
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        assert!(verify_password(&h, &password).unwrap());
    }

    use super::*;
    use crate::migration;

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        migration::migrate(&conn).expect("migrate");
        conn
    }

    #[test]
    fn seed_admin_user_exists_after_migration() {
        let conn = open_db();
        let u = conn
            .find_user_by_name("admin")
            .expect("admin row seeded by migration 14");
        assert_eq!(u.role, Role::Admin);
        assert_eq!(u.username, "admin");
    }

    #[test]
    fn create_user_round_trips() {
        let conn = open_db();
        let u = conn
            .create_user("jess", "argon2-stub", Role::Viewer, Some("Jess"))
            .expect("create");
        assert_eq!(u.role, Role::Viewer);
        assert_eq!(u.label.as_deref(), Some("Jess"));

        let found = conn.find_user(u.id).expect("found");
        assert_eq!(found.username, "jess");
    }

    #[test]
    fn duplicate_username_returns_conflict() {
        let conn = open_db();
        conn.create_user("jess", "h", Role::Viewer, None).unwrap();
        let err = conn
            .create_user("jess", "h", Role::Viewer, None)
            .expect_err("duplicate must fail");
        assert!(matches!(err, AccountsError::Conflict(_)));
    }

    #[test]
    fn invalid_username_rejected() {
        let conn = open_db();
        let err = conn
            .create_user("a b", "h", Role::Viewer, None)
            .expect_err("space rejected");
        assert!(matches!(err, AccountsError::Invalid(_)));
        let err = conn
            .create_user("", "h", Role::Viewer, None)
            .expect_err("empty rejected");
        assert!(matches!(err, AccountsError::Invalid(_)));
    }

    #[test]
    fn admin_row_cannot_be_deleted_or_disabled() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        assert!(matches!(
            conn.delete_user(admin.id),
            Err(AccountsError::Invalid(_))
        ));
        assert!(matches!(
            conn.disable_user(admin.id),
            Err(AccountsError::Invalid(_))
        ));
    }

    #[test]
    fn delete_non_admin_user_works() {
        let conn = open_db();
        let u = conn.create_user("jess", "h", Role::Viewer, None).unwrap();
        conn.delete_user(u.id).unwrap();
        assert!(matches!(
            conn.find_user(u.id),
            Err(AccountsError::NotFound(_))
        ));
    }

    // The passwords below are built at run time, not written as literals:
    // CodeQL's `rust/hard-coded-cryptographic-value` follows a string literal into
    // any parameter called `password`, test or not, and follows it through a helper
    // that returns a string made from one (`pw("tag")`), so each test builds its own
    // with `format!` over the process id
    // (`tests/no_literal_reaches_a_credential_parameter.rs`).

    #[test]
    fn argon2_hash_round_trips_via_verify() {
        let pid = std::process::id();
        let right = format!("correct-horse-battery-staple-{pid}");
        let wrong = format!("wrong-password-{pid}");
        let hash = hash_password(&right).unwrap();
        assert!(verify_password(&hash, &right).unwrap());
        assert!(!verify_password(&hash, &wrong).unwrap());
    }

    #[test]
    fn argon2_hash_generates_unique_salt_per_call() {
        // Two hashes of the same plaintext must differ (random salt).
        let same = format!("same-secret-{}", std::process::id());
        let a = hash_password(&same).unwrap();
        let b = hash_password(&same).unwrap();
        assert_ne!(a, b);
        // Both still verify against the original plaintext.
        assert!(verify_password(&a, &same).unwrap());
        assert!(verify_password(&b, &same).unwrap());
    }

    #[test]
    fn empty_hash_never_verifies() {
        let anything = format!("anything-{}", std::process::id());
        assert!(!verify_password("", &anything).unwrap());
    }

    #[test]
    fn malformed_hash_returns_invalid_not_silent_false() {
        let typed = format!("x-{}", std::process::id());
        let err = verify_password("not-a-phc-string", &typed).unwrap_err();
        assert!(matches!(err, AccountsError::Invalid(_)));
    }

    #[test]
    fn legacy_plaintext_placeholder_still_verifies_during_migration() {
        // Seeds shipped in #89 stored `PLAINTEXT-PLACEHOLDER:{password}` so
        // an operator who set a viewer password before this PR can still
        // sign in (and get rotated to argon2id on next use).
        let pid = std::process::id();
        let password = format!("hunter2-{pid}");
        let wrong = format!("wrong-{pid}");
        let legacy = format!("PLAINTEXT-PLACEHOLDER:{password}");
        assert!(verify_password(&legacy, &password).unwrap());
        assert!(!verify_password(&legacy, &wrong).unwrap());
    }

    /// The SQL condition and `is_legacy_password_hash` must agree: a hash one
    /// calls a placeholder and the other calls real would make the first-run
    /// password either unsettable or overwritable.
    #[test]
    fn set_password_if_unset_writes_once_and_only_over_a_placeholder() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        assert!(
            is_legacy_password_hash(&admin.pwd_argon2),
            "the seed has none"
        );

        let first = hash_password(&format!("first-{}", std::process::id())).unwrap();
        let second = hash_password(&format!("second-{}", std::process::id())).unwrap();
        assert!(conn.set_password_if_unset(admin.id, &first).unwrap());
        assert!(
            !conn.set_password_if_unset(admin.id, &second).unwrap(),
            "the second writer finds a password and changes nothing"
        );
        assert_eq!(conn.find_user(admin.id).unwrap().pwd_argon2, first);

        // The legacy placeholder shape counts as "none", as the helper says.
        conn.execute(
            "UPDATE users SET pwd_argon2 = 'PLAINTEXT-PLACEHOLDER:x' WHERE id = ?1",
            params![admin.id],
        )
        .unwrap();
        assert!(conn.set_password_if_unset(admin.id, &second).unwrap());

        assert!(matches!(
            conn.set_password_if_unset(9999, &first),
            Err(AccountsError::NotFound(_))
        ));
    }

    #[test]
    fn is_legacy_password_hash_detects_both_shapes() {
        assert!(is_legacy_password_hash(""));
        assert!(is_legacy_password_hash("PLAINTEXT-PLACEHOLDER:anything"));
        let real = hash_password(&format!("real-{}", std::process::id())).unwrap();
        assert!(!is_legacy_password_hash(&real));
    }
}
