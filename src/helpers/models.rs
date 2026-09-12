//! Which classifiers this machine will actually run (`G-10` Stage 2).
//!
//! The registry in `birdnet-core` loads what it is handed. This decides what
//! to hand it, which is a different question and a policy one: a second
//! classifier is several hundred megabytes resident, and on the boards this
//! project targets that is the difference between a station that runs for a
//! year and one that is killed at three in the morning by the kernel.
//!
//! # Why the decision lives here and not in `birdnet-core`
//!
//! It needs `/proc/meminfo` and the cgroup limit. `birdnet-core` is the
//! compute layer and deliberately reads neither; `G-33` already put the same
//! decision for the analytics engine in this layer, using the same
//! `detect_ceiling`. Two memory policies in two crates reading the machine two
//! ways is how they drift.
//!
//! # The rule, and why it is this conservative
//!
//! A model is admitted only if the classifiers already admitted, plus this
//! one, plus a working-set allowance, fit inside **half** the effective
//! ceiling — the smaller of physical RAM and any cgroup limit.
//!
//! Half, because the rest of the station is not free: ONNX Runtime's arenas,
//! the audio ring buffers, SQLite's page cache, the web server, and the
//! analytics engine's own quarter share when it is enabled. A model's file
//! size is a *floor* on its resident cost, not an estimate of it — the weights
//! are mapped, then the runtime allocates working space on top. Being wrong
//! in the admitting direction means an OOM kill on an unattended station,
//! which is the failure this whole check exists to prevent; being wrong in the
//! refusing direction means a station that runs with one model and says why.
//!
//! **These numbers are not measured on a Pi.** Nobody here has one. What is
//! measured is each model's size on disk; the fraction is a judgement, stated
//! so it can be argued with. What would falsify it is a station inside the
//! rule still being OOM-killed, and the answer then is a smaller fraction.

use std::collections::HashMap;
use std::path::PathBuf;

use birdnet_core::config::Config;
use birdnet_core::inference::registry::{MAX_MODELS, ModelSpec, model_size_mib, parse_routes};

/// The four settings that declare one extra classifier.
struct ExtraModelKeys {
    /// Its ONNX file.
    path_key: &'static str,
    /// Its labels.
    labels_key: &'static str,
    /// The operator's name for it.
    id_key: &'static str,
    /// Its own confidence threshold.
    threshold_key: &'static str,
    /// The name used when `id` is not given.
    default_id: &'static str,
}

/// Every extra classifier a station may declare, as **literal** key names.
///
/// Written out rather than built with `format!("MODEL_{n}_PATH")`, because
/// `tests/every_config_key_is_known.rs` proves that each key in
/// `KNOWN_CONFIG_KEYS` is actually read by scanning the source for literals. A
/// key assembled at runtime is invisible to that scan, so the drift gate would
/// report these as listed-but-unread — and the gate is right to: a key nothing
/// greppable reads is a key an operator can set with no effect and no warning.
const EXTRA_MODEL_KEYS: &[ExtraModelKeys] = &[
    ExtraModelKeys {
        path_key: "MODEL_2_PATH",
        labels_key: "MODEL_2_LABELS",
        id_key: "MODEL_2_ID",
        threshold_key: "MODEL_2_THRESHOLD",
        default_id: "model2",
    },
    ExtraModelKeys {
        path_key: "MODEL_3_PATH",
        labels_key: "MODEL_3_LABELS",
        id_key: "MODEL_3_ID",
        threshold_key: "MODEL_3_THRESHOLD",
        default_id: "model3",
    },
];

/// The table above covers every slot the registry allows, primary included.
const _: () = assert!(EXTRA_MODEL_KEYS.len() + 1 == MAX_MODELS);

/// Every `BIRDNET_*` variable this module reads.
///
/// Exported for `helpers::env_keys`, which proves `.env.example` documents
/// exactly what the binary reads. These are read as `BIRDNET_<key>` through
/// the lookup in [`plan`], so they never appear as `env::var("…")` literals
/// for that scan to find — the same reason `WATCHDOG_ENV_KEYS` exists. Listed
/// rather than derived by prefix from `KNOWN_CONFIG_KEYS`, because `MODEL` and
/// `MODEL_PATH` are also in there and are *not* read this way; deriving would
/// invent two reads that do not happen and quietly weaken the gate.
pub const MODEL_ENV_KEYS: &[&str] = &[
    "BIRDNET_MODEL_2_ID",
    "BIRDNET_MODEL_2_LABELS",
    "BIRDNET_MODEL_2_PATH",
    "BIRDNET_MODEL_2_THRESHOLD",
    "BIRDNET_MODEL_3_ID",
    "BIRDNET_MODEL_3_LABELS",
    "BIRDNET_MODEL_3_PATH",
    "BIRDNET_MODEL_3_THRESHOLD",
    "BIRDNET_MODEL_ID",
    "BIRDNET_MODEL_ROUTES",
    "BIRDNET_MODEL_THRESHOLD",
];

