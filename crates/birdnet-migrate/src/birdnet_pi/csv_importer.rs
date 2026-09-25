//! BirdNET-Pi CSV/TSV detection log importer.
//!
//! Three shapes reach this importer, and it reads all of them:
//!
//! * **BirdNET-Pi's `BirdDB.txt`** — semicolon-separated, the twelve columns
//!   below in this order, with or without a header line. The migration page
//!   names this file, and this importer used to read it as a single-column
//!   CSV: every line was skipped and the import reported success with 0 rows.
//! * **A tab- or comma-separated export** of the same twelve columns.
//! * **This station's own CSV export** (`/api/v2/detections/export`) — a
//!   header row, RFC 4180 quoting, a `'` guard in front of any text that looks
//!   like a spreadsheet formula, and five more columns after `File_Name`.
//!   Columns are matched by header name, so the extra ones are ignored rather
//!   than run together into `File_Name` (which made every re-imported row a
//!   new, duplicate detection pointing at a clip that did not exist).
//!
//! ```text
//! Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name
//! 2026-01-15;06:23:11;Turdus merula;Eurasian Blackbird;0.921;51.5;-0.1;0.7;3;1.0;0.0;rec.wav
//! ```
//!
//! A first line is a header when its first field is `Date`; otherwise it is
//! data. Missing optional fields become `NULL`; Windows line endings are
//! accepted. A file in which not one line could be read is an error, not an
//! import of nothing.

use std::io::{BufRead, BufReader};
use std::path::Path;

use rusqlite::Connection;

use crate::error::MigrateError;
use crate::progress::{MigrationProgress, MigrationStage, ProgressHandle};
use crate::provenance::{ImportOptions, SourceProfile};
use crate::traits::{MigrationSummary, Migrator};

/// Minimum number of fields required per data line.
const MIN_FIELDS: usize = 5; // Date, Time, Sci_Name, Com_Name, Confidence

/// Maximum bytes accepted for a single CSV/TSV line.
///
/// `BufRead::lines()` buffers each line into a `String` with no cap, so a
/// hostile or corrupt input with a single multi-GB line (no `\n`) would OOM the
/// Pi — and the importer is reachable from an admin upload. 1 MiB is far above
/// any realistic BirdNET-Pi line (the longest fixture lines are < 200 B), so
/// this trips only on malformed input.
const MAX_LINE_BYTES: usize = 1024 * 1024;

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

impl CsvImporter {
    /// Import with the reconciliation the operator chose, as
    /// [`super::BirdNetPiImporter::migrate_with_options`] does for a database:
    /// an `import_batches` row (kind `birdnet-pi-csv`) that every imported row
    /// points at, the clock conversion applied to each timestamp, and the
    /// source's modal site recorded with the distance to this station.
    ///
    /// `run_migration_with_options` used to send a CSV to plain
    /// [`Migrator::migrate`] and drop all of that, so a `BirdDB.txt` from
    /// another site became indistinguishable from this station's recordings.
    ///
    /// # Errors
    ///
    /// As [`Migrator::migrate`], and when the batch row cannot be written.
    pub fn migrate_with_options(
        &self,
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
        options: &ImportOptions,
        station: (Option<f64>, Option<f64>),
    ) -> Result<MigrationSummary, MigrateError> {
        Self::run(source_path, dest_path, progress, Some((options, station)))
    }
}

impl Migrator for CsvImporter {
    fn migrate(
        &self,
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
    ) -> Result<MigrationSummary, MigrateError> {
        Self::run(source_path, dest_path, progress, None)
    }
}

/// This station's `(lat, lon)`, as `run_migration_with_options` passes it.
type Station = (Option<f64>, Option<f64>);

/// Rows at each coordinate, rounded to three decimals (~110 m) as
/// [`SourceProfile::from_connection`] rounds them.
type SiteCounts = std::collections::HashMap<(i64, i64), u64>;

