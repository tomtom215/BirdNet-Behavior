//! Model checks: ONNX model file presence/size, labels file presence, and the
//! model's identity against the last analysis run's.

use std::path::{Path, PathBuf};

use birdnet_core::config::Config;

use super::Check;
use crate::cli::Cli;
use crate::helpers::db_path_from_config;

pub(super) fn check_model(cli: &Cli, config: Option<&Config>) -> Vec<Check> {
    // MODEL_PATH / LABELS_PATH are the keys the daemon resolves
    // (`daemon::config::resolve_required_paths`) and the installer writes.
    // The doctor must read the same keys, or a standard config-file install
    // reports SKIP and the model file is never actually validated.
    let model_path = cli
        .model
        .clone()
        .or_else(|| config?.get("MODEL_PATH").map(PathBuf::from));
    let labels_path = cli
        .labels
        .clone()
        .or_else(|| config?.get("LABELS_PATH").map(PathBuf::from));
    let mut out = Vec::new();

    if let Some(p) = model_path {
        if p.exists() {
            match std::fs::metadata(&p) {
                Ok(m) if m.len() > 1_000_000 => out.push(Check::pass(
                    "ONNX model file",
                    format!("{} ({} bytes)", p.display(), m.len()),
                )),
                Ok(m) => out.push(Check::warn(
                    "ONNX model file",
                    format!(
                        "{} is only {} bytes — likely truncated or empty",
                        p.display(),
                        m.len()
                    ),
                    "re-download the model (delete it; the entrypoint will fetch it again)",
                )),
                Err(e) => out.push(Check::fail(
                    "ONNX model file",
                    format!("{} could not be inspected: {e}", p.display()),
                    "check filesystem health and permissions",
                )),
            }
            out.push(check_identity(&p, &db_path_from_config(config)));
            out.push(check_model_loads(&p, labels_path.as_deref()));
        } else {
            out.push(Check::fail(
                "ONNX model file",
                format!("{} does not exist", p.display()),
                "either let the entrypoint download it (Docker), or run `install.sh` again",
            ));
        }
    } else {
        out.push(Check::skip(
            "ONNX model file",
            "no --model / MODEL_PATH configured (will use the bundled default at startup)",
        ));
    }

    if let Some(p) = labels_path {
        if p.exists() {
            out.push(Check::pass(
                "Labels file",
                format!("{} exists", p.display()),
            ));
        } else {
            out.push(Check::fail(
                "Labels file",
                format!("{} does not exist", p.display()),
                "the labels file ships alongside the model; re-run `install.sh`",
            ));
        }
    }

    out
}

/// The model's identity (R-1): its SHA-256 on disk, against the SHA-256 the
/// most recent analysis run registered. A model swapped since the last start
/// is not a fault — it is the reason `analysis_runs` exists — but it is the
/// one thing an operator about to restart should be told, because every row
/// from then on is keyed to the new run.
///
/// Reads the whole model file; the shipped FP32 model is 541 MB, which is
/// seconds on a Pi 4. The doctor is a diagnostic run by hand, not a poll.
/// Load the model with ONNX Runtime and compare its class width with the
/// labels file (ON-9).
///
/// The size check above is what the doctor used to stop at: a 3 MB stand-in
/// for a 541 MB model, a download cut off past the first megabyte, or a
/// labels file from another model version all passed it. Loading is what
/// tells a file from a model, and the output width against the label count
/// is what tells a matched pair from a mispaired one — species are assigned
/// positionally, so a mispaired station names every bird wrong and looks,
/// in its logs, like a classifier having a bad day.
fn check_model_loads(model: &Path, labels: Option<&Path>) -> Check {
    const NAME: &str = "Model integrity";
    let label_set = match labels {
        Some(path) if path.exists() => {
            match birdnet_core::inference::labels::LabelSet::load(path) {
                Ok(set) => Some(set),
                Err(e) => {
                    return Check::fail(
                        NAME,
                        format!("{} could not be read as a labels file: {e}", path.display()),
                        "the labels file ships alongside the model; re-run `install.sh`",
                    );
                }
            }
        }
        _ => None,
    };
    let label_count = label_set
        .as_ref()
        .map(birdnet_core::inference::labels::LabelSet::len);
    let loaded = birdnet_core::inference::model::BirdNetModel::load(
        model,
        label_set
            .unwrap_or_else(|| birdnet_core::inference::labels::LabelSet::from_entries(Vec::new())),
        birdnet_core::inference::model::ModelConfig {
            num_threads: 1,
            ..birdnet_core::inference::model::ModelConfig::default()
        },
    );
    let loaded = match loaded {
        Ok(m) => m,
        Err(e) => {
            return Check::fail(
                NAME,
                format!(
                    "{} is not a model ONNX Runtime can load ({e}); a truncated or corrupt \
                     download passes the size check and fails here",
                    model.display()
                ),
                "delete the file and let the entrypoint or `install.sh` download it again",
            );
        }
    };
    match (loaded.output_dimension(), label_count) {
        (Some(width), Some(labels)) if width != labels => Check::fail(
            NAME,
            format!(
                "the model scores {width} classes and the labels file names {labels}; species \
                 are assigned by position, so every detection would carry the wrong name"
            ),
            "install the labels file that shipped with this model, or the model that \
             matches these labels",
        ),
        (Some(width), Some(_)) => Check::pass(
            NAME,
            format!("loads; {width} classes, and the labels file names {width}"),
        ),
        (Some(width), None) => Check::pass(
            NAME,
            format!("loads; {width} classes (no labels file configured to check against)"),
        ),
        (None, _) => Check::warn(
            NAME,
            "loads, but declares a dynamic class width, so the label count cannot be checked \
             before the first inference",
            "nothing, unless detections carry unexpected names; the daemon warns once at \
             the first inference if the widths disagree",
        ),
    }
}

