//! Reads for the study bundle (RES-08): whole tables, every column, in a fixed
//! order, with each value kept as `SQLite` stored it.
//!
//! The bundle's job is to hand a researcher what the station recorded, not a
//! presentation of it, so these reads select every column a table has (read
//! from the table, so a column a later migration adds reaches the bundle
//! without an edit here) and order rows by a total key, so the same database
//! gives the same rows in the same order. A value is a [`Cell`]: `NULL` stays
//! distinct from an empty string, and a real number stays the `f64` that was
//! stored, which the writer prints in its shortest round-trip form.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, params_from_iter};

use crate::sqlite::connection::DbError;

/// One stored value.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// `NULL`.
    Null,
    /// An integer.
    Int(i64),
    /// A real, as stored.
    Real(f64),
    /// Text.
    Text(String),
    /// A blob, which no table the bundle reads should hold; its length only.
    Blob(usize),
}

/// A table's columns and rows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableDump {
    /// Column names, in the table's order.
    pub columns: Vec<String>,
    /// Rows, in the read's order.
    pub rows: Vec<Vec<Cell>>,
    /// Whether the read stopped at its row limit.
    pub truncated: bool,
}

/// The columns of `table`, in order.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
    let cols = stmt
        .query_map([table], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cols)
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The tables the bundle reads, each with the date filter and the total order
/// it is read in. Fixed here, never taken from a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudyTable {
    /// Every detection row of the range, rejected ones included.
    Detections,
    /// Each classifier's answer on those rows.
    ModelScores,
    /// Review history of the range's rows.
    ReviewEvents,
    /// Every analysis run.
    Runs,
    /// Every classifier of every run.
    RunModels,
    /// Weather hours of the range.
    Weather,
    /// The hourly effort ledger of the range, per source.
    Effort,
}

impl StudyTable {
    /// The table read.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Self::Detections => "detections",
            Self::ModelScores => "detection_model_scores",
            Self::ReviewEvents => "review_events",
            Self::Runs => "analysis_runs",
            Self::RunModels => "analysis_run_models",
            Self::Weather => "weather",
            Self::Effort => "recording_effort_hourly",
        }
    }

    /// The `WHERE` clause, over `?1` (first local date) and `?2` (last),
    /// or `None` for a table read whole.
    const fn filter(self) -> Option<&'static str> {
        match self {
            Self::Detections | Self::ModelScores => Some("Date >= ?1 AND Date <= ?2"),
            Self::ReviewEvents | Self::Effort => Some("date >= ?1 AND date <= ?2"),
            Self::Weather => Some("substr(at, 1, 10) >= ?1 AND substr(at, 1, 10) <= ?2"),
            Self::Runs | Self::RunModels => None,
        }
    }

    /// A total order: the table's natural key, then `rowid` for ties.
    const fn order(self) -> &'static str {
        match self {
            Self::Detections => {
                "Date, Time, Sci_Name, COALESCE(File_Name, ''), COALESCE(Source, ''), rowid"
            }
            Self::ModelScores => "Date, Time, Sci_Name, File_Name, model_id",
            Self::ReviewEvents | Self::Runs => "id",
            Self::RunModels => "run_id, model_id",
            Self::Weather => "at",
            Self::Effort => "date, hour, source",
        }
    }
}

/// Read `table` over `from..=to` (local dates; ignored by a table read whole),
/// at most `max_rows` rows.
///
/// A table this station's schema does not have reads as empty with no columns,
/// so an older database still exports.
///
/// # Errors
///
/// `DbError` on query failure.
pub fn dump(
    conn: &Connection,
    table: StudyTable,
    from: &str,
    to: &str,
    max_rows: usize,
) -> Result<TableDump, DbError> {
    let columns = table_columns(conn, table.table())?;
    if columns.is_empty() {
        return Ok(TableDump::default());
    }
    let select = columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let (where_sql, args): (String, Vec<&str>) =
        table.filter().map_or((String::new(), Vec::new()), |f| {
            (format!(" WHERE {f}"), vec![from, to])
        });
    let limit = max_rows.saturating_add(1);
    let sql = format!(
        "SELECT {select} FROM {}{where_sql} ORDER BY {} LIMIT {limit}",
        table.table(),
        table.order()
    );
    let mut stmt = conn.prepare(&sql)?;
    let width = columns.len();
    let mut rows = stmt
        .query_map(params_from_iter(args), |r| {
            (0..width)
                .map(|i| {
                    Ok(match r.get_ref(i)? {
                        ValueRef::Null => Cell::Null,
                        ValueRef::Integer(v) => Cell::Int(v),
                        ValueRef::Real(v) => Cell::Real(v),
                        ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
                        ValueRef::Blob(b) => Cell::Blob(b.len()),
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let truncated = rows.len() > max_rows;
    rows.truncate(max_rows);
    Ok(TableDump {
        columns,
        rows,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;

    #[test]
    fn a_dump_keeps_every_column_null_and_the_stored_real() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', '06:00:00', 'Pica pica', 'Magpie', 0.70004, NULL)",
            [],
        )
        .unwrap();
        let d = dump(
            &conn,
            StudyTable::Detections,
            "2026-05-01",
            "2026-05-01",
            10,
        )
        .unwrap();
        assert_eq!(
            d.columns,
            table_columns(&conn, "detections").unwrap(),
            "every column"
        );
        let score_at = d.columns.iter().position(|c| c == "Confidence").unwrap();
        let clip_at = d.columns.iter().position(|c| c == "File_Name").unwrap();
        assert_eq!(d.rows[0][score_at], Cell::Real(0.70004));
        assert_eq!(d.rows[0][clip_at], Cell::Null);
        assert!(!d.truncated);
        let none = dump(
            &conn,
            StudyTable::Detections,
            "2026-05-02",
            "2026-05-03",
            10,
        )
        .unwrap();
        assert!(none.rows.is_empty());
    }

    #[test]
    fn a_dump_stops_at_its_limit_and_says_so() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        for t in ["06:00:00", "06:00:01", "06:00:02"] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-05-01', ?1, 'Pica pica', 'Magpie', 0.9)",
                [t],
            )
            .unwrap();
        }
        let d = dump(&conn, StudyTable::Detections, "2026-05-01", "2026-05-01", 2).unwrap();
        assert_eq!(d.rows.len(), 2);
        assert!(d.truncated);
    }
}
