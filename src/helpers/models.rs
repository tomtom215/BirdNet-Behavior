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
use birdnet_core::inference::model::OutputScale;
use birdnet_core::inference::registry::{MAX_MODELS, ModelSpec, model_size_mib, parse_routes};

/// The settings that declare one extra classifier.
struct ExtraModelKeys {
    /// Its ONNX file.
    path_key: &'static str,
    /// Its labels.
    labels_key: &'static str,
    /// The operator's name for it.
    id_key: &'static str,
    /// Its own confidence threshold.
    threshold_key: &'static str,
    /// The sample rate it wants, when the shape cannot say.
    sample_rate_key: &'static str,
    /// How its output becomes a confidence, when the shape cannot say.
    output_scale_key: &'static str,
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
        sample_rate_key: "MODEL_2_SAMPLE_RATE",
        output_scale_key: "MODEL_2_OUTPUT_SCALE",
        default_id: "model2",
    },
    ExtraModelKeys {
        path_key: "MODEL_3_PATH",
        labels_key: "MODEL_3_LABELS",
        id_key: "MODEL_3_ID",
        threshold_key: "MODEL_3_THRESHOLD",
        sample_rate_key: "MODEL_3_SAMPLE_RATE",
        output_scale_key: "MODEL_3_OUTPUT_SCALE",
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
    "BIRDNET_MODEL_2_OUTPUT_SCALE",
    "BIRDNET_MODEL_2_PATH",
    "BIRDNET_MODEL_2_SAMPLE_RATE",
    "BIRDNET_MODEL_2_THRESHOLD",
    "BIRDNET_MODEL_3_ID",
    "BIRDNET_MODEL_3_LABELS",
    "BIRDNET_MODEL_3_OUTPUT_SCALE",
    "BIRDNET_MODEL_3_PATH",
    "BIRDNET_MODEL_3_SAMPLE_RATE",
    "BIRDNET_MODEL_3_THRESHOLD",
    "BIRDNET_MODEL_ID",
    "BIRDNET_MODEL_OUTPUT_SCALE",
    "BIRDNET_MODEL_ROUTES",
    "BIRDNET_MODEL_SAMPLE_RATE",
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
        &|key: &str| setting(config, key),
        primary_model,
        primary_labels,
        ceiling_mib,
    )
}

