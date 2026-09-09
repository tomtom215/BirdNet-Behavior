//! Registering an analysis run: the model this daemon start analyses with,
//! written to `analysis_runs` before the first detection is consumed (R-1).
//!
//! The processor writes the run's id on every row. It is handed the id, not
//! the paths, so a row cannot be inserted before the run exists — and a run
//! that cannot be registered stops the processor before it starts, which
//! stops the daemon (its event channel closes), which the strict health
//! verdict reports. A station that cannot say what model it is running does
//! not record detections; it says so instead.

use std::path::{Path, PathBuf};

use birdnet_core::inference::identity::{IdentityError, ModelIdentity};
use birdnet_core::inference::labels::{LabelError, LabelSet};
use birdnet_web::state::AppState;

/// Everything a run is registered with that is known before any file is read.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunManifest {
    /// The classifier.
    pub model_path: PathBuf,
    /// Its labels.
    pub labels_path: PathBuf,
    /// The occurrence-filter model, when one is configured.
    pub geomodel_path: Option<PathBuf>,
    /// The binary's version.
    pub app_version: &'static str,
    /// Global confidence floor.
    pub confidence: f64,
    /// Sigmoid sensitivity.
    pub sensitivity: f64,
    /// Window overlap, seconds.
    pub overlap: f64,
    /// Species-frequency threshold.
    pub sf_thresh: f64,
    /// Station latitude.
    pub lat: Option<f64>,
    /// Station longitude.
    pub lon: Option<f64>,
}

/// A run that exists in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RegisteredRun {
    /// `analysis_runs.id`.
    pub id: i64,
    /// What was hashed.
    pub identity: ModelIdentity,
    /// How many labels the labels file parsed to.
    pub label_count: usize,
}

