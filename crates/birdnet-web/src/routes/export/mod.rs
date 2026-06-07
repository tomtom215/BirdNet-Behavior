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

/// Maximum detection rows a single export will materialise.
///
/// The export endpoints are public (unauthenticated) and build the whole result
/// set plus the entire CSV string in memory, so an unfiltered export against a
/// long-running station could OOM a small Pi. Beyond this bound the handlers
/// return a 413 with guidance to narrow the date range. One million rows is far
/// above any realistic station's lifetime count yet caps peak memory at a known
/// ceiling rather than unbounded growth.
pub(super) const MAX_EXPORT_ROWS: u32 = 1_000_000;

/// Export routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections/export", get(csv::export_detections))
        .route("/species/export", get(csv::export_species))
        .route("/detections/export/ebird", get(ebird::export_ebird))
        .route("/detections/export/birddb", get(birddb::export_birddb))
}

/// Escape a value for CSV output (RFC 4180) and neutralize CSV formula
/// injection (CWE-1236).
///
/// Two protections, in order:
/// 1. **Formula-injection guard.** A cell whose first character is `=`, `+`,
///    `-`, `@`, TAB, or CR is interpreted as a *formula* by Excel / `LibreOffice`
///    / Google Sheets when the export is opened. A crafted species label or a
///    common name imported from a BirdNET-Pi database could therefore run a
///    formula (e.g. data exfiltration via `=HYPERLINK`/`WEBSERVICE`). We prefix
///    such values with a single quote, the OWASP-recommended neutralization,
///    which forces the spreadsheet to treat the cell as literal text. Numeric
///    columns (confidence, lat/lon, counts) are formatted from typed values and
///    never pass through here, so legitimate negative coordinates are untouched.
/// 2. **RFC 4180 quoting.** Wrap in double quotes if the (possibly guarded)
///    value contains a comma, quote, or newline, doubling embedded quotes.
pub(crate) fn escape_csv(value: &str) -> String {
    let mut field = if value
        .as_bytes()
        .first()
        .is_some_and(|b| matches!(b, b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r'))
    {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        field = format!("\"{}\"", field.replace('"', "\"\""));
    }
    field
}

/// `413 Payload Too Large` response shared by the export handlers when a
/// request would materialise more than [`MAX_EXPORT_ROWS`] rows. Directs the
/// caller to narrow the date range so the export stays within the memory bound.
pub(super) fn export_too_large() -> axum::response::Response {
    use axum::response::IntoResponse as _;
    (
        axum::http::StatusCode::PAYLOAD_TOO_LARGE,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "error": format!(
                "export exceeds {MAX_EXPORT_ROWS} rows; narrow it with ?from=YYYY-MM-DD&to=YYYY-MM-DD"
            )
        })
        .to_string(),
    )
        .into_response()
}

/// Strip record-structure-breaking characters (the delimiter and line breaks)
/// from a free-text field destined for the **unescaped** semicolon-delimited
/// `BirdDB.txt` legacy format. BirdNET-Pi's format carries no quoting, so a
/// `;` or newline embedded in a common/scientific name would otherwise split
/// the record into extra fields or inject a whole fake detection line. Replace
/// them with a space so the 12-field, one-line-per-record structure every
/// downstream `BirdDB` consumer relies on stays intact.
pub(crate) fn sanitize_birddb_field(value: &str) -> String {
    value.replace([';', '\n', '\r'], " ")
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

    #[test]
    fn escape_csv_neutralizes_formula_injection() {
        // Leading formula characters get a `'` prefix so the cell is text.
        assert_eq!(escape_csv("=1+1"), "'=1+1");
        assert_eq!(escape_csv("+1+1"), "'+1+1");
        assert_eq!(escape_csv("-1+1"), "'-1+1");
        assert_eq!(escape_csv("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(escape_csv("\tcmd"), "'\tcmd");
        assert_eq!(escape_csv("\rcmd"), "'\rcmd");
        // A real exfiltration payload is both guarded and RFC-4180 quoted
        // (it contains a comma), and stays inert on open.
        assert_eq!(
            escape_csv("=HYPERLINK(\"http://evil\",\"x\")"),
            "\"'=HYPERLINK(\"\"http://evil\"\",\"\"x\"\")\""
        );
    }

    #[test]
    fn escape_csv_leaves_normal_values_unguarded() {
        // Names and numbers that don't start with a formula char are untouched.
        assert_eq!(escape_csv("Turdus merula"), "Turdus merula");
        assert_eq!(escape_csv("2026-03-12"), "2026-03-12");
        assert_eq!(escape_csv("0.8700"), "0.8700");
    }

    #[test]
    fn sanitize_birddb_field_strips_structure_breakers() {
        assert_eq!(sanitize_birddb_field("Eurasian Blackbird"), "Eurasian Blackbird");
        assert_eq!(sanitize_birddb_field("Foo;Bar"), "Foo Bar");
        assert_eq!(
            sanitize_birddb_field("inject\n2026-01-01;00:00:00;Fake"),
            "inject 2026-01-01 00:00:00 Fake"
        );
        assert_eq!(sanitize_birddb_field("carriage\r\nreturn"), "carriage  return");
    }
}