impl CsvImporter {
    #[allow(clippy::too_many_lines)]
    fn run(
        source_path: &Path,
        dest_path: &Path,
        progress: &ProgressHandle,
        reconcile: Option<(&ImportOptions, Station)>,
    ) -> Result<MigrationSummary, MigrateError> {
        progress.set_stage(MigrationStage::Importing, "Opening CSV source file");

        let file = std::fs::File::open(source_path).map_err(MigrateError::Io)?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        // Read header line to detect delimiter.
        let header = match lines.next() {
            Some(Ok(h)) => h,
            Some(Err(e)) => return Err(MigrateError::Io(e)),
            None => return Err(MigrateError::CsvParse("file is empty".to_string())),
        };
        let layout = Layout::detect(&header);

        // Pre-scan to estimate total lines (for progress reporting).
        drop(lines);
        let total = count_lines(source_path)?.saturating_sub(usize::from(layout.has_header()));

        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 0,
            rows_total: total as u64,
            message: format!("Parsing {total} CSV lines"),
            error: None,
        });

        // Re-open for actual import. Read lines manually with a per-line byte
        // cap (`read_until` + length check) instead of `BufRead::lines()`, which
        // would buffer a hostile multi-GB single line into memory and OOM the
        // station before we ever see it.
        let file2 = std::fs::File::open(source_path).map_err(MigrateError::Io)?;
        let mut reader2 = BufReader::new(file2);
        // Skip the header, when there is one.
        if layout.has_header() {
            let mut hdr = Vec::new();
            let _ = read_capped_line(&mut reader2, &mut hdr)?;
        }

        // Open (or create) destination database and run schema migrations.
        let dest_conn = birdnet_db::sqlite::open_or_create(dest_path).map_err(|e| {
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

        let batch_id = match reconcile {
            Some((options, station)) => super::importer::record_import_batch(
                &dest_conn,
                "birdnet-pi-csv",
                source_path,
                // The site is not known until the rows are read; it is filled
                // in below, once they have been.
                &SourceProfile::default(),
                options,
                station,
            )?,
            None => None,
        };
        let mut sites = SiteCounts::new();

        let mut imported = 0u64;
        let mut skipped = 0u64;
        let mut unparseable = 0u64;
        let mut data_lines = 0u64;
        let mut first_error: Option<String> = None;
        let mut batch: Vec<CsvRow> = Vec::with_capacity(BATCH_SIZE);

        let mut buf: Vec<u8> = Vec::with_capacity(512);
        loop {
            buf.clear();
            let n = read_capped_line(&mut reader2, &mut buf)?;
            if n == 0 {
                break; // EOF
            }
            // Drop the trailing `\n` (and any `\r` from CRLF) before parsing.
            while matches!(buf.last(), Some(b'\n' | b'\r')) {
                buf.pop();
            }
            // CSV/TSV is text — reject non-UTF-8 rather than silently mangling.
            let Ok(line) = std::str::from_utf8(&buf) else {
                tracing::warn!("skipping non-UTF-8 CSV line");
                skipped += 1;
                continue;
            };
            // A header-less file's first *data* line carries the mark.
            let line = strip_bom(line);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            data_lines += 1;

            match parse_line(line, &layout) {
                Ok(mut row) => {
                    if let Some((options, _)) = reconcile {
                        reconcile_row(&dest_conn, &mut row, options, &mut sites);
                    }
                    batch.push(row);
                    if batch.len() >= BATCH_SIZE {
                        let (ins, sk) = flush_batch(&dest_conn, &batch, batch_id)?;
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
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                    }
                    skipped += 1;
                    unparseable += 1;
                }
            }
        }
        if data_lines > 0 && unparseable == data_lines {
            return Err(MigrateError::CsvParse(format!(
                "none of the {data_lines} lines could be read as a detection \
                 (expected {} columns: Date, Time, Sci_Name, Com_Name, Confidence, …); \
                 the first was refused because: {}",
                layout.describe(),
                first_error.as_deref().unwrap_or("(no reason recorded)")
            )));
        }

        // Flush remainder.
        if !batch.is_empty() {
            let (ins, sk) = flush_batch(&dest_conn, &batch, batch_id)?;
            imported += ins;
            skipped += sk;
        }

        if let (Some(id), Some((_, (station_lat, station_lon)))) = (batch_id, reconcile) {
            let profile = modal_site(&sites);
            dest_conn
                .execute(
                    "UPDATE import_batches
                        SET row_count = ?1, source_lat = ?2, source_lon = ?3, distance_km = ?4
                      WHERE id = ?5",
                    rusqlite::params![
                        i64::try_from(imported).unwrap_or(i64::MAX),
                        profile.modal_lat,
                        profile.modal_lon,
                        profile.distance_km_to(station_lat, station_lon),
                        id,
                    ],
                )
                .map_err(MigrateError::DataTransfer)?;
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

/// The twelve columns, in BirdNET-Pi's order, by the names a header uses.
const COLUMNS: [&str; 12] = [
    "date",
    "time",
    "sci_name",
    "com_name",
    "confidence",
    "lat",
    "lon",
    "cutoff",
    "week",
    "sens",
    "overlap",
    "file_name",
];

/// How a file's lines are laid out: the separator, and — when the first line
/// is a header — where each of [`COLUMNS`] sits.
pub(crate) struct Layout {
    delim: char,
    /// `positions[i]` is the field index of `COLUMNS[i]`; `None` when the file
    /// has no header, in which case the fields are in `COLUMNS` order.
    positions: Option<[Option<usize>; 12]>,
}

impl Layout {
    /// Read the layout from a file's first line.
    pub(crate) fn detect(first_line: &str) -> Self {
        // A file saved by Excel or Notepad starts with a UTF-8 byte-order
        // mark. Only the first field's `date` test used to strip it, so the
        // header was recognised and then its first column was named
        // "\u{feff}date" — no `Date` column, every row "missing date", and
        // the whole import refused.
        let first_line = strip_bom(first_line);
        // The separator that occurs most, among the three any of these files
        // uses. `BirdDB.txt` has eleven `;` and at most a comma or two in a
        // species name; a CSV has eleven or more `,`.
        let count = |c: char| first_line.matches(c).count();
        let delim = [';', '\t', ',']
            .into_iter()
            .max_by_key(|&c| (count(c), c == ';'))
            .unwrap_or(',');
        let fields = split_fields(first_line, delim);
        let is_header = fields
            .first()
            .is_some_and(|f| f.trim().eq_ignore_ascii_case("date"));
        let positions = is_header.then(|| {
            let names: Vec<String> = fields
                .iter()
                .map(|f| f.trim().to_ascii_lowercase())
                .collect();
            COLUMNS.map(|col| names.iter().position(|n| n == col))
        });
        Self { delim, positions }
    }

    /// Whether the first line is a header rather than a detection.
    pub(crate) const fn has_header(&self) -> bool {
        self.positions.is_some()
    }

    /// The field index of `COLUMNS[col]`.
    fn index(&self, col: usize) -> Option<usize> {
        self.positions.map_or(Some(col), |p| p[col])
    }

    fn describe(&self) -> String {
        let sep = match self.delim {
            ';' => "semicolon-separated",
            '\t' => "tab-separated",
            _ => "comma-separated",
        };
        format!(
            "{sep} {}",
            if self.has_header() {
                "with a header"
            } else {
                "without a header"
            }
        )
    }
}

/// Split one line on `delim`, honouring RFC 4180 quoting: a field that starts
/// with `"` runs to the matching `"`, and `""` inside it is one `"`.
fn split_fields(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;
    let mut at_start = true;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && at_start {
            quoted = true;
            at_start = false;
        } else if c == delim {
            out.push(std::mem::take(&mut field));
            at_start = true;
        } else {
            field.push(c);
            at_start = false;
        }
    }
    out.push(field);
    out
}

/// Undo the export's formula guard: `escape_csv` puts a `'` in front of text
/// that starts with `=`, `+`, `-`, `@`, a tab or a CR.
fn unguard(s: &str) -> &str {
    match s.strip_prefix('\'') {
        Some(rest) if rest.starts_with(['=', '+', '-', '@', '\t', '\r']) => rest,
        _ => s,
    }
}

/// Drop a leading UTF-8 byte-order mark.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Refuse a confidence that is not a probability.
///
/// Every reader of `Confidence` — thresholds, the quality dashboard, the
/// `0.7` cutoff comparisons — assumes `0.0..=1.0`. The CSV path stored what it
/// parsed: `85` from a sheet kept in percent sat above every threshold as a
/// certainty of 8 500 %; `inf` stored as-is; `NaN` binds as NULL, which the
/// `NOT NULL` column refused — aborting the import mid-file with the batches
/// before it already committed.
///
/// Refused rather than rescaled or clamped. Clamping (the database importer's
/// choice for a column that is *usually* right) would turn `85` into a
/// certain `1.0`, and dividing by 100 cannot be decided per row — `1` is a
/// valid probability and a valid percentage. A refused row is counted in
/// `skipped_rows`, and a file whose every row is refused fails with this
/// reason, so a percent-scale export is told why rather than imported wrong.
fn check_confidence(confidence: f64, raw: &str) -> Result<(), MigrateError> {
    if confidence.is_finite() && (0.0..=1.0).contains(&confidence) {
        return Ok(());
    }
    let hint = if confidence.is_finite() && confidence > 1.0 && confidence <= 100.0 {
        " — a percentage? confidence must be a fraction between 0 and 1"
    } else {
        " — confidence must be a number between 0 and 1"
    };
    Err(MigrateError::CsvParse(format!(
        "confidence out of range: '{raw}'{hint}"
    )))
}

/// Split `s` on `sep` into exactly `N` all-digit parts of 1..=`max_len` digits.
fn digit_parts<const N: usize>(s: &str, sep: char, max_len: usize) -> Option<[u32; N]> {
    let mut out = [0u32; N];
    let mut parts = s.split(sep);
    for slot in &mut out {
        let p = parts.next()?;
        if p.is_empty() || p.len() > max_len || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// `YYYY-M-D` → `YYYY-MM-DD`, or `None` if it names no calendar day.
///
/// `Date` is compared as text everywhere — `Date = ?`, `BETWEEN`, the unique
/// key — so `2026-4-1` was a different day from `2026-04-01`: it matched no
/// date filter, and a re-import of the same row in canonical form was not
/// recognised as a duplicate.
fn normalise_date(raw: &str) -> Option<String> {
    let [y, m, d] = digit_parts::<3>(raw, '-', 4)?;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => return None,
    };
    (y >= 1000 && (1..=days).contains(&d)).then(|| format!("{y:04}-{m:02}-{d:02}"))
}

/// `H:MM[:SS[.fff]]` → `HH:MM:SS`, or `None` if it names no time of day.
///
/// Fractional seconds are truncated, as the pipeline truncates them; a
/// missing seconds field is `:00`. `6:00:00` is otherwise a different string
/// from `06:00:00`, and sorted before `10:00:00` is the least of its problems:
/// `datetime()` returns NULL for it, so it had no `detected_at_utc`, no place
/// in any hour bucket, and no way to be shifted by the clock reconciliation.
fn normalise_time(raw: &str) -> Option<String> {
    let whole = raw.split_once('.').map_or(raw, |(w, frac)| {
        if !frac.is_empty() && frac.bytes().all(|b| b.is_ascii_digit()) {
            w
        } else {
            // Not a fraction: leave it to fail below.
            raw
        }
    });
    let (h, m, s) = match whole.matches(':').count() {
        1 => {
            let [h, m] = digit_parts::<2>(whole, ':', 2)?;
            (h, m, 0)
        }
        2 => digit_parts::<3>(whole, ':', 2)?.into(),
        _ => return None,
    };
    (h < 24 && m < 60 && s < 60).then(|| format!("{h:02}:{m:02}:{s:02}"))
}

/// Parse one data line into a `CsvRow`.
#[allow(clippy::similar_names)]
fn parse_line(line: &str, layout: &Layout) -> Result<CsvRow, MigrateError> {
    let fields = split_fields(line, layout.delim);
    if fields.len() < MIN_FIELDS {
        return Err(MigrateError::CsvParse(format!(
            "expected ≥{MIN_FIELDS} fields, got {}",
            fields.len()
        )));
    }
    let field = |col: usize| -> Option<&str> {
        layout
            .index(col)
            .and_then(|i| fields.get(i))
            .map(|f| unguard(f.trim()))
    };
    let required = |col: usize| -> Result<&str, MigrateError> {
        field(col)
            .filter(|f| !f.is_empty())
            .ok_or_else(|| MigrateError::CsvParse(format!("missing {}", COLUMNS[col])))
    };
    let absent = |s: &str| s.is_empty() || s == "\\N" || s == "NULL";
    let opt_f64 = |col: usize| {
        field(col)
            .filter(|s| !absent(s))
            .and_then(|s| s.parse().ok())
    };
    let opt_i64 = |col: usize| {
        field(col)
            .filter(|s| !absent(s))
            .and_then(|s| s.parse().ok())
    };

    let raw_confidence = required(4)?;
    let confidence: f64 = raw_confidence
        .parse()
        .map_err(|_| MigrateError::CsvParse(format!("invalid confidence: '{raw_confidence}'")))?;
    check_confidence(confidence, raw_confidence)?;
    let raw_date = required(0)?;
    let date = normalise_date(raw_date).ok_or_else(|| {
        MigrateError::CsvParse(format!("invalid date: '{raw_date}' (want YYYY-MM-DD)"))
    })?;
    let raw_time = required(1)?;
    let time = normalise_time(raw_time).ok_or_else(|| {
        MigrateError::CsvParse(format!("invalid time: '{raw_time}' (want HH:MM:SS)"))
    })?;

    Ok(CsvRow {
        date,
        time,
        sci_name: required(2)?.to_string(),
        com_name: required(3)?.to_string(),
        confidence,
        lat: opt_f64(5),
        lon: opt_f64(6),
        cutoff: opt_f64(7),
        week: opt_i64(8),
        sens: opt_f64(9),
        overlap: opt_f64(10),
        file_name: field(11).filter(|s| !absent(s)).map(str::to_string),
    })
}

/// Apply the operator's clock reconciliation to `row` and count its site.
fn reconcile_row(
    conn: &Connection,
    row: &mut CsvRow,
    options: &ImportOptions,
    sites: &mut SiteCounts,
) {
    if options.shifts_time() {
        // The source's offset wins when it is given, as in the database path.
        let (d, t) = options.source_utc_offset_secs.map_or_else(
            || super::importer::shift_timestamp(conn, &row.date, &row.time, options.shift_secs),
            |src| super::importer::to_local_here(conn, &row.date, &row.time, src),
        );
        row.date = d;
        row.time = t;
    }
    if let (Some(lat), Some(lon)) = (row.lat, row.lon)
        && !(lat == 0.0 && lon == 0.0)
    {
        #[allow(clippy::cast_possible_truncation)]
        let key = ((lat * 1000.0).round() as i64, (lon * 1000.0).round() as i64);
        *sites.entry(key).or_default() += 1;
    }
}

/// The commonest site among `sites`, as a profile that knows only that.
fn modal_site(sites: &SiteCounts) -> SourceProfile {
    let mut profile = SourceProfile::default();
    // Ties broken by the key, so the answer does not depend on hash order.
    if let Some((&(lat, lon), &n)) = sites.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))) {
        #[allow(clippy::cast_precision_loss)]
        {
            profile.modal_lat = Some(lat as f64 / 1000.0);
            profile.modal_lon = Some(lon as f64 / 1000.0);
        }
        profile.modal_rows = n;
    }
    profile
}

