//! Persistent settings store backed by `SQLite`.
//!
//! Settings are key-value pairs stored in a `settings` table alongside
//! detections.  The web admin panel reads and writes settings through this
//! module.  On startup the binary overlays file-based config with any
//! settings that have been saved through the UI.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Settings-specific errors.
#[derive(Debug)]
pub enum SettingsError {
    /// `SQLite` error.
    Sqlite(rusqlite::Error),
    /// Requested setting not found.
    NotFound(String),
    /// Value could not be parsed.
    Parse(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::NotFound(k) => write!(f, "setting not found: {k}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for SettingsError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// A setting category for grouping in the admin UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsCategory {
    Audio,
    Location,
    Detection,
    Notifications,
    Species,
    System,
    General,
}

impl SettingsCategory {
    const fn as_str(&self) -> &str {
        match self {
            Self::Audio => "audio",
            Self::Location => "location",
            Self::Detection => "detection",
            Self::Notifications => "notifications",
            Self::Species => "species",
            Self::System => "system",
            Self::General => "general",
        }
    }
}

impl std::str::FromStr for SettingsCategory {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "audio" => Ok(Self::Audio),
            "location" => Ok(Self::Location),
            "detection" => Ok(Self::Detection),
            "notifications" => Ok(Self::Notifications),
            "species" => Ok(Self::Species),
            "system" => Ok(Self::System),
            _ => Ok(Self::General),
        }
    }
}

/// A single persisted setting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    /// Unique key (e.g. `"alsa_device"`, `"latitude"`).
    pub key: String,
    /// String value.
    pub value: String,
    /// Grouping category.
    pub category: String,
    /// ISO-8601 timestamp of last update.
    pub updated_at: String,
}

/// Create the `settings` table if it does not exist.
///
/// Safe to call on every startup.
///
/// # Errors
///
/// Returns `SettingsError` on `SQLite` failure.
pub fn ensure_settings_table(conn: &Connection) -> Result<(), SettingsError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key        TEXT PRIMARY KEY NOT NULL,
            value      TEXT NOT NULL,
            category   TEXT NOT NULL DEFAULT 'general',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    Ok(())
}

/// Get a setting value by key.
///
/// # Errors
///
/// Returns `SettingsError::NotFound` if the key does not exist.
pub fn get(conn: &Connection, key: &str) -> Result<String, SettingsError> {
    let result = conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    );

    match result {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(SettingsError::NotFound(key.to_string())),
        Err(e) => Err(SettingsError::Sqlite(e)),
    }
}

/// Get a setting value, returning `default` if the key is absent.
///
/// # Errors
///
/// Returns `SettingsError` on `SQLite` failure.
pub fn get_or(conn: &Connection, key: &str, default: &str) -> Result<String, SettingsError> {
    match get(conn, key) {
        Ok(v) => Ok(v),
        Err(SettingsError::NotFound(_)) => Ok(default.to_string()),
        Err(e) => Err(e),
    }
}

/// Parse a setting value to type `T`.
///
/// # Errors
///
/// Returns `SettingsError::NotFound` or `SettingsError::Parse`.
pub fn get_parsed<T: std::str::FromStr>(conn: &Connection, key: &str) -> Result<T, SettingsError>
where
    T::Err: std::fmt::Display,
{
    let v = get(conn, key)?;
    v.parse::<T>()
        .map_err(|e| SettingsError::Parse(format!("key '{key}': {e}")))
}

/// Set (insert or update) a setting value.
///
/// # Errors
///
/// Returns `SettingsError` on `SQLite` failure.
pub fn set(
    conn: &Connection,
    key: &str,
    value: &str,
    category: SettingsCategory,
) -> Result<(), SettingsError> {
    conn.execute(
        "INSERT INTO settings (key, value, category, updated_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET
             value      = excluded.value,
             category   = excluded.category,
             updated_at = datetime('now')",
        params![key, value, category.as_str()],
    )?;
    Ok(())
}

/// Delete a setting.
///
/// # Errors
///
/// Returns `SettingsError` on `SQLite` failure.
pub fn delete(conn: &Connection, key: &str) -> Result<bool, SettingsError> {
    let n = conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(n > 0)
}

