//! Species occurrence frequency filter using a metadata ONNX model.
//!
//! `BirdNET` provides a metadata model that takes `(latitude, longitude, week_number)`
//! as input and outputs a probability vector for each of ~6000 species. Species
//! below the configurable `sf_thresh` threshold are filtered out. A whitelist,
//! include list, and exclude list allow fine-grained control over which species
//! are reported.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use tract_onnx::prelude::*;

use crate::inference::labels::LabelSet;
use crate::inference::model::InferenceError;

/// Configuration for the species occurrence filter.
#[derive(Debug, Clone)]
pub struct SpeciesFilterConfig {
    /// Species frequency threshold (species below this are filtered out).
    /// Default: 0.03.
    pub sf_thresh: f32,
    /// Species that always pass the filter regardless of model output.
    pub whitelist: HashSet<String>,
    /// If non-empty, only species in this list are considered (before threshold).
    pub include_list: Vec<String>,
    /// Species in this list are always excluded.
    pub exclude_list: Vec<String>,
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

/// Tract model plan type alias for the metadata model.
type MetadataModelPlan =
    SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// Species occurrence frequency filter.
///
/// Optionally loads a metadata ONNX model that predicts species occurrence
/// probability given location and time of year. When loaded, only species
/// above the threshold (plus whitelisted species) pass through.
pub struct SpeciesFilter {
    model: Option<MetadataModelPlan>,
    config: SpeciesFilterConfig,
    cache_key: Option<CacheKey>,
    cache_result: Option<HashSet<String>>,
}

impl fmt::Debug for SpeciesFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeciesFilter")
            .field("has_model", &self.model.is_some())
            .field("config", &self.config)
            .field("cached", &self.cache_key.is_some())
            .finish_non_exhaustive()
    }
}

impl SpeciesFilter {
    /// Create a species filter without a metadata model (no filtering).
    pub fn new_passthrough(config: SpeciesFilterConfig) -> Self {
        Self {
            model: None,
            config,
            cache_key: None,
            cache_result: None,
        }
    }

    /// Create a species filter with a metadata ONNX model.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError` if the model file cannot be loaded.
    pub fn load(path: &Path, config: SpeciesFilterConfig) -> Result<Self, InferenceError> {
        if !path.exists() {
            return Err(InferenceError::NotFound(path.display().to_string()));
        }

        tracing::info!(
            path = %path.display(),
            sf_thresh = config.sf_thresh,
            "loading metadata ONNX model for species filtering"
        );

        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(|e| InferenceError::Model(e.to_string()))?
            .into_optimized()
            .map_err(|e| InferenceError::Model(format!("optimization failed: {e}")))?
            .into_runnable()
            .map_err(|e| InferenceError::Model(format!("plan creation failed: {e}")))?;

        tracing::info!("metadata model loaded successfully");

        Ok(Self {
            model: Some(model),
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
    /// If no metadata model is loaded, returns all species from the label set
    /// (minus any in the exclude list, intersected with the include list if set).
    ///
    /// # Errors
    ///
    /// Returns `InferenceError` if metadata model inference fails.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn filter_species(
        &mut self,
        lat: f64,
        lon: f64,
        week: u32,
        labels: &LabelSet,
    ) -> Result<HashSet<String>, InferenceError> {
        let Some(ref model) = self.model else {
            return Ok(self.apply_lists(all_scientific_names(labels)));
        };

        // Check cache
        let key = CacheKey::new(lat, lon, week);
        if let Some(ref cached_key) = self.cache_key {
            if *cached_key == key {
                if let Some(ref cached) = self.cache_result {
                    return Ok(cached.clone());
                }
            }
        }

        // Run metadata model: input shape [1, 3] -> output [1, N]
        let input_data = vec![lat as f32, lon as f32, week as f32];
        let input_tensor = tract_ndarray::Array2::from_shape_vec((1, 3), input_data)
            .map_err(|e| InferenceError::Shape(e.to_string()))?;

        let outputs = model
            .run(tvec![input_tensor.into_tensor().into()])
            .map_err(|e| InferenceError::Runtime(e.to_string()))?;

        let probabilities = outputs[0]
            .to_array_view::<f32>()
            .map_err(|e| InferenceError::Runtime(format!("cannot extract probabilities: {e}")))?;

        let flat = probabilities
            .as_slice()
            .ok_or_else(|| InferenceError::Runtime("probabilities not contiguous".into()))?;

        // Collect species above threshold
        let mut passing = HashSet::new();
        for (i, &prob) in flat.iter().enumerate() {
            if prob >= self.config.sf_thresh {
                if let Some(label) = labels.get(i) {
                    passing.insert(label.scientific_name.clone());
                }
            }
        }

        // Add whitelisted species
        for name in &self.config.whitelist {
            passing.insert(name.clone());
        }

        let result = self.apply_lists(passing);

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

    /// Apply include and exclude lists to a set of species.
    fn apply_lists(&self, mut species: HashSet<String>) -> HashSet<String> {
        // Apply exclude list
        for name in &self.config.exclude_list {
            species.remove(name);
        }

        // Apply include list (if non-empty, intersect)
        if !self.config.include_list.is_empty() {
            let include_set: HashSet<&str> =
                self.config.include_list.iter().map(String::as_str).collect();
            species.retain(|s| include_set.contains(s.as_str()));
        }

        // Always add whitelisted species back (even if excluded or not in include list)
        for name in &self.config.whitelist {
            species.insert(name.clone());
        }

        species
    }

    /// Get the current configuration.
    pub const fn config(&self) -> &SpeciesFilterConfig {
        &self.config
    }

    /// Check if a metadata model is loaded.
    pub const fn has_model(&self) -> bool {
        self.model.is_some()
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
        let result = filter.filter_species(42.0, -71.0, 10, &labels).unwrap();
        assert_eq!(result.len(), 4);
        assert!(result.contains("Turdus merula"));
        assert!(result.contains("Erithacus rubecula"));
        assert!(result.contains("Parus major"));
        assert!(result.contains("Homo sapiens"));
    }

    #[test]
    fn exclude_list_removes_species() {
        let config = SpeciesFilterConfig {
            exclude_list: vec!["Homo sapiens".into()],
            ..SpeciesFilterConfig::default()
        };
        let mut filter = SpeciesFilter::new_passthrough(config);
        let labels = test_labels();
        let result = filter.filter_species(42.0, -71.0, 10, &labels).unwrap();
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
        let result = filter.filter_species(42.0, -71.0, 10, &labels).unwrap();
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
        let result = filter.filter_species(42.0, -71.0, 10, &labels).unwrap();
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
        let result = filter.filter_species(42.0, -71.0, 10, &labels).unwrap();
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
        let _ = filter.filter_species(42.0, -71.0, 10, &labels).unwrap();
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
        let result = SpeciesFilter::load(
            Path::new("/nonexistent/metadata.onnx"),
            SpeciesFilterConfig::default(),
        );
        assert!(matches!(result, Err(InferenceError::NotFound(_))));
    }
}
