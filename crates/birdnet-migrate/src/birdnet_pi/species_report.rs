//! Species-level migration statistics report.
//!
//! Queries the source BirdNET-Pi database to produce a per-species breakdown
//! of detection counts and date ranges.  This is displayed in the admin UI
//! before and after migration so users can verify their data was imported
//! correctly.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::MigrateError;
use crate::schema::open_source_readonly;

/// Statistics for a single species in the source database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeciesStats {
    /// Common name.
    pub common_name: String,
    /// Scientific name.
    pub scientific_name: String,
    /// Total detections.
    pub count: i64,
    /// First detection date (YYYY-MM-DD).
    pub first_date: String,
    /// Last detection date (YYYY-MM-DD).
    pub last_date: String,
    /// Average confidence score (0.0–1.0).
    pub avg_confidence: f64,
    /// Maximum confidence score.
    pub max_confidence: f64,
}

/// Full migration preview report for a source database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    /// Total detection rows in the source.
    pub total_rows: i64,
    /// Number of unique species.
    pub unique_species: usize,
    /// Date range covered (earliest to latest detection date).
    pub date_range: Option<(String, String)>,
    /// Top N species by detection count.
    pub top_species: Vec<SpeciesStats>,
    /// Number of rows with missing/null dates.
    pub null_date_rows: i64,
    /// Number of rows with invalid confidence values (< 0 or > 1).
    pub invalid_confidence_rows: i64,
    /// Number of duplicate rows (same date/time/species).
    pub duplicate_rows: i64,
    /// Whether the source passes data quality checks.
    pub quality_ok: bool,
}

/// Generate a full species-level report for the source database.
///
/// Opens the database read-only and performs several aggregating queries.
///
/// # Errors
///
/// Returns [`MigrateError`] if the database cannot be opened or any query fails.
pub fn generate_report(source_path: &std::path::Path) -> Result<MigrationReport, MigrateError> {
    let conn = open_source_readonly(source_path)?;

    let total_rows = count_total(&conn)?;
    let top_species = query_top_species(&conn, 20)?;
    let unique_species = count_unique_species(&conn)?;
    let date_range = query_date_range(&conn)?;
    let null_date_rows = count_null_dates(&conn)?;
    let invalid_confidence_rows = count_invalid_confidence(&conn)?;
    let duplicate_rows = count_duplicates(&conn)?;
    let quality_ok = null_date_rows == 0 && invalid_confidence_rows == 0;

    Ok(MigrationReport {
        total_rows,
        unique_species,
        date_range,
        top_species,
        null_date_rows,
        invalid_confidence_rows,
        duplicate_rows,
        quality_ok,
    })
}

/// Compare source and destination databases after migration.
///
/// Returns `(source_count, dest_count, per_species_match)`.
///
/// # Errors
///
/// Returns [`MigrateError`] on database access failure.
pub fn compare_source_dest(
    source_path: &std::path::Path,
    dest_path: &std::path::Path,
) -> Result<PostMigrationReport, MigrateError> {
    let src = open_source_readonly(source_path)?;
    let dst_conn = rusqlite::Connection::open(dest_path).map_err(MigrateError::DestinationOpen)?;

    let src_total = count_total(&src)?;

    // Count rows in destination that came from the migration window
    let dst_total: i64 = dst_conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0);

    // Per-species comparison
    let src_species = query_species_counts(&src)?;
    let dst_species = query_species_counts(&dst_conn)?;

    let mut species_diff: Vec<SpeciesDiff> = Vec::new();
    for (name, src_count) in &src_species {
        let dst_count = dst_species.get(name).copied().unwrap_or(0);
        species_diff.push(SpeciesDiff {
            common_name: name.clone(),
            source_count: *src_count,
            dest_count: dst_count,
            matched: *src_count == dst_count,
        });
    }
    species_diff.sort_by(|a, b| b.source_count.cmp(&a.source_count));

    let all_matched = species_diff.iter().all(|d| d.matched);

    Ok(PostMigrationReport {
        source_total: src_total,
        dest_total: dst_total,
        all_matched,
        species_diff,
    })
}

/// Post-migration comparison report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostMigrationReport {
    /// Total rows in source.
    pub source_total: i64,
    /// Total rows in destination (may include pre-existing data).
    pub dest_total: i64,
    /// Whether all source species have matching destination counts.
    pub all_matched: bool,
    /// Per-species count comparison.
    pub species_diff: Vec<SpeciesDiff>,
}

/// Per-species count comparison between source and destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeciesDiff {
    /// Common name.
    pub common_name: String,
    /// Count in source.
    pub source_count: i64,
    /// Count in destination.
    pub dest_count: i64,
    /// Whether counts match.
    pub matched: bool,
}

// ---------------------------------------------------------------------------
// Internal query helpers
// ---------------------------------------------------------------------------

fn count_total(conn: &Connection) -> Result<i64, MigrateError> {
    conn.query_row("SELECT COUNT(*) FROM detections", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|e| MigrateError::Query(e.to_string()))
}

fn count_unique_species(conn: &Connection) -> Result<usize, MigrateError> {
    let n: i64 = conn
        .query_row("SELECT COUNT(DISTINCT Com_Name) FROM detections", [], |r| {
            r.get(0)
        })
        .map_err(|e| MigrateError::Query(e.to_string()))?;
    Ok(n.max(0) as usize)
}

fn query_date_range(conn: &Connection) -> Result<Option<(String, String)>, MigrateError> {
    let result = conn.query_row(
        "SELECT MIN(Date), MAX(Date) FROM detections WHERE Date IS NOT NULL",
        [],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        },
    );
    match result {
        Ok((Some(min), Some(max))) => Ok(Some((min, max))),
        _ => Ok(None),
    }
}

