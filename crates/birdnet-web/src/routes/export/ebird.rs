//! eBird Record Format export.
//!
//! This is the one export whose output leaves the station and enters a public
//! database, so the rules here are eBird's, not ours. Each rule below was
//! checked against eBird's "Upload spreadsheet data" guidance (fetched
//! 2026-09-07) and against what BirdNET-Pi's `scripts/history.php` does at
//! `88985a3`, and each has a gate in
//! `tests/the_ebird_export_is_a_checklist_ebird_can_accept.rs`:
//!
//! * **Coordinates** come from the station's configured location. A caller may
//!   override them, both or neither; a station that has no location writes the
//!   columns blank — eBird treats them as optional — and never `0,0`, which
//!   places every checklist at Null Island.
//! * **A confidence floor**, 0.75 by default as upstream, movable by the
//!   caller. A checklist is a claim of presence; the detections under the floor
//!   are exactly the ones that must not be claimed.
//! * **One record per species per hour.** An autonomous recorder counts
//!   vocalisations, not individuals, so the tally is written into
//!   `Species Comments` and `Number` is `X` — eBird's own notation for
//!   "present but not counted". Upstream writes `1`; the previous version of
//!   this file wrote the raw tally, so one blackbird singing two hundred times
//!   became "200 birds".
//! * **Effort fields** — protocol, observer count, region, completeness — are
//!   the submitter's facts, not the station's, so they come from the caller.
//!   The defaults are the ones an unattended station warrants: `Stationary`,
//!   one observer, and `N` for "all observations reported", because a recorder
//!   cannot report the birds that did not vocalise.
//! * **No header row**: eBird's Record Format is rejected with one.
//! * Rows are read from `detections_analytic`, so a detection the reviewer
//!   rejected, or an imported batch the operator excluded, never reaches the
//!   checklist.

use std::collections::BTreeMap;
use std::fmt::Write;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{MAX_EXPORT_ROWS, escape_csv, export_too_large};
use crate::routes::is_valid_date;
use crate::state::AppState;

/// The floor BirdNET-Pi applies (`history.php`: `Confidence > 0.75`), kept as
/// the default so a station migrated from upstream publishes the same claims.
pub(super) const DEFAULT_MIN_CONFIDENCE: f64 = 0.75;

/// Location name written when the station has neither a `station_name`
/// setting nor a caller-supplied name.
const DEFAULT_LOCATION_NAME: &str = "BirdNet-Behavior Station";

#[derive(Deserialize)]
pub(super) struct EbirdQuery {
    /// Date to export (YYYY-MM-DD). Defaults to every date.
    date: Option<String>,
    /// Latitude override; must be paired with `lon`.
    lat: Option<f64>,
    /// Longitude override; must be paired with `lat`.
    lon: Option<f64>,
    /// Location name; defaults to the `station_name` setting.
    location: Option<String>,
    /// Lowest confidence a detection may have to be claimed (0–1).
    min_confidence: Option<f64>,
    /// eBird protocol, one word (`Stationary`, `Traveling`, `Incidental`, …).
    protocol: Option<String>,
    /// Number of observers.
    observers: Option<u32>,
    /// State or province code (1–3 characters).
    state: Option<String>,
    /// Country code (2 characters).
    country: Option<String>,
    /// Whether every species heard is reported (`Y`); defaults to `N`.
    complete: Option<bool>,
}

/// The per-checklist facts the caller supplies, resolved against the station's
/// settings. Kept separate from the query so the CSV writer takes exactly what
/// it writes and nothing it could default behind the caller's back.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ChecklistEffort {
    pub(super) lat: Option<f64>,
    pub(super) lon: Option<f64>,
    pub(super) location: String,
    pub(super) protocol: String,
    pub(super) observers: u32,
    pub(super) state: String,
    pub(super) country: String,
    pub(super) complete: bool,
}

fn bad_request(msg: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": msg }).to_string(),
    )
        .into_response()
}

/// Resolve the coordinates: the caller's pair if given, else the station's,
/// else none. Half a pair, or a value off the globe, is an error rather than a
/// guess — a checklist placed wrongly is worse than one the submitter has to
/// place by hand.
fn resolve_coordinates(
    query: (Option<f64>, Option<f64>),
    configured: (Option<f64>, Option<f64>),
) -> Result<(Option<f64>, Option<f64>), &'static str> {
    let pair = match query {
        (Some(lat), Some(lon)) => (Some(lat), Some(lon)),
        (None, None) => configured,
        _ => return Err("lat and lon must be given together"),
    };
    if let (Some(lat), Some(lon)) = pair
        && (!(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon))
    {
        return Err("lat must be within -90..=90 and lon within -180..=180");
    }
    Ok(pair)
}

