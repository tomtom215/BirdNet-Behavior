//! BirdNET-Pi pre- and post-migration validation.

use std::path::Path;

use rusqlite::params;

use crate::error::MigrateError;
use crate::schema::{open_source_readonly, row_count};
use crate::traits::{ValidationCheck, ValidationReport, Validator};

/// Validates a BirdNET-Pi source database and verifies import completeness.
#[derive(Debug, Clone, Default)]
pub struct BirdNetPiValidator;

impl Validator for BirdNetPiValidator {
    fn validate_source(&self, source_path: &Path) -> Result<ValidationReport, MigrateError> {
        let conn = open_source_readonly(source_path)?;
        let mut checks = Vec::new();

        // 1. Check that the table exists and is readable.
        let count = match row_count(&conn, "detections") {
            Ok(n) => {
                checks.push(ValidationCheck::pass(
                    "table_readable",
                    format!("detections table has {n} rows"),
                ));
                n
            }
            Err(e) => {
                checks.push(ValidationCheck::fail(
                    "table_readable",
                    format!("cannot read detections table: {e}"),
                    true,
                ));
                return Ok(ValidationReport::new("BirdNET-Pi", 0, checks));
            }
        };

        // 2. Warn on empty database (not an error — user might want to import anyway).
        if count == 0 {
            checks.push(ValidationCheck::fail(
                "non_empty",
                "source database has no detections".to_string(),
                false, // not required
            ));
        } else {
            checks.push(ValidationCheck::pass(
                "non_empty",
                format!("{count} detections to import"),
            ));
        }

        // 3. Check that Date/Time name a real point in time.
        let bad_dates = count_bad_dates(&conn).unwrap_or(0);
        if bad_dates > 0 {
            // Not "will be skipped": nothing skips them. The importer copies
            // these rows through verbatim (turning a NULL Date into ""), our
            // own `Date TEXT NOT NULL` accepts them, and they sync to DuckDB.
            // They are simply unplaceable in time from then on, so they are
            // absent from the analytics dashboards while still counting toward
            // the station's detection total. Saying so is the difference
            // between an operator who knows why the numbers disagree and one
            // who finds out by arithmetic.
            checks.push(ValidationCheck::fail(
                "date_format",
                format!(
                    "{bad_dates} rows have a Date/Time that names no point in time; \
                     they will be imported and counted, but cannot appear in any \
                     date- or time-based analytics"
                ),
                false,
            ));
        } else {
            checks.push(ValidationCheck::pass(
                "date_format",
                "every Date/Time value parses".to_string(),
            ));
        }

        // 4. Check confidence values are in range.
        let bad_conf = count_out_of_range_confidence(&conn).unwrap_or(0);
        if bad_conf > 0 {
            checks.push(ValidationCheck::fail(
                "confidence_range",
                format!("{bad_conf} rows have confidence outside [0, 1] (will be clamped)"),
                false,
            ));
        } else {
            checks.push(ValidationCheck::pass(
                "confidence_range",
                "all confidence values are in [0, 1]".to_string(),
            ));
        }

        Ok(ValidationReport::new("BirdNET-Pi", count, checks))
    }

    fn validate_destination(
        &self,
        source_path: &Path,
        dest_path: &Path,
    ) -> Result<ValidationReport, MigrateError> {
        let src_conn = open_source_readonly(source_path)?;
        let src_count = row_count(&src_conn, "detections")?;

        let dest_conn = open_source_readonly(dest_path)?;
        let dest_count = row_count(&dest_conn, "detections")?;

        let mut checks = Vec::new();

        if dest_count >= src_count {
            checks.push(ValidationCheck::pass(
                "row_count",
                format!("destination has {dest_count} rows (source had {src_count})"),
            ));
        } else {
            checks.push(ValidationCheck::fail(
                "row_count",
                format!(
                    "destination has {dest_count} rows but source had {src_count} — {n} rows missing",
                    n = src_count - dest_count
                ),
                false, // existing rows in dest inflate count legitimately
            ));
        }

        Ok(ValidationReport::new("BirdNET-Pi", src_count, checks))
    }
}