/// Share of the effective ceiling all classifiers together may occupy.
///
/// See the module header: the rest of the process needs the other half.
const CLASSIFIER_SHARE_DENOMINATOR: u64 = 2;

/// Assumed working set per loaded classifier, on top of its file size, in MiB.
///
/// ONNX Runtime allocates arenas for activations and its own scratch space; a
/// model's file is the weights alone. 256 MiB is a deliberately blunt
/// allowance in the safe direction — see the module header on which direction
/// that is.
const PER_MODEL_WORKING_SET_MIB: u64 = 256;

/// What was decided about the extra classifiers, for the log and `--doctor`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPlan {
    /// The classifiers that will be loaded, primary first.
    pub specs: Vec<ModelSpec>,
    /// Per-source routing, already parsed.
    pub routes: HashMap<String, Vec<String>>,
    /// Human-readable notes: refusals, malformed routes, what was admitted.
    ///
    /// Reported rather than returned as errors because none of them should
    /// stop a station recording birds — but every one of them must be visible
    /// to somebody reading a journal months later.
    pub notes: Vec<String>,
}

/// Read the classifier configuration and decide what this machine can run.
///
/// The primary classifier is always included: it is the station. Extra ones
/// are admitted in declaration order while they fit, and the first that does
/// not is refused **along with every one after it**, so the set is a prefix
/// and an operator reading the log sees one boundary rather than a scatter.
#[must_use]
pub fn plan(
    config: Option<&Config>,
    primary_model: PathBuf,
    primary_labels: PathBuf,
    ceiling_mib: Option<u64>,
) -> ModelPlan {
    plan_with(
        &|key: &str| {
            std::env::var(format!("BIRDNET_{key}"))
                .ok()
                .or_else(|| config.and_then(|c| c.get(key).map(str::to_owned)))
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        },
        primary_model,
        primary_labels,
        ceiling_mib,
    )
}

