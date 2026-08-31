//! Species occurrence frequency filter using a metadata ONNX model.
//!
//! `BirdNET` provides a metadata model (the "geomodel") that takes
//! `(latitude, longitude, week_number)` as input and returns an occurrence
//! probability for each species it knows about. Species below the configurable
//! `sf_thresh` threshold are filtered out. A whitelist, include list, and
//! exclude list allow fine-grained control over which species are reported.
//!
//! # Two vocabularies, not one
//!
//! The metadata model and the classifier do not score the same species list.
//! BirdNET Geomodel v3.0 covers 12 012 species; the V3.0 Global 11K classifier
//! emits 11 560. So an output *index* from one is meaningless to the other, and
//! the model's own label file is the only thing that says which species a given
//! output belongs to.
//!
//! [`SpeciesFilter::load_with_vocabulary`] therefore takes the metadata model's
//! labels alongside it and matches by scientific name; when no such file is
//! given it requires the two vocabularies to be the same width and refuses the
//! model otherwise. Reading one list's index into the other is exactly the bug
//! this guard exists to prevent: it is silent, and it reports one bird as
//! another with full confidence.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use ort::session::Session;
use ort::value::{Tensor, ValueType};

use crate::inference::labels::{LabelSet, SpeciesLabel};
use crate::inference::model::InferenceError;

/// Configuration for the species occurrence filter.
///
/// The three name lists are **operator intent**, entered by hand; the
/// `sf_thresh` is the metadata model's. That distinction matters in two places:
/// the lists are matched leniently (see [`matches_species`]) because a person
/// typed them, and they apply whether or not the station has coordinates,
/// because only the model needs to know where it is.
#[derive(Debug, Clone)]
pub struct SpeciesFilterConfig {
    /// Species frequency threshold (species below this are filtered out).
    /// Default: 0.03.
    pub sf_thresh: f32,
    /// Species that always pass the filter regardless of model output.
    ///
    /// Entries may be a common or a scientific name.
    pub whitelist: HashSet<String>,
    /// If non-empty, only species in this list are considered (before threshold).
    ///
    /// Entries may be a common or a scientific name.
    pub include_list: Vec<String>,
    /// Species in this list are always excluded.
    ///
    /// Entries may be a common or a scientific name.
    pub exclude_list: Vec<String>,
}

/// Whether an operator-entered list `entry` names the species `label`.
///
/// Matches the **common or the scientific** name, case-insensitively, ignoring
/// surrounding whitespace. Both halves of that matter in the field:
///
/// * `/admin/species` collects *common* names ("Add species common name") while
///   the filter works in *scientific* names, so a strict scientific-name
///   comparison would have matched nothing an operator could actually enter —
///   the list would fill up, the page would confirm each addition, and every
///   excluded bird would keep being recorded.
/// * The per-species *threshold* control immediately below it on the same page
///   takes scientific names, so operators reasonably type either.
///
/// Exposed so the `/admin/species/test` preview and the detection path decide
/// with the same function rather than two implementations that can drift — the
/// preview is offered as "preview the filter before it affects live
/// detections", which is only true if it is the same predicate.
#[must_use]
pub fn matches_species(entry: &str, label: &SpeciesLabel) -> bool {
    let entry = entry.trim();
    !entry.is_empty()
        && (entry.eq_ignore_ascii_case(label.common_name.trim())
            || entry.eq_ignore_ascii_case(label.scientific_name.trim()))
}

/// The operator's include/exclude species lists.
///
/// Entries may be common or scientific names — see [`matches_species`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeciesLists {
    /// If non-empty, only these species are reported.
    pub include: Vec<String>,
    /// These species are never reported.
    pub exclude: Vec<String>,
}

/// A callback the daemon re-reads on a TTL to pick up list changes.
///
/// The lists live in the application's settings database, which `birdnet-core`
/// must not depend on, so the query is injected as this callback — the same
/// shape as [`crate::audio::capture::LockedFilesProvider`], for the same
/// reason.
///
/// It exists at all because excluding a species is something an operator does
/// *because it is spamming them right now*: a snapshot taken when the daemon
/// started would mean the change did nothing until the next service restart,
/// with the page confirming the save and the bird still arriving. That is the
/// trap the startup snapshots of per-species thresholds and locked clips both
/// set before they were made to refresh.
#[derive(Clone)]
pub struct SpeciesListsProvider(std::sync::Arc<dyn Fn() -> SpeciesLists + Send + Sync>);

