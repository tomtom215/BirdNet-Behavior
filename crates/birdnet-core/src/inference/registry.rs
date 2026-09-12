//! The classifiers a station runs, and which sources reach which (`G-10`
//! Stage 2).
//!
//! Stage 1 made "a classifier" a trait. This is the set of them, and the map
//! from an audio source to the ones that judge it — a garden microphone and a
//! pond camera need not run the same model, and on a station that adds a bat
//! classifier they certainly should not.
//!
//! # Written for a station nobody is watching
//!
//! Every decision here is shaped by the same question: what happens at three
//! in the morning, four months in, with nobody on site? Three invariants
//! follow, and each is a gate.
//!
//! **A registry always has at least one classifier.** A station with none is
//! not a degraded station, it is a station that has silently stopped being
//! what it is for. [`ClassifierRegistry::load`] refuses to build one.
//!
//! **An unknown route fails at startup.** `MODEL_ROUTES=front-door:pecrh` is a
//! typo that must stop the station before it starts, where the journal and
//! `--doctor` will show it. Accepting it and discovering at runtime that one
//! source routes to nothing would cost months of recordings from that
//! microphone, and the loss would be invisible: the station stays up, the
//! other sources keep detecting, and nothing says the front door went quiet.
//!
//! **A source with no route gets the primary model, never nothing.** The
//! fallback for any ambiguity is "keep detecting birds with the model that was
//! always there". Silence must never be reachable by omission.
//!
//! # What this does not decide
//!
//! Whether the machine can afford a second model. That is a memory policy, it
//! needs `/proc` and cgroup reads this crate deliberately does not do, and
//! `G-33` already put that decision in the application layer beside the
//! analytics one. This module loads what it is handed and reports what it
//! loaded; `src/helpers` decides what to hand it and says why.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::inference::labels::LabelSet;
use crate::inference::model::{BirdNetModel, InputSpec, ModelConfig};

/// How many classifiers a station may run at once.
///
/// Three is not arbitrary: it is BirdNET, one regional or general second
/// opinion (Perch), and one non-bird instrument (the bat classifier) — the
/// three the gap analysis actually names. The cap exists because each model is
/// hundreds of megabytes resident on a machine that may have one gigabyte, and
/// an operator who has mistyped a loop into their config should be told so
/// rather than discovering it through the out-of-memory killer.
pub const MAX_MODELS: usize = 3;

/// What an operator declared about one classifier, before it is loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    /// The operator's name for it, used in routes and recorded on detections.
    pub id: String,
    /// The ONNX file.
    pub model_path: PathBuf,
    /// Its labels. Positional against the model's output, so a mismatched
    /// pair reports species under other species' names.
    pub labels_path: PathBuf,
    /// A confidence threshold for this model alone, or `None` to use the
    /// station's. Two models are not calibrated alike, and forcing one number
    /// on both means the stricter model is effectively off or the looser one
    /// floods the log.
    pub threshold: Option<f32>,
    /// The sample rate this classifier wants, when it must be declared.
    ///
    /// # Why this cannot be derived (`G-10` Stage 4)
    ///
    /// [`crate::inference::model::infer_sample_rate_from_shape`] reads the
    /// rate off the input shape, which works only because it is a lookup over
    /// two known BirdNET shapes with 48 kHz as the default. It is not
    /// derivable in general, and Perch v2 is the proof: it declares
    /// `[-1, 160_000]`, which is 5 s at 32 kHz — and is equally 3⅓ s at
    /// 48 kHz. Nothing in the tensor distinguishes them, so the lookup
    /// silently called it a 48 kHz model and would have resampled every
    /// recording to the wrong rate.
    ///
    /// `None` keeps the derivation, which remains right for the BirdNET
    /// shapes it was built from.
    pub sample_rate: Option<u32>,
}

/// A loaded classifier and what was declared about it.
pub struct RegisteredModel {
    /// The operator's name for it.
    pub id: String,
    /// The classifier itself.
    pub model: BirdNetModel,
    /// Its own threshold, if it was given one.
    pub threshold: Option<f32>,
    /// What it needs fed to it, after any declared sample rate is applied.
    ///
    /// This and not [`BirdNetModel::input_spec`] is what the pipeline reads:
    /// the model can only derive its rate from the shape, and for anything
    /// but the two BirdNET shapes that derivation is a guess.
    pub spec: InputSpec,
}

