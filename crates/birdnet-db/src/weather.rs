//! Weather rows cached from Open-Meteo for the O-23 signal overlays.
//!
//! Schema lives in `migration::Migration { version: 16, … }`. Rows are
//! written by the background poll job in
//! `birdnet_integrations::weather` (off by default; opt in with
//! `BNB_WEATHER_ENABLED=1`) and read by the overlay renderers in
//! `birdnet_web::routes::pages::overlays`.
//!
//! Synchronous SQLite per the project rule. The web server bridges this
//! through `AppState::with_db`.

use std::fmt;

use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
pub enum WeatherError {
    Sqlite(rusqlite::Error),
}

impl fmt::Display for WeatherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
        }
    }
}

impl std::error::Error for WeatherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for WeatherError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// One hourly weather observation.
#[derive(Debug, Clone)]
pub struct WeatherRow {
    /// ISO-8601 UTC, e.g. `2026-05-28T14:00:00Z`.
    pub at: String,
    pub temp_c: Option<f32>,
    pub precip_mm: Option<f32>,
    pub wind_kt: Option<f32>,
    pub wind_dir_deg: Option<i32>,
    pub pressure_hpa: Option<f32>,
    pub cloud_pct: Option<i32>,
    /// WMO weather code, used for the legend icon.
    pub code: Option<i32>,
}

pub trait WeatherStore {
    /// Upsert one row by `at` (replaces existing).
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Sqlite`] on database failure.
    fn upsert(&self, row: &WeatherRow) -> Result<(), WeatherError>;

    /// Return every row in `[from, to]` (inclusive), in chronological order.
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Sqlite`] on database failure.
    fn range(&self, from: &str, to: &str) -> Result<Vec<WeatherRow>, WeatherError>;

    /// Return the most recent row, if any.
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Sqlite`] on database failure.
    fn latest(&self) -> Result<Option<WeatherRow>, WeatherError>;

    /// Delete rows older than `cutoff` (`at < cutoff`). Returns the row count.
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Sqlite`] on database failure.
    fn prune_older_than(&self, cutoff: &str) -> Result<usize, WeatherError>;

    /// Delete rows older than `days` days. Resolved via SQLite's
    /// `julianday()`, so it tolerates any ISO-8601-shaped `at` string
    /// (Open-Meteo emits `YYYY-MM-DDTHH:MM`; the today/dawn-chorus
    /// renderers use `YYYY-MM-DDTHH:MM:SSZ`).
    ///
    /// # Errors
    ///
    /// Returns [`WeatherError::Sqlite`] on database failure.
    fn prune_older_than_days(&self, days: u32) -> Result<usize, WeatherError>;
}

impl WeatherStore for Connection {
    fn upsert(&self, row: &WeatherRow) -> Result<(), WeatherError> {
        self.execute(
            "INSERT INTO weather (
                at, temp_c, precip_mm, wind_kt, wind_dir_deg,
                pressure_hpa, cloud_pct, code, fetched_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            ON CONFLICT(at) DO UPDATE SET
                temp_c       = excluded.temp_c,
                precip_mm    = excluded.precip_mm,
                wind_kt      = excluded.wind_kt,
                wind_dir_deg = excluded.wind_dir_deg,
                pressure_hpa = excluded.pressure_hpa,
                cloud_pct    = excluded.cloud_pct,
                code         = excluded.code,
                fetched_at   = datetime('now')",
            params![
                row.at,
                row.temp_c.map(f64::from),
                row.precip_mm.map(f64::from),
                row.wind_kt.map(f64::from),
                row.wind_dir_deg,
                row.pressure_hpa.map(f64::from),
                row.cloud_pct,
                row.code,
            ],
        )?;
        Ok(())
    }

    fn range(&self, from: &str, to: &str) -> Result<Vec<WeatherRow>, WeatherError> {
        let mut stmt = self.prepare(
            "SELECT at, temp_c, precip_mm, wind_kt, wind_dir_deg,
                    pressure_hpa, cloud_pct, code
             FROM weather
             WHERE at BETWEEN ?1 AND ?2
             ORDER BY at ASC",
        )?;
        let rows = stmt
            .query_map(params![from, to], row_to_weather)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn latest(&self) -> Result<Option<WeatherRow>, WeatherError> {
        self.query_row(
            "SELECT at, temp_c, precip_mm, wind_kt, wind_dir_deg,
                    pressure_hpa, cloud_pct, code
             FROM weather ORDER BY at DESC LIMIT 1",
            [],
            row_to_weather,
        )
        .optional()
        .map_err(Into::into)
    }

    fn prune_older_than(&self, cutoff: &str) -> Result<usize, WeatherError> {
        let n = self.execute("DELETE FROM weather WHERE at < ?1", params![cutoff])?;
        Ok(n)
    }

    fn prune_older_than_days(&self, days: u32) -> Result<usize, WeatherError> {
        let n = self.execute(
            "DELETE FROM weather
             WHERE julianday(at) < julianday('now', '-' || ?1 || ' days')",
            params![days],
        )?;
        Ok(n)
    }
}

fn row_to_weather(row: &rusqlite::Row<'_>) -> rusqlite::Result<WeatherRow> {
    let temp: Option<f64> = row.get(1)?;
    let precip: Option<f64> = row.get(2)?;
    let wind: Option<f64> = row.get(3)?;
    let pressure: Option<f64> = row.get(5)?;
    #[allow(clippy::cast_possible_truncation)]
    Ok(WeatherRow {
        at: row.get(0)?,
        temp_c: temp.map(|v| v as f32),
        precip_mm: precip.map(|v| v as f32),
        wind_kt: wind.map(|v| v as f32),
        wind_dir_deg: row.get(4)?,
        pressure_hpa: pressure.map(|v| v as f32),
        cloud_pct: row.get(6)?,
        code: row.get(7)?,
    })
}

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

