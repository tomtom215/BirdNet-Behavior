//! Accounts, sessions, and audit log (O-15).
//!
//! This module defines the shared data shapes and re-exports the synchronous
//! `SQLite`-backed stores for the accounts surface described in the O-15
//! accounts design proposal. The cookie path that the `sessions` table binds
//! itself to is plumbed by `birdnet-web::session` (O-14).
//!
//! ## Shape
//!
//! - [`User`] / [`Role`]: who can sign in.
//! - [`Session`]: a bound cookie row, one per outstanding session.
//! - [`AuditEntry`]: an immutable record of an admin-side mutation.
//! - [`UserStore`] (incl. the password-hashing helpers), [`SessionStore`],
//!   [`AuditLog`]: synchronous stores keyed on the existing
//!   `rusqlite::Connection` shared via the web server's `AppState::with_db`.
//!   Each lives in its own submodule beside its tests.
//!
//! All store impls follow the project rule: hand-rolled error types, no
//! `anyhow`/`thiserror`, no async — the web server bridges these stores
//! through `tokio::task::spawn_blocking` when it needs to.

use std::fmt;

mod audit;
mod sessions;
mod users;

pub use audit::AuditLog;
pub use sessions::SessionStore;
pub use users::{UserStore, hash_password, is_legacy_password_hash, verify_password};

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
    /// Full mutation rights across the `/admin/*` panel.
    Admin,
    /// Read-only access: can view overview, quality, notifications, system status, and the
    /// audit log, but cannot reach settings, audio sources, alert rules, migrations, or
    /// system controls.
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
    /// Auto-incremented primary key.
    pub id: i64,
    /// Login username; must be unique within the station.
    pub username: String,
    /// Argon2id hash of the password. Populated by O-15-followup when the
    /// auth wire is flipped onto the cookie path — until then the seed
    /// row writes an empty string and the basic-auth middleware keeps
    /// reading `CADDY_PWD` from the environment.
    pub pwd_argon2: String,
    /// Role controlling which admin panel sections this user may access.
    pub role: Role,
    /// Optional display name shown in the admin UI alongside the username.
    pub label: Option<String>,
    /// ISO-8601 timestamp when this account was created.
    pub created_at: String,
    /// ISO-8601 timestamp when the account was disabled, if applicable.
    pub disabled_at: Option<String>,
}

/// One row from the `sessions` table.
///
/// A session is the persistent twin of O-14's stateless cookie: the
/// cookie carries a `bnb-session` token, the `sessions` row binds it to
/// a user, device label, and last-seen time.
#[derive(Debug, Clone)]
pub struct Session {
    /// Opaque session token stored in the `bnb-session` cookie.
    pub id: String,
    /// Foreign key into the `users` table.
    pub user_id: i64,
    /// ISO-8601 timestamp when the session was created.
    pub issued_at: String,
    /// ISO-8601 timestamp of the most recent authenticated request.
    pub last_seen: String,
    /// ISO-8601 timestamp after which the session is considered expired.
    pub expires_at: String,
    /// `User-Agent` header from the browser that created the session, if available.
    pub user_agent: Option<String>,
    /// SHA-256 hash of the client IP address (privacy-safe for display/logs).
    pub ip_hash: Option<String>,
}

/// One row from the `audit_log` table.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Auto-incremented primary key.
    pub id: i64,
    /// ISO-8601 timestamp of the audited event.
    pub at: String,
    /// User who performed the action; `None` for system-initiated events.
    pub user_id: Option<i64>,
    /// Short verb describing the action (e.g. `create_user`, `revoke_session`).
    pub action: String,
    /// Identifier of the affected entity (e.g. username, session id).
    pub target: Option<String>,
    /// JSON blob with additional context (diff, before/after values, etc.).
    pub metadata: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_round_trips_through_string() {
        assert_eq!("admin".parse::<Role>().unwrap(), Role::Admin);
        assert_eq!("viewer".parse::<Role>().unwrap(), Role::Viewer);
        assert!("other".parse::<Role>().is_err());
        assert_eq!(Role::Admin.as_str(), "admin");
        assert_eq!(Role::Viewer.as_str(), "viewer");
    }
}
