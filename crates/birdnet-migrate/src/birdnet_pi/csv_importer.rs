//! BirdNET-Pi CSV/TSV detection log importer.
//!
//! BirdNET-Pi writes detections to a tab-separated text file in the format:
//!
//! ```text
//! Date\tTime\tSci_Name\tCom_Name\tConfidence\tLat\tLon\tCutoff\tWeek\tSens\tOverlap\tFile_Name
//! 2026-01-15\t06:23:11\tTurdus merula\tEurasian Blackbird\t0.921\t51.5\t-0.1\t0.7\t3\t1.0\t0.0\trec.wav
//! ```
//!
//! The first line is a header (may vary); the remaining lines are data.
//! Lines with < 12 fields are skipped with a warning.
//!
//! The importer is tolerant of:
//! - Missing optional fields (replaced by `NULL`)
//! - Comma-separated files (auto-detected if no tab in header)
//! - Windows line endings (`\r\n`)

use std::io::{BufRead, BufReader};
use std::path::Path;

use rusqlite::Connection;

use crate::error::MigrateError;
use crate::progress::{MigrationProgress, MigrationStage, ProgressHandle};
use crate::traits::{MigrationSummary, Migrator};

/// Minimum number of fields required per data line.
const MIN_FIELDS: usize = 5; // Date, Time, Sci_Name, Com_Name, Confidence

/// Batch size for transactions.
const BATCH_SIZE: usize = 500;

/// Intermediate parsed row.
struct CsvRow {
    date: String,
    time: String,
    sci_name: String,
    com_name: String,
    confidence: f64,
    lat: Option<f64>,
    lon: Option<f64>,
    cutoff: Option<f64>,
    week: Option<i64>,
    sens: Option<f64>,
    overlap: Option<f64>,
    file_name: Option<String>,
}

/// Importer for BirdNET-Pi TSV/CSV detection log files.
#[derive(Debug, Clone, Default)]
pub struct CsvImporter;

impl Migrator for CsvImporter {
    fn migrate(
        &self,
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
    ) -> Result<MigrationSummary, MigrateError> {
        progress.set_stage(MigrationStage::Importing, "Opening CSV source file");

        let file = std::fs::File::open(source_path).map_err(|e| {
            MigrateError::Io(e)
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        // Read header line to detect delimiter.
        let header = match lines.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => return Err(MigrateError::Io(e)),
            None => return Err(MigrateError::CsvParse("file is empty".to_string())),
        };
        let delim = if header.contains('\t') { '\t' } else { ',' };

        // Pre-scan to estimate total lines (for progress reporting).
        drop(lines);
        let total = count_lines(source_path)?.saturating_sub(1); // minus header

        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 0,
            rows_total: total as u64,
            message: format!("Parsing {total} CSV lines"),
            error: None,
        });

        // Re-open for actual import.
        let file2 = std::fs::File::open(source_path).map_err(MigrateError::Io)?;
        let reader2 = BufReader::new(file2);
        let mut lines2 = reader2.lines();
        // Skip header.
        let _ = lines2.next();

