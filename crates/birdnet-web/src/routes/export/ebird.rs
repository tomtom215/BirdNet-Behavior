//! eBird-compatible CSV export.
//!
//! Produces CSV files that can be imported into Cornell Lab's eBird platform.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use super::{MAX_EXPORT_ROWS, escape_csv, export_too_large};
use crate::routes::is_valid_date;
use crate::state::AppState;

#[derive(Deserialize)]
pub(super) struct EbirdQuery {
    /// Date to export (YYYY-MM-DD). Defaults to today.
    date: Option<String>,
    /// Station latitude.
    lat: Option<f64>,
    /// Station longitude.
    lon: Option<f64>,
    /// Location name.
    location: Option<String>,
}

pub(super) async fn export_ebird(
    State(state): State<AppState>,
    Query(query): Query<EbirdQuery>,
) -> impl IntoResponse {
    let date = query.date.clone();
    let date_for_filename = date.clone();

    if let Some(ref d) = date
        && !is_valid_date(d)
    {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "invalid date format, expected YYYY-MM-DD"}).to_string(),
        )
            .into_response();
    }

    let lat = query.lat.unwrap_or(0.0);
    let lon = query.lon.unwrap_or(0.0);
    let location = query.location.clone().unwrap_or_default();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let from = date.as_deref();
            let to = date.as_deref();
            birdnet_db::sqlite::all_detections(conn, from, to, MAX_EXPORT_ROWS)
        })
    })
    .await;

    match result {
        Ok(Ok((detections, truncated))) => {
            if truncated {
                return export_too_large();
            }
            let csv = detections_to_ebird_csv(&detections, lat, lon, &location);
            let filename = date_for_filename.as_deref().unwrap_or("all");
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("attachment; filename=\"ebird_export_{filename}.csv\""),
                    ),
                ],
                csv,
            )
                .into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
        )
            .into_response(),
    }
}

/// Reformat a `YYYY-MM-DD` date to eBird's `MM/DD/YYYY`, passing anything that
/// isn't a well-formed date through unchanged.
///
/// Gating on [`is_valid_date`] (10 ASCII bytes, digits, dashes) guarantees the
/// byte-index slices land on char boundaries, so a malformed or multibyte date
/// from the database can't panic the handler — which, with `panic = "abort"`,
/// would crash the whole process — and a non-date is left as-is for eBird to
/// reject rather than being silently mangled.
fn reformat_ymd_to_ebird(date: &str) -> String {
    if is_valid_date(date) {
        format!("{}/{}/{}", &date[5..7], &date[8..10], &date[..4])
    } else {
        date.to_string()
    }
}

/// Build an eBird-compatible CSV from detection rows.
fn detections_to_ebird_csv(
    rows: &[birdnet_db::sqlite::DetectionRow],
    lat: f64,
    lon: f64,
    location: &str,
) -> String {
    let mut csv = String::with_capacity(rows.len() * 200);
    csv.push_str("Common Name,Genus,Species,Number,Species Comments,Location Name,Latitude,Longitude,Date,Start Time,State/Province,Country Code,Protocol,Number of Observers,Duration (Min),All Species Reported,Effort Distance (km),Effort Area (ha),Submission Comments\n");

    // Group by species to get counts per species per date
    let mut species_groups: std::collections::HashMap<
        String,
        Vec<&birdnet_db::sqlite::DetectionRow>,
    > = std::collections::HashMap::new();
    for row in rows {
        species_groups
            .entry(format!("{}|{}", row.com_name, row.date))
            .or_default()
            .push(row);
    }

    for group in species_groups.values() {
        let first = group[0];
        let count = group.len();
        let (genus, species) = first
            .sci_name
            .split_once(' ')
            .unwrap_or((&first.sci_name, "sp."));

        let start_time = group
            .iter()
            .map(|d| d.time.as_str())
            .min()
            .unwrap_or(&first.time);

        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let avg_conf: f64 = group.iter().map(|d| d.confidence).sum::<f64>() / count as f64;
        let comment = format!("BirdNET avg confidence: {:.0}%", avg_conf * 100.0);

        // Convert date from YYYY-MM-DD to MM/DD/YYYY for eBird.
        let ebird_date = reformat_ymd_to_ebird(&first.date);

        let loc = if location.is_empty() {
            "BirdNet-Behavior Station"
        } else {
            location
        };

        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{},{},{},{},,,S,1,,,,,",
            escape_csv(&first.com_name),
            escape_csv(genus),
            escape_csv(species),
            count,
            escape_csv(&comment),
            escape_csv(loc),
            lat,
            lon,
            ebird_date,
            escape_csv(start_time),
        );
    }

    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reformat_ymd_to_ebird_converts_valid_date() {
        assert_eq!(reformat_ymd_to_ebird("2026-03-12"), "03/12/2026");
        assert_eq!(reformat_ymd_to_ebird("1970-01-01"), "01/01/1970");
    }

    #[test]
    fn reformat_ymd_to_ebird_passes_through_non_dates() {
        // Wrong shape / length → unchanged (eBird rejects it downstream rather
        // than us mangling it).
        assert_eq!(reformat_ymd_to_ebird(""), "");
        assert_eq!(reformat_ymd_to_ebird("2026"), "2026");
        assert_eq!(reformat_ymd_to_ebird("not-a-date"), "not-a-date");
    }

    #[test]
    fn reformat_ymd_to_ebird_does_not_panic_on_multibyte_date() {
        // Regression: the previous `date[5..7]` byte-slicing panicked when a
        // multibyte UTF-8 char straddled a slice boundary, which with `panic =
        // "abort"` crashed the process. `is_valid_date` rejects it, so it now
        // passes through untouched instead of panicking. "2026-1é-9" is 10
        // bytes with the boundary mid-char.
        let multibyte = "2026-1\u{e9}-9";
        assert_eq!(multibyte.len(), 10);
        assert_eq!(reformat_ymd_to_ebird(multibyte), multibyte);
    }
}