/// One classifier setting as [`plan`] reads it: `BIRDNET_<key>` from the
/// environment, else `<key>` from the configuration file, trimmed, and absent
/// when empty.
fn setting(config: Option<&Config>, key: &str) -> Option<String> {
    std::env::var(format!("BIRDNET_{key}"))
        .ok()
        .or_else(|| config.and_then(|c| c.get(key).map(str::to_owned)))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// The primary classifier's declared sample rate (`MODEL_SAMPLE_RATE`), read
/// as [`plan`] reads it, for `--doctor`'s source band check. A value that is
/// not a rate is `None` here; [`plan`] is what notes it.
#[must_use]
pub fn primary_sample_rate(config: Option<&Config>) -> Option<u32> {
    model_sample_rate(
        &|key: &str| setting(config, key),
        "MODEL_SAMPLE_RATE",
        &mut Vec::new(),
    )
}

/// The primary classifier's declared output scale (`MODEL_OUTPUT_SCALE`), read
/// as [`plan`] reads it, for `--doctor`'s load of the primary alone. A value
/// that is not a scale is `None` here; [`plan`] is what notes it.
#[must_use]
pub fn primary_output_scale(config: Option<&Config>) -> Option<OutputScale> {
    model_output_scale(
        &|key: &str| setting(config, key),
        "MODEL_OUTPUT_SCALE",
        &mut Vec::new(),
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
        threshold: model_threshold(get, "MODEL_THRESHOLD", &mut notes),
        sample_rate: model_sample_rate(get, "MODEL_SAMPLE_RATE", &mut notes),
        output_scale: model_output_scale(get, "MODEL_OUTPUT_SCALE", &mut notes),
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
                    threshold: model_threshold(get, keys.threshold_key, &mut notes),
                    sample_rate: model_sample_rate(get, keys.sample_rate_key, &mut notes),
                    output_scale: model_output_scale(get, keys.output_scale_key, &mut notes),
                });
            }
        }
    }

    let (mut routes, bad) =
        get("MODEL_ROUTES").map_or_else(|| (HashMap::new(), Vec::new()), |raw| parse_routes(&raw));
    for entry in bad {
        notes.push(format!(
            "MODEL_ROUTES entry `{entry}` could not be read (expected `source:model` or \
             `source:model+model`); that source falls back to the primary classifier"
        ));
    }

    // A classifier that was declared but not admitted above is not a typo, so
    // a route naming it must not reach the registry, which refuses an unknown
    // route target and with it the whole station (MMP-22). Its sources keep
    // the classifiers that did load, or fall back to the primary. A name that
    // was never declared stays, so the registry still refuses the typo.
    let declined: Vec<String> = EXTRA_MODEL_KEYS
        .iter()
        .filter(|keys| get(keys.path_key).is_some())
        .map(|keys| get(keys.id_key).unwrap_or_else(|| keys.default_id.to_owned()))
        .filter(|id| specs.iter().all(|s| s.id != *id))
        .collect();
    let mut sources: Vec<&String> = routes.keys().collect();
    sources.sort();
    let mut dropped = Vec::new();
    for source in sources {
        for id in &declined {
            if routes[source].contains(id) {
                dropped.push((source.clone(), id.clone()));
            }
        }
    }
    for (source, id) in dropped {
        if let Some(targets) = routes.get_mut(&source) {
            targets.retain(|t| *t != id);
        }
        notes.push(format!(
            "MODEL_ROUTES sends `{source}` to classifier `{id}`, which is not running (see \
             above); `{source}` is judged by the other classifiers its route names, or by the \
             primary"
        ));
    }

    ModelPlan {
        specs,
        routes,
        notes,
    }
}

/// Highest sample rate a declared `MODEL_*_SAMPLE_RATE` may name: the top of
/// what any capture hardware this runs on produces (ultrasonic bat
/// recorders). Anything above is a typo, not a model.
const MAX_DECLARED_SAMPLE_RATE: u32 = 384_000;

/// Lowest declared sample rate accepted. Zero divides the chunk arithmetic by
/// nothing; single-digit rates are a slipped key, not a classifier.
const MIN_DECLARED_SAMPLE_RATE: u32 = 1_000;

/// Read a per-classifier threshold, refusing what cannot be one.
///
/// Parsed with the decimal comma accepted (`0,8`), as
/// [`birdnet_core::config::Config::get_parsed`] does: `.parse()` alone read
/// that as nothing and silently ran the classifier at the station threshold.
/// Outside `[0, 1]` — `75`, the percentage slip in a 0–1 field — was accepted
/// and put that classifier's bar above any confidence it can produce, so it
/// detected nothing, for good, with no word anywhere.
///
/// Refused here, with a note naming the key, rather than as a `validate()`
/// error: an error makes the whole configuration unusable and
/// `startup_config::choose` then reverts *every* setting to the last-good
/// file. A mistyped per-model threshold is worth one classifier falling back
/// to the station's threshold, not a silent rollback of everything else.
fn model_threshold(
    get: &dyn Fn(&str) -> Option<String>,
    key: &str,
    notes: &mut Vec<String>,
) -> Option<f32> {
    let raw = get(key)?;
    let parsed = raw.parse::<f32>().ok().or_else(|| {
        birdnet_core::config::locale::normalize_decimal(&raw)
            .parse()
            .ok()
    });
    match parsed {
        Some(t) if t.is_finite() && (0.0..=1.0).contains(&t) => Some(t),
        Some(t) => {
            notes.push(format!(
                "{key}={raw} is not a confidence between 0 and 1{hint}; ignoring it, so that \
                 classifier runs at the station threshold",
                hint = if (1.0..=100.0).contains(&t) {
                    format!(" (a percentage? {} would be {:.2})", raw, t / 100.0)
                } else {
                    String::new()
                }
            ));
            None
        }
        None => {
            notes.push(format!(
                "{key}={raw} is not a number; ignoring it, so that classifier runs at the \
                 station threshold"
            ));
            None
        }
    }
}