/// Insert a batch of rows into the destination, tagged with `batch_id`,
/// returning (inserted, skipped).
fn flush_batch(
    conn: &Connection,
    batch: &[CsvRow],
    batch_id: Option<i64>,
) -> Result<(u64, u64), MigrateError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(MigrateError::DataTransfer)?;
    let mut inserted = 0u64;
    let mut skipped = 0u64;

    // A row already held under any chunk offset is this row. The unique key
    // includes `chunk_offset_secs`, which no CSV carries — this station's own
    // export has no such column — so every row read here is offered with the
    // default 0.0. Against the key alone, re-importing an export duplicated
    // every detection that was not the first chunk of its recording: all of
    // them, on a station with several chunks a clip. `(Date, Time, Sci_Name,
    // File_Name)` names one detection since migration 24 put each chunk on
    // its own second, so matching on it regardless of the offset is exact.
    let mut held = tx
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM detections
                WHERE Date = ?1 AND Time = ?2 AND Sci_Name = ?3
                  AND COALESCE(File_Name, '') = COALESCE(?4, ''))",
        )
        .map_err(MigrateError::DataTransfer)?;

    for row in batch {
        let exists: bool = held
            .query_row(
                rusqlite::params![row.date, row.time, row.sci_name, row.file_name],
                |r| r.get(0),
            )
            .map_err(MigrateError::DataTransfer)?;
        if exists {
            skipped += 1;
            continue;
        }
        let rows_changed = tx
            .execute(
                "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name,
                  import_batch_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(Date, Time, Sci_Name, COALESCE(File_Name, ''), chunk_offset_secs) DO NOTHING",
                rusqlite::params![
                    row.date, row.time, row.sci_name, row.com_name,
                    row.confidence, row.lat, row.lon, row.cutoff,
                    row.week, row.sens, row.overlap, row.file_name, batch_id,
                ],
            )
            .map_err(MigrateError::DataTransfer)?;

        if rows_changed == 0 {
            skipped += 1;
        } else {
            inserted += 1;
        }
    }

    drop(held);
    tx.commit().map_err(MigrateError::DataTransfer)?;
    Ok((inserted, skipped))
}