impl fmt::Debug for SpeciesListsProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SpeciesListsProvider(<callback>)")
    }
}

impl SpeciesListsProvider {
    /// Wrap a closure that reads the current lists.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn() -> SpeciesLists + Send + Sync + 'static,
    {
        Self(std::sync::Arc::new(f))
    }

    /// Read the current lists.
    #[must_use]
    pub fn get(&self) -> SpeciesLists {
        (self.0)()
    }
}

/// Resolve operator-entered names to the scientific names the filter works in.
///
/// An entry that matches nothing in the label set is dropped: it names a
/// species this model cannot predict, so it can neither be excluded nor
/// included, and carrying it forward would only make an include list wrongly
/// narrow.
fn resolve_to_scientific<'a, I>(entries: I, labels: &LabelSet) -> HashSet<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut out = HashSet::new();
    for entry in entries {
        for label in labels.iter() {
            if matches_species(entry, label) {
                out.insert(label.scientific_name.clone());
            }
        }
    }
    out
}

impl Default for SpeciesFilterConfig {
    fn default() -> Self {
        Self {
            sf_thresh: 0.03,
            whitelist: HashSet::new(),
            include_list: Vec::new(),
            exclude_list: Vec::new(),
        }
    }
}

/// Cache key for metadata model results.
#[derive(Debug, Clone, PartialEq)]
struct CacheKey {
    lat: u64,
    lon: u64,
    week: u32,
}

impl CacheKey {
    const fn new(lat: f64, lon: f64, week: u32) -> Self {
        Self {
            lat: lat.to_bits(),
            lon: lon.to_bits(),
            week,
        }
    }
}

/// Species occurrence frequency filter.
///
/// Optionally loads a metadata ONNX model that predicts species occurrence
/// probability given location and time of year. When loaded, only species
/// above the threshold (plus whitelisted species) pass through.
pub struct SpeciesFilter {
    session: Option<Session>,
    /// The metadata model's *own* species vocabulary, when it has one.
    ///
    /// The BirdNET geomodel scores 12 012 species; the V3.0 classifier emits
    /// 11 560. The two lists are neither the same length nor the same order,
    /// so the model's output index means nothing to the classifier — only the
    /// scientific name at that index does. `None` means the caller asserted
    /// the two vocabularies are index-identical, which
    /// [`SpeciesFilter::load_with_vocabulary`] verifies against the model's
    /// declared output width before accepting it.
    meta_labels: Option<LabelSet>,
    /// How many species the loaded model is expected to score: the metadata
    /// label count when there is one, otherwise the classifier's. Re-checked
    /// against the actual output on every inference, because a model that
    /// declares a dynamic output width cannot be checked at load.
    expected_width: Option<usize>,
    config: SpeciesFilterConfig,
    cache_key: Option<CacheKey>,
    cache_result: Option<HashSet<String>>,
}

impl fmt::Debug for SpeciesFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeciesFilter")
            .field("has_model", &self.session.is_some())
            .field(
                "meta_species",
                &self.meta_labels.as_ref().map(LabelSet::len),
            )
            .field("config", &self.config)
            .field("cached", &self.cache_key.is_some())
            .finish_non_exhaustive()
    }
}

impl SpeciesFilter {
    /// Create a species filter without a metadata model (no filtering).
    pub const fn new_passthrough(config: SpeciesFilterConfig) -> Self {
        Self {
            session: None,
            meta_labels: None,
            expected_width: None,
            config,
            cache_key: None,
            cache_result: None,
        }
    }