/// Read a per-classifier declared sample rate, refusing what cannot be one.
///
/// `0` was accepted and became the rate the pipeline resampled to and divided
/// its chunk length by. Refused with a note, and the rate is then derived
/// from the model's input shape as if nothing had been declared.
fn model_sample_rate(
    get: &dyn Fn(&str) -> Option<String>,
    key: &str,
    notes: &mut Vec<String>,
) -> Option<u32> {
    let raw = get(key)?;
    match raw.parse::<u32>() {
        Ok(rate) if (MIN_DECLARED_SAMPLE_RATE..=MAX_DECLARED_SAMPLE_RATE).contains(&rate) => {
            Some(rate)
        }
        _ => {
            notes.push(format!(
                "{key}={raw} is not a sample rate in Hz between {MIN_DECLARED_SAMPLE_RATE} and \
                 {MAX_DECLARED_SAMPLE_RATE}; ignoring it, so that classifier's rate is derived \
                 from its input shape"
            ));
            None
        }
    }
}

/// Read a per-classifier declared output scale (MMP-11), refusing what cannot
/// be one.
///
/// `logit`, `probability` or `softmax` ([`OutputScale::parse`]). Anything else
/// is refused with a note naming the key, and the scale is then derived from
/// the model's input shape as if nothing had been declared, which for a shape
/// the station does not recognise means the classifier is refused at load
/// with a message naming the same key.
fn model_output_scale(
    get: &dyn Fn(&str) -> Option<String>,
    key: &str,
    notes: &mut Vec<String>,
) -> Option<OutputScale> {
    let raw = get(key)?;
    let scale = OutputScale::parse(&raw);
    if scale.is_none() {
        notes.push(format!(
            "{key}={raw} is not an output scale (logit, probability or softmax); ignoring it, \
             so that classifier's scale is derived from its input shape"
        ));
    }
    scale
}

/// Where `--install-model` puts what it installs, and where the setup page's
/// install lands.
///
/// See [`crate::helpers::station_defaults::model_dir`] for the order. It used to
/// end in `/data/model` whatever the machine, which a person running the bare
/// binary cannot write to.
#[must_use]
pub fn model_dir(config: Option<&Config>) -> PathBuf {
    crate::helpers::station_defaults::model_dir(config)
}

/// `--install-model <id>`: fetch and verify a classifier, or list what is
/// available. Returns a process exit code.
///
/// Foreground and synchronous. A 400 MB download over a field station's uplink
/// is measured in hours, and an operator should be able to watch it, stop it,
/// and see it fail — none of which a background task or an API call offers.
///
/// **Blocking.** It builds a `reqwest::blocking` client, which panics if
/// constructed inside a tokio runtime — and `main` is `#[tokio::main]`, so the
/// caller must reach this through `spawn_blocking`. Found by running the
/// command rather than by reading it: every unit test here is a plain sync
/// function and none of them entered a runtime.
///
/// With `from_file`, the model is copied from that file instead of downloaded,
/// and held to the same size and sha256 (OOB-20).
#[must_use]
pub fn install_from_catalogue(
    id: &str,
    dest: &std::path::Path,
    from_file: Option<&std::path::Path>,
) -> i32 {
    use birdnet_integrations::model_catalog::{Use, install, install_from_file};

    if id.trim().is_empty() || id.trim().eq_ignore_ascii_case("list") {
        print!("{}", catalogue_listing());
        return 0;
    }

    let result = from_file.map_or_else(
        || {
            println!("Installing `{id}` into {}", dest.display());
            install(id, dest)
        },
        |file| {
            println!(
                "Installing `{id}` into {} from {}",
                dest.display(),
                file.display()
            );
            install_from_file(id, file, dest)
        },
    );
    match result {
        Ok(installed) => {
            println!("\nInstalled and verified.");
            println!("  model:  {}", installed.model_path.display());
            if installed.usage != Use::RangeFilter {
                match &installed.labels_path {
                    Some(p) => println!("  labels: {}", p.display()),
                    None => println!(
                        "  labels: not shipped with this model — supply your own, with one row \
                         per class, or species are reported under other species' names"
                    ),
                }
            }
            print_next_steps(&installed);
            0
        }
        Err(e) => {
            eprintln!("\nInstall failed: {e}");
            1
        }
    }
}