/// Count non-empty lines in a file (including header).
fn count_lines(path: &Path) -> Result<usize, MigrateError> {
    let file = std::fs::File::open(path).map_err(MigrateError::Io)?;
    let mut reader = BufReader::new(file);
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    let mut count = 0;
    loop {
        buf.clear();
        let n = read_capped_line(&mut reader, &mut buf)?;
        if n == 0 {
            break;
        }
        // Trim trailing newline + whitespace before the empty check.
        while matches!(buf.last(), Some(b'\n' | b'\r' | b' ' | b'\t')) {
            buf.pop();
        }
        if !buf.is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

/// Read one line into `out`, capped at `MAX_LINE_BYTES` bytes (including the
/// trailing newline). Returns the number of bytes read (0 at EOF). A line
/// exceeding the cap returns `MigrateError::CsvParse` rather than swallowing
/// arbitrary memory.
fn read_capped_line<R: BufRead>(reader: &mut R, out: &mut Vec<u8>) -> Result<usize, MigrateError> {
    let mut total = 0;
    loop {
        let available = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(MigrateError::Io(e)),
        };
        if available.is_empty() {
            return Ok(total);
        }
        // Search for the line terminator. `read_until` would do the same, but
        // we need the byte budget check between buffer refills.
        let (chunk, done) = available
            .iter()
            .position(|&b| b == b'\n')
            .map_or((available, false), |i| (&available[..=i], true));
        if total.saturating_add(chunk.len()) > MAX_LINE_BYTES {
            return Err(MigrateError::CsvParse(format!(
                "line exceeds {MAX_LINE_BYTES}-byte cap (truncated/corrupt input?)"
            )));
        }
        out.extend_from_slice(chunk);
        let consumed = chunk.len();
        reader.consume(consumed);
        total += consumed;
        if done {
            return Ok(total);
        }
    }
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
    fn read_capped_line_reads_normal_line_with_newline() {
        use std::io::Cursor;
        let mut reader = Cursor::new(b"hello\nworld\n".as_slice());
        let mut buf = Vec::new();
        let n = read_capped_line(&mut reader, &mut buf).unwrap();
        assert_eq!(n, 6, "should consume 'hello\\n'");
        assert_eq!(&buf, b"hello\n");
    }

    #[test]
    fn read_capped_line_returns_zero_at_eof() {
        use std::io::Cursor;
        let mut reader = Cursor::new(b"".as_slice());
        let mut buf = Vec::new();
        let n = read_capped_line(&mut reader, &mut buf).unwrap();
        assert_eq!(n, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn read_capped_line_rejects_oversize_line() {
        // A pathological single line larger than the cap (no newline) must be
        // rejected with `CsvParse`, not buffered into memory. We send slightly
        // more than `MAX_LINE_BYTES` and assert the error variant.
        use std::io::Cursor;
        let hostile: Vec<u8> = vec![b'a'; MAX_LINE_BYTES + 64];
        let mut reader = Cursor::new(hostile);
        let mut buf = Vec::new();
        let err = read_capped_line(&mut reader, &mut buf).expect_err("should reject oversize line");
        assert!(
            matches!(err, MigrateError::CsvParse(_)),
            "expected CsvParse, got {err:?}"
        );
    }

    #[test]
    fn read_capped_line_handles_no_trailing_newline() {
        // Last line of file with no `\n` should still be returned.
        use std::io::Cursor;
        let mut reader = Cursor::new(b"final".as_slice());
        let mut buf = Vec::new();
        let n = read_capped_line(&mut reader, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"final");
    }

    #[test]
    fn parse_tab_separated_line() {
        let line = "2026-01-15\t06:23:11\tTurdus merula\tEurasian Blackbird\t0.921\t51.5\t-0.1\t0.7\t3\t1.0\t0.0\trec.wav";
        let row = parse_line(line, &Layout::detect(line)).unwrap();
        assert_eq!(row.date, "2026-01-15");
        assert_eq!(row.com_name, "Eurasian Blackbird");
        assert!((row.confidence - 0.921).abs() < 1e-6);
        assert_eq!(row.file_name.as_deref(), Some("rec.wav"));
    }

    #[test]
    fn parse_comma_separated_line() {
        let line = "2026-01-15,06:23:11,Turdus merula,Eurasian Blackbird,0.80,,,,,,,";
        let row = parse_line(line, &Layout::detect(line)).unwrap();
        assert_eq!(row.com_name, "Eurasian Blackbird");
        assert!(row.lat.is_none());
    }

    #[test]
    fn parse_line_too_few_fields() {
        let line = "2026-01-15\t06:23:11";
        let result = parse_line(line, &Layout::detect(line));
        assert!(result.is_err());
    }

    fn import(text: &str) -> Result<(MigrationSummary, NamedTempFile), MigrateError> {
        let src = make_csv(text);
        let dst = NamedTempFile::new().unwrap();
        let progress = crate::progress::ProgressHandle::new();
        CsvImporter
            .migrate(src.path(), dst.path(), &progress)
            .map(|s| (s, dst))
    }

    fn file_names(dst: &NamedTempFile) -> Vec<Option<String>> {
        let conn = rusqlite::Connection::open(dst.path()).unwrap();
        let mut stmt = conn
            .prepare("SELECT File_Name FROM detections ORDER BY Date, Time")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// BirdNET-Pi's `BirdDB.txt` — the file the migration page names — is
    /// semicolon-separated. This importer read it as a one-column CSV, skipped
    /// every line, and reported a successful import of 0 rows.
    #[test]
    fn birddb_txt_is_semicolon_separated_with_or_without_a_header() {
        let body = "2026-01-15;06:23:11;Turdus merula;Eurasian Blackbird;0.921;51.5;-0.1;0.7;3;1.0;0.0;rec.wav\n\
                    2026-01-16;07:00:00;Passer domesticus;House Sparrow;0.85;51.5;-0.1;0.7;3;1.0;0.0;b.wav\n";
        let (summary, dst) = import(body).expect("headerless BirdDB.txt");
        assert_eq!(
            summary.imported_rows, 2,
            "the first line is data, not a header"
        );
        assert_eq!(
            file_names(&dst),
            vec![Some("rec.wav".into()), Some("b.wav".into())]
        );

        let with_header = format!(
            "Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name\n{body}"
        );
        let (summary, _) = import(&with_header).expect("BirdDB.txt with a header");
        assert_eq!(summary.imported_rows, 2);
    }

    /// This station's own CSV export, read back: a header, five more columns
    /// after `File_Name`, RFC 4180 quoting and the formula guard. Read
    /// positionally, the extra columns ran into `File_Name`, so every row was a
    /// new duplicate pointing at a clip that did not exist.
    #[test]
    fn this_stations_own_export_reads_back_as_itself() {
        let export = "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name,\
Event_Date,Detected_At_UTC,Run_Id,Model_Name,Model_SHA256\n\
2026-04-01,03:30:00,Strix aluco,\"Owl, Tawny\",0.9100,51.48,-0.13,0.75,13,1.25,0,clip.wav,2026-04-01,2026-04-01T02:30:00Z,3,BirdNET,abc\n\
2026-04-01,03:31:00,Strix aluco,'-odd name,0.8000,,,,,,,,2026-04-01,,,,\n";
        let (summary, dst) = import(export).expect("own export");
        assert_eq!(summary.imported_rows, 2);
        assert_eq!(file_names(&dst), vec![Some("clip.wav".into()), None]);
        let conn = rusqlite::Connection::open(dst.path()).unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT Com_Name FROM detections ORDER BY Time")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            names,
            vec!["Owl, Tawny".to_string(), "-odd name".to_string()]
        );

        // Re-importing the same export adds nothing.
        let src = make_csv(export);
        let again = CsvImporter
            .migrate(
                src.path(),
                dst.path(),
                &crate::progress::ProgressHandle::new(),
            )
            .expect("re-import");
        assert_eq!(
            again.imported_rows, 0,
            "a re-import duplicated the station's history"
        );
    }

    /// A file none of whose lines is a detection is an error, not a success
    /// with 0 rows. (Counterpart to the above: a re-import that finds only
    /// duplicates is still a success.)
    #[test]
    fn a_file_with_no_readable_line_is_an_error() {
        assert!(import("this is not\na detection log\n").is_err());
    }

    #[test]
    fn csv_import_roundtrip() {
        let tsv = "Date\tTime\tSci_Name\tCom_Name\tConfidence\tLat\tLon\tCutoff\tWeek\tSens\tOverlap\tFile_Name\n\
                   2026-01-15\t06:23:11\tTurdus merula\tEurasian Blackbird\t0.921\t51.5\t-0.1\t0.7\t3\t1.0\t0.0\trec.wav\n\
                   2026-01-16\t07:00:00\tPasser domesticus\tHouse Sparrow\t0.85\t\t\t\t\t\t\t\n";

        let src = make_csv(tsv);
        let dst = NamedTempFile::new().unwrap();
        let progress = crate::progress::ProgressHandle::new();

        let summary = CsvImporter
            .migrate(src.path(), dst.path(), &progress)
            .unwrap();
        assert_eq!(summary.imported_rows, 2);
        assert_eq!(summary.schema_name, "BirdNET-Pi CSV");
    }

    /// Import `text` into an existing destination.
    fn import_into(text: &str, dst: &NamedTempFile) -> MigrationSummary {
        let src = make_csv(text);
        CsvImporter
            .migrate(
                src.path(),
                dst.path(),
                &crate::progress::ProgressHandle::new(),
            )
            .expect("import")
    }

    fn rows(dst: &NamedTempFile) -> Vec<(String, String, f64)> {
        let conn = rusqlite::Connection::open(dst.path()).unwrap();
        conn.prepare("SELECT Date, Time, Confidence FROM detections ORDER BY Date, Time")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    const HEADER: &str =
        "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name\n";

    /// Finding 2: this station's export carries no `chunk_offset_secs`, so a
    /// re-import offered every chunk after a clip's first at offset 0 — a
    /// different unique key — and duplicated it.
    #[test]
    fn re_importing_the_export_does_not_duplicate_later_chunks() {
        let dst = NamedTempFile::new().unwrap();
        {
            let conn = birdnet_db::sqlite::open_or_create(dst.path()).unwrap();
            for (time, offset) in [("06:00:00", 0.0), ("06:00:03", 3.0), ("06:00:06", 6.0)] {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, chunk_offset_secs)
                     VALUES ('2026-05-01', ?1, 'Turdus merula', 'Blackbird', 0.9, 'seg.wav', ?2)",
                    rusqlite::params![time, offset],
                )
                .unwrap();
            }
        }
        let export = format!(
            "{HEADER}2026-05-01,06:00:00,Turdus merula,Blackbird,0.9000,,,,,,,seg.wav\n\
             2026-05-01,06:00:03,Turdus merula,Blackbird,0.9000,,,,,,,seg.wav\n\
             2026-05-01,06:00:06,Turdus merula,Blackbird,0.9000,,,,,,,seg.wav\n"
        );
        let summary = import_into(&export, &dst);
        assert_eq!(summary.imported_rows, 0, "every row is already held");
        assert_eq!(summary.skipped_rows, 3);
        assert_eq!(rows(&dst).len(), 3);
    }

    /// Counterpart: a detection of the same second from another clip is a
    /// different detection, and still lands.
    #[test]
    fn a_row_from_another_clip_at_the_same_second_is_still_imported() {
        let dst = NamedTempFile::new().unwrap();
        import_into(
            &format!("{HEADER}2026-05-01,06:00:03,Turdus merula,Blackbird,0.9,,,,,,,cam1.wav\n"),
            &dst,
        );
        let summary = import_into(
            &format!("{HEADER}2026-05-01,06:00:03,Turdus merula,Blackbird,0.9,,,,,,,cam2.wav\n"),
            &dst,
        );
        assert_eq!(summary.imported_rows, 1);
    }

    /// Finding 3: a confidence that is not a probability is refused and
    /// counted — not stored as 85, not stored as infinity, and not allowed to
    /// abort the import half-way (NaN binds as NULL into a NOT NULL column).
    #[test]
    fn a_confidence_outside_zero_to_one_is_refused_and_counted() {
        let dst = NamedTempFile::new().unwrap();
        let summary = import_into(
            &format!(
                "{HEADER}2026-05-01,06:00:00,Turdus merula,Blackbird,0.9,,,,,,,a.wav\n\
                 2026-05-01,06:00:01,Turdus merula,Blackbird,85,,,,,,,b.wav\n\
                 2026-05-01,06:00:02,Turdus merula,Blackbird,NaN,,,,,,,c.wav\n\
                 2026-05-01,06:00:03,Turdus merula,Blackbird,inf,,,,,,,d.wav\n\
                 2026-05-01,06:00:04,Turdus merula,Blackbird,-0.1,,,,,,,e.wav\n\
                 2026-05-01,06:00:05,Turdus merula,Blackbird,1,,,,,,,f.wav\n"
            ),
            &dst,
        );
        assert_eq!(summary.imported_rows, 2, "0.9 and 1 are probabilities");
        assert_eq!(summary.skipped_rows, 4);
        assert!(rows(&dst).iter().all(|(_, _, c)| (0.0..=1.0).contains(c)));
    }

    /// A file kept in percent is refused whole, and says why.
    #[test]
    fn a_percent_scale_file_is_refused_with_the_reason() {
        let err = import(&format!(
            "{HEADER}2026-05-01,06:00:00,Turdus merula,Blackbird,85,,,,,,,a.wav\n"
        ))
        .map(|(s, _)| s)
        .expect_err("a percent file must not import");
        assert!(err.to_string().contains("a percentage?"), "{err}");
    }

    /// Finding 8: a byte-order mark in front of the header named the first
    /// column "\u{feff}date", so no row had a date and the import failed.
    #[test]
    fn a_byte_order_mark_does_not_hide_the_date_column() {
        let (summary, dst) = import(&format!(
            "\u{feff}{HEADER}2026-05-01,06:00:00,Turdus merula,Blackbird,0.9,,,,,,,a.wav\n"
        ))
        .expect("a BOM-prefixed export imports");
        assert_eq!(summary.imported_rows, 1);
        assert_eq!(rows(&dst)[0].0, "2026-05-01");

        // And on a header-less file it is not stored inside the first date.
        let (summary, dst) =
            import("\u{feff}2026-05-01;06:00:00;Turdus merula;Blackbird;0.9;;;;;;;a.wav\n")
                .expect("a BOM-prefixed BirdDB.txt imports");
        assert_eq!(summary.imported_rows, 1);
        assert_eq!(rows(&dst)[0].0, "2026-05-01");
    }

    /// Finding 9: `6:00:00`, `06:00`, `2026-5-1` are stored canonically, so
    /// they match date filters, sort, convert with `datetime()`, and collide
    /// with the same row written the canonical way.
    #[test]
    fn dates_and_times_are_stored_canonically() {
        let dst = NamedTempFile::new().unwrap();
        let summary = import_into(
            &format!(
                "{HEADER}2026-5-1,6:00:00,Turdus merula,Blackbird,0.9,,,,,,,a.wav\n\
                 2026-05-01,07:30,Turdus merula,Blackbird,0.9,,,,,,,b.wav\n\
                 2026-05-01,08:15:42.750,Turdus merula,Blackbird,0.9,,,,,,,c.wav\n"
            ),
            &dst,
        );
        assert_eq!(summary.imported_rows, 3);
        let got: Vec<(String, String)> = rows(&dst).into_iter().map(|(d, t, _)| (d, t)).collect();
        assert_eq!(
            got,
            vec![
                ("2026-05-01".into(), "06:00:00".into()),
                ("2026-05-01".into(), "07:30:00".into()),
                ("2026-05-01".into(), "08:15:42".into()),
            ]
        );
        // The canonical spelling of the first row is the same detection.
        let again = import_into(
            &format!("{HEADER}2026-05-01,06:00:00,Turdus merula,Blackbird,0.9,,,,,,,a.wav\n"),
            &dst,
        );
        assert_eq!(again.imported_rows, 0);
    }

    /// Counterpart: a date or time that names nothing is refused, not stored.
    #[test]
    fn a_date_or_time_that_names_nothing_is_refused() {
        for bad in [
            "2026-02-30,06:00:00",
            "2026-13-01,06:00:00",
            "01/05/2026,06:00:00",
            "2026-05-01,24:00:00",
            "2026-05-01,06:60:00",
            "2026-05-01,6",
            "2026-05-01,06:00:00pm",
        ] {
            let text = format!("{HEADER}{bad},Turdus merula,Blackbird,0.9,,,,,,,a.wav\n");
            assert!(import(&text).is_err(), "{bad} was accepted");
        }
        assert_eq!(normalise_date("2024-02-29").as_deref(), Some("2024-02-29"));
        assert_eq!(normalise_date("2100-02-29"), None);
    }
}