    /// Load a metadata (occurrence / "geo") ONNX model, refusing one whose
    /// vocabulary cannot be aligned with the classifier's.
    ///
    /// The model takes `(latitude, longitude, week)` and returns one
    /// probability per species it knows about. Two vocabularies are therefore
    /// in play, and they are not the same list:
    ///
    /// * `meta_labels` — the species the *metadata model* scores, in its own
    ///   order. The BirdNET geomodel ships its own label file for exactly this
    ///   reason: it covers 12 012 species where the V3.0 classifier emits
    ///   11 560. Pass `Some(..)` and the outputs are resolved through it and
    ///   matched to the classifier by **scientific name**.
    /// * `classifier_species` — how many species the classifier emits. Pass
    ///   `meta_labels: None` only when the model is known to be indexed
    ///   identically to the classifier (a matched BirdNET pair, e.g. a V2.4
    ///   `MData` model beside V2.4 labels).
    ///
    /// Whichever mode is asked for, the model's declared output width must
    /// equal the number of species that mode expects, or the model is
    /// rejected here. It used to be accepted and its outputs read positionally
    /// against whatever the classifier happened to have, which quietly
    /// admitted and rejected birds under other birds' names — the failure was
    /// invisible in the logs and looked like a bad classifier.
    ///
    /// A model that declares a *dynamic* output width cannot be checked at
    /// load; [`Self::filter_species`] re-checks the real width on every
    /// inference, so such a model fails on its first prediction rather than
    /// mislabelling one.
    ///
    /// # Errors
    ///
    /// * [`InferenceError::NotFound`] — no file at `path`.
    /// * [`InferenceError::Model`] — the file is not a loadable ONNX model.
    /// * [`InferenceError::Shape`] — the model has no outputs, or its output
    ///   width disagrees with the vocabulary it was given.
    pub fn load_with_vocabulary(
        path: &Path,
        meta_labels: Option<LabelSet>,
        classifier_species: usize,
        config: SpeciesFilterConfig,
    ) -> Result<Self, InferenceError> {
        if !path.exists() {
            return Err(InferenceError::NotFound(path.display().to_string()));
        }

        tracing::info!(
            path = %path.display(),
            sf_thresh = config.sf_thresh,
            "loading metadata ONNX model for species occurrence filtering"
        );

        let session = Session::builder()
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .commit_from_file(path)
            .map_err(|e| InferenceError::Model(e.to_string()))?;

        let expected_width = meta_labels
            .as_ref()
            .map_or(classifier_species, LabelSet::len);
        let declared = declared_output_width(&session)?;

        if let Some(actual) = declared
            && actual != expected_width
        {
            return Err(InferenceError::Shape(vocabulary_mismatch_message(
                actual,
                expected_width,
                meta_labels.is_some(),
            )));
        }

        tracing::info!(
            meta_species = meta_labels.as_ref().map(LabelSet::len),
            classifier_species,
            declared_outputs = declared,
            matching = if meta_labels.is_some() {
                "by scientific name, through the model's own labels"
            } else {
                "by index, against the classifier's labels"
            },
            "metadata model loaded; species occurrence filtering is active"
        );

        Ok(Self {
            session: Some(session),
            meta_labels,
            expected_width: Some(expected_width),
            config,
            cache_key: None,
            cache_result: None,
        })
    }

