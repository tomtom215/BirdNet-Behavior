//! Accounts, sessions, and audit log (O-15).
//!
//! This module defines the data shapes and the synchronous `SQLite`-backed
//! stores for the accounts surface described in
//! `docs/proposed_changes/O-15_accounts/DIFF.md`. The cookie path that the
//! `sessions` table binds itself to is plumbed by `birdnet-web::session`
//! (O-14); these stores stay quiet until the auth wire is flipped — see
//! the `TODO(O-15-followup)` markers below.
//!
//! ## Shape
//!
//! - [`User`] / [`Role`]: who can sign in.
//! - [`Session`]: a bound cookie row, one per outstanding session.
//! - [`AuditEntry`]: an immutable record of an admin-side mutation.
//! - [`UserStore`], [`SessionStore`], [`AuditLog`]: synchronous stores
//!   keyed on the existing `rusqlite::Connection` shared via the web
//!   server's `AppState::with_db`.
//!
//! All store impls follow the project rule: hand-rolled error types, no
//! `anyhow`/`thiserror`, no async — the web server bridges these stores
//! through `tokio::task::spawn_blocking` when it needs to.

use std::fmt;

use argon2::Argon2;
use password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use rusqlite::{Connection, OptionalExtension, params};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from the accounts/sessions/audit stores.
#[derive(Debug)]
pub enum AccountsError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// Lookup returned no row.
    NotFound(String),
    /// Constraint violation (e.g. duplicate username).
    Conflict(String),
    /// Invalid input (role string, username length, …).
    Invalid(String),
}

impl fmt::Display for AccountsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::Invalid(msg) => write!(f, "invalid: {msg}"),
        }
    }
}

impl std::error::Error for AccountsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for AccountsError {
    fn from(e: rusqlite::Error) -> Self {
        // Surface UNIQUE failures as Conflict so callers can render a
        // friendly inline error rather than a 500.
        if let rusqlite::Error::SqliteFailure(ref code, _) = e
            && code.code == rusqlite::ErrorCode::ConstraintViolation
        {
            return Self::Conflict("constraint violation".to_string());
        }
        Self::Sqlite(e)
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Account role.
///
/// Two values today: `admin` (full mutation rights) and `viewer`
/// (read-only on the `/admin/*` panel — sees overview, quality,
/// notification history, system status, and the audit log; cannot reach
/// settings, audio sources, alert rules, migrations, or system controls).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    /// SQL-on-disk form. Matches the `users.role CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Viewer => "viewer",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = AccountsError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "viewer" => Ok(Self::Viewer),
            other => Err(AccountsError::Invalid(format!("unknown role: {other}"))),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row from the `users` table.
#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    /// Argon2id hash of the password. Populated by O-15-followup when the
    /// auth wire is flipped onto the cookie path — until then the seed
    /// row writes an empty string and the basic-auth middleware keeps
    /// reading `CADDY_PWD` from the environment.
    pub pwd_argon2: String,
    pub role: Role,
    pub label: Option<String>,
    pub created_at: String,
    pub disabled_at: Option<String>,
}

/// One row from the `sessions` table.
///
/// A session is the persistent twin of O-14's stateless cookie: the
/// cookie carries a `bnb-session` token, the `sessions` row binds it to
/// a user, device label, and last-seen time.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub issued_at: String,
    pub last_seen: String,
    pub expires_at: String,
    pub user_agent: Option<String>,
    pub ip_hash: Option<String>,
}

/// One row from the `audit_log` table.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub user_id: Option<i64>,
    pub action: String,
    pub target: Option<String>,
    pub metadata: Option<String>,
}

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
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
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

// ---------------------------------------------------------------------------
// SessionStore
// ---------------------------------------------------------------------------

/// Operations on the `sessions` table.
pub trait SessionStore {
    /// Issue a new session row for `user_id`. The caller is responsible
    /// for generating the `id` (a 26-char base32 of 128 random bits per
    /// the O-15 DIFF) and the `expires_at` timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError`] on any database failure.
    fn create_session(
        &self,
        id: &str,
        user_id: i64,
        expires_at: &str,
        user_agent: Option<&str>,
        ip_hash: Option<&str>,
    ) -> Result<Session, AccountsError>;

    /// List a user's active sessions (most recent first; expired rows
    /// excluded by `expires_at > now`).
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn list_sessions(&self, user_id: i64) -> Result<Vec<Session>, AccountsError>;

    /// Touch the `last_seen` timestamp on a session.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::NotFound`] if the session id is unknown.
    fn touch_session(&self, id: &str) -> Result<(), AccountsError>;

    /// Delete one session by id.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn revoke_session(&self, id: &str) -> Result<(), AccountsError>;

