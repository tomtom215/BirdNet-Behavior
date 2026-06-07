//! The `sessions` table store — bound cookie rows, one per outstanding session.

use rusqlite::{Connection, OptionalExtension, params};

use super::{AccountsError, Session};

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

    /// Look up an unexpired session by id. Used by the cookie middleware
    /// on every request to bind the cookie to a session row.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::NotFound`] if no row matches or the row
    /// has expired (caller treats both as "must sign in again").
    fn find_active_session(&self, id: &str) -> Result<Session, AccountsError>;

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

    fn find_active_session(&self, id: &str) -> Result<Session, AccountsError> {
        self.query_row(
            "SELECT id, user_id, issued_at, last_seen, expires_at, user_agent, ip_hash
             FROM sessions
             WHERE id = ?1 AND expires_at > datetime('now')",
            params![id],
            row_to_session,
        )
        .optional()?
        .ok_or_else(|| AccountsError::NotFound(format!("active session id={id}")))
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

#[cfg(test)]
mod tests {
    use super::*;
    // `find_user_by_name` (to fetch the seeded admin a session binds to)
    // comes from the sibling user store.
    use super::super::UserStore;
    use crate::migration;

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        migration::migrate(&conn).expect("migrate");
        conn
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
}
