//! BirdNET-Pi source support.
//!
//! Provides `SchemaDetector`, `Migrator`, and `Validator` implementations
//! for the BirdNET-Pi `BirdDB.txt` `SQLite` format **and** the CSV/TSV
//! detection log export.
//!
//! | Format | Extension(s) | Importer |
//! |--------|-------------|----------|
//! | SQLite | `.db`, `.txt`, `.sqlite` | [`BirdNetPiImporter`] |
//! | CSV/TSV | `.csv`, `.tsv`, `.txt` (tab-delimited) | [`CsvImporter`] |

pub mod csv_importer;
pub mod detector;
pub mod importer;
pub mod species_report;
pub mod validator;

pub use csv_importer::CsvImporter;
pub use detector::BirdNetPiDetector;
pub use importer::BirdNetPiImporter;
pub use species_report::{
    MigrationReport, PostMigrationReport, SpeciesDiff, SpeciesStats, compare_source_dest,
    generate_report,
};
pub use validator::BirdNetPiValidator;

use std::path::Path;

use crate::error::MigrateError;
use crate::progress::ProgressHandle;
use crate::schema::DetectedSchema;
use crate::traits::{MigrationSummary, ValidationReport};

/// High-level entry point: detect, validate, and import a BirdNET-Pi database
/// **or** CSV/TSV detection log.
///
/// This is the function called by the web admin migration endpoint.
/// All three steps are run in sequence; validation failures are non-fatal
/// (warnings) unless `strict` is `true`.
///
/// The source file is **never modified**.
///
/// # Errors
///
/// Returns `MigrateError` if detection, validation (in strict mode), or
/// import fails.
pub fn run_migration(
    source_path: &Path,
    dest_path: &Path,
    strict: bool,
    progress: &ProgressHandle,
) -> Result<MigrationSummary, MigrateError> {
    run_migration_with_options(
        source_path,
        dest_path,
        strict,
        progress,
        &crate::provenance::ImportOptions::default(),
        (None, None),
    )
}

/// [`run_migration`] plus the reconciliation the operator chose.
///
/// `options` carries the clock shift and the label for the source station;
/// `station` is this station's own `(lat, lon)` so the recorded batch can say
/// how far apart the two sites are. Both are recorded whether or not they are
/// set, because "we did not know" is itself worth being able to read back.
///
/// # Errors
///
/// Returns `MigrateError` if detection, validation (in strict mode), or import
/// fails.
pub fn run_migration_with_options(
    source_path: &Path,
    dest_path: &Path,
    strict: bool,
    progress: &ProgressHandle,
    options: &crate::provenance::ImportOptions,
    station: (Option<f64>, Option<f64>),
) -> Result<MigrationSummary, MigrateError> {
    use crate::traits::{SchemaDetector, Validator};

    // CSV/TSV path — if the file is not a SQLite database, try CSV import.
    if is_csv_file(source_path) {
        return CsvImporter.migrate_with_options(
            source_path,
            dest_path,
            progress,
            options,
            station,
        );
    }

    let detector = BirdNetPiDetector;
    let validator = BirdNetPiValidator;
    let importer = BirdNetPiImporter;

    // Step 1: Detect schema.
    let schema = detector.detect(source_path)?;

    if matches!(schema, DetectedSchema::BirdNetBehavior { .. }) && strict {
        return Err(MigrateError::UnsupportedSchema(
            "source is already a BirdNet-Behavior database — no migration needed".to_string(),
        ));
    }

    // Step 2: Validate source.
    let report = validator.validate_source(source_path)?;
    if !report.passed && strict {
        let failures: Vec<&str> = report
            .checks
            .iter()
            .filter(|c| !c.passed && c.required)
            .map(|c| c.detail.as_str())
            .collect();
        return Err(MigrateError::ValidationFailed(failures.join("; ")));
    }

    // Step 3: Import, carrying the operator's reconciliation choices so the
    // merged history stays attributable to the site and clock it came from.
    importer.migrate_with_options(source_path, dest_path, progress, options, station)
}

/// Run validation only (without importing).  Used by the web UI pre-flight check.
///
/// Returns a tuple of `(schema, validation_report, migration_report)` for
/// the admin UI to display a comprehensive pre-migration preview.
///
/// For CSV files, returns an estimated schema and a minimal validation report.
///
/// # Errors
///
/// Returns `MigrateError` if the source cannot be opened.
pub fn validate_source(
    source_path: &Path,
) -> Result<(DetectedSchema, ValidationReport, MigrationReport), MigrateError> {
    validate_source_against_station(source_path, None, None)
}