    /// Delete every session for `user_id` *except* the one whose id
    /// matches `keep_id`. Used by the "Sign out of every other device"
    /// button on `/admin/accounts`.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn revoke_others(&self, user_id: i64, keep_id: &str) -> Result<usize, AccountsError>;

    /// Drop every expired session row. Should be called periodically by
    /// the operator (or by a background tidy in O-15-followup) to keep
    /// the table compact.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn prune_expired_sessions(&self) -> Result<usize, AccountsError>;
}

impl SessionStore for Connection {
    fn create_session(
        &self,
        id: &str,
        user_id: i64,
        expires_at: &str,
        user_agent: Option<&str>,
        ip_hash: Option<&str>,
    ) -> Result<Session, AccountsError> {
        self.execute(
            "INSERT INTO sessions (id, user_id, expires_at, user_agent, ip_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, user_id, expires_at, user_agent, ip_hash],
        )?;
        self.query_row(
            "SELECT id, user_id, issued_at, last_seen, expires_at, user_agent, ip_hash
             FROM sessions WHERE id = ?1",
            params![id],
            row_to_session,
        )
        .map_err(Into::into)
    }

    fn list_sessions(&self, user_id: i64) -> Result<Vec<Session>, AccountsError> {
        let mut stmt = self.prepare(
            "SELECT id, user_id, issued_at, last_seen, expires_at, user_agent, ip_hash
             FROM sessions
             WHERE user_id = ?1 AND expires_at > datetime('now')
             ORDER BY last_seen DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], row_to_session)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn touch_session(&self, id: &str) -> Result<(), AccountsError> {
        let n = self.execute(
            "UPDATE sessions SET last_seen = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        if n == 0 {
            return Err(AccountsError::NotFound(format!("session id={id}")));
        }
        Ok(())
    }

    fn revoke_session(&self, id: &str) -> Result<(), AccountsError> {
        self.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn revoke_others(&self, user_id: i64, keep_id: &str) -> Result<usize, AccountsError> {
        let n = self.execute(
            "DELETE FROM sessions WHERE user_id = ?1 AND id != ?2",
            params![user_id, keep_id],
        )?;
        Ok(n)
    }

    fn prune_expired_sessions(&self) -> Result<usize, AccountsError> {
        let n = self.execute(
            "DELETE FROM sessions WHERE expires_at <= datetime('now')",
            [],
        )?;
        Ok(n)
    }
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        user_id: row.get(1)?,
        issued_at: row.get(2)?,
        last_seen: row.get(3)?,
        expires_at: row.get(4)?,
        user_agent: row.get(5)?,
        ip_hash: row.get(6)?,
    })
}

// ---------------------------------------------------------------------------
// AuditLog
// ---------------------------------------------------------------------------

/// Operations on the `audit_log` table.
pub trait AuditLog {
    /// Append a row. Returns the inserted row.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn record(
        &self,
        user_id: Option<i64>,
        action: &str,
        target: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<AuditEntry, AccountsError>;

    /// Return the most recent `limit` rows.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn recent(&self, limit: usize) -> Result<Vec<AuditEntry>, AccountsError>;

    /// Delete rows older than `retention_days`. Returns the row count.
    /// O-15's documented retention default is 180 days.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn prune(&self, retention_days: u32) -> Result<usize, AccountsError>;
}