        // Open (or create) destination database and run schema migrations.
        let dest_conn =
            birdnet_db::sqlite::open_or_create(dest_path).map_err(|e| {
                MigrateError::DestinationOpen(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error {
                        code: rusqlite::ffi::ErrorCode::CannotOpen,
                        extended_code: 0,
                    },
                    Some(e.to_string()),
                ))
            })?;
        birdnet_db::migration::migrate(&dest_conn).map_err(|e| {
            MigrateError::DestinationOpen(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::CannotOpen,
                    extended_code: 0,
                },
                Some(e.to_string()),
            ))
        })?;

        let mut imported = 0u64;
        let mut skipped = 0u64;
        let mut batch: Vec<CsvRow> = Vec::with_capacity(BATCH_SIZE);

        for line_result in lines2 {
            let line = line_result.map_err(MigrateError::Io)?;
            let line = line.trim_end_matches('\r').to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            match parse_line(&line, delim) {
                Ok(row) => {
                    batch.push(row);
                    if batch.len() >= BATCH_SIZE {
                        let (ins, sk) = flush_batch(&dest_conn, &batch)?;
                        imported += ins;
                        skipped += sk;
                        batch.clear();
                        progress.update(MigrationProgress {
                            stage: MigrationStage::Importing,
                            rows_imported: imported,
                            rows_total: total as u64,
                            message: format!("Imported {imported} rows…"),
                            error: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(err = %e, line = %line, "skipping unparseable CSV line");
                    skipped += 1;
                }
            }
        }

        // Flush remainder.
        if !batch.is_empty() {
            let (ins, sk) = flush_batch(&dest_conn, &batch)?;
            imported += ins;
            skipped += sk;
        }

        Ok(MigrationSummary {
            source_rows: total as u64,
            imported_rows: imported,
            skipped_rows: skipped,
            schema_name: "BirdNET-Pi CSV".to_string(),
            source_path: source_path.display().to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse one data line into a `CsvRow`.
fn parse_line(line: &str, delim: char) -> Result<CsvRow, MigrateError> {
    let fields: Vec<&str> = line.splitn(12, delim).collect();
    if fields.len() < MIN_FIELDS {
        return Err(MigrateError::CsvParse(format!(
            "expected ≥{MIN_FIELDS} fields, got {}",
            fields.len()
        )));
    }

    let parse_opt_f64 = |s: &str| -> Option<f64> {
        let s = s.trim();
        if s.is_empty() || s == "\\N" || s == "NULL" { None } else { s.parse().ok() }
    };
    let parse_opt_i64 = |s: &str| -> Option<i64> {
        let s = s.trim();
        if s.is_empty() || s == "\\N" || s == "NULL" { None } else { s.parse().ok() }
    };
    let parse_opt_str = |s: &str| -> Option<String> {
        let s = s.trim();
        if s.is_empty() || s == "\\N" || s == "NULL" { None } else { Some(s.to_string()) }
    };

    let confidence: f64 = fields[4].trim().parse().map_err(|_| {
        MigrateError::CsvParse(format!("invalid confidence: '{}'", fields[4]))
    })?;

    Ok(CsvRow {
        date: fields[0].trim().to_string(),
        time: fields[1].trim().to_string(),
        sci_name: fields[2].trim().to_string(),
        com_name: fields[3].trim().to_string(),
        confidence,
        lat: fields.get(5).copied().and_then(parse_opt_f64),
        lon: fields.get(6).copied().and_then(parse_opt_f64),
        cutoff: fields.get(7).copied().and_then(parse_opt_f64),
        week: fields.get(8).copied().and_then(parse_opt_i64),
        sens: fields.get(9).copied().and_then(parse_opt_f64),
        overlap: fields.get(10).copied().and_then(parse_opt_f64),
        file_name: fields.get(11).copied().and_then(parse_opt_str),
    })
}

/// Insert a batch of rows into the destination, returning (inserted, skipped).
fn flush_batch(conn: &Connection, batch: &[CsvRow]) -> Result<(u64, u64), MigrateError> {
    let tx = conn.unchecked_transaction().map_err(MigrateError::DataTransfer)?;
    let mut inserted = 0u64;
    let mut skipped = 0u64;

    for row in batch {
        let rows_changed = tx
            .execute(
                "INSERT OR IGNORE INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    row.date, row.time, row.sci_name, row.com_name,
                    row.confidence, row.lat, row.lon, row.cutoff,
                    row.week, row.sens, row.overlap, row.file_name,
                ],
            )
            .map_err(MigrateError::DataTransfer)?;

        if rows_changed == 0 { skipped += 1; } else { inserted += 1; }
    }

    tx.commit().map_err(MigrateError::DataTransfer)?;
    Ok((inserted, skipped))
}

/// Count non-empty lines in a file (including header).
fn count_lines(path: &Path) -> Result<usize, MigrateError> {
    let file = std::fs::File::open(path).map_err(MigrateError::Io)?;
    let reader = BufReader::new(file);
    Ok(reader.lines().filter(|l| l.as_ref().map_or(false, |s| !s.trim().is_empty())).count())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    fn make_csv(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parse_tab_separated_line() {
        let line = "2026-01-15\t06:23:11\tTurdus merula\tEurasian Blackbird\t0.921\t51.5\t-0.1\t0.7\t3\t1.0\t0.0\trec.wav";
        let row = parse_line(line, '\t').unwrap();
        assert_eq!(row.date, "2026-01-15");
        assert_eq!(row.com_name, "Eurasian Blackbird");
        assert!((row.confidence - 0.921).abs() < 1e-6);
        assert_eq!(row.file_name.as_deref(), Some("rec.wav"));
    }

    #[test]
    fn parse_comma_separated_line() {
        let line = "2026-01-15,06:23:11,Turdus merula,Eurasian Blackbird,0.80,,,,,,,";
        let row = parse_line(line, ',').unwrap();
        assert_eq!(row.com_name, "Eurasian Blackbird");
        assert!(row.lat.is_none());
    }

    #[test]
    fn parse_line_too_few_fields() {
        let result = parse_line("2026-01-15\t06:23:11", '\t');
        assert!(result.is_err());
    }

    #[test]
    fn csv_import_roundtrip() {
        let tsv = "Date\tTime\tSci_Name\tCom_Name\tConfidence\tLat\tLon\tCutoff\tWeek\tSens\tOverlap\tFile_Name\n\
                   2026-01-15\t06:23:11\tTurdus merula\tEurasian Blackbird\t0.921\t51.5\t-0.1\t0.7\t3\t1.0\t0.0\trec.wav\n\
                   2026-01-16\t07:00:00\tPasser domesticus\tHouse Sparrow\t0.85\t\t\t\t\t\t\t\n";

        let src = make_csv(tsv);
        let dst = NamedTempFile::new().unwrap();
        let progress = crate::progress::ProgressHandle::new();

        let summary = CsvImporter.migrate(src.path(), dst.path(), &progress).unwrap();
        assert_eq!(summary.imported_rows, 2);
        assert_eq!(summary.schema_name, "BirdNET-Pi CSV");
    }
}