/// [`plan`] with the settings lookup supplied.
///
/// Split out because the process environment is global and shared by every
/// test in the binary, so a test that set `BIRDNET_MODEL_2_PATH` would leak
/// into whatever ran beside it — and because `std::env::set_var` is `unsafe`
/// in this edition, which this workspace forbids outright. Handing the lookup
/// in makes the policy a pure function of what it is told.
#[must_use]
pub fn plan_with(
    get: &dyn Fn(&str) -> Option<String>,
    primary_model: PathBuf,
    primary_labels: PathBuf,
    ceiling_mib: Option<u64>,
) -> ModelPlan {
    let mut notes = Vec::new();
    let mut specs = vec![ModelSpec {
        id: get("MODEL_ID").unwrap_or_else(|| "birdnet".to_owned()),
        model_path: primary_model,
        labels_path: primary_labels,
        threshold: get("MODEL_THRESHOLD").and_then(|v| v.parse().ok()),
    }];

    // Budget in MiB for every classifier together, or `None` when the machine
    // will not say how much memory it has — in which case no extra classifier
    // is admitted, because "unknown" must not read as "plenty".
    let budget = ceiling_mib.map(|c| c / CLASSIFIER_SHARE_DENOMINATOR);
    let mut used = model_size_mib(&specs[0].model_path)
        .map_or(PER_MODEL_WORKING_SET_MIB, |m| m + PER_MODEL_WORKING_SET_MIB);

    for keys in EXTRA_MODEL_KEYS {
        let Some(path) = get(keys.path_key) else {
            continue;
        };
        let Some(labels) = get(keys.labels_key) else {
            notes.push(format!(
                "{} is set but {} is not; skipping that classifier. Labels are positional \
                 against a model's output, so running one without its own labels would \
                 report species under other species' names",
                keys.path_key, keys.labels_key
            ));
            continue;
        };
        let path = PathBuf::from(path);
        let id = get(keys.id_key).unwrap_or_else(|| keys.default_id.to_owned());

        let Some(size) = model_size_mib(&path) else {
            notes.push(format!(
                "classifier `{id}` at {} cannot be read; skipping it. A model whose size is \
                 unknown is treated as unaffordable rather than free",
                path.display()
            ));
            continue;
        };
        let want = size + PER_MODEL_WORKING_SET_MIB;

        match budget {
            None => {
                notes.push(format!(
                    "classifier `{id}` skipped: this machine does not report how much memory \
                     it has, so a second classifier cannot be shown to fit. Running one that \
                     does not fit is an out-of-memory kill on an unattended station"
                ));
                break;
            }
            Some(budget) if used + want > budget => {
                notes.push(format!(
                    "classifier `{id}` skipped: it needs about {want} MiB and {used} MiB of the \
                     {budget} MiB classifier budget is already committed. The station runs on \
                     the classifiers it can afford rather than being OOM-killed later"
                ));
                break;
            }
            Some(_) => {
                used += want;
                notes.push(format!(
                    "classifier `{id}` admitted: about {want} MiB, {used} MiB committed"
                ));
                specs.push(ModelSpec {
                    id,
                    model_path: path,
                    labels_path: PathBuf::from(labels),
                    threshold: get(keys.threshold_key).and_then(|v| v.parse().ok()),
                });
            }
        }
    }

    let (routes, bad) =
        get("MODEL_ROUTES").map_or_else(|| (HashMap::new(), Vec::new()), |raw| parse_routes(&raw));
    for entry in bad {
        notes.push(format!(
            "MODEL_ROUTES entry `{entry}` could not be read (expected `source:model` or \
             `source:model+model`); that source falls back to the primary classifier"
        ));
    }

    ModelPlan {
        specs,
        routes,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::{CLASSIFIER_SHARE_DENOMINATOR, PER_MODEL_WORKING_SET_MIB, plan_with};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Write a file of `mib` megabytes and return its path.
    fn model_of(dir: &std::path::Path, name: &str, mib: u64) -> PathBuf {
        let p = dir.join(name);
        let bytes = usize::try_from(mib * 1024 * 1024).expect("fits");
        std::fs::write(&p, vec![0u8; bytes]).expect("write");
        p
    }

    /// A settings lookup over a fixed map — no process environment involved,
    /// so these tests cannot leak into each other or into anything else in
    /// this binary.
    fn settings(pairs: &[(&str, String)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    /// **The station is always in the plan.** Whatever the memory says, the
    /// primary classifier is what the station *is*; refusing it would be
    /// refusing to detect birds at all.
    #[test]
    fn the_primary_classifier_survives_any_memory_pressure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let m = model_of(dir.path(), "primary.onnx", 1);
        let s = settings(&[]);
        // A ceiling far too small for anything.
        let p = plan_with(
            &|k| s.get(k).cloned(),
            m,
            PathBuf::from("labels.txt"),
            Some(1),
        );
        assert_eq!(p.specs.len(), 1, "the primary is never dropped");
        assert_eq!(p.specs[0].id, "birdnet");
    }

    /// **A machine that will not say how much memory it has gets one
    /// classifier.** "Unknown" must not read as "plenty" — that reading is
    /// what gets an unattended station OOM-killed at three in the morning.
    ///
    /// Observed failing with the `None` arm admitting the model: a second
    /// classifier was planned on a machine whose capacity was unknown.
    #[test]
    fn an_unknown_ceiling_admits_no_second_classifier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 1);
        let second = model_of(dir.path(), "s.onnx", 1);
        let s = settings(&[
            ("MODEL_2_PATH", second.display().to_string()),
            ("MODEL_2_LABELS", "s.txt".to_owned()),
        ]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            None,
        );
        assert_eq!(p.specs.len(), 1, "no ceiling → no extra classifier");
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("does not report how much memory")),
            "{:?}",
            p.notes
        );
    }

    /// A second classifier that fits is admitted, and the note carries the
    /// arithmetic — an operator reading a journal wants the numbers, not a
    /// verdict.
    #[test]
    fn a_second_classifier_that_fits_is_admitted_with_its_cost() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 10);
        let second = model_of(dir.path(), "s.onnx", 10);
        let s = settings(&[
            ("MODEL_2_PATH", second.display().to_string()),
            ("MODEL_2_LABELS", "s.txt".to_owned()),
            ("MODEL_2_ID", "perch".to_owned()),
        ]);
        // (10+256)*2 = 532 MiB of classifiers; a 2 GiB ceiling gives 1 GiB.
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(2048),
        );
        assert_eq!(p.specs.len(), 2, "{:?}", p.notes);
        assert_eq!(p.specs[1].id, "perch");
        assert!(
            p.notes.iter().any(|n| n.contains("admitted")),
            "{:?}",
            p.notes
        );
    }

    /// **The refusal that matters.** A 1 GiB board gets a 512 MiB classifier
    /// budget; the primary plus its working set already commits 276, so a
    /// second costing 276 would total 552 and is refused. The station then
    /// runs on one classifier rather than dying at three in the morning.
    ///
    /// Observed failing with the `used + want > budget` comparison inverted:
    /// both classifiers were planned on a board that cannot hold them.
    #[test]
    fn a_second_classifier_that_does_not_fit_is_refused_with_the_arithmetic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 20);
        let second = model_of(dir.path(), "s.onnx", 20);
        let s = settings(&[
            ("MODEL_2_PATH", second.display().to_string()),
            ("MODEL_2_LABELS", "s.txt".to_owned()),
            ("MODEL_2_ID", "perch".to_owned()),
        ]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(1024),
        );
        assert_eq!(
            p.specs.len(),
            1,
            "the second must be refused: {:?}",
            p.notes
        );
        let note = p.notes.join(" | ");
        assert!(note.contains("perch"), "{note}");
        assert!(note.contains("skipped"), "{note}");
        assert!(note.contains("512"), "the budget must appear: {note}");
        assert!(
            note.contains("OOM-killed"),
            "the reason must be legible: {note}"
        );
    }

    /// A model declared without its labels is skipped, and the note says why
    /// that is not pedantry: labels are positional, so the wrong ones report
    /// species under other species' names.
    #[test]
    fn a_classifier_without_labels_is_skipped_and_says_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 1);
        let second = model_of(dir.path(), "s.onnx", 1);
        let s = settings(&[("MODEL_2_PATH", second.display().to_string())]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(8192),
        );
        assert_eq!(p.specs.len(), 1);
        assert!(
            p.notes.iter().any(|n| n.contains("other species' names")),
            "{:?}",
            p.notes
        );
    }

    /// A model file that cannot be read is treated as unaffordable, never as
    /// free — the same reasoning as an unknown ceiling.
    #[test]
    fn an_unreadable_model_file_is_skipped_rather_than_assumed_weightless() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 1);
        let s = settings(&[
            ("MODEL_2_PATH", "/nonexistent/perch.onnx".to_owned()),
            ("MODEL_2_LABELS", "s.txt".to_owned()),
        ]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(8192),
        );
        assert_eq!(p.specs.len(), 1);
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("unaffordable rather than free")),
            "{:?}",
            p.notes
        );
    }

    /// A malformed route is reported and its source falls back, rather than
    /// stopping a station whose other routing is sound.
    #[test]
    fn a_malformed_route_is_reported_without_stopping_the_station() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 1);
        let s = settings(&[("MODEL_ROUTES", "pond:birdnet,garbage".to_owned())]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(8192),
        );
        assert_eq!(p.routes["pond"], vec!["birdnet"]);
        assert!(
            p.notes.iter().any(|n| n.contains("`garbage`")),
            "{:?}",
            p.notes
        );
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("falls back to the primary")),
            "{:?}",
            p.notes
        );
    }

    /// `MODEL_ENV_KEYS` is a hand-written mirror of the key table, and a
    /// mirror is a thing that drifts. Adding a fourth classifier slot without
    /// adding its four names here would leave them undocumented in
    /// `.env.example` with nothing to notice.
    ///
    /// Observed failing with `BIRDNET_MODEL_3_THRESHOLD` removed from
    /// `MODEL_ENV_KEYS`: the set difference was non-empty.
    #[test]
    fn the_exported_env_names_cover_every_key_the_table_reads() {
        use super::{EXTRA_MODEL_KEYS, MODEL_ENV_KEYS};
        let mut expected: Vec<String> = vec![
            "BIRDNET_MODEL_ID".to_owned(),
            "BIRDNET_MODEL_THRESHOLD".to_owned(),
            "BIRDNET_MODEL_ROUTES".to_owned(),
        ];
        for k in EXTRA_MODEL_KEYS {
            for name in [k.path_key, k.labels_key, k.id_key, k.threshold_key] {
                expected.push(format!("BIRDNET_{name}"));
            }
        }
        expected.sort_unstable();
        let mut listed: Vec<String> = MODEL_ENV_KEYS.iter().map(|k| (*k).to_owned()).collect();
        listed.sort_unstable();
        assert_eq!(
            listed, expected,
            "MODEL_ENV_KEYS has drifted from the keys the table actually reads"
        );
    }

    /// The fraction and the allowance are the two numbers this policy turns
    /// on, and both are judgements rather than measurements. Pinning them
    /// makes changing either a deliberate act with this test in the diff.
    #[test]
    fn the_policy_constants_are_what_the_module_header_says() {
        assert_eq!(CLASSIFIER_SHARE_DENOMINATOR, 2, "half the ceiling");
        assert_eq!(PER_MODEL_WORKING_SET_MIB, 256);
    }
}
