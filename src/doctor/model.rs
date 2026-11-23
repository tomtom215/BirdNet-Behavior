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
            if let Some(labels) = labels_path.as_deref() {
                out.extend(check_classifiers(
                    config,
                    &p,
                    labels,
                    birdnet_behavioral::memory::detect_ceiling().map(|(mib, _)| mib),
                ));
            }
        } else {
            out.push(Check::fail(
                "ONNX model file",
                format!("{} does not exist", p.display()),
                "either let the entrypoint download it (Docker), or run `install.sh` again",
            ));
        }
    } else {
        out.push(Check::warn(
            "ONNX model file",
            "no --model / MODEL_PATH configured: the web UI will run, but nothing will be analysed until a classifier is installed",
            "open the station in a browser and install one under Settings → Capture, or run `birdnet-behavior --install-model birdnet-v24`",
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

/// Every classifier the detection daemon would load, and its routes (MMP-02).
///
/// Built from the same [`crate::helpers::models::plan`] and
/// [`birdnet_core::inference::registry::ClassifierRegistry::load`] the daemon
/// runs, so a route naming a classifier that is not configured, a second
/// classifier whose file will not load, or two classifiers with one id fail
/// here before a restart instead of only in the journal after it. A classifier
/// the plan declined (no memory to spare, no labels) is a warning: the station
/// runs without it. Nothing is reported for a station with one classifier and
/// no routes, which `check_model_loads` has already covered.
///
/// `ceiling_mib` is the machine's memory ceiling, as the daemon reads it.
fn check_classifiers(
    config: Option<&Config>,
    model: &Path,
    labels: &Path,
    ceiling_mib: Option<u64>,
) -> Vec<Check> {
    use std::fmt::Write as _;
    const NAME: &str = "Classifiers";
    let plan = crate::helpers::models::plan(
        config,
        model.to_path_buf(),
        labels.to_path_buf(),
        ceiling_mib,
    );
    if plan.specs.len() == 1 && plan.routes.is_empty() && plan.notes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let declined: Vec<&str> = plan
        .notes
        .iter()
        .map(String::as_str)
        .filter(|n| !n.contains("` admitted: "))
        .collect();
    if !declined.is_empty() {
        out.push(Check::warn(
            "Classifier plan",
            declined.join("; "),
            "the station runs without what is named here; correct the MODEL_* settings it \
             names, or remove them",
        ));
    }
    let registry = match birdnet_core::inference::registry::ClassifierRegistry::load(
        &plan.specs,
        &plan.routes,
        &birdnet_core::inference::model::ModelConfig {
            num_threads: 1,
            ..birdnet_core::inference::model::ModelConfig::default()
        },
    ) {
        Ok(registry) => registry,
        Err(e) => {
            out.push(Check::fail(
                NAME,
                format!("the detection daemon will refuse to start: {e}"),
                "correct the MODEL_2_* / MODEL_3_* / MODEL_ROUTES setting it names; until \
                 then the web interface runs and nothing is detected",
            ));
            return out;
        }
    };
    for idx in 1..registry.len() {
        let Some(m) = registry.model(idx) else {
            continue;
        };
        if let Some(width) = m.model.output_dimension()
            && width != m.model.label_count()
        {
            out.push(Check::fail(
                NAME,
                format!(
                    "classifier `{}` scores {width} classes and its labels file names {}; \
                     species are assigned by position, so its detections would carry the \
                     wrong names",
                    m.id,
                    m.model.label_count()
                ),
                "install the labels file that shipped with that model",
            ));
            return out;
        }
    }
    let loaded: Vec<String> = registry
        .info()
        .iter()
        .map(|i| {
            format!(
                "{} ({} Hz, {:.1} s windows)",
                i.id, i.sample_rate, i.window_secs
            )
        })
        .collect();
    let mut routes: Vec<String> = plan
        .routes
        .iter()
        .map(|(source, targets)| format!("{source} → {}", targets.join("+")))
        .collect();
    routes.sort();
    let mut message = format!("{} load: {}", loaded.len(), loaded.join(", "));
    if routes.is_empty() {
        message.push_str("; no MODEL_ROUTES, so every source is judged by the primary alone");
    } else {
        let _ = write!(message, "; routes: {}", routes.join(", "));
    }
    let extra = registry.extra_sample_rates();
    if !extra.is_empty() {
        let rates: Vec<String> = extra.iter().map(u32::to_string).collect();
        let _ = write!(
            message,
            "; each recording is also resampled to {} Hz",
            rates.join(" and ")
        );
    }
    let unrouted = registry.unrouted();
    if unrouted.is_empty() {
        out.push(Check::pass(NAME, message));
    } else {
        let names: Vec<String> = unrouted.iter().map(|id| format!("`{id}`")).collect();
        out.push(Check::warn(
            NAME,
            format!(
                "{message}. No route names {}, so it is loaded, holds its memory and judges \
                 nothing",
                names.join(" or ")
            ),
            "name a source for it in MODEL_ROUTES (`<source id>:<primary>+<classifier>`), \
             or remove its MODEL_n_* settings to free the memory",
        ));
    }
    out
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

        std::fs::write(&model, b"abx").unwrap();
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

        // LC-2's remaining half: a download cut off part-way. The size check
        // cannot see it (the shipped model is 541 MB and the cut can land
        // anywhere past the first megabyte); loading it can.
        let truncated = dir.path().join("truncated.onnx");
        std::fs::write(&truncated, &TINY_V30_MODEL[..TINY_V30_MODEL.len() / 2]).unwrap();
        let cut = check_model_loads(&truncated, Some(&labels_file(dir.path(), 11)));
        assert_eq!(cut.status, Status::Fail, "{cut:?}");
        assert!(cut.message.contains("not a model"), "{cut:?}");

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

    /// No model configured means the detection daemon does not start (the web UI
    /// does), so this is a warning that names the two ways to install one, and
    /// must not claim a bundled default: none ships.
    #[test]
    fn warns_when_unconfigured_and_says_how_to_install_one() {
        let checks = check_model(&cli(), None);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Warn);
        let text = format!(
            "{} {}",
            checks[0].message,
            checks[0].remediation.as_deref().unwrap_or("")
        );
        assert!(!text.contains("bundled"), "no model is bundled: {text}");
        assert!(
            text.contains("--install-model"),
            "names the CLI route: {text}"
        );
        assert!(text.contains("Settings"), "names the browser route: {text}");
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

    /// A station whose config names a primary, a second classifier and
    /// `extra` further lines, written beside two tiny models in `dir`.
    fn station(dir: &std::path::Path, extra: &str) -> Config {
        const TINY_V24: &[u8] =
            include_bytes!("../../crates/birdnet-core/src/testdata/tiny_v24_test.onnx");
        let labels = dir.join("labels.txt");
        std::fs::write(
            &labels,
            (0..11)
                .map(|i| format!("Species{i}_Bird {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        for name in ["birdnet.onnx", "perch.onnx"] {
            std::fs::write(dir.join(name), TINY_V24).unwrap();
        }
        Config::parse(&format!(
            "MODEL_PATH={d}/birdnet.onnx\nLABELS_PATH={d}/labels.txt\n\
             MODEL_2_PATH={d}/perch.onnx\nMODEL_2_LABELS={d}/labels.txt\nMODEL_2_ID=perch\n\
             DB_PATH={d}/birds.db\n{extra}",
            d = dir.display()
        ))
        .unwrap()
    }

    fn failures(checks: &[Check]) -> Vec<&Check> {
        checks.iter().filter(|c| c.status == Status::Fail).collect()
    }

    /// The classifier checks for `config`, on a machine with 8 GiB, so a
    /// second tiny classifier is always admitted whatever this one has.
    fn classifier_checks(dir: &std::path::Path, config: &Config) -> Vec<Check> {
        check_classifiers(
            Some(config),
            &dir.join("birdnet.onnx"),
            &dir.join("labels.txt"),
            Some(8192),
        )
    }

    /// MMP-02: a route naming a classifier that is not configured stops the
    /// detection daemon at startup, and the registry's own doc says
    /// "`--doctor` will show it". It did not: the doctor read `MODEL_PATH`
    /// and `LABELS_PATH` and nothing else.
    #[test]
    fn a_route_to_a_classifier_that_does_not_exist_fails_the_doctor() {
        let dir = tempfile::tempdir().unwrap();
        let config = station(dir.path(), "MODEL_ROUTES=pond:pecrh\n");
        let checks = classifier_checks(dir.path(), &config);
        let failed = failures(&checks);
        assert!(
            failed.iter().any(|c| c.message.contains("pecrh")),
            "no failing check names the mistyped route: {checks:#?}"
        );
    }

    /// MMP-02: a second classifier whose file will not load stops the
    /// detection daemon at startup; the doctor said nothing about it.
    #[test]
    fn a_second_classifier_that_will_not_load_fails_the_doctor() {
        let dir = tempfile::tempdir().unwrap();
        let config = station(dir.path(), "");
        std::fs::write(dir.path().join("perch.onnx"), b"not an onnx file").unwrap();
        let checks = classifier_checks(dir.path(), &config);
        let failed = failures(&checks);
        assert!(
            failed.iter().any(|c| c.message.contains("perch")),
            "no failing check names the broken classifier: {checks:#?}"
        );
    }

    /// Counterpart: Perch beside the default V2.4 (two sample rates) with a
    /// sound route passes, and says the recording is resampled for it, so the
    /// gates above are not a check that fails every second classifier.
    #[test]
    fn a_sound_second_classifier_at_another_rate_passes() {
        let dir = tempfile::tempdir().unwrap();
        let config = station(
            dir.path(),
            "MODEL_2_SAMPLE_RATE=32000\nMODEL_ROUTES=pond:birdnet+perch\n",
        );
        // Wiring: `check_model`, which `--doctor` runs, includes the check
        // (whether this machine's memory admits the second classifier or not).
        assert!(
            check_model(&cli(), Some(&config))
                .iter()
                .any(|c| c.name == "Classifiers"),
            "the doctor does not run the classifier check"
        );
        let checks = classifier_checks(dir.path(), &config);
        let classifiers: Vec<&Check> = checks.iter().filter(|c| c.name == "Classifiers").collect();
        assert_eq!(classifiers.len(), 1, "{checks:#?}");
        assert_eq!(classifiers[0].status, Status::Pass, "{classifiers:#?}");
        assert!(classifiers[0].message.contains("32000"), "{classifiers:#?}");
        assert!(classifiers[0].message.contains("perch"), "{classifiers:#?}");
    }

    /// A second classifier the memory plan declines is a warning naming it,
    /// not a failure: the station runs without it. Its route falls back
    /// (MMP-22), so the classifiers that do run still pass.
    #[test]
    fn a_second_classifier_declined_for_memory_is_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let config = station(dir.path(), "MODEL_ROUTES=pond:birdnet+perch\n");
        let checks = check_classifiers(
            Some(&config),
            &dir.path().join("birdnet.onnx"),
            &dir.path().join("labels.txt"),
            Some(600),
        );
        assert!(failures(&checks).is_empty(), "{checks:#?}");
        let plan = checks
            .iter()
            .find(|c| c.name == "Classifier plan")
            .unwrap_or_else(|| panic!("no warning names the declined classifier: {checks:#?}"));
        assert_eq!(plan.status, Status::Warn);
        assert!(plan.message.contains("`perch` skipped"), "{plan:?}");
    }

    /// A station with one classifier and no routes gets no classifier check:
    /// `check_model_loads` has already loaded and checked that model.
    #[test]
    fn one_classifier_and_no_routes_adds_no_check() {
        let dir = tempfile::tempdir().unwrap();
        station(dir.path(), "");
        let only = Config::parse(&format!(
            "MODEL_PATH={d}/birdnet.onnx\nLABELS_PATH={d}/labels.txt\n",
            d = dir.path().display()
        ))
        .unwrap();
        assert!(classifier_checks(dir.path(), &only).is_empty());
    }

    /// MMP-12: a second classifier no route names is loaded, holds its memory
    /// and judges nothing, because a source with no route gets the primary
    /// alone. The doctor said "loads" and nothing else.
    #[test]
    fn a_second_classifier_no_route_names_is_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let config = station(dir.path(), "");
        let checks = classifier_checks(dir.path(), &config);
        let unrouted = checks
            .iter()
            .find(|c| c.name == "Classifiers")
            .unwrap_or_else(|| panic!("no classifier check: {checks:#?}"));
        assert_eq!(unrouted.status, Status::Warn, "{unrouted:?}");
        assert!(unrouted.message.contains("`perch`"), "{unrouted:?}");

        // Counterpart: routed, it is not a warning.
        let config = station(dir.path(), "MODEL_ROUTES=pond:perch\n");
        let checks = classifier_checks(dir.path(), &config);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "Classifiers" && c.status == Status::Pass),
            "{checks:#?}"
        );
    }
}
