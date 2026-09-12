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
}

/// A loaded classifier and what was declared about it.
pub struct RegisteredModel {
    /// The operator's name for it.
    pub id: String,
    /// The classifier itself.
    pub model: BirdNetModel,
    /// Its own threshold, if it was given one.
    pub threshold: Option<f32>,
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
            models.push(RegisteredModel {
                id: spec.id.clone(),
                model,
                threshold: spec.threshold,
            });
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
            .map(|m| (m.id.as_str(), m.model.input_spec()))
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
        let specs = [
            spec(dir.path(), "birdnet", TINY_V24, None),
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
            spec(dir.path(), "birdnet", TINY_V24, None),
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