impl fmt::Debug for RegisteredModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredModel")
            .field("id", &self.id)
            .field("threshold", &self.threshold)
            .field("input_spec", &self.model.input_spec())
            .finish_non_exhaustive()
    }
}

/// What can go wrong building a registry.
#[derive(Debug)]
pub enum RegistryError {
    /// No classifiers were declared at all.
    Empty,
    /// More classifiers than [`MAX_MODELS`].
    TooMany {
        /// How many were declared.
        declared: usize,
    },
    /// Two classifiers share an id, so a route naming it is ambiguous.
    DuplicateId {
        /// The repeated id.
        id: String,
    },
    /// A classifier could not be loaded.
    Load {
        /// Which one.
        id: String,
        /// Why.
        why: String,
    },
    /// A classifier needs audio prepared differently from the primary.
    IncompatibleInput {
        /// Which classifier.
        id: String,
        /// What it needs.
        wants: InputSpec,
        /// What the primary needs, and therefore what the pipeline prepares.
        primary: InputSpec,
    },
    /// A route names a classifier that was never declared.
    UnknownRouteTarget {
        /// The source whose route is wrong.
        source: String,
        /// The classifier id it names.
        target: String,
        /// The ids that do exist, to make the typo obvious.
        known: Vec<String>,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "no classifier was configured; a station with none detects nothing"
            ),
            Self::TooMany { declared } => write!(
                f,
                "{declared} classifiers declared but at most {MAX_MODELS} may run at once; \
                 each is hundreds of megabytes resident"
            ),
            Self::DuplicateId { id } => write!(
                f,
                "two classifiers are both called `{id}`; a route naming it could mean either"
            ),
            Self::Load { id, why } => write!(f, "classifier `{id}` could not be loaded: {why}"),
            Self::IncompatibleInput { id, wants, primary } => write!(
                f,
                "classifier `{id}` wants {} Hz and the primary wants {} Hz. Refusing to start. \
                 A recording is decoded and resampled once, and resampling it twice would \
                 double the most expensive step in the pipeline on the boards this runs on — \
                 with no rate that is correct for both. Differing *window lengths* are fine; \
                 it is the rate that cannot be reconciled",
                wants.sample_rate, primary.sample_rate,
            ),
            Self::UnknownRouteTarget {
                source,
                target,
                known,
            } => write!(
                f,
                "source `{source}` is routed to classifier `{target}`, which is not configured \
                 (configured: {}). Refusing to start: a route to nothing would leave that \
                 source detecting nothing, with no other symptom",
                known.join(", ")
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// The classifiers a station runs, and the per-source routing between them.
pub struct ClassifierRegistry {
    models: Vec<RegisteredModel>,
    /// Source id → indices into `models`. A source absent from this map uses
    /// [`ClassifierRegistry::default_route`].
    routes: HashMap<String, Vec<usize>>,
}

impl ClassifierRegistry {
    /// Load every declared classifier and resolve the routing table.
    ///
    /// `routes` maps an audio source id to the classifier ids that should
    /// judge it. A source named here must route to classifiers that exist; a
    /// source not named here uses the primary.
    ///
    /// # Errors
    ///
    /// [`RegistryError`] for an empty or oversized set, a duplicate id, a
    /// classifier that will not load, or a route naming one that does not
    /// exist. Every one of these is refused **at startup**, which is the whole
    /// point: an unattended station must fail where somebody can see it, not
    /// months later as a source that quietly stopped producing detections.
    pub fn load(
        specs: &[ModelSpec],
        routes: &HashMap<String, Vec<String>>,
        model_config: &ModelConfig,
    ) -> Result<Self, RegistryError> {
        if specs.is_empty() {
            return Err(RegistryError::Empty);
        }
        if specs.len() > MAX_MODELS {
            return Err(RegistryError::TooMany {
                declared: specs.len(),
            });
        }
        let mut seen: Vec<&str> = Vec::with_capacity(specs.len());
        for spec in specs {
            if seen.contains(&spec.id.as_str()) {
                return Err(RegistryError::DuplicateId {
                    id: spec.id.clone(),
                });
            }
            seen.push(&spec.id);
        }

        let mut models = Vec::with_capacity(specs.len());
        for spec in specs {
            let labels = LabelSet::load(&spec.labels_path).map_err(|e| RegistryError::Load {
                id: spec.id.clone(),
                why: format!("labels: {e}"),
            })?;
            // A per-classifier threshold is applied to its own config here,
            // once, rather than swapped in around every inference call. Two
            // models are not calibrated alike: forcing the station's single
            // number on both means the stricter one is effectively off or the
            // looser one floods the log.
            let mut config = model_config.clone();
            if let Some(threshold) = spec.threshold {
                config.confidence_threshold = threshold;
            }
            let model = BirdNetModel::load(&spec.model_path, labels, config).map_err(|e| {
                RegistryError::Load {
                    id: spec.id.clone(),
                    why: e.to_string(),
                }
            })?;
            // Declared rate wins over the shape-derived guess; the window in
            // samples still comes from the tensor, which does state it.
            let mut effective = model.input_spec();
            if let Some(rate) = spec.sample_rate {
                effective.sample_rate = rate;
            }
            models.push(RegisteredModel {
                id: spec.id.clone(),
                model,
                threshold: spec.threshold,
                spec: effective,
            });
        }

        // Every classifier must want audio at the **same rate**. Differing
        // window lengths are fine and handled by the chunk arithmetic (see
        // `window_bounds`); differing rates are not, because the pipeline
        // resamples a recording once and doing it twice would double that work
        // on a board where it is already the expensive part — and there is no
        // principled rate to pick that is right for both.
        //
        // Window differences used to be refused here too, while Stage 4's
        // question — what a merged detection means when two classifiers judged
        // different spans — was open. It is answered now: the chunk is cut to
        // the longest window, the step is the shortest, each classifier reads
        // its own window from the shared chunk start, and agreement means both
        // reported a species from audio beginning at the same instant.
        let primary_spec = models[0].spec;
        for m in models.iter().skip(1) {
            if m.spec.sample_rate != primary_spec.sample_rate {
                return Err(RegistryError::IncompatibleInput {
                    id: m.id.clone(),
                    wants: m.spec,
                    primary: primary_spec,
                });
            }
        }

        // Resolved here, once, so a typo cannot survive into the loop.
        let mut resolved: HashMap<String, Vec<usize>> = HashMap::new();
        for (source, targets) in routes {
            let mut idxs = Vec::with_capacity(targets.len());
            for target in targets {
                let Some(idx) = models.iter().position(|m| m.id == *target) else {
                    return Err(RegistryError::UnknownRouteTarget {
                        source: source.clone(),
                        target: target.clone(),
                        known: models.iter().map(|m| m.id.clone()).collect(),
                    });
                };
                if !idxs.contains(&idx) {
                    idxs.push(idx);
                }
            }
            // An empty route is not "no models" — see the module header. It is
            // an operator who wrote `front-door:` and meant nothing by it, and
            // the safe reading is the primary.
            if !idxs.is_empty() {
                resolved.insert(source.clone(), idxs);
            }
        }

        Ok(Self {
            models,
            routes: resolved,
        })
    }

    /// A registry wrapping one already-loaded classifier.
    ///
    /// For callers that have a [`BirdNetModel`] in hand and no configuration
    /// to resolve — the end-to-end tests that load the real model directly,
    /// and anything embedding the pipeline. It skips the checks [`Self::load`]
    /// performs because there is nothing to check: one classifier, no routes,
    /// and the id is the caller's own.
    #[must_use]
    pub fn single(id: impl Into<String>, model: BirdNetModel) -> Self {
        Self {
            models: vec![RegisteredModel {
                spec: model.input_spec(),
                id: id.into(),
                model,
                threshold: None,
            }],
            routes: HashMap::new(),
        }
    }

    /// The classifier every station has, and the one any ambiguity falls back
    /// to. Index 0, which [`Self::load`] guarantees exists.
    #[must_use]
    pub fn primary(&self) -> &RegisteredModel {
        &self.models[0]
    }

    /// Mutable access to one classifier by index, for running inference.
    #[must_use]
    pub fn model_mut(&mut self, idx: usize) -> Option<&mut RegisteredModel> {
        self.models.get_mut(idx)
    }

    /// One classifier by index.
    #[must_use]
    pub fn model(&self, idx: usize) -> Option<&RegisteredModel> {
        self.models.get(idx)
    }

    /// How many classifiers are loaded. Always at least one.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.models.len()
    }

    /// Always `false` — a registry cannot be empty. Present because clippy
    /// asks for it beside `len`, and answering honestly is better than
    /// omitting it and having a caller assume the other answer is reachable.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// The indices every source falls back to: the primary alone.
    ///
    /// Not "all models". A station that adds a bat classifier has not asked
    /// for every microphone to be judged by it, and quietly doubling the
    /// inference cost of an unrouted source is how a Pi that was keeping up
    /// stops keeping up.
    #[must_use]
    pub const fn default_route() -> [usize; 1] {
        [0]
    }

    /// Which classifiers judge `source_id`.
    ///
    /// A source with no route of its own gets the primary — never an empty
    /// slice, which would mean that microphone silently detects nothing.
    #[must_use]
    pub fn route_for(&self, source_id: Option<&str>) -> Vec<usize> {
        source_id
            .and_then(|id| self.routes.get(id))
            .filter(|idxs| !idxs.is_empty())
            .map_or_else(|| Self::default_route().to_vec(), Clone::clone)
    }

    /// The longest and shortest window any loaded classifier wants, in
    /// samples (`G-10` Stage 4).
    ///
    /// The chunk is cut to the longest and stepped by the shortest. That pair
    /// is what keeps every classifier's coverage at least what it would be
    /// running alone: stepping by the longest would leave a shorter-window
    /// model a blind spot at the tail of every chunk, which nothing would
    /// report — see [`crate::detection::pipeline::PipelineConfig::chunk_step_secs`].
    #[must_use]
    pub fn window_bounds(&self) -> (usize, usize) {
        let mut longest = self.models[0].spec.window_samples;
        let mut shortest = longest;
        for m in &self.models {
            longest = longest.max(m.spec.window_samples);
            shortest = shortest.min(m.spec.window_samples);
        }
        (longest, shortest)
    }

    /// Every classifier's id, in load order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.models.iter().map(|m| m.id.as_str()).collect()
    }

