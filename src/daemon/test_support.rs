//! Shared fixtures for the daemon submodule unit tests.

use std::collections::HashMap;

/// Build a per-species threshold map from `(sci_name, threshold)` pairs.
pub(super) fn thresholds(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}