/// [`validate_source`] with the station's own coordinates, so the report can
/// say whether the source came from *here*.
///
/// Split out rather than folded in because the migrate crate has no access to
/// the station's settings — those live in the destination database, which
/// validation deliberately does not open. The caller supplies them.
///
/// # Errors
///
/// Returns `MigrateError` if the source cannot be opened.
pub fn validate_source_against_station(
    source_path: &Path,
    station_lat: Option<f64>,
    station_lon: Option<f64>,
) -> Result<(DetectedSchema, ValidationReport, MigrationReport), MigrateError> {
    use crate::traits::{SchemaDetector, Validator};

    if is_csv_file(source_path) {
        let (schema, report) = validate_csv_source(source_path)?;
        let migration_report = MigrationReport {
            total_rows: i64::try_from(schema.row_count()).unwrap_or(i64::MAX),
            unique_species: 0,
            date_range: None,
            top_species: vec![],
            null_date_rows: 0,
            invalid_confidence_rows: 0,
            duplicate_rows: 0,
            quality_ok: report.passed,
        };
        return Ok((schema, report, migration_report));
    }

    let detector = BirdNetPiDetector;
    let validator = BirdNetPiValidator;

    let schema = detector.detect(source_path)?;
    let mut report = validator.validate_source(source_path)?;

    // The check the four original ones never made: is this the same place?
    // Appended here rather than inside the Validator so it stays a pure
    // source-vs-station comparison; the validator itself never sees the
    // destination.
    // `passed` is recomputed the way `ValidationReport::new` does — a check
    // that is not `required` never flips it. That matters here: the location
    // check is advisory by design, and a station that has simply not set its
    // coordinates yet must not find strict-mode imports blocked by a check that
    // could not be performed.
    let profile = crate::provenance::SourceProfile::read(source_path)?;
    report.checks.push(crate::provenance::location_check(
        &profile,
        station_lat,
        station_lon,
    ));
    report.passed = report.checks.iter().all(|c| c.passed || !c.required);

    let migration_report = generate_report(source_path)?;
    Ok((schema, report, migration_report))
}

/// Cheap CSV validation: check file exists, is readable, has ≥1 data line.
fn validate_csv_source(path: &Path) -> Result<(DetectedSchema, ValidationReport), MigrateError> {
    use crate::traits::{ValidationCheck, ValidationReport};
    use std::io::BufReader;

    // Validation reads the file the way the import will: a regular file only,
    // and one line at a time under the importer's byte cap. It used
    // `BufRead::lines()`, which buffers a whole line however long: one POST naming
    // a 300 MB single-line file, or `/dev/zero`, took the process's memory with it.
    let file = csv_importer::open_regular_file(path)?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::with_capacity(512);

    let header = match csv_importer::read_capped_line(&mut reader, &mut line) {
        Ok(n) if n > 0 => String::from_utf8_lossy(&line)
            .trim_end_matches(['\n', '\r'])
            .to_owned(),
        Err(MigrateError::CsvParse(why)) => {
            return Ok((
                DetectedSchema::BirdNetPiCsv { row_count: 0 },
                ValidationReport::new(
                    "BirdNET-Pi CSV",
                    0,
                    vec![ValidationCheck::fail("readable", why, true)],
                ),
            ));
        }
        _ => String::new(),
    };
    if header.is_empty() {
        return Ok((
            DetectedSchema::BirdNetPiCsv { row_count: 0 },
            ValidationReport::new(
                "BirdNET-Pi CSV",
                0,
                vec![ValidationCheck::fail(
                    "readable",
                    "file is empty or unreadable",
                    true,
                )],
            ),
        ));
    }

    // The same layout detection the importer uses, so the check and the
    // import cannot disagree about what the file is.
    let has_expected_header = csv_importer::Layout::detect(&header).has_header();

    let mut row_count = 0_u64;
    loop {
        line.clear();
        match csv_importer::read_capped_line(&mut reader, &mut line) {
            Ok(0) => break,
            Ok(_) => {
                if !String::from_utf8_lossy(&line).trim().is_empty() {
                    row_count += 1;
                }
            }
            Err(MigrateError::CsvParse(why)) => {
                return Ok((
                    DetectedSchema::BirdNetPiCsv { row_count },
                    ValidationReport::new(
                        "BirdNET-Pi CSV",
                        row_count,
                        vec![ValidationCheck::fail("readable", why, true)],
                    ),
                ));
            }
            Err(e) => return Err(e),
        }
    }

    let checks = vec![
        if has_expected_header {
            ValidationCheck::pass("header", "header row matches expected format")
        } else {
            ValidationCheck::fail(
                "header",
                "header does not match expected BirdNET-Pi CSV format",
                false,
            )
        },
        if row_count > 0 {
            ValidationCheck::pass("row_count", format!("{row_count} data rows found"))
        } else {
            ValidationCheck::fail("row_count", "no data rows found", true)
        },
    ];

    let schema = DetectedSchema::BirdNetPiCsv { row_count };
    let report = ValidationReport::new("BirdNET-Pi CSV", row_count, checks);
    Ok((schema, report))
}