    /// The input spec of each classifier, for a startup log an operator can
    /// check against what they think they configured.
    #[must_use]
    pub fn specs(&self) -> Vec<(&str, InputSpec)> {
        self.models
            .iter()
            .map(|m| (m.id.as_str(), m.spec))
            .collect()
    }
}

impl fmt::Debug for ClassifierRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClassifierRegistry")
            .field("models", &self.models)
            .field("routes", &self.routes)
            .finish()
    }
}

/// Parse a `MODEL_ROUTES` value: `source:model[+model],source2:model`.
///
/// Returns the map and the entries it could not read, rather than failing:
/// the caller decides how loud to be, and a station must not refuse to start
/// over a stray comma when the rest of the routing is sound. An unreadable
/// entry is reported and its source falls back to the primary.
#[must_use]
pub fn parse_routes(raw: &str) -> (HashMap<String, Vec<String>>, Vec<String>) {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut bad = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((source, targets)) = entry.split_once(':') else {
            bad.push(entry.to_owned());
            continue;
        };
        let source = source.trim();
        let targets: Vec<String> = targets
            .split('+')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect();
        if source.is_empty() || targets.is_empty() {
            bad.push(entry.to_owned());
            continue;
        }
        out.entry(source.to_owned()).or_default().extend(targets);
    }
    (out, bad)
}