    /// Filter species based on location and week.
    ///
    /// Runs the metadata model with `(lat, lon, week)` and returns the set of
    /// scientific names that pass the threshold. Results are cached for
    /// identical `(lat, lon, week)` inputs.
    ///
    /// `location` is `None` when the station has no coordinates configured. The
    /// metadata model cannot run without them, so its occurrence filtering is
    /// skipped — but the operator's include/exclude lists still apply, because
    /// they are an explicit instruction that has nothing to do with where the
    /// station is. Gating the whole filter on coordinates (which the caller used
    /// to do) meant a station that never set a latitude kept recording every
    /// species its operator had asked to suppress.
    ///
    /// Likewise when no metadata model is loaded: all species pass the model
    /// stage, and the lists still apply.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError` if metadata model inference fails.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn filter_species(
        &mut self,
        location: Option<(f64, f64)>,
        week: u32,
        labels: &LabelSet,
    ) -> Result<HashSet<String>, InferenceError> {
        let (Some((lat, lon)), Some(session)) = (location, self.session.as_mut()) else {
            return Ok(self.apply_lists(all_scientific_names(labels), labels));
        };

        // Check cache
        let key = CacheKey::new(lat, lon, week);
        if let Some(ref cached_key) = self.cache_key
            && *cached_key == key
            && let Some(ref cached) = self.cache_result
        {
            return Ok(cached.clone());
        }

        // Run metadata model: input shape [1, 3] -> output [1, N]
        let input_data = vec![lat as f32, lon as f32, week as f32];
        let input_tensor = Tensor::<f32>::from_array(([1usize, 3], input_data))
            .map_err(|e| InferenceError::Shape(e.to_string()))?;

        let outputs = session
            .run(ort::inputs![input_tensor])
            .map_err(|e| InferenceError::Runtime(e.to_string()))?;

        // Collect probabilities into a Vec to release the borrow on session/outputs.
        let probabilities: Vec<f32> = {
            let (_, flat) = outputs[0].try_extract_tensor::<f32>().map_err(|e| {
                InferenceError::Runtime(format!("cannot extract probabilities: {e}"))
            })?;
            flat.to_vec()
        };
        drop(outputs);

        // The model's output width is the last chance to catch a vocabulary it
        // was never meant to score. `load_with_vocabulary` checks the declared
        // width, but a model with a dynamic output dimension declares nothing,
        // so the real width is only knowable here. Erroring is the point: the
        // alternative is reading species `i` of one list as species `i` of
        // another, which produces confident detections of the wrong birds.
        if let Some(expected) = self.expected_width
            && probabilities.len() != expected
        {
            return Err(InferenceError::Shape(vocabulary_mismatch_message(
                probabilities.len(),
                expected,
                self.meta_labels.is_some(),
            )));
        }

        // Collect species above threshold.
        //
        // Which list the index refers to depends on how the model was loaded.
        // With the model's own labels the index names a species in *its*
        // vocabulary, and only the ones the classifier can also emit are worth
        // carrying forward — a geomodel entry the classifier has never heard of
        // can never be detected, and letting it through would put a name in the
        // passing set that no detection can ever match.
        let mut passing = HashSet::new();
        let vocabulary = self.meta_labels.as_ref().unwrap_or(labels);
        let by_name = self.meta_labels.is_some();
        for (i, &prob) in probabilities.iter().enumerate() {
            if prob < self.config.sf_thresh {
                continue;
            }
            let Some(label) = vocabulary.get(i) else {
                continue;
            };
            if by_name
                && labels
                    .find_by_scientific_name(&label.scientific_name)
                    .is_none()
            {
                continue;
            }
            passing.insert(label.scientific_name.clone());
        }

        // Add whitelisted species. Resolved through the label set so an entry
        // typed as a common name lands as the scientific name the rest of the
        // pipeline compares against.
        passing.extend(resolve_to_scientific(self.config.whitelist.iter(), labels));

        let result = self.apply_lists(passing, labels);

        tracing::debug!(
            lat,
            lon,
            week,
            passing_count = result.len(),
            total_labels = labels.len(),
            "species filter applied"
        );

        // Cache the result
        self.cache_key = Some(key);
        self.cache_result = Some(result.clone());

        Ok(result)
    }

    /// Apply include and exclude lists to a set of scientific names.
    ///
    /// `labels` is needed to resolve the operator's entries, which may be
    /// common or scientific names, to the scientific names `species` holds.
    fn apply_lists(&self, mut species: HashSet<String>, labels: &LabelSet) -> HashSet<String> {
        // Apply exclude list.
        let exclude = resolve_to_scientific(self.config.exclude_list.iter(), labels);
        species.retain(|s| !exclude.contains(s));

        // Apply include list (if non-empty, intersect).
        //
        // An include list that resolves to nothing is treated as no include
        // list at all. Otherwise a single typo — a name matching no label —
        // would silently suppress *every* species on the station, which is the
        // most destructive possible reading of an operator's typo.
        if !self.config.include_list.is_empty() {
            let include = resolve_to_scientific(self.config.include_list.iter(), labels);
            if include.is_empty() {
                tracing::warn!(
                    entries = self.config.include_list.len(),
                    "species include list matches no known species; ignoring it rather than \
                     suppressing every detection"
                );
            } else {
                species.retain(|s| include.contains(s));
            }
        }

        // Always add whitelisted species back (even if excluded or not in include list).
        species.extend(resolve_to_scientific(self.config.whitelist.iter(), labels));

        species
    }

    /// Replace the operator's include/exclude lists at runtime.
    ///
    /// Invalidates the cached model result, since the cache holds the
    /// post-list species set. Used by the daemon to pick up a change made on
    /// `/admin/species` without a restart — excluding a species is something an
    /// operator does *because* it is spamming them right now, so waiting for the
    /// next service restart is the wrong answer.
    pub fn set_lists(&mut self, include: Vec<String>, exclude: Vec<String>) {
        self.config.include_list = include;
        self.config.exclude_list = exclude;
        self.invalidate_cache();
    }

    /// Get the current configuration.
    pub const fn config(&self) -> &SpeciesFilterConfig {
        &self.config
    }

    /// Check if a metadata model is loaded.
    pub const fn has_model(&self) -> bool {
        self.session.is_some()
    }

    /// Update the species frequency threshold.
    pub fn set_sf_thresh(&mut self, thresh: f32) {
        self.config.sf_thresh = thresh;
        // Invalidate cache when threshold changes
        self.cache_key = None;
        self.cache_result = None;
    }

    /// Invalidate the cached filter result.
    pub fn invalidate_cache(&mut self) {
        self.cache_key = None;
        self.cache_result = None;
    }
}