/// Why a run could not be registered.
#[derive(Debug)]
pub(super) enum RunError {
    /// A classifier file could not be hashed.
    Hash(IdentityError),
    /// The labels file did not parse, so its count is unknown.
    Labels(PathBuf, LabelError),
    /// The row could not be written.
    Db(birdnet_db::sqlite::DbError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hash(e) => write!(f, "{e}"),
            Self::Labels(path, e) => write!(f, "cannot parse labels {}: {e}", path.display()),
            Self::Db(e) => write!(f, "cannot write analysis_runs: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Hash the classifier files named by `manifest` and write the run.
///
/// Reads the whole model file once (541 MB for the shipped FP32 model), so
/// this belongs on a blocking thread, which is where the processor already
/// runs.
///
/// # Errors
///
/// The first file that cannot be read or parsed, or the failed insert.
pub(super) fn register_run(
    state: &AppState,
    manifest: &RunManifest,
) -> Result<RegisteredRun, RunError> {
    let identity = ModelIdentity::resolve(
        &manifest.model_path,
        &manifest.labels_path,
        manifest.geomodel_path.as_deref(),
    )
    .map_err(RunError::Hash)?;
    let label_count = LabelSet::load(&manifest.labels_path)
        .map_err(|e| RunError::Labels(manifest.labels_path.clone(), e))?
        .len();
    let new = birdnet_db::sqlite::NewAnalysisRun {
        app_version: manifest.app_version,
        model_name: &identity.model_name,
        model_path: &path_text(&identity.model_path),
        model_sha256: &identity.model.sha256,
        model_bytes: i64::try_from(identity.model.bytes).unwrap_or(i64::MAX),
        labels_path: &path_text(&identity.labels_path),
        labels_sha256: &identity.labels.sha256,
        label_count: i64::try_from(label_count).unwrap_or(i64::MAX),
        geomodel_sha256: identity.geomodel.as_ref().map(|d| d.sha256.as_str()),
        confidence: manifest.confidence,
        sensitivity: manifest.sensitivity,
        overlap: manifest.overlap,
        sf_thresh: manifest.sf_thresh,
        lat: manifest.lat,
        lon: manifest.lon,
    };
    let id = state
        .with_db(|conn| birdnet_db::sqlite::insert_analysis_run(conn, &new))
        .map_err(RunError::Db)?;
    tracing::info!(
        run_id = id,
        model = %identity.model_name,
        model_sha256 = %identity.model.sha256,
        model_bytes = identity.model.bytes,
        labels_sha256 = %identity.labels.sha256,
        label_count,
        geomodel_sha256 = identity.geomodel.as_ref().map_or("none", |d| d.sha256.as_str()),
        "analysis run registered"
    );
    Ok(RegisteredRun {
        id,
        identity,
        label_count,
    })
}

/// Register the run and build what every row of it will say about itself.
///
/// `None` means the run could not be registered; the reason is logged at
/// error level here, and the caller must not start the processor — a row
/// without its model's identity is the defect `analysis_runs` exists to end.
pub(super) fn provenance_for(
    state: &AppState,
    manifest: &RunManifest,
) -> Option<super::processor::RunProvenance> {
    match register_run(state, manifest) {
        Ok(run) => Some(super::processor::RunProvenance {
            run_id: run.id,
            algorithm: birdnet_integrations::birdweather::algorithm_for_model(
                &run.identity.model_name,
            ),
            lat: manifest.lat,
            lon: manifest.lon,
            sensitivity: manifest.sensitivity,
            overlap: manifest.overlap,
        }),
        Err(e) => {
            tracing::error!(
                error = %e,
                "detection daemon stopped: the analysis run could not be registered, and no \
                 detection is recorded without the identity of the model that made it"
            );
            None
        }
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(dir: &Path) -> RunManifest {
        RunManifest {
            model_path: dir.join("BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"),
            labels_path: dir.join("labels.txt"),
            geomodel_path: None,
            app_version: "0.0.0-test",
            confidence: 0.7,
            sensitivity: 1.0,
            overlap: 0.0,
            sf_thresh: 0.03,
            lat: Some(51.48),
            lon: Some(-0.13),
        }
    }

    #[test]
    fn a_run_is_the_hash_of_the_files_on_disk_and_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest(tmp.path());
        std::fs::write(&m.model_path, b"abc").unwrap();
        std::fs::write(
            &m.labels_path,
            "Turdus merula_Eurasian Blackbird\nPica pica_Eurasian Magpie\n",
        )
        .unwrap();
        let state = AppState::new(tmp.path().join("birds.db")).unwrap();

        let run = register_run(&state, &m).unwrap();
        assert_eq!(run.label_count, 2);
        let row = state
            .with_db(|c| birdnet_db::sqlite::analysis_run(c, run.id))
            .unwrap()
            .unwrap();
        assert_eq!(
            row.model_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(row.model_bytes, 3);
        assert_eq!(row.model_name, "BirdNET+_V3.0-preview3_Global_11K_FP32");
        assert_eq!(row.label_count, 2);
        assert_eq!(row.app_version, "0.0.0-test");
        assert_eq!(
            (row.confidence, row.sensitivity, row.sf_thresh),
            (0.7, 1.0, 0.03)
        );
        assert_eq!((row.lat, row.lon), (Some(51.48), Some(-0.13)));
        assert!(row.geomodel_sha256.is_none());
        // The paths are the row's, verbatim: a re-analysis (R-5) finds the
        // model by them, so a placeholder there is a silent loss.
        assert_eq!(row.model_path, m.model_path.to_string_lossy());
        assert_eq!(row.labels_path, m.labels_path.to_string_lossy());
    }

    #[test]
    fn a_second_start_is_a_second_run_even_with_the_same_model() {
        // The row is per start, not per model: a restart with unchanged files
        // is a boundary a researcher can see (a settings change, a binary
        // upgrade), and the checksum says whether the model moved.
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest(tmp.path());
        std::fs::write(&m.model_path, b"abc").unwrap();
        std::fs::write(&m.labels_path, "Pica pica_Eurasian Magpie\n").unwrap();
        let state = AppState::new(tmp.path().join("birds.db")).unwrap();
        let a = register_run(&state, &m).unwrap();
        let b = register_run(&state, &m).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.identity, b.identity);
    }

    #[test]
    fn the_provenance_is_the_runs_id_and_the_manifests_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest(tmp.path());
        std::fs::write(&m.model_path, b"abc").unwrap();
        std::fs::write(&m.labels_path, "Pica pica_Eurasian Magpie\n").unwrap();
        let state = AppState::new(tmp.path().join("birds.db")).unwrap();
        let p = provenance_for(&state, &m).expect("registered");
        let latest = state
            .with_db(birdnet_db::sqlite::latest_analysis_run)
            .unwrap()
            .unwrap();
        assert_eq!(p.run_id, latest.id);
        assert_eq!(
            (p.lat, p.lon, p.sensitivity, p.overlap),
            (Some(51.48), Some(-0.13), 1.0, 0.0)
        );
        std::fs::remove_file(&m.model_path).unwrap();
        assert!(provenance_for(&state, &m).is_none());
    }

    #[test]
    fn a_missing_model_refuses_to_register() {
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest(tmp.path());
        std::fs::write(&m.labels_path, "Pica pica_Eurasian Magpie\n").unwrap();
        let state = AppState::new(tmp.path().join("birds.db")).unwrap();
        let err = register_run(&state, &m).unwrap_err();
        assert!(matches!(err, RunError::Hash(_)), "{err}");
        assert!(err.to_string().contains("FP32.onnx"), "{err}");
        assert!(
            state
                .with_db(birdnet_db::sqlite::latest_analysis_run)
                .unwrap()
                .is_none(),
            "no run may be written for a model that could not be hashed"
        );
    }

    #[test]
    fn an_unparseable_labels_file_refuses_to_register() {
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest(tmp.path());
        std::fs::write(&m.model_path, b"abc").unwrap();
        std::fs::write(&m.labels_path, "").unwrap();
        let state = AppState::new(tmp.path().join("birds.db")).unwrap();
        let err = register_run(&state, &m).unwrap_err();
        assert!(matches!(err, RunError::Labels(..)), "{err}");
    }
}