    fn sample(at: &str, temp: f32) -> WeatherRow {
        WeatherRow {
            at: at.to_string(),
            temp_c: Some(temp),
            precip_mm: Some(0.0),
            wind_kt: Some(4.0),
            wind_dir_deg: Some(120),
            pressure_hpa: Some(1013.0),
            cloud_pct: Some(20),
            code: Some(1),
        }
    }

    #[test]
    fn fresh_table_is_empty() {
        let conn = open_db();
        assert!(conn.latest().unwrap().is_none());
        assert!(conn.range("2026-01-01", "2099-01-01").unwrap().is_empty());
    }

    #[test]
    fn upsert_round_trips() {
        let conn = open_db();
        conn.upsert(&sample("2026-05-28T12:00:00Z", 18.5)).unwrap();
        let rows = conn.range("2026-05-28", "2026-05-29").unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].temp_c.unwrap() - 18.5).abs() < 1e-4);
    }

    #[test]
    fn upsert_replaces_existing_row() {
        let conn = open_db();
        conn.upsert(&sample("2026-05-28T12:00:00Z", 18.5)).unwrap();
        conn.upsert(&sample("2026-05-28T12:00:00Z", 22.0)).unwrap();
        let rows = conn.range("2026-05-28", "2026-05-29").unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].temp_c.unwrap() - 22.0).abs() < 1e-4);
    }

    #[test]
    fn latest_returns_most_recent_row() {
        let conn = open_db();
        conn.upsert(&sample("2026-05-28T10:00:00Z", 10.0)).unwrap();
        conn.upsert(&sample("2026-05-28T14:00:00Z", 14.0)).unwrap();
        conn.upsert(&sample("2026-05-28T12:00:00Z", 12.0)).unwrap();
        let row = conn.latest().unwrap().unwrap();
        assert_eq!(row.at, "2026-05-28T14:00:00Z");
    }

    #[test]
    fn range_returns_chronological() {
        let conn = open_db();
        for h in [14_i32, 10, 12] {
            #[allow(clippy::cast_precision_loss)]
            let temp = h as f32;
            conn.upsert(&sample(&format!("2026-05-28T{h:02}:00:00Z"), temp))
                .unwrap();
        }
        let rows = conn.range("2026-05-28", "2026-05-29").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].at, "2026-05-28T10:00:00Z");
        assert_eq!(rows[1].at, "2026-05-28T12:00:00Z");
        assert_eq!(rows[2].at, "2026-05-28T14:00:00Z");
    }

    #[test]
    fn prune_drops_old_rows_only() {
        let conn = open_db();
        conn.upsert(&sample("2025-01-01T00:00:00Z", 5.0)).unwrap();
        conn.upsert(&sample("2026-05-28T00:00:00Z", 18.0)).unwrap();
        let removed = conn.prune_older_than("2026-01-01").unwrap();
        assert_eq!(removed, 1);
        let rows = conn.range("2025-01-01", "2026-12-31").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].at, "2026-05-28T00:00:00Z");
    }

    #[test]
    fn prune_older_than_days_drops_old_rows_only() {
        let conn = open_db();
        // Insert an older row by hand-shifting julianday via raw INSERT so
        // the test doesn't depend on system clock for "today".
        conn.execute(
            "INSERT INTO weather (at, temp_c, fetched_at) VALUES (?1, ?2, datetime('now', '-90 days'))",
            params!["2024-01-01T00:00:00Z", 4.0_f64],
        )
        .unwrap();
        conn.upsert(&sample("9999-01-01T00:00:00Z", 18.0)).unwrap();
        let removed = conn.prune_older_than_days(30).unwrap();
        assert_eq!(removed, 1);
        let rows = conn.range("0000-01-01", "9999-12-31").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].at, "9999-01-01T00:00:00Z");
    }

    #[test]
    fn prune_older_than_days_tolerates_open_meteo_format() {
        // Open-Meteo emits `YYYY-MM-DDTHH:MM` (no seconds, no zulu).
        // julianday() must still parse it for the prune query to work.
        let conn = open_db();
        conn.execute(
            "INSERT INTO weather (at, temp_c, fetched_at) VALUES (?1, ?2, datetime('now', '-90 days'))",
            params!["2024-05-28T13:00", 12.0_f64],
        )
        .unwrap();
        let removed = conn.prune_older_than_days(30).unwrap();
        assert_eq!(removed, 1);
    }

    #[test]
    fn nullable_fields_round_trip_as_none() {
        let conn = open_db();
        let row = WeatherRow {
            at: "2026-05-28T12:00:00Z".to_string(),
            temp_c: None,
            precip_mm: None,
            wind_kt: None,
            wind_dir_deg: None,
            pressure_hpa: None,
            cloud_pct: None,
            code: None,
        };
        conn.upsert(&row).unwrap();
        let stored = conn.latest().unwrap().unwrap();
        assert!(stored.temp_c.is_none());
        assert!(stored.wind_dir_deg.is_none());
    }
}