fn setting_f64(conn: &rusqlite::Connection, key: &str) -> Option<f64> {
    birdnet_db::settings::get_or(conn, key, "")
        .ok()?
        .trim()
        .parse()
        .ok()
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
        return bad_request("invalid date format, expected YYYY-MM-DD");
    }

    let min_confidence = query.min_confidence.unwrap_or(DEFAULT_MIN_CONFIDENCE);
    if !(0.0..=1.0).contains(&min_confidence) {
        return bad_request("min_confidence must be within 0..=1");
    }
    if let Some(ref p) = query.protocol
        && (p.is_empty() || p.chars().any(char::is_whitespace))
    {
        return bad_request("protocol must be one word, e.g. Stationary or Traveling");
    }

    let query_coords = (query.lat, query.lon);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let configured = (
                setting_f64(conn, "latitude"),
                setting_f64(conn, "longitude"),
            );
            let station_name =
                birdnet_db::settings::get_or(conn, "station_name", "").unwrap_or_default();
            let rows = birdnet_db::sqlite::analytic_detections_above(
                conn,
                date.as_deref(),
                date.as_deref(),
                min_confidence,
                MAX_EXPORT_ROWS,
            )?;
            Ok::<_, birdnet_db::sqlite::DbError>((configured, station_name, rows))
        })
    })
    .await;

    let (configured, station_name, (detections, truncated)) = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
            )
                .into_response();
        }
    };
    if truncated {
        return export_too_large();
    }

    let (lat, lon) = match resolve_coordinates(query_coords, configured) {
        Ok(pair) => pair,
        Err(msg) => return bad_request(msg),
    };
    let location = query
        .location
        .filter(|l| !l.trim().is_empty())
        .or_else(|| (!station_name.trim().is_empty()).then_some(station_name))
        .unwrap_or_else(|| DEFAULT_LOCATION_NAME.to_string());
    let effort = ChecklistEffort {
        lat,
        lon,
        location,
        protocol: query.protocol.unwrap_or_else(|| "Stationary".to_string()),
        observers: query.observers.unwrap_or(1),
        state: query.state.unwrap_or_default(),
        country: query.country.unwrap_or_default(),
        complete: query.complete.unwrap_or(false),
    };

    let csv = detections_to_ebird_csv(&detections, &effort);
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

/// The hour a `HH:MM:SS` local time falls in, as `HH`; a time that is not in
/// that shape is its own bucket so it is neither dropped nor merged wrongly.
fn hour_bucket(time: &str) -> String {
    match time.get(0..2) {
        Some(hh)
            if hh.bytes().all(|b| b.is_ascii_digit()) && time.as_bytes().get(2) == Some(&b':') =>
        {
            hh.to_string()
        }
        _ => time.to_string(),
    }
}

fn format_coordinate(value: Option<f64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// One checklist record per species per hour, in eBird Record Format column
/// order, with no header row.
fn detections_to_ebird_csv(
    rows: &[birdnet_db::sqlite::DetectionRow],
    effort: &ChecklistEffort,
) -> String {
    // Keyed (date, hour, common name, scientific name) so the output order is
    // the order a person reading the checklist expects, and stable.
    let mut groups: BTreeMap<(String, String, String, String), (usize, f64)> = BTreeMap::new();
    for row in rows {
        let entry = groups
            .entry((
                row.date.clone(),
                hour_bucket(&row.time),
                row.com_name.clone(),
                row.sci_name.clone(),
            ))
            .or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 = entry.1.max(row.confidence);
    }

    let lat = format_coordinate(effort.lat);
    let lon = format_coordinate(effort.lon);
    let complete = if effort.complete { "Y" } else { "N" };

    let mut csv = String::with_capacity(groups.len() * 200);
    for ((date, hour, com_name, sci_name), (count, max_conf)) in &groups {
        let (genus, species) = sci_name.split_once(' ').unwrap_or((sci_name, "sp."));
        let start_time = if hour.len() == 2 {
            format!("{hour}:00")
        } else {
            hour.clone()
        };
        // No comma and no quote in the comment: eBird's guidance asks for
        // neither, and a quoted field is one more thing for its importer to
        // trip on.
        let comment = format!(
            "BirdNET: {count} detection{} this hour; highest confidence {:.0}%",
            if *count == 1 { "" } else { "s" },
            max_conf * 100.0
        );
        let _ = writeln!(
            csv,
            "{},{},{},X,{},{},{lat},{lon},{},{},{},{},{},{},60,{complete},,,",
            escape_csv(com_name),
            escape_csv(genus),
            escape_csv(species),
            escape_csv(&comment),
            escape_csv(&effort.location),
            reformat_ymd_to_ebird(date),
            escape_csv(&start_time),
            escape_csv(&effort.state),
            escape_csv(&effort.country),
            escape_csv(&effort.protocol),
            effort.observers,
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

    #[test]
    fn hour_bucket_takes_the_hour_and_keeps_a_malformed_time_whole() {
        assert_eq!(hour_bucket("06:30:00"), "06");
        assert_eq!(hour_bucket("23:59:59"), "23");
        assert_eq!(hour_bucket("6:30"), "6:30");
        assert_eq!(hour_bucket(""), "");
        assert_eq!(hour_bucket("\u{e9}\u{e9}:00"), "\u{e9}\u{e9}:00");
    }

    #[test]
    fn coordinates_resolve_caller_then_station_then_blank() {
        let station = (Some(51.5), Some(-0.1));
        assert_eq!(
            resolve_coordinates((Some(1.0), Some(2.0)), station),
            Ok((Some(1.0), Some(2.0)))
        );
        assert_eq!(resolve_coordinates((None, None), station), Ok(station));
        assert_eq!(
            resolve_coordinates((None, None), (None, None)),
            Ok((None, None))
        );
        assert!(resolve_coordinates((Some(1.0), None), station).is_err());
        assert!(resolve_coordinates((None, None), (Some(95.0), Some(0.0))).is_err());
        assert!(resolve_coordinates((Some(0.0), Some(181.0)), station).is_err());
    }
}
