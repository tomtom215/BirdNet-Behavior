//! What the database is: its engine settings, its tables' sizes, and how
//! fast detections are arriving — birdnet-go's database overview.

use rusqlite::Connection;

use crate::sqlite::connection::DbError;

/// The `SQLite` engine and the settings this connection runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDetails {
    /// `sqlite_version()`.
    pub version: String,
    /// `PRAGMA journal_mode`.
    pub journal_mode: String,
    /// `PRAGMA page_size`, in bytes.
    pub page_size: i64,
    /// `PRAGMA freelist_count`: pages allocated to the file and unused.
    pub freelist_pages: i64,
    /// `PRAGMA cache_size` in bytes: a negative setting is KiB, a positive
    /// one pages.
    pub cache_size_bytes: i64,
}

/// Read the engine's version and settings.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn engine_details(conn: &Connection) -> Result<EngineDetails, DbError> {
    let pragma = |name: &str| -> Result<i64, DbError> {
        Ok(conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))?)
    };
    let page_size = pragma("page_size")?;
    let cache = pragma("cache_size")?;
    Ok(EngineDetails {
        version: conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))?,
        journal_mode: conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
        page_size,
        freelist_pages: pragma("freelist_count")?,
        cache_size_bytes: if cache < 0 {
            cache.saturating_neg().saturating_mul(1024)
        } else {
            cache.saturating_mul(page_size)
        },
    })
}

/// One table's rows and the bytes its pages, and its indexes', occupy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSize {
    /// Table name.
    pub name: String,
    /// Rows in it.
    pub row_count: i64,
    /// Bytes of the pages holding it and its indexes.
    pub size_bytes: i64,
}

/// Each table's rows and size, the largest first, read from the `dbstat`
/// virtual table, with an index's pages counted to its table.
///
/// This reads every page of the file. On a station with a season of history
/// on an SD card that takes long enough that the caller should not run it on
/// every request.
///
/// # Errors
///
/// Returns `DbError` on query failure, `dbstat` missing from the build
/// included.
pub fn table_sizes(conn: &Connection) -> Result<Vec<TableSize>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT m.tbl_name, SUM(d.pgsize) AS bytes
         FROM dbstat d JOIN sqlite_master m ON m.name = d.name
         WHERE m.type IN ('table', 'index') AND m.tbl_name NOT LIKE 'sqlite_%'
         GROUP BY m.tbl_name
         ORDER BY bytes DESC, m.tbl_name",
    )?;
    let sized = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    sized
        .into_iter()
        .map(|(name, size_bytes)| {
            // The name comes from sqlite_master; quoting it doubles any quote
            // inside, so it cannot end the identifier.
            let row_count = conn.query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\"")),
                [],
                |r| r.get(0),
            )?;
            Ok(TableSize {
                name,
                row_count,
                size_bytes,
            })
        })
        .collect()
}

/// Detections stored in each UTC hour of the 24 hours before `now_unix`, as
/// `("YYYY-MM-DDTHH:00:00Z", count)`, oldest first; hours with none are left
/// out. Rows without an instant are in no hour.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_rate_24h(conn: &Connection, now_unix: i64) -> Result<Vec<(String, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-%m-%dT%H:00:00Z', detected_at_utc, 'unixepoch') AS hour, COUNT(*)
         FROM detections
         WHERE detected_at_utc > ?1 - 86400 AND detected_at_utc <= ?1
         GROUP BY hour ORDER BY hour",
    )?;
    let rows = stmt
        .query_map([now_unix], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    #[test]
    fn the_engine_reports_its_settings() {
        let conn = station();
        conn.execute_batch("PRAGMA cache_size = -2000;")
            .expect("pragma");
        let e = engine_details(&conn).expect("details");
        assert!(e.version.starts_with("3."), "{e:?}");
        assert_eq!(e.journal_mode, "memory", "an in-memory database: {e:?}");
        assert!(e.page_size >= 512);
        assert_eq!(e.cache_size_bytes, 2000 * 1024, "negative is KiB");
        conn.execute_batch("PRAGMA cache_size = 100;")
            .expect("pragma");
        assert_eq!(
            engine_details(&conn).expect("details").cache_size_bytes,
            100 * e.page_size,
            "positive is pages"
        );
    }

    /// Every table is listed with its rows, an index counted to its table,
    /// the largest first.
    #[test]
    fn tables_are_sized_with_their_indexes() {
        let conn = station();
        for i in 0..300 {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-05-01', printf('%02d:%02d:00', ?1 / 60, ?1 % 60),
                         'Turdus merula', 'Blackbird', 0.9)",
                [i],
            )
            .expect("insert");
        }
        let sizes = table_sizes(&conn).expect("sizes");
        assert_eq!(sizes[0].name, "detections", "{sizes:?}");
        assert_eq!(sizes[0].row_count, 300);
        let page: i64 = conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap();
        let own: i64 = conn
            .query_row(
                "SELECT SUM(pgsize) FROM dbstat WHERE name = 'detections'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sizes[0].size_bytes > own, "its indexes are counted to it");
        assert_eq!(sizes[0].size_bytes % page, 0);
        assert!(sizes.windows(2).all(|w| w[0].size_bytes >= w[1].size_bytes));
        assert!(sizes.iter().all(|t| !t.name.starts_with("sqlite_")));
        let views: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(views > 0, "precondition: the schema has views");
        assert!(
            sizes.iter().all(|t| t.name != "detections_analytic"),
            "a view has no pages"
        );
    }

    /// The last 24 hours by UTC hour, the window's start excluded.
    #[test]
    fn the_last_day_is_counted_by_utc_hour() {
        let conn = station();
        let now = 1_800_000_000_i64; // 2027-01-15T08:00:00Z
        for (i, at) in [
            now,
            now - 60,
            now - 3_600,
            now - 86_400,
            now - 86_399,
            now + 1,
        ]
        .into_iter()
        .enumerate()
        {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2027-01-15', printf('00:00:%02d', ?2), 'Turdus merula', 'Blackbird', 0.9)",
                rusqlite::params![at, i64::try_from(i).unwrap()],
            )
            .expect("insert");
            conn.execute(
                "UPDATE detections SET detected_at_utc = ?1 WHERE Time = printf('00:00:%02d', ?2)",
                rusqlite::params![at, i64::try_from(i).unwrap()],
            )
            .expect("stamp");
        }
        assert_eq!(
            detection_rate_24h(&conn, now).expect("rate"),
            [
                ("2027-01-14T08:00:00Z".to_owned(), 1),
                ("2027-01-15T07:00:00Z".to_owned(), 2),
                ("2027-01-15T08:00:00Z".to_owned(), 1),
            ]
        );
    }
}