/// What `--install-model list` prints: every entry, what it is for, and
/// where it comes from.
#[must_use]
pub fn catalogue_listing() -> String {
    use birdnet_integrations::model_catalog::{CATALOG, Use, model_origins, taxonomy_edition};
    use std::fmt::Write as _;

    let mut out = String::from("Models this build can install:\n\n");
    for entry in CATALOG {
        let _ = writeln!(out, "  {}", entry.id);
        let _ = writeln!(out, "    {}", entry.name);
        let _ = writeln!(
            out,
            "    {} MiB, sha256 {}",
            entry.model_bytes / (1024 * 1024),
            entry.model_sha256
        );
        let _ = match entry.usage {
            Use::Classifier => writeln!(
                out,
                "    a second classifier; needs MODEL_n_SAMPLE_RATE={}",
                entry.sample_rate
            ),
            Use::PrimaryReplacement => writeln!(
                out,
                "    replaces the primary classifier ({} Hz)",
                entry.sample_rate
            ),
            Use::RangeFilter => {
                writeln!(out, "    a species-occurrence filter (METADATA_MODEL_PATH)")
            }
        };
        if entry.usage != Use::RangeFilter {
            let _ = writeln!(
                out,
                "    labels: {}",
                if entry.ships_labels() {
                    "installed alongside, verified"
                } else {
                    "not shipped — supply your own"
                }
            );
            // RES-32: the checklist the labels' names come from, only where
            // it can be cited.
            let _ = match entry.labels_sha256.and_then(taxonomy_edition) {
                Some(t) => writeln!(
                    out,
                    "    taxonomy: {} (read from {})",
                    t.edition, t.cited_from
                ),
                None => writeln!(out, "    taxonomy: not recorded"),
            };
        }
        // MMP-20: who made it, under what licence, and where that was read.
        let _ = writeln!(out, "    source: {}", entry.source);
        let _ = writeln!(
            out,
            "    licence: {} (read from {})",
            entry.licence.unwrap_or("not recorded"),
            entry.licence_cited_from
        );
        let _ = writeln!(
            out,
            "    downloaded from, in order: {}",
            model_origins(entry).join(", then ")
        );
        let _ = writeln!(out, "    {}\n", entry.notes);
    }
    out.push_str("Install one with:  birdnet-behavior --install-model <id>\n");
    out.push_str(
        "From a file already here:  birdnet-behavior --install-model <id> --from-file <path>\n",
    );
    out
}