/// Resolve a model file's size on disk, for the memory decision the
/// application layer makes before handing specs to [`ClassifierRegistry::load`].
///
/// Returns `None` when the file cannot be stated — which the caller must treat
/// as "cannot afford it", not as "free".
#[must_use]
pub fn model_size_mib(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .map(|m| m.len() / (1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::{
        ClassifierRegistry, MAX_MODELS, ModelSpec, RegistryError, model_size_mib, parse_routes,
    };
    use crate::inference::model::ModelConfig;
    use std::collections::HashMap;
    use std::path::PathBuf;

    const TINY_V24: &[u8] = include_bytes!("../testdata/tiny_v24_test.onnx");
    const TINY_V30: &[u8] = include_bytes!("../testdata/tiny_v30_test.onnx");

    /// Write a model and its labels into `dir`, returning a spec for them.
    fn spec(dir: &std::path::Path, id: &str, bytes: &[u8], threshold: Option<f32>) -> ModelSpec {
        let model_path = dir.join(format!("{id}.onnx"));
        std::fs::write(&model_path, bytes).expect("write model");
        let labels_path = dir.join(format!("{id}.txt"));
        let labels: Vec<String> = (0..11).map(|i| format!("Sp{i}_Bird {i}")).collect();
        std::fs::write(&labels_path, labels.join("\n")).expect("write labels");
        ModelSpec {
            id: id.to_owned(),
            model_path,
            labels_path,
            threshold,
            sample_rate: None,
        }
    }

    fn no_routes() -> HashMap<String, Vec<String>> {
        HashMap::new()
    }

    // ── the three field-deployment invariants ───────────────────────────

    /// **A station with no classifier is not a degraded station.** It is one
    /// that has silently stopped doing the only thing it exists for, and
    /// nothing downstream would report it — the web UI would show a station
    /// that is up, with no detections, exactly like a quiet night.
    ///
    /// Observed failing with the `specs.is_empty()` check removed: `load`
    /// returned a registry, and `primary()` then panicked on an empty Vec —
    /// a crash at the first audio file rather than a refusal at startup.
    #[test]
    fn a_registry_with_no_classifier_is_refused() {
        let err = ClassifierRegistry::load(&[], &no_routes(), &ModelConfig::default())
            .expect_err("an empty registry must be refused");
        assert!(matches!(err, RegistryError::Empty), "{err:?}");
        assert!(err.to_string().contains("detects nothing"), "{err}");
    }

    /// **A typo in a route must stop the station at startup.** Accepting
    /// `front-door:pecrh` and discovering at runtime that the source routes
    /// nowhere would cost months of recordings from that microphone, and the
    /// loss is invisible: the station stays up, other sources keep detecting,
    /// nothing says the front door went quiet.
    ///
    /// Observed failing with the unknown-target branch replaced by `continue`:
    /// `load` succeeded and the mistyped source silently fell back, so the
    /// operator's intent was lost with no error anywhere.
    #[test]
    fn a_route_to_a_classifier_that_does_not_exist_refuses_at_startup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let specs = [spec(dir.path(), "birdnet", TINY_V24, None)];
        let mut routes = HashMap::new();
        routes.insert("front-door".to_owned(), vec!["pecrh".to_owned()]);

        let err = ClassifierRegistry::load(&specs, &routes, &ModelConfig::default())
            .expect_err("a route to a missing classifier must be refused");
        match &err {
            RegistryError::UnknownRouteTarget {
                source,
                target,
                known,
            } => {
                assert_eq!(source, "front-door");
                assert_eq!(target, "pecrh");
                assert_eq!(
                    known,
                    &["birdnet".to_owned()],
                    "the error must list what does exist"
                );
            }
            other => panic!("wrong error: {other:?}"),
        }
        // The message has to make the typo obvious to somebody reading a
        // journal months later, not just name an error kind.
        let msg = err.to_string();
        assert!(msg.contains("pecrh") && msg.contains("birdnet"), "{msg}");
        assert!(msg.contains("detecting nothing"), "{msg}");
    }

    /// **Silence must never be reachable by omission.** A source nobody routed
    /// gets the primary classifier, not an empty set.
    ///
    /// Observed failing with `route_for`'s fallback changed to `Vec::new()`:
    /// an unrouted source returned no classifiers, which downstream means a
    /// microphone that records and is never judged.
    #[test]
    fn a_source_with_no_route_gets_the_primary_not_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Both V3.0-shaped: these test *routing*, and two classifiers that
        // want different audio are refused before routing is even resolved
        // (see `a_classifier_wanting_different_audio_is_refused_at_startup`).
        let specs = [
            spec(dir.path(), "birdnet", TINY_V30, None),
            spec(dir.path(), "perch", TINY_V30, None),
        ];
        let reg = ClassifierRegistry::load(&specs, &no_routes(), &ModelConfig::default())
            .expect("two classifiers load");

        for source in [None, Some("never-configured"), Some("")] {
            let route = reg.route_for(source);
            assert_eq!(route, vec![0], "{source:?} must fall back to the primary");
            assert!(!route.is_empty(), "an empty route means a deaf microphone");
        }
        assert_eq!(reg.primary().id, "birdnet");
    }

    /// The counterpart: a source that *is* routed gets what it was routed to,
    /// or the fallback gate above would pass against a registry that ignored
    /// routing entirely.
    #[test]
    fn a_routed_source_reaches_exactly_its_classifiers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let specs = [
            spec(dir.path(), "birdnet", TINY_V30, None),
            spec(dir.path(), "perch", TINY_V30, None),
        ];
        let mut routes = HashMap::new();
        routes.insert("pond".to_owned(), vec!["perch".to_owned()]);
        routes.insert(
            "garden".to_owned(),
            vec!["birdnet".to_owned(), "perch".to_owned()],
        );
        let reg =
            ClassifierRegistry::load(&specs, &routes, &ModelConfig::default()).expect("loads");

        assert_eq!(reg.route_for(Some("pond")), vec![1]);
        assert_eq!(reg.route_for(Some("garden")), vec![0, 1]);
        assert_eq!(reg.route_for(Some("shed")), vec![0], "unrouted → primary");
    }

    // ── the rest of the refusals ────────────────────────────────────────

    /// Two classifiers under one name make every route naming it ambiguous,
    /// and picking either silently is worse than refusing.
    #[test]
    fn two_classifiers_with_the_same_id_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = spec(dir.path(), "birdnet", TINY_V24, None);
        let mut b = spec(dir.path(), "other", TINY_V30, None);
        b.id = "birdnet".to_owned();
        let err = ClassifierRegistry::load(&[a, b], &no_routes(), &ModelConfig::default())
            .expect_err("a duplicate id must be refused");
        assert!(matches!(err, RegistryError::DuplicateId { .. }), "{err:?}");
    }

    /// More classifiers than the machine can hold is a configuration error,
    /// and the out-of-memory killer is a bad way to learn about it.
    #[test]
    fn more_classifiers_than_the_cap_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let specs: Vec<ModelSpec> = (0..=MAX_MODELS)
            .map(|i| spec(dir.path(), &format!("m{i}"), TINY_V24, None))
            .collect();
        let err = ClassifierRegistry::load(&specs, &no_routes(), &ModelConfig::default())
            .expect_err("over the cap must be refused");
        assert!(matches!(err, RegistryError::TooMany { .. }), "{err:?}");
    }

    /// A model file that will not load names *which* one, because a station
    /// with three configured and one bad must not send its operator hunting.
    #[test]
    fn a_classifier_that_cannot_load_names_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = spec(dir.path(), "birdnet", TINY_V24, None);
        let mut bad = spec(dir.path(), "broken", TINY_V30, None);
        std::fs::write(&bad.model_path, b"not an onnx file").expect("write");
        bad.labels_path = good.labels_path.clone();
        let err = ClassifierRegistry::load(&[good, bad], &no_routes(), &ModelConfig::default())
            .expect_err("a corrupt model must be refused");
        match &err {
            RegistryError::Load { id, .. } => assert_eq!(id, "broken"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    /// The single-model station — every station today — loads exactly as it
    /// did, with no routing and one classifier.
    #[test]
    fn the_single_classifier_station_is_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let specs = [spec(dir.path(), "birdnet", TINY_V24, None)];
        let reg = ClassifierRegistry::load(&specs, &no_routes(), &ModelConfig::default())
            .expect("one classifier loads");
        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());
        assert_eq!(reg.primary().id, "birdnet");
        assert_eq!(reg.route_for(None), vec![0]);
        assert_eq!(reg.ids(), vec!["birdnet"]);
        assert_eq!(reg.specs().len(), 1);
    }

    /// **A differing sample rate is the one thing that cannot be
    /// reconciled.** A recording is decoded and resampled once; resampling it
    /// twice would double the most expensive step in the pipeline on the
    /// boards this runs on, and there is no rate that is right for both.
    ///
    /// This gate used to cover differing *windows* as well. It no longer
    /// does, because the chunk arithmetic answers that case — the chunk is cut
    /// to the longest window and stepped by the shortest, so every classifier
    /// reads its own window from a shared start with no coverage gap. The
    /// refusal narrowed to what actually cannot be handled.
    ///
    /// Observed failing with the rate comparison removed from `load`: the
    /// registry accepted a 48 kHz classifier beside a 32 kHz one, and one of
    /// them would have been fed audio resampled for the other.
    #[test]
    fn a_classifier_wanting_a_different_sample_rate_is_refused_at_startup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = spec(dir.path(), "birdnet", TINY_V30, None);
        // The tiny V2.4 fixture is 48 kHz where V3.0 is 32 kHz.
        let other = spec(dir.path(), "perch", TINY_V24, None);

        let err =
            ClassifierRegistry::load(&[primary, other], &no_routes(), &ModelConfig::default())
                .expect_err("a classifier needing a different rate must be refused");
        match &err {
            RegistryError::IncompatibleInput { id, wants, primary } => {
                assert_eq!(id, "perch");
                assert_ne!(wants.sample_rate, primary.sample_rate);
            }
            other => panic!("wrong error: {other:?}"),
        }
        let msg = err.to_string();
        assert!(msg.contains("Refusing to start"), "{msg}");
        assert!(
            msg.contains("resampling it twice"),
            "the reason must be legible: {msg}"
        );
        assert!(
            msg.contains("window lengths* are fine"),
            "it must not imply windows are the problem: {msg}"
        );
    }

    /// **Differing windows at the same rate are accepted**, which is the whole
    /// point of the chunk arithmetic. Perch v2 wants 5 s where BirdNET+ V3.0
    /// wants 4.5 s, at 32 kHz both.
    ///
    /// Observed failing with the window comparison restored to `load`: the
    /// two were refused and no station could run them together.
    #[test]
    fn classifiers_wanting_different_windows_at_one_rate_load_together() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Genuinely different windows at one rate, which is the case under
        // test: the V3.0 fixture is 96 000 samples and the V2.4 fixture is
        // 144 000, and declaring the second at 32 kHz puts them on the same
        // rate. That is Perch-beside-BirdNET in the shapes this crate has.
        //
        // An earlier version used the same fixture twice, so both windows were
        // 96 000 and restoring the window comparison refused nothing — the
        // gate passed while proving nothing about its own name.
        let a = spec(dir.path(), "birdnet", TINY_V30, None);
        let mut b = spec(dir.path(), "longer", TINY_V24, None);
        b.sample_rate = Some(32_000);

        let reg = ClassifierRegistry::load(&[a, b], &no_routes(), &ModelConfig::default())
            .expect("one rate, two windows, must load");
        let (longest, shortest) = reg.window_bounds();
        assert_eq!(longest, 144_000, "the V2.4 fixture's window");
        assert_eq!(shortest, 96_000, "the V3.0 fixture's window");
        assert_ne!(longest, shortest, "this gate needs them to differ");
    }

    /// `window_bounds` is what the chunk arithmetic reads, so it must report
    /// the extremes rather than the primary's or the first one's — including
    /// when the longer window belongs to the *second* classifier, which is the
    /// ordering that catches a `bounds` that just returns the primary's.
    #[test]
    fn window_bounds_reports_the_longest_and_shortest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let only = spec(dir.path(), "birdnet", TINY_V30, None);
        let reg = ClassifierRegistry::load(&[only], &no_routes(), &ModelConfig::default())
            .expect("loads");
        assert_eq!(
            reg.window_bounds(),
            (96_000, 96_000),
            "one classifier makes both bounds its own window"
        );

        // Two, with the longer one second.
        let a = spec(dir.path(), "short", TINY_V30, None);
        let mut b = spec(dir.path(), "long", TINY_V24, None);
        b.sample_rate = Some(32_000);
        let reg = ClassifierRegistry::load(&[a, b], &no_routes(), &ModelConfig::default())
            .expect("loads");
        assert_eq!(
            reg.window_bounds(),
            (144_000, 96_000),
            "the longest must be found even when it is not the primary"
        );
    }

    /// Its counterpart: two classifiers wanting the *same* audio load fine, or
    /// the gate above would pass against a registry that refused every second
    /// classifier.
    #[test]
    fn two_classifiers_wanting_the_same_audio_load_together() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = spec(dir.path(), "birdnet", TINY_V30, None);
        let b = spec(dir.path(), "second", TINY_V30, None);
        let reg = ClassifierRegistry::load(&[a, b], &no_routes(), &ModelConfig::default())
            .expect("matching specs load");
        assert_eq!(reg.len(), 2);
    }

    /// **A declared sample rate overrides the shape-derived guess.** Perch v2
    /// declares `[-1, 160_000]`, which the derivation reads as 48 kHz because
    /// 48 kHz is its default for an unrecognised length. It is 32 kHz, and
    /// only the operator can say so.
    ///
    /// Observed failing with the `spec.sample_rate` override dropped from
    /// `load`: the classifier came back at the derived rate and its window
    /// read as 3 s rather than 4.5 s.
    #[test]
    fn a_declared_sample_rate_beats_the_shape_derived_guess() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The V2.4 fixture is [1, 144_000], derived as 48 kHz / 3.0 s.
        let mut s = spec(dir.path(), "birdnet", TINY_V24, None);
        assert_eq!(s.sample_rate, None);
        let derived = ClassifierRegistry::load(&[s.clone()], &no_routes(), &ModelConfig::default())
            .expect("loads");
        assert_eq!(derived.primary().spec.sample_rate, 48_000);
        assert!((derived.primary().spec.window_secs() - 3.0).abs() < 1e-6);

        // Declared as 32 kHz, the same 144 000 samples are 4.5 seconds.
        s.sample_rate = Some(32_000);
        let declared =
            ClassifierRegistry::load(&[s], &no_routes(), &ModelConfig::default()).expect("loads");
        assert_eq!(declared.primary().spec.sample_rate, 32_000);
        assert!(
            (declared.primary().spec.window_secs() - 4.5).abs() < 1e-6,
            "got {}",
            declared.primary().spec.window_secs()
        );
    }

    // ── route parsing ───────────────────────────────────────────────────

    /// The shape an operator writes, including two models on one source.
    #[test]
    fn routes_parse_into_sources_and_targets() {
        let (routes, bad) = parse_routes("pond:perch, garden:birdnet+perch ,shed:birdnet");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(routes["pond"], vec!["perch"]);
        assert_eq!(routes["garden"], vec!["birdnet", "perch"]);
        assert_eq!(routes["shed"], vec!["birdnet"]);
    }

    /// A malformed entry is reported rather than swallowed, and does not take
    /// the sound entries with it — a station must not refuse to start over a
    /// stray comma when the rest of the routing is fine.
    ///
    /// Observed failing with the `bad` vector dropped: the caller had nothing
    /// to warn about and the mistyped source fell back in silence.
    #[test]
    fn a_malformed_route_entry_is_reported_and_the_rest_survive() {
        let (routes, bad) = parse_routes("pond:perch,garbage,garden:birdnet,:,x:");
        assert_eq!(routes["pond"], vec!["perch"]);
        assert_eq!(routes["garden"], vec!["birdnet"]);
        assert!(bad.contains(&"garbage".to_owned()), "{bad:?}");
        assert!(bad.contains(&":".to_owned()), "{bad:?}");
        assert!(bad.contains(&"x:".to_owned()), "{bad:?}");
    }

    #[test]
    fn an_empty_route_setting_is_no_routes_not_an_error() {
        let (routes, bad) = parse_routes("");
        assert!(routes.is_empty());
        assert!(bad.is_empty());
    }

    /// A file that cannot be stated reports `None`, which the caller must read
    /// as "cannot afford it" rather than "costs nothing".
    #[test]
    fn an_unstatable_model_has_no_size_rather_than_a_zero_one() {
        assert_eq!(
            model_size_mib(&PathBuf::from("/nonexistent/model.onnx")),
            None
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("m.onnx");
        std::fs::write(&p, vec![0u8; 3 * 1024 * 1024]).expect("write");
        assert_eq!(model_size_mib(&p), Some(3));
    }
}