impl AuditLog for Connection {
    fn record(
        &self,
        user_id: Option<i64>,
        action: &str,
        target: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<AuditEntry, AccountsError> {
        self.execute(
            "INSERT INTO audit_log (user_id, action, target, metadata)
             VALUES (?1, ?2, ?3, ?4)",
            params![user_id, action, target, metadata],
        )?;
        let id = self.last_insert_rowid();
        self.query_row(
            "SELECT id, at, user_id, action, target, metadata
             FROM audit_log WHERE id = ?1",
            params![id],
            row_to_audit_entry,
        )
        .map_err(Into::into)
    }

    fn recent(&self, limit: usize) -> Result<Vec<AuditEntry>, AccountsError> {
        // `at` is `datetime('now')` granularity (whole seconds), so two
        // rapid inserts share a timestamp. Use `id DESC` as a tiebreaker
        // to make the ordering deterministic regardless of clock
        // resolution.
        let mut stmt = self.prepare(
            "SELECT id, at, user_id, action, target, metadata
             FROM audit_log ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![i64::try_from(limit).unwrap_or(50)], row_to_audit_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn prune(&self, retention_days: u32) -> Result<usize, AccountsError> {
        let n = self.execute(
            "DELETE FROM audit_log WHERE at < datetime('now', '-' || ?1 || ' days')",
            params![retention_days],
        )?;
        Ok(n)
    }
}

fn row_to_audit_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    Ok(AuditEntry {
        id: row.get(0)?,
        at: row.get(1)?,
        user_id: row.get(2)?,
        action: row.get(3)?,
        target: row.get(4)?,
        metadata: row.get(5)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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

    #[test]
    fn session_round_trip_and_revoke() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        let s = conn
            .create_session(
                "sess-aaa",
                admin.id,
                "2099-01-01 00:00:00",
                Some("Mozilla/5.0"),
                Some("hash"),
            )
            .expect("create session");
        assert_eq!(s.user_id, admin.id);
        let active = conn.list_sessions(admin.id).unwrap();
        assert_eq!(active.len(), 1);
        conn.revoke_session("sess-aaa").unwrap();
        assert!(conn.list_sessions(admin.id).unwrap().is_empty());
    }

    #[test]
    fn revoke_others_keeps_current() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        conn.create_session("a", admin.id, "2099-01-01 00:00:00", None, None)
            .unwrap();
        conn.create_session("b", admin.id, "2099-01-01 00:00:00", None, None)
            .unwrap();
        conn.create_session("c", admin.id, "2099-01-01 00:00:00", None, None)
            .unwrap();
        let n = conn.revoke_others(admin.id, "b").unwrap();
        assert_eq!(n, 2);
        let rest = conn.list_sessions(admin.id).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, "b");
    }

    #[test]
    fn list_sessions_hides_expired_rows() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        conn.create_session("future", admin.id, "2099-01-01 00:00:00", None, None)
            .unwrap();
        conn.create_session("past", admin.id, "1999-01-01 00:00:00", None, None)
            .unwrap();
        let active = conn.list_sessions(admin.id).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "future");
    }

    #[test]
    fn audit_record_and_recent() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        conn.record(Some(admin.id), "settings.update", Some("audio"), None)
            .unwrap();
        conn.record(Some(admin.id), "rule.toggle", Some("rule:nightjar"), None)
            .unwrap();
        let recent = conn.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        // Most recent first.
        assert_eq!(recent[0].action, "rule.toggle");
        assert_eq!(recent[1].action, "settings.update");
    }

    #[test]
    fn audit_prune_drops_old_rows() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        // Insert one ancient row.
        conn.execute(
            "INSERT INTO audit_log (at, user_id, action) VALUES (datetime('now','-365 days'), ?1, 'old')",
            params![admin.id],
        )
        .unwrap();
        conn.record(Some(admin.id), "recent", None, None).unwrap();
        let removed = conn.prune(180).unwrap();
        assert_eq!(removed, 1);
        let rest = conn.recent(10).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].action, "recent");
    }

    #[test]
    fn argon2_hash_round_trips_via_verify() {
        let hash = hash_password("correct-horse-battery-staple").unwrap();
        assert!(verify_password(&hash, "correct-horse-battery-staple").unwrap());
        assert!(!verify_password(&hash, "wrong-password").unwrap());
    }

    #[test]
    fn argon2_hash_generates_unique_salt_per_call() {
        // Two hashes of the same plaintext must differ (random salt).
        let a = hash_password("same-secret").unwrap();
        let b = hash_password("same-secret").unwrap();
        assert_ne!(a, b);
        // Both still verify against the original plaintext.
        assert!(verify_password(&a, "same-secret").unwrap());
        assert!(verify_password(&b, "same-secret").unwrap());
    }

    #[test]
    fn empty_hash_never_verifies() {
        assert!(!verify_password("", "anything").unwrap());
    }

    #[test]
    fn malformed_hash_returns_invalid_not_silent_false() {
        let err = verify_password("not-a-phc-string", "x").unwrap_err();
        assert!(matches!(err, AccountsError::Invalid(_)));
    }

    #[test]
    fn legacy_plaintext_placeholder_still_verifies_during_migration() {
        // Seeds shipped in #89 stored `PLAINTEXT-PLACEHOLDER:{password}` so
        // an operator who set a viewer password before this PR can still
        // sign in (and get rotated to argon2id on next use).
        let legacy = "PLAINTEXT-PLACEHOLDER:hunter2";
        assert!(verify_password(legacy, "hunter2").unwrap());
        assert!(!verify_password(legacy, "wrong").unwrap());
    }

    #[test]
    fn is_legacy_password_hash_detects_both_shapes() {
        assert!(is_legacy_password_hash(""));
        assert!(is_legacy_password_hash("PLAINTEXT-PLACEHOLDER:anything"));
        let real = hash_password("real").unwrap();
        assert!(!is_legacy_password_hash(&real));
    }

    #[test]
    fn role_round_trips_through_string() {
        assert_eq!("admin".parse::<Role>().unwrap(), Role::Admin);
        assert_eq!("viewer".parse::<Role>().unwrap(), Role::Viewer);
        assert!("other".parse::<Role>().is_err());
        assert_eq!(Role::Admin.as_str(), "admin");
        assert_eq!(Role::Viewer.as_str(), "viewer");
    }
}
