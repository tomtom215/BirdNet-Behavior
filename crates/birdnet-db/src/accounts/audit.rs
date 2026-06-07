//! The `audit_log` table store — an immutable record of admin-side mutations.

use rusqlite::{Connection, params};

use super::{AccountsError, AuditEntry};

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

    /// Query rows in the inclusive date range `[from, to]`, optionally
    /// filtered by a `LIKE` pattern on the action column. Most recent
    /// first; capped at `limit` rows so a wide range can't ship a
    /// 100 000-row page.
    ///
    /// `from` and `to` are `YYYY-MM-DD` strings (the `audit_log.at`
    /// column carries `YYYY-MM-DD HH:MM:SS` so lex comparison sorts
    /// chronologically when extended with ` 00:00:00` / ` 23:59:59`).
    /// An empty `action_like` skips the action filter; otherwise the
    /// pattern is matched via SQL `LIKE` so the caller can pass a
    /// prefix like `"rule.%"` or an infix `"%password%"`.
    ///
    /// # Errors
    ///
    /// Returns [`AccountsError::Sqlite`] on database failure.
    fn query(
        &self,
        from: &str,
        to: &str,
        action_like: &str,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, AccountsError>;

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
            .query_map(
                params![i64::try_from(limit).unwrap_or(50)],
                row_to_audit_entry,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn query(
        &self,
        from: &str,
        to: &str,
        action_like: &str,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, AccountsError> {
        // Widen `from` / `to` (YYYY-MM-DD) into half-open day bounds so
        // lex comparison against the column's full `YYYY-MM-DD HH:MM:SS`
        // shape works.
        let from_bound = format!("{from} 00:00:00");
        let to_bound = format!("{to} 23:59:59");
        let lim = i64::try_from(limit).unwrap_or(200);
        let rows: Vec<AuditEntry> = if action_like.is_empty() {
            let mut stmt = self.prepare(
                "SELECT id, at, user_id, action, target, metadata
                 FROM audit_log
                 WHERE at BETWEEN ?1 AND ?2
                 ORDER BY at DESC, id DESC
                 LIMIT ?3",
            )?;
            stmt.query_map(params![from_bound, to_bound, lim], row_to_audit_entry)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.prepare(
                "SELECT id, at, user_id, action, target, metadata
                 FROM audit_log
                 WHERE at BETWEEN ?1 AND ?2 AND action LIKE ?3
                 ORDER BY at DESC, id DESC
                 LIMIT ?4",
            )?;
            stmt.query_map(
                params![from_bound, to_bound, action_like, lim],
                row_to_audit_entry,
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    // `find_user_by_name` (to fetch the seeded admin that owns audit rows)
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
    fn audit_query_filters_by_date_and_action() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        // Spread a few rows across different days + action prefixes.
        // SQLite's `audit_log.at` defaults to `datetime('now')` so we
        // pin specific dates by using the literal INSERT form.
        conn.execute(
            "INSERT INTO audit_log (at, user_id, action, target) VALUES (?1, ?2, ?3, ?4)",
            params!["2026-05-20 10:00:00", admin.id, "settings.update", "audio"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audit_log (at, user_id, action, target) VALUES (?1, ?2, ?3, ?4)",
            params![
                "2026-05-21 11:00:00",
                admin.id,
                "rule.toggle",
                "rule:nightjar"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audit_log (at, user_id, action, target) VALUES (?1, ?2, ?3, ?4)",
            params!["2026-05-25 09:00:00", admin.id, "rule.create", "rule:robin"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audit_log (at, user_id, action, target) VALUES (?1, ?2, ?3, ?4)",
            params![
                "2026-06-01 14:00:00",
                admin.id,
                "settings.update",
                "detection"
            ],
        )
        .unwrap();

        // Range [2026-05-20, 2026-05-30] — 3 of 4 rows; date filter excludes the June row.
        let in_range = conn.query("2026-05-20", "2026-05-30", "", 100).unwrap();
        assert_eq!(in_range.len(), 3);
        // Most recent first.
        assert_eq!(in_range[0].at, "2026-05-25 09:00:00");

        // Add action filter `rule.%` — only the two rule.* rows survive.
        let rules = conn
            .query("2026-05-20", "2026-05-30", "rule.%", 100)
            .unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules.iter().all(|r| r.action.starts_with("rule.")));

        // Tight infix match still works.
        let toggles = conn
            .query("2026-05-20", "2026-05-30", "%toggle%", 100)
            .unwrap();
        assert_eq!(toggles.len(), 1);
        assert_eq!(toggles[0].action, "rule.toggle");

        // Limit caps the response even when the predicate matches more.
        let capped = conn.query("2026-05-20", "2026-05-30", "", 2).unwrap();
        assert_eq!(capped.len(), 2);
    }

    #[test]
    fn audit_query_empty_when_range_matches_nothing() {
        let conn = open_db();
        let admin = conn.find_user_by_name("admin").unwrap();
        conn.record(Some(admin.id), "settings.update", None, None)
            .unwrap();
        // Range entirely in the past.
        let rows = conn.query("1990-01-01", "1990-01-02", "", 10).unwrap();
        assert!(rows.is_empty());
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
}