/// The declared width of a session's first output, when it is static.
///
/// Returns `None` for a dynamic (symbolic, `-1`) trailing dimension — a model
/// that does not commit to a species count cannot be checked before it runs.
/// The species axis is the last dimension: the geomodel's output is
/// `[batch, species]`.
fn declared_output_width(session: &Session) -> Result<Option<usize>, InferenceError> {
    let output = session
        .outputs()
        .first()
        .ok_or_else(|| InferenceError::Shape("metadata model has no outputs".into()))?;

    match output.dtype() {
        ValueType::Tensor { shape, .. } => Ok(shape
            .last()
            .copied()
            .filter(|d| *d > 0)
            .and_then(|d| usize::try_from(d).ok())),
        other => Err(InferenceError::Shape(format!(
            "expected a Tensor output from the metadata model, got {other:?}"
        ))),
    }
}

/// The operator-facing explanation of a metadata-model vocabulary mismatch.
///
/// Both counts appear because the number alone does not say which file is
/// wrong, and the remedy differs: a model that scores its own vocabulary needs
/// its label file supplied, while a mismatched *pair* needs one of the two
/// files replaced. Split out from the two call sites so the wording is checked
/// once and cannot drift between the load-time and inference-time guards.
fn vocabulary_mismatch_message(actual: usize, expected: usize, has_meta_labels: bool) -> String {
    let against = if has_meta_labels {
        "the metadata label file supplied beside it"
    } else {
        "the classifier's label set"
    };
    let remedy = if has_meta_labels {
        "the label file does not describe this model; supply the label file it shipped with"
    } else {
        "this model scores its own species list, so it needs its own label file \
         (BIRDNET_METADATA_LABELS / METADATA_LABELS_PATH) to be matched by name"
    };
    format!("metadata model scores {actual} species but {against} has {expected}: {remedy}")
}