fn check_identity(model: &Path, db_path: &Path) -> Check {
    let digest = match birdnet_core::inference::identity::file_digest(model) {
        Ok(d) => d,
        Err(e) => {
            return Check::fail(
                "Model identity",
                format!("{} could not be hashed: {e}", model.display()),
                "check filesystem health and permissions",
            );
        }
    };
    let name = birdnet_core::inference::identity::model_name_of(model);
    let short = &digest.sha256[..12];
    let last = birdnet_db::sqlite::open_readonly(db_path)
        .ok()
        .and_then(|conn| {
            birdnet_db::sqlite::latest_analysis_run(&conn)
                .ok()
                .flatten()
        });
    match last {
        Some(run) if run.model_sha256 != digest.sha256 => Check::warn(
            "Model identity",
            format!(
                "{name} sha256 {short}… on disk; the last run (#{}, {}) analysed with {} sha256 {}… \
                 — the model has changed since the station last started",
                run.id,
                run.started_at,
                run.model_name,
                &run.model_sha256[..12.min(run.model_sha256.len())],
            ),
            "if the swap was intended, nothing: the next start registers a new run and every \
             row says which model made it. If it was not, restore the previous model file",
        ),
        Some(run) => Check::pass(
            "Model identity",
            format!(
                "{name} sha256 {short}… ({} bytes), the model of the last run (#{})",
                digest.bytes, run.id
            ),
        ),
        None => Check::pass(
            "Model identity",
            format!(
                "{name} sha256 {short}… ({} bytes); no analysis run registered yet",
                digest.bytes
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;
    use clap::Parser;

    fn identity_of(checks: &[Check]) -> &Check {
        checks
            .iter()
            .find(|c| c.name == "Model identity")
            .expect("a present model has an identity check")
    }

    /// R-1: a model swapped since the last registered run is reported, and
    /// an unchanged one is not.
    #[test]
    fn the_doctor_notices_a_model_that_is_not_the_last_runs() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, b"abc").unwrap();
        let db = dir.path().join("birds.db");
        let conn = birdnet_db::sqlite::open_or_create(&db).unwrap();
        let run = fixture_run("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        birdnet_db::sqlite::insert_analysis_run(&conn, &run).unwrap();
        drop(conn);

        let unchanged = check_identity(&model, &db);
        assert_eq!(unchanged.status, Status::Pass, "{unchanged:?}");
        assert!(unchanged.message.contains("ba7816bf8f01"), "{unchanged:?}");

        std::fs::write(&model, b"abd").unwrap();
        let swapped = check_identity(&model, &db);
        assert_eq!(swapped.status, Status::Warn, "{swapped:?}");
        assert!(swapped.message.contains("has changed"), "{swapped:?}");

        // A later run with the new model makes it the last run's model again.
        let conn = birdnet_db::sqlite::open_or_create(&db).unwrap();
        let new_sha = birdnet_core::inference::identity::file_digest(&model)
            .unwrap()
            .sha256;
        birdnet_db::sqlite::insert_analysis_run(&conn, &fixture_run(&new_sha)).unwrap();
        drop(conn);
        assert_eq!(check_identity(&model, &db).status, Status::Pass);
    }

    #[test]
    fn a_station_that_has_never_run_still_gets_its_models_hash() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, b"abc").unwrap();
        let check = check_identity(&model, &dir.path().join("absent.db"));
        assert_eq!(check.status, Status::Pass, "{check:?}");
        assert!(check.message.contains("no analysis run"), "{check:?}");
        assert!(check.message.contains("ba7816bf8f01"), "{check:?}");
    }

    #[test]
    fn the_identity_check_is_part_of_the_model_checks() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, vec![0u8; 1_000_001]).unwrap();
        let mut cli = cli();
        cli.model = Some(model);
        let checks = check_model(&cli, None);
        assert_eq!(identity_of(&checks).status, Status::Pass);
    }

    const TINY_V30_MODEL: &[u8] =
        include_bytes!("../../crates/birdnet-core/src/testdata/tiny_v30_test.onnx");

    /// A labels file naming `n` species, in the V2.4 text form.
    fn labels_file(dir: &Path, n: usize) -> PathBuf {
        let path = dir.join("labels.txt");
        let body: Vec<String> = (0..n).map(|i| format!("Species_{i}_Bird {i}")).collect();
        std::fs::write(&path, body.join("\n")).unwrap();
        path
    }

    /// The gate for ON-9: a file that is not a model fails, a model whose
    /// class width is not the label count fails naming both, and a matched
    /// pair passes.
    #[test]
    fn the_doctor_loads_the_model_and_checks_its_width_against_the_labels() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, TINY_V30_MODEL).unwrap();

        let matched = check_model_loads(&model, Some(&labels_file(dir.path(), 11)));
        assert_eq!(matched.status, Status::Pass, "{matched:?}");
        assert!(matched.message.contains("11 classes"), "{matched:?}");

        let mispaired = check_model_loads(&model, Some(&labels_file(dir.path(), 12)));
        assert_eq!(mispaired.status, Status::Fail, "{mispaired:?}");
        assert!(
            mispaired.message.contains("scores 11 classes")
                && mispaired.message.contains("names 12"),
            "both counts must be named: {mispaired:?}"
        );

        // The 3 MB stand-in the row describes: past the size check, not a model.
        let stand_in = dir.path().join("stand-in.onnx");
        std::fs::write(&stand_in, vec![0u8; 3_000_000]).unwrap();
        let garbage = check_model_loads(&stand_in, Some(&labels_file(dir.path(), 11)));
        assert_eq!(garbage.status, Status::Fail, "{garbage:?}");
        assert!(garbage.message.contains("not a model"), "{garbage:?}");

        let unchecked = check_model_loads(&model, None);
        assert_eq!(unchecked.status, Status::Pass, "{unchecked:?}");
        assert!(
            unchecked.message.contains("no labels file"),
            "{unchecked:?}"
        );
    }

    fn fixture_run(model_sha256: &str) -> birdnet_db::sqlite::NewAnalysisRun<'_> {
        birdnet_db::sqlite::NewAnalysisRun {
            app_version: "0.0.0-test",
            model_name: "model",
            model_path: "/models/model.onnx",
            model_sha256,
            model_bytes: 3,
            labels_path: "/models/labels.csv",
            labels_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
            label_count: 1,
            geomodel_sha256: None,
            confidence: 0.7,
            sensitivity: 1.0,
            overlap: 0.0,
            sf_thresh: 0.03,
            lat: None,
            lon: None,
        }
    }

    fn cli() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    #[test]
    fn skip_when_unconfigured() {
        let checks = check_model(&cli(), None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Skip);
    }

    /// A config-file-driven install (the standard systemd setup) must have
    /// its model and labels validated through the same `MODEL_PATH` /
    /// `LABELS_PATH` keys the daemon resolves — not skipped.
    #[test]
    fn resolves_paths_from_config_file_keys() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, vec![0u8; 1_000_001]).unwrap();
        let labels = dir.path().join("labels.csv");
        std::fs::write(&labels, "Pica pica_Eurasian Magpie").unwrap();

        let cfg = birdnet_core::config::Config::parse(&format!(
            "MODEL_PATH={}\nLABELS_PATH={}",
            model.display(),
            labels.display()
        ))
        .unwrap();

        let checks = check_model(&cli(), Some(&cfg));
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("ONNX model") && c.status == Status::Pass),
            "model check should PASS via MODEL_PATH, got: {checks:?}"
        );
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Labels") && c.status == Status::Pass),
            "labels check should PASS via LABELS_PATH, got: {checks:?}"
        );
    }

    #[test]
    fn pass_for_large_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, vec![0u8; 1_000_001]).unwrap();
        let mut cli = cli();
        cli.model = Some(model);
        let checks = check_model(&cli, None);
        assert_eq!(checks[0].status, Status::Pass);
        assert!(checks[0].name.contains("ONNX model"));
    }

    #[test]
    fn warn_for_tiny_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, b"tiny").unwrap();
        let mut cli = cli();
        cli.model = Some(model);
        let checks = check_model(&cli, None);
        assert_eq!(checks[0].status, Status::Warn);
        assert!(checks[0].message.contains("truncated"));
    }

    #[test]
    fn fail_for_missing_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = cli();
        cli.model = Some(dir.path().join("absent.onnx"));
        let checks = check_model(&cli, None);
        assert_eq!(checks[0].status, Status::Fail);
        assert!(checks[0].message.contains("does not exist"));
    }

    #[test]
    fn labels_pass_when_present_and_fail_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let labels = dir.path().join("labels.txt");
        std::fs::write(&labels, "Turdus merula_Common Blackbird").unwrap();
        let mut cli_present = cli();
        cli_present.labels = Some(labels);
        let checks = check_model(&cli_present, None);
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Labels") && c.status == Status::Pass)
        );

        let mut cli_absent = cli();
        cli_absent.labels = Some(dir.path().join("absent-labels.txt"));
        let checks = check_model(&cli_absent, None);
        assert!(
            checks
                .iter()
                .any(|c| c.name.contains("Labels") && c.status == Status::Fail)
        );
    }
}
