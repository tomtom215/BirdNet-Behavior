//! `analysis_runs`: one row per detection-daemon start, recording the model
//! the run analysed with (migration 43, R-1).
//!
//! A detection row's `run_id` points here. The checksum columns are the
//! identity of the classifier files as bytes; everything else is the run-wide
//! configuration that a per-row column does not already carry.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::sqlite::connection::DbError;

/// What a run is registered with.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAnalysisRun<'a> {
    /// The binary's version (`CARGO_PKG_VERSION`): the pipeline around the
    /// model changes too, and a resampler or spectrogram change is a run
    /// boundary a researcher needs to see.
    pub app_version: &'a str,
    /// Operator-facing name of the model, the file's stem.
    pub model_name: &'a str,
    /// Where the model was read from.
    pub model_path: &'a str,
    /// Lowercase hex SHA-256 of the model file.
    pub model_sha256: &'a str,
    /// The model file's length in bytes.
    pub model_bytes: i64,
    /// Where the labels were read from.
    pub labels_path: &'a str,
    /// Lowercase hex SHA-256 of the labels file.
    pub labels_sha256: &'a str,
    /// How many labels the file parsed to — the model's output width.
    pub label_count: i64,
    /// SHA-256 of the occurrence-filter model, when one is configured.
    pub geomodel_sha256: Option<&'a str>,
    /// The global confidence floor the run admits at.
    pub confidence: f64,
    /// BirdNET sigmoid sensitivity.
    pub sensitivity: f64,
    /// Analysis-window overlap, seconds.
    pub overlap: f64,
    /// Species-frequency (occurrence) threshold.
    pub sf_thresh: f64,
    /// Station latitude, `None` when the station has none.
    pub lat: Option<f64>,
    /// Station longitude, `None` when the station has none.
    pub lon: Option<f64>,
}

/// A registered run, read back.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnalysisRun {
    /// Primary key; what `detections.run_id` holds.
    pub id: i64,
    /// When the run started, UTC `YYYY-MM-DD HH:MM:SS`.
    pub started_at: String,
    /// The binary's version.
    pub app_version: String,
    /// Operator-facing name of the model.
    pub model_name: String,
    /// Where the model was read from.
    pub model_path: String,
    /// Lowercase hex SHA-256 of the model file.
    pub model_sha256: String,
    /// The model file's length in bytes.
    pub model_bytes: i64,
    /// Where the labels were read from.
    pub labels_path: String,
    /// Lowercase hex SHA-256 of the labels file.
    pub labels_sha256: String,
    /// How many labels the file parsed to.
    pub label_count: i64,
    /// SHA-256 of the occurrence-filter model, when one was configured.
    pub geomodel_sha256: Option<String>,
    /// The global confidence floor.
    pub confidence: f64,
    /// BirdNET sigmoid sensitivity.
    pub sensitivity: f64,
    /// Analysis-window overlap, seconds.
    pub overlap: f64,
    /// Species-frequency threshold.
    pub sf_thresh: f64,
    /// Station latitude.
    pub lat: Option<f64>,
    /// Station longitude.
    pub lon: Option<f64>,
}

/// The two model fields an export attaches to every row: the label and the
/// identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunModel {
    /// Operator-facing name of the model.
    pub model_name: String,
    /// Lowercase hex SHA-256 of the model file.
    pub model_sha256: String,
}

const RUN_COLS: &str = "id, started_at, app_version, model_name, model_path, model_sha256, \
                        model_bytes, labels_path, labels_sha256, label_count, geomodel_sha256, \
                        confidence, sensitivity, overlap, sf_thresh, lat, lon";

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnalysisRun> {
    Ok(AnalysisRun {
        id: row.get(0)?,
        started_at: row.get(1)?,
        app_version: row.get(2)?,
        model_name: row.get(3)?,
        model_path: row.get(4)?,
        model_sha256: row.get(5)?,
        model_bytes: row.get(6)?,
        labels_path: row.get(7)?,
        labels_sha256: row.get(8)?,
        label_count: row.get(9)?,
        geomodel_sha256: row.get(10)?,
        confidence: row.get(11)?,
        sensitivity: row.get(12)?,
        overlap: row.get(13)?,
        sf_thresh: row.get(14)?,
        lat: row.get(15)?,
        lon: row.get(16)?,
    })
}