/// Return `true` if the path extension suggests CSV/TSV.
///
/// We also try to peek at the first bytes to distinguish `SQLite` (magic `SQLite format 3`)
/// from plain text.
fn is_csv_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);

    if matches!(ext.as_deref(), Some("csv" | "tsv")) {
        return true;
    }

    // Opening a FIFO to peek would block until something writes to it, and a
    // device node would read forever: anything but a regular file is left for
    // the SQLite route, which refuses it by name.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return false;
    }

    // Peek: SQLite files begin with "SQLite format 3\0"
    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::Read;
        let mut magic = [0u8; 16];
        if f.read_exact(&mut magic).is_ok() {
            return &magic[..15] != b"SQLite format 3";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::ProgressHandle;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    /// Validation reads a CSV under the importer's one-megabyte line cap, as the
    /// import does, and says so in the report: it used `BufRead::lines()`, which
    /// buffered a line however long (300 MB of one line took the process from
    /// 123 MB to 709 MB; `/dev/zero` took it all).
    #[test]
    fn csv_validation_refuses_a_line_longer_than_the_import_allows() {
        use std::io::Write as _;
        let mut f = NamedTempFile::with_suffix(".csv").unwrap();
        writeln!(f, "Date;Time;Sci_Name;Com_Name;Confidence").unwrap();
        f.write_all(&vec![b'a'; 3 * 1024 * 1024]).unwrap();
        f.write_all(b"\n").unwrap();
        let (_, report) = validate_csv_source(f.path()).unwrap();
        assert!(!report.passed, "{report:?}");
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "readable" && c.detail.contains("cap")),
            "{report:?}"
        );
    }

    /// The counterpart: an ordinary CSV still validates and counts its rows.
    #[test]
    fn csv_validation_still_counts_an_ordinary_file() {
        use std::io::Write as _;
        let mut f = NamedTempFile::with_suffix(".csv").unwrap();
        writeln!(
            f,
            "Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name"
        )
        .unwrap();
        for i in 0..5 {
            writeln!(f, "2026-01-0{};10:00:00;Pica pica;Eurasian Magpie;0.9;51.5;-0.1;0.7;1;1.25;0.0;x{i}.wav", i + 1).unwrap();
        }
        let (schema, report) = validate_csv_source(f.path()).unwrap();
        assert!(
            matches!(schema, DetectedSchema::BirdNetPiCsv { row_count: 5 }),
            "{schema:?}"
        );
        assert!(report.passed, "{report:?}");
    }

    /// Anything that is not a regular file is refused by name, without being
    /// opened for reading: a directory, and a device that never ends.
    #[test]
    fn a_source_that_is_not_a_regular_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for path in [dir.path(), Path::new("/dev/zero")] {
            let csv = validate_csv_source(path).expect_err("csv");
            assert!(
                matches!(csv, MigrateError::SourceNotFound(ref m) if m.contains("regular file")),
                "{csv:?}"
            );
            let db = crate::schema::open_source_readonly(path).expect_err("sqlite");
            assert!(
                matches!(db, MigrateError::SourceNotFound(ref m) if m.contains("regular file")),
                "{db:?}"
            );
            assert!(
                !is_csv_file(path),
                "{path:?} must not be routed to the CSV reader"
            );
        }
    }

    fn make_sqlite_source(n: usize) -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
            Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
            Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
            Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
        for i in 0..n {
            conn.execute(
                "INSERT INTO detections VALUES (?1,'06:00:00','Turdus merula','Blackbird',0.9,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
                rusqlite::params![format!("2026-01-{:02}", (i % 28) + 1)],
            ).unwrap();
        }
        drop(conn);
        tmp
    }

    #[test]
    fn run_migration_sqlite_end_to_end() {
        let src = make_sqlite_source(20);
        let dst = NamedTempFile::new().unwrap();
        let progress = ProgressHandle::new();

        let summary = run_migration(src.path(), dst.path(), false, &progress).unwrap();
        assert_eq!(summary.source_rows, 20);
        assert_eq!(summary.imported_rows, 20);
    }

    #[test]
    fn validate_source_returns_schema_and_report() {
        let src = make_sqlite_source(5);
        let (schema, report, _) = validate_source(src.path()).unwrap();
        assert!(matches!(schema, DetectedSchema::BirdNetPi { .. }));
        assert!(report.passed);
    }

    #[test]
    fn csv_detected_by_extension() {
        use std::io::Write as _;
        let mut tmp = NamedTempFile::with_suffix(".csv").unwrap();
        writeln!(tmp, "Date\tTime\tSci_Name\tCom_Name\tConfidence").unwrap();
        writeln!(tmp, "2026-01-01\t06:00:00\tTurdus merula\tBlackbird\t0.9").unwrap();
        assert!(is_csv_file(tmp.path()));
    }
}