/// List all settings, optionally filtered by category.
///
/// # Errors
///
/// Returns `SettingsError` on `SQLite` failure.
pub fn list(
    conn: &Connection,
    category: Option<&SettingsCategory>,
) -> Result<Vec<Setting>, SettingsError> {
    let rows = if let Some(cat) = category {
        let mut stmt = conn.prepare(
            "SELECT key, value, category, updated_at FROM settings
             WHERE category = ?1 ORDER BY key",
        )?;
        stmt.query_map(params![cat.as_str()], |row| {
            Ok(Setting {
                key: row.get(0)?,
                value: row.get(1)?,
                category: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        let mut stmt = conn.prepare(
            "SELECT key, value, category, updated_at FROM settings ORDER BY category, key",
        )?;
        stmt.query_map([], |row| {
            Ok(Setting {
                key: row.get(0)?,
                value: row.get(1)?,
                category: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };

    Ok(rows)
}

/// Bulk-set multiple settings in a single transaction.
///
/// Each item is `(key, value, category)`.
///
/// # Errors
///
/// Returns `SettingsError` on any `SQLite` failure; the transaction is rolled back.
pub fn set_many(
    conn: &Connection,
    items: &[(&str, &str, SettingsCategory)],
) -> Result<(), SettingsError> {
    let mut stmt = conn.prepare(
        "INSERT INTO settings (key, value, category, updated_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET
             value      = excluded.value,
             category   = excluded.category,
             updated_at = datetime('now')",
    )?;

    for (key, value, category) in items {
        stmt.execute(params![key, value, category.as_str()])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_settings_table(&conn).unwrap();
        conn
    }

    #[test]
    fn set_and_get() {
        let conn = setup();
        set(&conn, "latitude", "51.5", SettingsCategory::Location).unwrap();
        assert_eq!(get(&conn, "latitude").unwrap(), "51.5");
    }

    #[test]
    fn get_missing_returns_not_found() {
        let conn = setup();
        let err = get(&conn, "nonexistent").unwrap_err();
        assert!(matches!(err, SettingsError::NotFound(_)));
    }

    #[test]
    fn get_or_default() {
        let conn = setup();
        let v = get_or(&conn, "confidence", "0.7").unwrap();
        assert_eq!(v, "0.7");
    }

    #[test]
    fn get_parsed_f64() {
        let conn = setup();
        set(&conn, "sensitivity", "1.25", SettingsCategory::Detection).unwrap();
        let v: f64 = get_parsed(&conn, "sensitivity").unwrap();
        assert!((v - 1.25).abs() < 1e-9);
    }

    #[test]
    fn update_existing_key() {
        let conn = setup();
        set(&conn, "latitude", "51.5", SettingsCategory::Location).unwrap();
        set(&conn, "latitude", "52.0", SettingsCategory::Location).unwrap();
        assert_eq!(get(&conn, "latitude").unwrap(), "52.0");
    }

    #[test]
    fn delete_setting() {
        let conn = setup();
        set(&conn, "key1", "val1", SettingsCategory::General).unwrap();
        assert!(delete(&conn, "key1").unwrap());
        assert!(!delete(&conn, "key1").unwrap()); // already gone
    }

    #[test]
    fn list_all() {
        let conn = setup();
        set(&conn, "latitude", "51.5", SettingsCategory::Location).unwrap();
        set(&conn, "longitude", "-0.1", SettingsCategory::Location).unwrap();
        set(&conn, "alsa_device", "plughw:1,0", SettingsCategory::Audio).unwrap();

        let all = list(&conn, None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn list_by_category() {
        let conn = setup();
        set(&conn, "latitude", "51.5", SettingsCategory::Location).unwrap();
        set(&conn, "longitude", "-0.1", SettingsCategory::Location).unwrap();
        set(&conn, "alsa_device", "plughw:1,0", SettingsCategory::Audio).unwrap();

        let loc = list(&conn, Some(&SettingsCategory::Location)).unwrap();
        assert_eq!(loc.len(), 2);
        assert!(loc.iter().all(|s| s.category == "location"));
    }

    #[test]
    fn set_many_bulk() {
        let conn = setup();
        let items = vec![
            ("latitude", "51.5", SettingsCategory::Location),
            ("longitude", "-0.1", SettingsCategory::Location),
        ];
        set_many(&conn, &items).unwrap();
        assert_eq!(get(&conn, "latitude").unwrap(), "51.5");
        assert_eq!(get(&conn, "longitude").unwrap(), "-0.1");
    }
}