/// What to configure after an install, by what the file is for.
fn print_next_steps(installed: &birdnet_integrations::model_catalog::Installed) {
    use birdnet_integrations::model_catalog::Use;
    let model = installed.model_path.display();
    match installed.usage {
        Use::Classifier => {
            let labels = installed.labels_path.as_ref().map_or_else(
                || "<your labels file>".to_owned(),
                |p| p.display().to_string(),
            );
            println!("\nTo run it as a second classifier, set:");
            println!("  BIRDNET_MODEL_2_PATH={model}");
            println!("  BIRDNET_MODEL_2_LABELS={labels}");
            println!("  BIRDNET_MODEL_2_ID={}", installed.id);
            println!("  BIRDNET_MODEL_2_SAMPLE_RATE={}", installed.sample_rate);
        }
        Use::PrimaryReplacement => {
            let labels = installed.labels_path.as_ref().map_or_else(
                || "<your labels file>".to_owned(),
                |p| p.display().to_string(),
            );
            println!(
                "\nTo make it the station's classifier, set (birdnet.conf, or the BIRDNET_* variables), then restart:"
            );
            println!("  MODEL_PATH={model}      (BIRDNET_MODEL)");
            println!("  LABELS_PATH={labels}      (BIRDNET_LABELS)");
            println!("\nIts scores are BirdNET-Pi's when CONFIDENCE and SENSITIVITY are:");
            println!("  CONFIDENCE=0.7     (BirdNET-Pi's default, and this station's)");
            println!("  SENSITIVITY=1.25   (the default here too, with BirdNET-Pi's meaning)");
            println!("\nFor the species-occurrence filter:");
            println!("  birdnet-behavior --install-model birdnet-v24-range-v2");
        }
        Use::RangeFilter => {
            println!("\nTo use it, set your LATITUDE and LONGITUDE and:");
            println!("  METADATA_MODEL_PATH={model}      (BIRDNET_METADATA_MODEL)");
            println!(
                "No METADATA_LABELS_PATH is needed: it is indexed like the V2.4 labels. Use it \
                 only with the BirdNET V2.4 classifier."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CLASSIFIER_SHARE_DENOMINATOR, OutputScale, PER_MODEL_WORKING_SET_MIB, plan_with};
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

    fn primary_plan(pairs: &[(&str, &str)]) -> super::ModelPlan {
        let s: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        plan_with(
            &|k| s.get(k).cloned(),
            PathBuf::from("/nonexistent/p.onnx"),
            PathBuf::from("p.txt"),
            None,
        )
    }

    /// `MODEL_THRESHOLD=75` — the percentage slip in a 0–1 field — put the
    /// classifier's bar above anything it can score, and the station stopped
    /// detecting with no word anywhere. Refused, named, and the station
    /// threshold stands (`None`).
    #[test]
    fn an_out_of_range_model_threshold_is_refused_and_named() {
        let p = primary_plan(&[("MODEL_THRESHOLD", "75")]);
        assert_eq!(p.specs[0].threshold, None);
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("MODEL_THRESHOLD=75") && n.contains("0.75")),
            "{:?}",
            p.notes
        );
        let p = primary_plan(&[("MODEL_THRESHOLD", "-0.2")]);
        assert_eq!(p.specs[0].threshold, None);
        let p = primary_plan(&[("MODEL_THRESHOLD", "high")]);
        assert_eq!(p.specs[0].threshold, None);
        assert!(p.notes.iter().any(|n| n.contains("MODEL_THRESHOLD=high")));
    }

    /// `0,8` (a decimal comma) was silently read as nothing. Counterpart to
    /// the refusal above: an ordinary value, in either spelling, is kept.
    #[test]
    fn a_model_threshold_with_a_decimal_comma_is_read() {
        let p = primary_plan(&[("MODEL_THRESHOLD", "0,8")]);
        assert_eq!(p.specs[0].threshold, Some(0.8));
        let p = primary_plan(&[("MODEL_THRESHOLD", "0.8")]);
        assert_eq!(p.specs[0].threshold, Some(0.8));
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    /// A declared rate of zero was accepted, and became the rate the
    /// pipeline resampled to and divided its window by.
    #[test]
    fn a_zero_sample_rate_is_refused_and_named() {
        let p = primary_plan(&[("MODEL_SAMPLE_RATE", "0")]);
        assert_eq!(p.specs[0].sample_rate, None);
        assert!(
            p.notes.iter().any(|n| n.contains("MODEL_SAMPLE_RATE=0")),
            "{:?}",
            p.notes
        );
        let p = primary_plan(&[("MODEL_SAMPLE_RATE", "32000")]);
        assert_eq!(p.specs[0].sample_rate, Some(32_000));
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    /// MMP-11: `MODEL_OUTPUT_SCALE` reaches the primary's spec, and a value
    /// that is not a scale is refused with a note naming it rather than read
    /// as one.
    #[test]
    fn a_declared_output_scale_reaches_the_primary_and_nonsense_is_named() {
        let p = primary_plan(&[("MODEL_OUTPUT_SCALE", "softmax")]);
        assert_eq!(p.specs[0].output_scale, Some(OutputScale::Softmax));
        assert!(p.notes.is_empty(), "{:?}", p.notes);
        let p = primary_plan(&[("MODEL_OUTPUT_SCALE", "sigmod")]);
        assert_eq!(p.specs[0].output_scale, None);
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("MODEL_OUTPUT_SCALE=sigmod")),
            "{:?}",
            p.notes
        );
        assert_eq!(primary_plan(&[]).specs[0].output_scale, None);
    }

    /// MMP-22: a classifier the plan declines for memory is still named by
    /// the operator's routes, and the registry read that as a route to a
    /// classifier that does not exist and refused to start: a 1 GB board that
    /// added Perch and routed a microphone to it, as the manual describes,
    /// detected nothing. The declined classifier's routes fall back to the
    /// classifiers that did load, and the note says so.
    #[test]
    fn a_route_to_a_classifier_declined_for_memory_does_not_stop_detection() {
        const TINY_V24: &[u8] =
            include_bytes!("../../crates/birdnet-core/src/testdata/tiny_v24_test.onnx");
        let dir = tempfile::tempdir().expect("tempdir");
        let labels = dir.path().join("labels.txt");
        std::fs::write(
            &labels,
            (0..11)
                .map(|i| format!("Sp{i}_Bird {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .expect("labels");
        let primary = dir.path().join("p.onnx");
        let second = dir.path().join("perch.onnx");
        std::fs::write(&primary, TINY_V24).expect("primary");
        std::fs::write(&second, TINY_V24).expect("second");
        let s = settings(&[
            ("MODEL_2_PATH", second.display().to_string()),
            ("MODEL_2_LABELS", labels.display().to_string()),
            ("MODEL_2_ID", "perch".to_owned()),
            ("MODEL_ROUTES", "pond:birdnet+perch,garden:pecrh".to_owned()),
        ]);
        // 600 MiB: a budget of 300, which the primary's 256 MiB allowance
        // leaves no room in for a second.
        let p = plan_with(&|k| s.get(k).cloned(), primary, labels, Some(600));
        assert_eq!(
            p.specs.len(),
            1,
            "precondition: perch declined: {:?}",
            p.notes
        );

        let mut routes = p.routes.clone();
        routes.remove("garden");
        let registry = birdnet_core::inference::registry::ClassifierRegistry::load(
            &p.specs,
            &routes,
            &birdnet_core::inference::model::ModelConfig::default(),
        );
        assert!(
            registry.is_ok(),
            "a route to a declined classifier stopped the station: {:?}",
            registry.err()
        );
        assert_eq!(p.routes.get("pond"), Some(&vec!["birdnet".to_owned()]));
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("pond") && n.contains("perch")),
            "{:?}",
            p.notes
        );
        // Counterpart: a typo, naming nothing that was declared, is kept, so
        // the registry still refuses it at startup.
        assert_eq!(p.routes.get("garden"), Some(&vec!["pecrh".to_owned()]));
    }

    /// The extra classifiers read their threshold through the same rule.
    #[test]
    fn a_second_classifiers_threshold_is_bounded_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 1);
        let second = model_of(dir.path(), "s.onnx", 1);
        let s = settings(&[
            ("MODEL_2_PATH", second.display().to_string()),
            ("MODEL_2_LABELS", "s.txt".to_owned()),
            ("MODEL_2_THRESHOLD", "75".to_owned()),
            ("MODEL_2_SAMPLE_RATE", "0".to_owned()),
        ]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(8192),
        );
        assert_eq!(p.specs.len(), 2, "{:?}", p.notes);
        assert_eq!(p.specs[1].threshold, None);
        assert_eq!(p.specs[1].sample_rate, None);
        assert!(p.notes.iter().any(|n| n.contains("MODEL_2_THRESHOLD=75")));
        assert!(p.notes.iter().any(|n| n.contains("MODEL_2_SAMPLE_RATE=0")));
    }

    /// `MODEL_2_OUTPUT_SCALE` reaches the second classifier's spec (MMP-11).
    #[test]
    fn a_second_classifiers_declared_output_scale_reaches_its_spec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = model_of(dir.path(), "p.onnx", 1);
        let second = model_of(dir.path(), "s.onnx", 1);
        let s = settings(&[
            ("MODEL_2_PATH", second.display().to_string()),
            ("MODEL_2_LABELS", "s.txt".to_owned()),
            ("MODEL_2_OUTPUT_SCALE", "Probability".to_owned()),
        ]);
        let p = plan_with(
            &|k| s.get(k).cloned(),
            primary,
            PathBuf::from("p.txt"),
            Some(8192),
        );
        assert_eq!(p.specs.len(), 2, "{:?}", p.notes);
        assert_eq!(p.specs[0].output_scale, None);
        assert_eq!(p.specs[1].output_scale, Some(OutputScale::Probability));
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
            "BIRDNET_MODEL_SAMPLE_RATE".to_owned(),
            "BIRDNET_MODEL_OUTPUT_SCALE".to_owned(),
        ];
        for k in EXTRA_MODEL_KEYS {
            for name in [
                k.path_key,
                k.labels_key,
                k.id_key,
                k.threshold_key,
                k.sample_rate_key,
                k.output_scale_key,
            ] {
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

    /// The block `--install-model list` prints for one id.
    fn listed(id: &str) -> String {
        let all = super::catalogue_listing();
        let start = all
            .find(&format!("\n  {id}\n"))
            .unwrap_or_else(|| panic!("{id} is not listed:\n{all}"));
        // To the next entry: the next line indented by exactly two spaces.
        let rest = &all[start + 1..];
        let end = rest
            .match_indices('\n')
            .map(|(i, _)| i)
            .find(|&i| {
                let line = &rest[i + 1..];
                line.starts_with("  ") && !line.starts_with("   ")
            })
            .unwrap_or(rest.len());
        rest[..end].to_owned()
    }

    /// MMP-20: every entry says where it comes from and under what licence,
    /// and the licences are the ones the sources cited in the catalogue say:
    /// two networks under a non-commercial licence and one under Apache-2.0
    /// are not interchangeable for an operator who sells what the station
    /// records.
    #[test]
    fn the_listing_names_each_models_source_and_licence() {
        for (id, licence) in [
            ("birdnet-v24", "CC BY-NC-SA 4.0"),
            ("birdnet-v24-range-v2", "CC BY-NC-SA 4.0"),
            ("birdnet-v3", "CC BY-SA 4.0"),
            ("perch-v2", "Apache-2.0"),
        ] {
            let block = listed(id);
            assert!(
                block.contains("    source: "),
                "{id}: no source line:\n{block}"
            );
            assert!(
                block.contains(&format!("    licence: {licence} (")),
                "{id}: licence is not {licence}, with where it was read:\n{block}"
            );
        }
    }

    /// RES-32: a classifier's labels say which taxonomy they follow where that
    /// can be cited, and say "not recorded" where it cannot, rather than
    /// guessing. The range filter has no labels and no such line.
    #[test]
    fn the_listing_says_which_taxonomy_the_labels_follow_or_that_it_is_unknown() {
        assert!(
            listed("perch-v2").contains("    taxonomy: iNaturalist 2024"),
            "{}",
            listed("perch-v2")
        );
        for id in ["birdnet-v24", "birdnet-v3"] {
            assert!(
                listed(id).contains("    taxonomy: not recorded"),
                "{id}: {}",
                listed(id)
            );
        }
        assert!(!listed("birdnet-v24-range-v2").contains("taxonomy:"));
    }
}