/// Register a run and return its id.
///
/// # Errors
///
/// Returns `DbError` when the insert fails. The caller must not start
/// consuming detections without an id: a row with no run is the defect this
/// table exists to end.
pub fn insert_analysis_run(conn: &Connection, run: &NewAnalysisRun<'_>) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO analysis_runs \
         (app_version, model_name, model_path, model_sha256, model_bytes, labels_path, \
          labels_sha256, label_count, geomodel_sha256, confidence, sensitivity, overlap, \
          sf_thresh, lat, lon) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            run.app_version,
            run.model_name,
            run.model_path,
            run.model_sha256,
            run.model_bytes,
            run.labels_path,
            run.labels_sha256,
            run.label_count,
            run.geomodel_sha256,
            run.confidence,
            run.sensitivity,
            run.overlap,
            run.sf_thresh,
            run.lat,
            run.lon,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// One run by id.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn analysis_run(conn: &Connection, id: i64) -> Result<Option<AnalysisRun>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {RUN_COLS} FROM analysis_runs WHERE id = ?1"),
            params![id],
            map_run,
        )
        .optional()?)
}

/// The most recently registered run — the one the daemon is writing under if
/// it is running.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn latest_analysis_run(conn: &Connection) -> Result<Option<AnalysisRun>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {RUN_COLS} FROM analysis_runs ORDER BY id DESC LIMIT 1"),
            [],
            map_run,
        )
        .optional()?)
}

/// The most recent `limit` runs, newest first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn list_analysis_runs(conn: &Connection, limit: u32) -> Result<Vec<AnalysisRun>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM analysis_runs ORDER BY id DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map(params![limit], map_run)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Every run's model label and identity, keyed by run id — what an export
/// joins onto its rows in memory. One row per daemon start, so this is a few
/// hundred rows on a station that has run for years.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn run_models(conn: &Connection) -> Result<HashMap<i64, RunModel>, DbError> {
    let mut stmt = conn.prepare("SELECT id, model_name, model_sha256 FROM analysis_runs")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            RunModel {
                model_name: row.get(1)?,
                model_sha256: row.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

/// How many detection rows carry each run, for the runs that have any.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_counts_by_run(conn: &Connection) -> Result<HashMap<i64, i64>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT run_id, COUNT(*) FROM detections WHERE run_id IS NOT NULL GROUP BY run_id",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
    Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
}

#[cfg(test)]
pub(crate) const fn fixture_run<'a>(
    model_sha256: &'a str,
    model_name: &'a str,
) -> NewAnalysisRun<'a> {
    NewAnalysisRun {
        app_version: "0.0.0-test",
        model_name,
        model_path: "/models/test.onnx",
        model_sha256,
        model_bytes: 4096,
        labels_path: "/models/test.csv",
        labels_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
        label_count: 3,
        geomodel_sha256: None,
        confidence: 0.7,
        sensitivity: 1.0,
        overlap: 0.0,
        sf_thresh: 0.03,
        lat: Some(51.5),
        lon: Some(-0.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::connection::open_or_create;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn a_registered_run_reads_back_whole() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let mut new = fixture_run(SHA_A, "model-a");
        new.geomodel_sha256 = Some(SHA_B);
        let id = insert_analysis_run(&conn, &new).unwrap();
        let run = analysis_run(&conn, id).unwrap().unwrap();
        assert_eq!(run.id, id);
        assert_eq!(run.model_sha256, SHA_A);
        assert_eq!(run.model_name, "model-a");
        assert_eq!(run.geomodel_sha256.as_deref(), Some(SHA_B));
        assert_eq!(run.label_count, 3);
        assert_eq!(run.lat, Some(51.5));
        assert_eq!(
            run.started_at.len(),
            "YYYY-MM-DD HH:MM:SS".len(),
            "{}",
            run.started_at
        );
        assert_eq!(latest_analysis_run(&conn).unwrap().unwrap().id, id);
        assert!(analysis_run(&conn, id + 1).unwrap().is_none());
    }

    #[test]
    fn runs_list_newest_first_and_models_key_by_id() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let a = insert_analysis_run(&conn, &fixture_run(SHA_A, "a")).unwrap();
        let b = insert_analysis_run(&conn, &fixture_run(SHA_B, "b")).unwrap();
        let listed = list_analysis_runs(&conn, 10).unwrap();
        assert_eq!(listed.iter().map(|r| r.id).collect::<Vec<_>>(), vec![b, a]);
        assert_eq!(list_analysis_runs(&conn, 1).unwrap().len(), 1);
        let models = run_models(&conn).unwrap();
        assert_eq!(models[&a].model_sha256, SHA_A);
        assert_eq!(models[&b].model_name, "b");
    }

    #[test]
    fn an_empty_station_has_no_latest_run() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        assert!(latest_analysis_run(&conn).unwrap().is_none());
        assert!(run_models(&conn).unwrap().is_empty());
        assert!(detection_counts_by_run(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_detection_cannot_claim_a_run_that_does_not_exist() {
        // The foreign key is the whole guarantee: with it off, a stale or
        // mistyped id would silently file rows under nothing.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_or_create(tmp.path()).unwrap();
        let err = conn
            .execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, run_id) \
                 VALUES ('2026-05-01', '06:00:00', 'Pica pica', 'Magpie', 0.9, 'x.wav', 999)",
                [],
            )
            .unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    }
}

