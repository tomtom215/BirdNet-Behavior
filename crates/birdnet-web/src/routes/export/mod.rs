//! Export endpoints for bulk data download.
//!
//! Provides CSV and JSON export of detection data, compatible with the
//! original BirdNET-Pi `BirdDB.txt` CSV format.
//!
//! | Module    | Responsibility                            |
//! |-----------|-------------------------------------------|
//! | `csv`     | CSV/JSON detection and species export     |
//! | `ebird`   | eBird-compatible CSV export               |
//! | `birddb`  | BirdNET-Pi BirdDB.txt legacy export       |

mod birddb;
mod csv;
mod ebird;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Export routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections/export", get(csv::export_detections))
        .route("/species/export", get(csv::export_species))
        .route("/detections/export/ebird", get(ebird::export_ebird))
        .route("/detections/export/birddb", get(birddb::export_birddb))
}

/// Escape a value for CSV output (RFC 4180).
///
/// Wraps the value in double quotes if it contains commas, quotes, or newlines.
pub(crate) fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_csv_plain_text() {
        assert_eq!(escape_csv("hello"), "hello");
    }

    #[test]
    fn escape_csv_with_comma() {
        assert_eq!(escape_csv("hello, world"), "\"hello, world\"");
    }

    #[test]
    fn escape_csv_with_quotes() {
        assert_eq!(escape_csv("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