fn query_top_species(conn: &Connection, limit: usize) -> Result<Vec<SpeciesStats>, MigrateError> {
    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut stmt = conn
        .prepare(
            "SELECT Com_Name, Sci_Name, COUNT(*) as cnt,
                    MIN(Date), MAX(Date),
                    AVG(Confidence), MAX(Confidence)
             FROM detections
             WHERE Com_Name IS NOT NULL
             GROUP BY Com_Name
             ORDER BY cnt DESC
             LIMIT ?1",
        )
        .map_err(|e| MigrateError::Query(e.to_string()))?;

    let rows = stmt
        .query_map(params![limit_i64], |row| {
            Ok(SpeciesStats {
                common_name: row.get(0)?,
                scientific_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                count: row.get::<_, i64>(2)?,
                first_date: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                last_date: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                avg_confidence: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                max_confidence: row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
            })
        })
        .map_err(|e| MigrateError::Query(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MigrateError::Query(e.to_string()))?;

    Ok(rows)
}

fn query_species_counts(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, i64>, MigrateError> {
    let mut stmt = conn
        .prepare("SELECT Com_Name, COUNT(*) FROM detections GROUP BY Com_Name")
        .map_err(|e| MigrateError::Query(e.to_string()))?;

    let map: std::collections::HashMap<String, i64> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| MigrateError::Query(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(map)
}

fn count_null_dates(conn: &Connection) -> Result<i64, MigrateError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Date IS NULL OR Date = ''",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map_err(|e| MigrateError::Query(e.to_string()))
}

fn count_invalid_confidence(conn: &Connection) -> Result<i64, MigrateError> {
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Confidence < 0 OR Confidence > 1",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map_err(|e| MigrateError::Query(e.to_string()))
}

fn count_duplicates(conn: &Connection) -> Result<i64, MigrateError> {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| MigrateError::Query(e.to_string()))?;

    let distinct: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (
                SELECT DISTINCT Date, Time, Sci_Name FROM detections
             )",
            [],
            |r| r.get(0),
        )
        .map_err(|e| MigrateError::Query(e.to_string()))?;

    Ok(total.saturating_sub(distinct))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn make_source_db(rows: &[(&str, &str, &str, f64)]) -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT
            );",
        )
        .unwrap();
        for (date, sci, com, conf) in rows {
            conn.execute(
                "INSERT INTO detections(Date,Time,Sci_Name,Com_Name,Confidence)
                 VALUES (?1,'07:00',?2,?3,?4)",
                rusqlite::params![date, sci, com, conf],
            )
            .unwrap();
        }
        drop(conn);
        tmp
    }

    #[test]
    fn generate_report_basic() {
        let db = make_source_db(&[
            ("2026-01-01", "Erithacus rubecula", "Robin", 0.9),
            ("2026-01-02", "Erithacus rubecula", "Robin", 0.85),
            ("2026-01-01", "Troglodytes troglodytes", "Wren", 0.7),
        ]);
        let report = generate_report(db.path()).unwrap();
        assert_eq!(report.total_rows, 3);
        assert_eq!(report.unique_species, 2);
        assert!(report.quality_ok);
        assert_eq!(report.duplicate_rows, 0);
    }

    #[test]
    fn generate_report_null_dates() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch("CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT, Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);").unwrap();
        conn.execute_batch("INSERT INTO detections(Date,Time,Sci_Name,Com_Name,Confidence) VALUES (NULL,'07:00','sp','Robin',0.9);").unwrap();
        drop(conn);
        let report = generate_report(tmp.path()).unwrap();
        assert_eq!(report.null_date_rows, 1);
        assert!(!report.quality_ok);
    }

    #[test]
    fn generate_report_date_range() {
        let db = make_source_db(&[
            ("2026-01-01", "sp", "Robin", 0.9),
            ("2026-03-15", "sp", "Robin", 0.8),
        ]);
        let report = generate_report(db.path()).unwrap();
        let (start, end) = report.date_range.unwrap();
        assert_eq!(start, "2026-01-01");
        assert_eq!(end, "2026-03-15");
    }

    #[test]
    fn generate_report_top_species_ordered() {
        let db = make_source_db(&[
            ("2026-01-01", "sp1", "Robin", 0.9),
            ("2026-01-02", "sp1", "Robin", 0.9),
            ("2026-01-03", "sp1", "Robin", 0.9),
            ("2026-01-01", "sp2", "Wren", 0.8),
        ]);
        let report = generate_report(db.path()).unwrap();
        assert_eq!(report.top_species[0].common_name, "Robin");
        assert_eq!(report.top_species[0].count, 3);
    }

    #[test]
    fn compare_source_dest_matching() {
        let rows = &[
            ("2026-01-01", "sp1", "Robin", 0.9),
            ("2026-01-02", "sp1", "Robin", 0.85),
        ];
        let src = make_source_db(rows);
        let dst_tmp = NamedTempFile::new().unwrap();

        // Copy all rows to destination
        let dst = Connection::open(dst_tmp.path()).unwrap();
        birdnet_db::sqlite::open_or_create(dst_tmp.path()).unwrap();
        drop(dst);

        // Run migration
        use crate::traits::Migrator as _;
        let progress = crate::progress::ProgressHandle::new();
        crate::birdnet_pi::importer::BirdNetPiImporter
            .migrate(src.path(), dst_tmp.path(), &progress)
            .unwrap();

        let rpt = compare_source_dest(src.path(), dst_tmp.path()).unwrap();
        assert_eq!(rpt.source_total, 2);
        assert!(rpt.all_matched);
    }
}