/// Collect all scientific names from a label set.
fn all_scientific_names(labels: &LabelSet) -> HashSet<String> {
    labels.iter().map(|l| l.scientific_name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_labels() -> LabelSet {
        LabelSet::from_entries(vec![
            ("Turdus merula".into(), "Eurasian Blackbird".into()),
            ("Erithacus rubecula".into(), "European Robin".into()),
            ("Parus major".into(), "Great Tit".into()),
            ("Homo sapiens".into(), "Human".into()),
        ])
    }

    #[test]
    fn passthrough_returns_all_species() {
        let config = SpeciesFilterConfig::default();
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 4);
        assert!(result.contains("Turdus merula"));
        assert!(result.contains("Erithacus rubecula"));
        assert!(result.contains("Parus major"));
        assert!(result.contains("Homo sapiens"));
    }

    // ── operator lists are matched the way an operator types them ───────
    //
    // `/admin/species` collects *common* names; the filter works in
    // *scientific* names. A strict scientific-name comparison — which is what
    // this did — matched nothing an operator could enter through the UI, so
    // the list filled up, the page confirmed each addition, and every excluded
    // bird kept being recorded.

    #[test]
    fn exclude_matches_a_common_name_as_typed_in_the_ui() {
        let config = SpeciesFilterConfig {
            exclude_list: vec!["Human".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert!(
            !result.contains("Homo sapiens"),
            "an exclude entered as a common name must suppress the species"
        );
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn exclude_matching_is_case_and_whitespace_insensitive() {
        for entry in ["  human  ", "HUMAN", "homo sapiens", " Homo Sapiens "] {
            let config = SpeciesFilterConfig {
                exclude_list: vec![entry.into()],
                ..SpeciesFilterConfig::default()
            };
            let mut filter = SpeciesFilter::new_passthrough(config);
            let labels = test_labels();
            let result = filter
                .filter_species(Some((42.0, -71.0)), 10, &labels)
                .unwrap();
            assert!(
                !result.contains("Homo sapiens"),
                "{entry:?} should have matched"
            );
        }
    }

    #[test]
    fn include_matches_either_name_form() {
        // One common name, one scientific — both must land.
        let config = SpeciesFilterConfig {
            include_list: vec!["Great Tit".into(), "Turdus merula".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains("Parus major"));
        assert!(result.contains("Turdus merula"));
    }

    #[test]
    fn an_include_list_that_matches_nothing_is_ignored() {
        // The destructive reading of a typo: intersecting with an empty set
        // would suppress every detection the station makes. One misspelt name
        // must not silence the whole station.
        let config = SpeciesFilterConfig {
            include_list: vec!["Turdus merulaa".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 4, "a typo must not suppress everything");
    }

    #[test]
    fn an_unmatched_exclude_entry_is_harmless() {
        let config = SpeciesFilterConfig {
            exclude_list: vec!["Nonexistent bird".into(), "Human".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 3);
        assert!(!result.contains("Homo sapiens"));
    }

    #[test]
    fn blank_entries_match_nothing() {
        // A trailing comma in the settings value produces an empty entry; it
        // must not match every species and blank the station.
        let config = SpeciesFilterConfig {
            exclude_list: vec![String::new(), "   ".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 4);
    }

    // ── lists apply with no coordinates ─────────────────────────────────

    #[test]
    fn lists_apply_without_a_location() {
        // The metadata model needs coordinates; the operator's instruction does
        // not. A station that never set a latitude used to keep recording every
        // species its operator had excluded.
        let config = SpeciesFilterConfig {
            exclude_list: vec!["Human".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter.filter_species(None, 10, &labels).unwrap();
        assert!(!result.contains("Homo sapiens"));
        assert_eq!(result.len(), 3);
    }

    // ── live reload ─────────────────────────────────────────────────────

    #[test]
    fn set_lists_takes_effect_immediately() {
        let mut filter = SpeciesFilter::new_passthrough(SpeciesFilterConfig::default());
        let labels = test_labels();
        assert_eq!(
            filter
                .filter_species(Some((42.0, -71.0)), 10, &labels)
                .unwrap()
                .len(),
            4
        );

        filter.set_lists(Vec::new(), vec!["Human".into()]);
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert!(
            !result.contains("Homo sapiens"),
            "a list change must apply to the very next file, not the next restart"
        );
    }

    #[test]
    fn matches_species_predicate_is_shared_and_strict_about_blanks() {
        let label = SpeciesLabel {
            index: 0,
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            class: None,
        };
        assert!(matches_species("Eurasian Blackbird", &label));
        assert!(matches_species("turdus merula", &label));
        assert!(matches_species("  Turdus Merula ", &label));
        assert!(!matches_species("", &label));
        assert!(!matches_species("   ", &label));
        assert!(!matches_species("Turdus", &label), "no partial matching");
    }

    #[test]
    fn exclude_list_removes_species() {
        let config = SpeciesFilterConfig {
            exclude_list: vec!["Homo sapiens".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 3);
        assert!(!result.contains("Homo sapiens"));
    }

    #[test]
    fn include_list_limits_species() {
        let config = SpeciesFilterConfig {
            include_list: vec!["Turdus merula".into(), "Parus major".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains("Turdus merula"));
        assert!(result.contains("Parus major"));
    }

    #[test]
    fn whitelist_always_passes() {
        let config = SpeciesFilterConfig {
            include_list: vec!["Turdus merula".into()],
            whitelist: HashSet::from(["Parus major".into()]),
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains("Turdus merula"));
        assert!(result.contains("Parus major"));
    }

    #[test]
    fn whitelist_overrides_exclude() {
        let config = SpeciesFilterConfig {
            exclude_list: vec!["Turdus merula".into()],
            whitelist: HashSet::from(["Turdus merula".into()]),
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        assert!(result.contains("Turdus merula"));
    }

    #[test]
    fn default_config_has_correct_threshold() {
        let config = SpeciesFilterConfig::default();
        assert!((config.sf_thresh - 0.03).abs() < f32::EPSILON);
        assert!(config.whitelist.is_empty());
        assert!(config.include_list.is_empty());
        assert!(config.exclude_list.is_empty());
    }

    #[test]
    fn cache_key_equality() {
        let k1 = CacheKey::new(42.0, -71.0, 10);
        let k2 = CacheKey::new(42.0, -71.0, 10);
        let k3 = CacheKey::new(42.0, -71.0, 11);
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn set_sf_thresh_invalidates_cache() {
        let config = SpeciesFilterConfig::default();
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let _ = filter
            .filter_species(Some((42.0, -71.0)), 10, &labels)
            .unwrap();
        filter.set_sf_thresh(0.05);
        assert!(filter.cache_key.is_none());
        assert!(filter.cache_result.is_none());
    }

    #[test]
    fn has_model_without_model() {
        let filter = SpeciesFilter::new_passthrough(SpeciesFilterConfig::default());
        assert!(!filter.has_model());
    }

    #[test]
    fn load_nonexistent_model_returns_error() {
        let result = SpeciesFilter::load_with_vocabulary(
            Path::new("/nonexistent/metadata.onnx"),
            None,
            6522,
            SpeciesFilterConfig::default(),
        );
        assert!(matches!(result, Err(InferenceError::NotFound(_))));
    }
}

// ── the metadata model's vocabulary is its own ──────────────────────────
//
// These gates were written against the pre-fix code and observed failing.
// Before the fix `filter_species` mapped the metadata model's output index
// straight onto the classifier's label index with no check that the two
// vocabularies were the same size, let alone the same species. The tiny
// metadata model below scores five species in an order deliberately unlike
// the classifier's four, so index-mapping and name-mapping cannot agree:
//
//   meta idx | meta species        | p      | classifier idx | classifier species
//   ---------+---------------------+--------+----------------+-------------------
//        0   | Pica pica           | 0.9002 |       0        | Turdus merula
//        1   | Parus major         | 0.0998 |       1        | Erithacus rubecula
//        2   | Corvus corax        | 0.8022 |       2        | Parus major
//        3   | Turdus merula       | 0.0474 |       3        | Homo sapiens
//        4   | Erithacus rubecula  | 0.7006 |      --        |
//
// At `sf_thresh = 0.5` the correct answer is {Erithacus rubecula}; the old
// index mapping produced {Turdus merula, Parus major}. Disjoint, so a test
// that asserts either one cannot pass by accident on the other.
#[cfg(test)]
mod metadata_vocabulary_tests {
    use super::*;

    /// A 291-byte ONNX model: `Sigmoid(MatMul(input[1,3], zeros[3,5]) + B)`.
    /// The zero weight matrix makes the output exactly `sigmoid(B)` — fixed,
    /// independent of `(lat, lon, week)` — while keeping the real `[1, 3]`
    /// input contract the BirdNET geomodel uses. Generated once with Python's
    /// `onnx` library, like the tiny classifier models beside it.
    const TINY_META_MODEL: &[u8] = include_bytes!("../testdata/tiny_meta_test.onnx");

    /// The classifier's four species. Same set as `tests::test_labels`.
    fn classifier_labels() -> LabelSet {
        LabelSet::from_entries(vec![
            ("Turdus merula".into(), "Eurasian Blackbird".into()),
            ("Erithacus rubecula".into(), "European Robin".into()),
            ("Parus major".into(), "Great Tit".into()),
            ("Homo sapiens".into(), "Human".into()),
        ])
    }

    /// The metadata model's own five species, in its own order.
    fn meta_labels() -> LabelSet {
        LabelSet::from_entries(vec![
            ("Pica pica".into(), "Eurasian Magpie".into()),
            ("Parus major".into(), "Great Tit".into()),
            ("Corvus corax".into(), "Common Raven".into()),
            ("Turdus merula".into(), "Eurasian Blackbird".into()),
            ("Erithacus rubecula".into(), "European Robin".into()),
        ])
    }

    fn write_model(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let p = dir.path().join("meta.onnx");
        std::fs::write(&p, TINY_META_MODEL).unwrap();
        p
    }

    fn config() -> SpeciesFilterConfig {
        SpeciesFilterConfig {
            sf_thresh: 0.5,
            ..SpeciesFilterConfig::default()
        }
    }

    /// The gate for the defect: a metadata model with its own vocabulary is
    /// resolved through its own labels and matched to the classifier by
    /// scientific name.
    ///
    /// Fails on the old code with `{Turdus merula, Parus major}` — the
    /// species sitting at the passing *indices* rather than the species the
    /// model actually scored.
    #[test]
    fn own_vocabulary_is_resolved_by_name_not_by_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut filter = SpeciesFilter::load_with_vocabulary(
            &write_model(&dir),
            Some(meta_labels()),
            4,
            config(),
        )
        .expect("a metadata model with matching labels must load");

        let passing = filter
            .filter_species(Some((42.0, -71.0)), 10, &classifier_labels())
            .unwrap();

        assert_eq!(
            passing,
            HashSet::from(["Erithacus rubecula".to_owned()]),
            "only the species the metadata model scored above threshold AND the \
             classifier can emit may pass; got {passing:?}"
        );
    }

    /// The counterpart: species the metadata model scores but the classifier
    /// cannot emit are dropped rather than carried through under some other
    /// name. Without this the test above would also pass on an implementation
    /// that simply returned every metadata name above threshold.
    #[test]
    fn species_outside_the_classifier_vocabulary_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut filter = SpeciesFilter::load_with_vocabulary(
            &write_model(&dir),
            Some(meta_labels()),
            4,
            config(),
        )
        .unwrap();

        let passing = filter
            .filter_species(Some((42.0, -71.0)), 10, &classifier_labels())
            .unwrap();

        assert!(
            !passing.contains("Pica pica") && !passing.contains("Corvus corax"),
            "Pica pica (0.90) and Corvus corax (0.80) clear the threshold but are \
             not in the classifier's label set, so neither may appear: {passing:?}"
        );
    }

    /// Without a metadata label file the only sound reading of the model's
    /// outputs is "same index as the classifier", which is only true when the
    /// two vocabularies are the same size. A five-output model against a
    /// four-species classifier must be refused at load, not index-mapped.
    ///
    /// Fails on the old code: `SpeciesFilter::load` accepted any model.
    #[test]
    fn width_mismatch_without_labels_is_refused_at_load() {
        let dir = tempfile::tempdir().unwrap();
        let err = SpeciesFilter::load_with_vocabulary(&write_model(&dir), None, 4, config())
            .expect_err("a 5-output model must not be accepted for a 4-species classifier");

        assert!(
            matches!(err, InferenceError::Shape(_)),
            "expected a shape error naming the mismatch, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains('5') && msg.contains('4'),
            "the error must name both widths so an operator can see which file is \
             wrong: {msg}"
        );
    }

    /// The counterpart to the refusal: a metadata model whose width *does*
    /// match the classifier keeps working without a label file, so the guard
    /// is a discriminator rather than a blanket refusal of label-less models.
    #[test]
    fn matching_width_without_labels_still_loads_and_index_maps() {
        let dir = tempfile::tempdir().unwrap();
        // Five classifier species, so the model's five outputs line up.
        let five = LabelSet::from_entries(
            (0..5)
                .map(|i| (format!("Genus species{i}"), format!("Bird {i}")))
                .collect(),
        );
        let mut filter = SpeciesFilter::load_with_vocabulary(&write_model(&dir), None, 5, config())
            .expect("matching widths must load");

        let passing = filter
            .filter_species(Some((42.0, -71.0)), 10, &five)
            .unwrap();

        // sigmoid(B) = [0.9002, 0.0998, 0.8022, 0.0474, 0.7006]; indices 0, 2, 4
        // clear 0.5.
        assert_eq!(
            passing,
            HashSet::from([
                "Genus species0".to_owned(),
                "Genus species2".to_owned(),
                "Genus species4".to_owned(),
            ]),
            "got {passing:?}"
        );
    }

    /// A metadata label file that does not describe the model it accompanies
    /// is the same class of error as a mismatched classifier, and must be
    /// caught at load rather than silently mislabelling every prediction.
    #[test]
    fn labels_that_do_not_match_the_model_width_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let wrong = LabelSet::from_entries(vec![("Pica pica".into(), "Eurasian Magpie".into())]);
        let err = SpeciesFilter::load_with_vocabulary(&write_model(&dir), Some(wrong), 4, config())
            .expect_err("1 label against a 5-output model must be refused");
        assert!(matches!(err, InferenceError::Shape(_)), "got {err:?}");
    }

    /// `has_model` must reflect a real, aligned model — the daemon and the
    /// diagnostics both read it to decide whether to say occurrence filtering
    /// is on.
    #[test]
    fn has_model_is_true_only_for_a_loaded_aligned_model() {
        let dir = tempfile::tempdir().unwrap();
        let filter = SpeciesFilter::load_with_vocabulary(
            &write_model(&dir),
            Some(meta_labels()),
            4,
            config(),
        )
        .unwrap();
        assert!(filter.has_model());
        assert!(!SpeciesFilter::new_passthrough(config()).has_model());
    }
}