/// Count rows whose `Date`/`Time` name no point in time.
///
/// Scans the whole table rather than a leading sample. The previous
/// `LIMIT 1000` check reported on the first thousand rows a table scan happened
/// to return, so a multi-year database whose bad rows sat anywhere later — the
/// normal case, since these arrive with a hardware clock reset or an
/// interrupted write — passed validation as "all sampled Date values are
/// well-formed". The import that follows reads every row anyway, so one extra
/// pass buys an answer about the actual database instead of its prefix.
///
/// Three conditions, because each catches rows the others miss:
///   * `Date`/`Time` NULL — the case the old check was *structurally* unable to
///     see, since `NULL NOT GLOB …` is NULL, not true, and so never counted.
///     It is also the most common one: the importer maps a NULL `Date` to `""`.
///   * `Date` not `YYYY-MM-DD` — the format the OLAP copy must parse.
///   * the concatenation failing SQLite's own datetime parser, which is the
///     closest available proxy for the `TRY_CAST` DuckDB will apply, and the
///     only one of the three that inspects `Time` for anything but NULL.
fn count_bad_dates(conn: &rusqlite::Connection) -> rusqlite::Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM detections
          WHERE Date IS NULL
             OR Time IS NULL
             OR Date NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
             OR datetime(Date || ' ' || Time) IS NULL",
        params![],
        |row| row.get(0),
    )?;
    #[allow(clippy::cast_sign_loss)]
    let result = count.max(0) as u64;
    Ok(result)
}

/// Count rows with confidence outside [0, 1].
fn count_out_of_range_confidence(conn: &rusqlite::Connection) -> rusqlite::Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Confidence < 0 OR Confidence > 1",
        params![],
        |row| row.get(0),
    )?;
    #[allow(clippy::cast_sign_loss)]
    let result = count.max(0) as u64;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn make_pi_db_with_rows(rows: usize) -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
        for i in 0..rows {
            conn.execute(
                "INSERT INTO detections VALUES (?1,'06:00:00','Turdus merula',
                 'Blackbird',0.9,51.5,-0.1,0.7,1,1.0,0.0,'rec.wav')",
                params![format!("2026-01-{:02}", (i % 28) + 1)],
            )
            .unwrap();
        }
        drop(conn);
        tmp
    }

    /// The unplaceable-row check must see NULLs, a bad `Time`, and rows past
    /// the first thousand.
    ///
    /// Each row here defeats the previous check for a different reason: the
    /// NULL is invisible to `Date NOT GLOB …` (which yields NULL, not true, so
    /// the row was never counted); the bad `Time` was never examined at all;
    /// and the last row sits beyond the `LIMIT 1000` sample that decided the
    /// verdict for the whole database.
    #[test]
    fn unplaceable_rows_are_counted_including_nulls_time_and_beyond_the_first_1000() {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
        for i in 0..1200 {
            conn.execute(
                "INSERT INTO detections VALUES (?1,'06:00:00','Turdus merula',
                 'Blackbird',0.9,51.5,-0.1,0.7,1,1.0,0.0,'rec.wav')",
                params![format!("2026-01-{:02}", (i % 28) + 1)],
            )
            .unwrap();
        }
        conn.execute_batch(
            "INSERT INTO detections VALUES (NULL,NULL,'Parus major','Great Tit',0.8,NULL,NULL,NULL,NULL,NULL,NULL,'b.wav');
             INSERT INTO detections VALUES ('','','Erithacus rubecula','Robin',0.7,NULL,NULL,NULL,NULL,NULL,NULL,'c.wav');
             INSERT INTO detections VALUES ('2026-02-01','not-a-time','Corvus corax','Raven',0.6,NULL,NULL,NULL,NULL,NULL,NULL,'d.wav');",
        )
        .unwrap();

        assert_eq!(count_bad_dates(&conn).unwrap(), 3);

        drop(conn);
        let report = BirdNetPiValidator.validate_source(tmp.path()).unwrap();
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "date_format")
            .expect("date_format check");
        assert!(!check.passed);
        assert!(
            !check.detail.contains("skipped"),
            "nothing skips these rows; saying so sent operators looking for a \
             filter that does not exist: {}",
            check.detail
        );
    }

    #[test]
    fn validate_source_passes() {
        let tmp = make_pi_db_with_rows(10);
        let v = BirdNetPiValidator;
        let report = v.validate_source(tmp.path()).unwrap();
        assert!(report.passed);
        assert_eq!(report.source_rows, 10);
    }

    #[test]
    fn validate_source_empty_is_warning_not_error() {
        let tmp = make_pi_db_with_rows(0);
        let v = BirdNetPiValidator;
        let report = v.validate_source(tmp.path()).unwrap();
        // empty database has a failing non-required check → still passes
        assert!(report.passed);
    }
}
