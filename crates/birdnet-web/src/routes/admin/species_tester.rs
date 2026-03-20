//! Species filter tester/preview route.
//!
//! Provides an admin endpoint to test species filter configuration before
//! applying it, so operators can preview which species pass or fail a given
//! include/exclude list and threshold.
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET | `/admin/species/test` | Preview species filter results (JSON) |

use axum::extract::Query;
use axum::response::Json;
use axum::{Router, routing::get};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Build the species tester router.
pub fn router() -> Router<AppState> {
    Router::new().route("/admin/species/test", get(test_species_filter))
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters for the species filter test endpoint.
#[derive(Debug, Deserialize)]
struct FilterParams {
    /// Comma-separated list of species names to include (case-insensitive
    /// substring match). If empty or absent, all species are included.
    #[serde(default)]
    include: Option<String>,
    /// Comma-separated list of species names to exclude (case-insensitive
    /// substring match).
    #[serde(default)]
    exclude: Option<String>,
    /// Species frequency threshold (0.0 – 1.0). Species with a simulated
    /// frequency below this value are filtered out.
    #[serde(default)]
    sf_thresh: Option<f64>,
    /// Latitude (unused in simulation, included for API completeness).
    #[serde(default)]
    lat: Option<f64>,
    /// Longitude (unused in simulation, included for API completeness).
    #[serde(default)]
    lon: Option<f64>,
    /// ISO week number (unused in simulation, included for API completeness).
    #[serde(default)]
    week: Option<u32>,
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// JSON response for the species filter test endpoint.
#[derive(Debug, Serialize)]
struct FilterResult {
    /// Number of species that pass the filter.
    total_species: usize,
    /// First 50 species names that pass the filter.
    species_list: Vec<String>,
    /// Number of species that were filtered out.
    excluded_count: usize,
    /// Human-readable description of the active filter.
    filter_summary: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Test species filter configuration and return a JSON preview.
///
/// ```text
/// GET /admin/species/test?include=Robin,Blackbird&exclude=Sparrow&sf_thresh=0.03&lat=51.5&lon=-0.12&week=11
/// ```
async fn test_species_filter(Query(params): Query<FilterParams>) -> Json<FilterResult> {
    // Parse comma-separated lists into lowercase tokens.
    let include_list = parse_csv(&params.include);
    let exclude_list = parse_csv(&params.exclude);
    let sf_thresh = params.sf_thresh.unwrap_or(0.0).clamp(0.0, 1.0);

    // Load the full label set. In production this would come from the ONNX
    // model labels file; here we use a built-in test set.
    let all_species = built_in_species_list();

    // Apply filters.
    let mut passed: Vec<String> = Vec::new();
    let mut excluded_count: usize = 0;

    for (idx, species) in all_species.iter().enumerate() {
        let lower = species.to_lowercase();

        // Include filter: if an include list is provided the species must
        // match at least one entry.
        if !include_list.is_empty() && !include_list.iter().any(|inc| lower.contains(inc.as_str()))
        {
            excluded_count += 1;
            continue;
        }

        // Exclude filter: if the species matches any exclude entry, skip it.
        if exclude_list.iter().any(|exc| lower.contains(exc.as_str())) {
            excluded_count += 1;
            continue;
        }

        // Simulated species frequency: derive a deterministic pseudo-frequency
        // from the index so the threshold filter is demonstrable without a real
        // model file.
        let simulated_freq = simulated_frequency(idx, all_species.len());
        if simulated_freq < sf_thresh {
            excluded_count += 1;
            continue;
        }

        passed.push(species.clone());
    }

    // Build summary.
    let filter_summary = build_summary(
        &include_list,
        &exclude_list,
        sf_thresh,
        params.lat,
        params.lon,
        params.week,
    );

    let total_species = passed.len();
    let species_list: Vec<String> = passed.into_iter().take(50).collect();

    Json(FilterResult {
        total_species,
        species_list,
        excluded_count,
        filter_summary,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse an optional comma-separated string into a `Vec` of lowercase,
/// trimmed, non-empty tokens.
fn parse_csv(input: &Option<String>) -> Vec<String> {
    input
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Derive a deterministic pseudo-frequency in `[0.0, 1.0]` for a species
/// at position `idx` out of `total` species.
#[allow(clippy::cast_precision_loss)]
fn simulated_frequency(idx: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    // Simple linear ramp so that the first species has the highest frequency
    // and the last has the lowest. This gives a reasonable spread for
    // threshold testing.
    1.0 - (idx as f64 / total as f64)
}

/// Build a human-readable summary of the active filter.
fn build_summary(
    include: &[String],
    exclude: &[String],
    sf_thresh: f64,
    lat: Option<f64>,
    lon: Option<f64>,
    week: Option<u32>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if !include.is_empty() {
        parts.push(format!("include species matching: {}", include.join(", ")));
    }
    if !exclude.is_empty() {
        parts.push(format!("exclude species matching: {}", exclude.join(", ")));
    }
    if sf_thresh > 0.0 {
        parts.push(format!("species frequency threshold >= {sf_thresh:.3}"));
    }
    if let (Some(la), Some(lo)) = (lat, lon) {
        parts.push(format!("location: ({la:.2}, {lo:.2})"));
    }
    if let Some(w) = week {
        parts.push(format!("week: {w}"));
    }

    if parts.is_empty() {
        "no filters applied (showing all species)".to_string()
    } else {
        parts.join("; ")
    }
}

/// Built-in test species list.
///
/// In a production deployment this would be loaded from the BirdNET ONNX
/// model labels file. This representative list covers common species for
/// filter testing.
fn built_in_species_list() -> Vec<String> {
    [
        "American Robin",
        "European Robin",
        "Eurasian Blackbird",
        "House Sparrow",
        "Song Sparrow",
        "Eurasian Tree Sparrow",
        "Great Tit",
        "Blue Tit",
        "Coal Tit",
        "Eurasian Wren",
        "Common Chaffinch",
        "European Goldfinch",
        "Eurasian Bullfinch",
        "Common Blackcap",
        "Willow Warbler",
        "Chiffchaff",
        "European Starling",
        "Common Wood Pigeon",
        "Eurasian Collared Dove",
        "Barn Swallow",
        "House Martin",
        "Common Swift",
        "Eurasian Magpie",
        "Eurasian Jay",
        "Carrion Crow",
        "Common Raven",
        "Eurasian Blue Tit",
        "Long-tailed Tit",
        "Eurasian Nuthatch",
        "Eurasian Treecreeper",
        "European Green Woodpecker",
        "Great Spotted Woodpecker",
        "Common Cuckoo",
        "Tawny Owl",
        "Barn Owl",
        "Common Buzzard",
        "Eurasian Sparrowhawk",
        "Common Kestrel",
        "Grey Heron",
        "Mallard",
        "Mute Swan",
        "Canada Goose",
        "Common Moorhen",
        "Eurasian Coot",
        "Black-headed Gull",
        "Herring Gull",
        "Common Tern",
        "Northern Lapwing",
        "Eurasian Oystercatcher",
        "Common Redshank",
        "Eurasian Curlew",
        "Common Snipe",
        "Eurasian Skylark",
        "Meadow Pipit",
        "Pied Wagtail",
        "Dunnock",
        "European Stonechat",
        "Northern Wheatear",
        "Spotted Flycatcher",
        "Pied Flycatcher",
        "Common Redstart",
        "European Serin",
        "Eurasian Siskin",
        "Common Linnet",
        "Yellowhammer",
        "Reed Bunting",
        "Corn Bunting",
        "Sedge Warbler",
        "Reed Warbler",
        "Garden Warbler",
        "Lesser Whitethroat",
        "Common Whitethroat",
        "Blackbird",
        "Song Thrush",
        "Mistle Thrush",
        "Redwing",
        "Fieldfare",
        "Ring Ouzel",
        "Goldcrest",
        "Firecrest",
        "Spotted Woodpecker",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_empty() {
        assert!(parse_csv(&None).is_empty());
        assert!(parse_csv(&Some(String::new())).is_empty());
    }

    #[test]
    fn parse_csv_values() {
        let input = Some("Robin, Blackbird , sparrow".to_string());
        let result = parse_csv(&input);
        assert_eq!(result, vec!["robin", "blackbird", "sparrow"]);
    }

    #[test]
    fn simulated_frequency_range() {
        let f0 = simulated_frequency(0, 100);
        let f99 = simulated_frequency(99, 100);
        assert!(f0 > f99);
        assert!(f0 <= 1.0);
        assert!(f99 >= 0.0);
    }

    #[test]
    fn summary_no_filters() {
        let s = build_summary(&[], &[], 0.0, None, None, None);
        assert!(s.contains("no filters"));
    }

    #[test]
    fn summary_with_filters() {
        let s = build_summary(
            &["robin".to_string()],
            &["sparrow".to_string()],
            0.03,
            Some(51.5),
            Some(-0.12),
            Some(11),
        );
        assert!(s.contains("robin"));
        assert!(s.contains("sparrow"));
        assert!(s.contains("0.030"));
        assert!(s.contains("51.50"));
        assert!(s.contains("week: 11"));
    }
}