/// One classifier of a run, as [`insert_run_classifier`] records it
/// (migration 67).
#[derive(Debug, Clone, Copy)]
pub struct NewRunClassifier<'a> {
    /// The `analysis_runs` row.
    pub run_id: i64,
    /// The id the classifier's rows carry in `model_id`.
    pub model_id: &'a str,
    /// Operator-facing name of its model.
    pub model_name: &'a str,
    /// Lowercase hex SHA-256 of the model file it loaded.
    pub model_sha256: &'a str,
    /// Lowercase hex SHA-256 of its labels file.
    pub labels_sha256: &'a str,
    /// Whether it is the run's primary.
    pub is_primary: bool,
}

/// Record one classifier of a run.
///
/// # Errors
///
/// Returns `DbError` on insert failure, including a second record of the same
/// classifier for the same run.
pub fn insert_run_classifier(conn: &Connection, new: &NewRunClassifier<'_>) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO analysis_run_models
             (run_id, model_id, model_name, model_sha256, labels_sha256, is_primary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            new.run_id,
            new.model_id,
            new.model_name,
            new.model_sha256,
            new.labels_sha256,
            new.is_primary
        ],
    )?;
    Ok(())
}

/// Which model file made a detection row, as far as the station recorded it
/// (review F19, MMP-08).
///
/// A run registered since migration 67 recorded each of its classifiers, and
/// a row resolves through its `(run_id, model_id)`: the file of the classifier
/// that answered, or nothing for a classifier the run did not record. A run
/// from before recorded only its primary; its rows keep that file when they
/// all name one classifier (or none), which is a run where one classifier
/// answered, and get nothing when they name more than one, because which
/// file made which row was never written down.
#[derive(Debug, Clone, Default)]
pub struct ModelAttribution {
    runs: HashMap<i64, RunModel>,
    per_classifier: HashMap<(i64, String), RunModel>,
    recorded: std::collections::HashSet<i64>,
    ambiguous: std::collections::HashSet<i64>,
}

impl ModelAttribution {
    /// Attribution from the run table alone, every run treated as one from
    /// before migration 67 with one classifier answering: what exports did
    /// before it, for a caller that has nothing else.
    #[must_use]
    pub fn from_runs(runs: HashMap<i64, RunModel>) -> Self {
        Self {
            runs,
            ..Self::default()
        }
    }

    /// The model file that made a row of `run_id` answered by `model_id`;
    /// `None` when the station did not record it.
    #[must_use]
    pub fn for_row(&self, run_id: Option<i64>, model_id: Option<&str>) -> Option<&RunModel> {
        let run = run_id?;
        if self.recorded.contains(&run) {
            return self.per_classifier.get(&(run, model_id?.to_owned()));
        }
        if self.ambiguous.contains(&run) {
            return None;
        }
        self.runs.get(&run)
    }
}

/// [`ModelAttribution`] for this station.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn model_attribution(conn: &Connection) -> Result<ModelAttribution, DbError> {
    let runs = run_models(conn)?;
    let mut per_classifier = HashMap::new();
    let mut recorded = std::collections::HashSet::new();
    let mut stmt =
        conn.prepare("SELECT run_id, model_id, model_name, model_sha256 FROM analysis_run_models")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            RunModel {
                model_name: row.get(2)?,
                model_sha256: row.get(3)?,
            },
        ))
    })?;
    for row in rows {
        let (run, id, model) = row?;
        recorded.insert(run);
        per_classifier.insert((run, id), model);
    }
    let mut stmt = conn.prepare(
        "SELECT run_id FROM detections WHERE run_id IS NOT NULL AND model_id IS NOT NULL \
         GROUP BY run_id HAVING COUNT(DISTINCT model_id) > 1",
    )?;
    let ambiguous = stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<_, _>>()?;
    Ok(ModelAttribution {
        runs,
        per_classifier,
        recorded,
        ambiguous,
    })
}
