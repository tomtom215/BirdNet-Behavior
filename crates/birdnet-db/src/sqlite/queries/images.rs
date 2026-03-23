//! Image blacklist queries.
//!
//! Manages a list of species image URLs that should not be shown,
//! replacing BirdNET-Pi's `blacklisted_images.txt` file.

use rusqlite::{Connection, params};

use crate::sqlite::connection::DbError;

/// A row from the `image_blacklist` table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageBlacklist {
    /// Row ID.
    pub id: i64,
    /// Scientific name of the species.
    pub sci_name: String,
    /// Blocked image URL.
    pub url: String,
    /// Optional reason for blacklisting.
    pub reason: Option<String>,
    /// When this entry was added.
    pub blacklisted_at: String,
}

/// Add an image URL to the blacklist.
///
/// Returns `true` if inserted, `false` if already present (duplicate).
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn add_image_blacklist(
    conn: &Connection,
    sci_name: &str,
    url: &str,
    reason: Option<&str>,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO image_blacklist (sci_name, url, reason) VALUES (?1, ?2, ?3)",
        params![sci_name, url, reason],
    )?;
    Ok(changed > 0)
}

/// Remove an entry from the blacklist by ID.
///
/// Returns `true` if a row was deleted.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn remove_image_blacklist(conn: &Connection, id: i64) -> Result<bool, DbError> {
    let changed = conn.execute("DELETE FROM image_blacklist WHERE id = ?1", params![id])?;
    Ok(changed > 0)
}

/// List all blacklisted images, ordered by most recently added first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn list_image_blacklist(conn: &Connection) -> Result<Vec<ImageBlacklist>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, sci_name, url, reason, blacklisted_at \
         FROM image_blacklist ORDER BY blacklisted_at DESC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ImageBlacklist {
                id: row.get(0)?,
                sci_name: row.get(1)?,
                url: row.get(2)?,
                reason: row.get(3)?,
                blacklisted_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// List all blacklisted URLs for a specific species.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn blacklisted_urls_for_species(
    conn: &Connection,
    sci_name: &str,
) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT url FROM image_blacklist WHERE sci_name = ?1 ORDER BY blacklisted_at DESC",
    )?;
    let rows = stmt
        .query_map(params![sci_name], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Check if a specific image URL is blacklisted for a species.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn is_image_blacklisted(conn: &Connection, sci_name: &str, url: &str) -> Result<bool, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM image_blacklist WHERE sci_name = ?1 AND url = ?2",
        params![sci_name, url],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;

    fn temp_db() -> (tempfile::NamedTempFile, Connection) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        // Run migrations to create image_blacklist and other schema additions.
        crate::migration::migrate(&conn).unwrap();
        (tmp, conn)
    }

    #[test]
    fn add_and_list_blacklist() {
        let (_tmp, conn) = temp_db();
        let added = add_image_blacklist(
            &conn,
            "Turdus merula",
            "https://example.com/img.jpg",
            Some("test"),
        )
        .unwrap();
        assert!(added);
        let list = list_image_blacklist(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].sci_name, "Turdus merula");
        assert_eq!(list[0].url, "https://example.com/img.jpg");
        assert_eq!(list[0].reason, Some("test".to_string()));
    }

    #[test]
    fn add_duplicate_returns_false() {
        let (_tmp, conn) = temp_db();
        let first =
            add_image_blacklist(&conn, "Turdus merula", "https://example.com/img.jpg", None)
                .unwrap();
        let second =
            add_image_blacklist(&conn, "Turdus merula", "https://example.com/img.jpg", None)
                .unwrap();
        assert!(first);
        assert!(!second);
    }

    #[test]
    fn remove_blacklist_entry() {
        let (_tmp, conn) = temp_db();
        add_image_blacklist(&conn, "Turdus merula", "https://example.com/img.jpg", None).unwrap();
        let list = list_image_blacklist(&conn).unwrap();
        let id = list[0].id;
        let removed = remove_image_blacklist(&conn, id).unwrap();
        assert!(removed);
        assert!(list_image_blacklist(&conn).unwrap().is_empty());
    }

    #[test]
    fn is_image_blacklisted_check() {
        let (_tmp, conn) = temp_db();
        add_image_blacklist(&conn, "Turdus merula", "https://bad.com/img.jpg", None).unwrap();
        assert!(is_image_blacklisted(&conn, "Turdus merula", "https://bad.com/img.jpg").unwrap());
        assert!(!is_image_blacklisted(&conn, "Turdus merula", "https://good.com/img.jpg").unwrap());
    }
}
