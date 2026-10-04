//! Analytics API endpoints (`DuckDB`-powered).
//!
//! These endpoints are backed by `DuckDB` with the `duckdb-behavioral` extension
//! for advanced bird activity analytics. If the `DuckDB` database or behavioral
//! extension is not available, endpoints return a descriptive status message.
//!
//! Enable the `analytics` feature to compile the `DuckDB` connection code.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(feature = "analytics")]
use birdnet_behavioral::connection::AnalyticsDb;

use crate::state::AppState;

/// Analytics routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/analytics/sessions", get(sessions))
        .route("/analytics/retention", get(retention))
        .route("/analytics/funnel", get(funnel))
        .route("/analytics/funnel-events", get(funnel_events))
        .route("/analytics/patterns", get(patterns))
        .route("/analytics/sequence-count", get(sequence_count))
        .route(
            "/analytics/sequence-match-events",
            get(sequence_match_events),
        )
        .route("/analytics/next-species", get(next_species))
        .route("/analytics/previous-species", get(previous_species))
        .route("/analytics/abundance", get(abundance))
        .route("/analytics/phenology", get(phenology))
        .route("/analytics/status", get(analytics_status))
}

/// Query for the effort-corrected abundance and phenology endpoints.
#[derive(Deserialize)]
#[allow(dead_code)]
struct PhenologyQuery {
    /// Calendar year. Defaults to the current one.
    year: Option<u32>,
    /// Restrict to one species (common name).
    species: Option<String>,
    /// Row cap.
    limit: Option<u32>,
    /// Minimum detections per bucket before it is reported.
    min_detections: Option<u32>,
}

// -- Query parameter types --
// Fields are read via Deserialize when used with axum's Query extractor.
// Without the `analytics` feature, the non-analytics handlers still extract
// these types but don't read individual fields.

#[derive(Deserialize)]
#[allow(dead_code)]
struct SessionsQuery {
    species: Option<String>,
    gap: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RetentionQuery {
    min_detections: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct FunnelQuery {
    species: Option<String>,
    window: Option<u32>,
    hour_start: Option<u32>,
    hour_end: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct NextSpeciesQuery {
    after: Option<String>,
    window: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct PreviousSpeciesQuery {
    before: Option<String>,
    window: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct PatternsQuery {
    species: Option<String>,
    max_gap: Option<u32>,
    hour_start: Option<u32>,
    hour_end: Option<u32>,
}

// -- Parameter clamps --
// These endpoints are public (the LAN dashboard is unauthenticated by design),
// so a single client request must not be able to force an oversized result set
// or sequence on a small Pi. The ceilings sit far above any legitimate dashboard
// use — a station has at most a few hundred distinct species and a bounded
// session history.

/// Upper bound on `?limit=` for `/analytics/sessions`.
#[cfg(feature = "analytics")]
const MAX_SESSIONS_LIMIT: u32 = 10_000;

/// Upper bound on `?limit=` for `/analytics/next-species` and
/// `/analytics/previous-species`.
#[cfg(feature = "analytics")]
const MAX_NEXT_SPECIES_LIMIT: u32 = 1_000;

/// Upper bound on the number of species in a `?species=a,b,c` sequence (funnel /
/// patterns), capping the `Vec` built from an attacker-influenced query string
/// before it reaches the analytics query builder.
#[cfg(feature = "analytics")]
const MAX_SPECIES_SEQUENCE: usize = 64;

/// Parse a comma-separated `?species=` list into a trimmed sequence, capping the
/// element count at [`MAX_SPECIES_SEQUENCE`]. `None` falls back to `default`.
#[cfg(feature = "analytics")]
fn parse_species_sequence(raw: Option<String>, default: Vec<String>) -> Vec<String> {
    raw.map_or(default, |s| {
        s.split(',')
            .take(MAX_SPECIES_SEQUENCE)
            .map(|part| part.trim().to_string())
            .collect()
    })
}

// -- Handler implementations --

#[cfg(feature = "analytics")]
async fn sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionsQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("sessionize");
    }

    let params = birdnet_behavioral::types::SessionizeParams {
        species: query.species,
        gap_minutes: query.gap.unwrap_or(30),
        limit: query.limit.unwrap_or(100).min(MAX_SESSIONS_LIMIT),
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.sessionize(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(sessions)) => {
            let total = sessions.len();
            (
                StatusCode::OK,
                Json(json!({
                    "sessions": sessions,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => extension_error("sessionize", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn abundance(
    State(_state): State<AppState>,
    Query(_query): Query<PhenologyQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("effort-corrected abundance")
}

#[cfg(not(feature = "analytics"))]
async fn phenology(
    State(_state): State<AppState>,
    Query(_query): Query<PhenologyQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("phenology")
}

#[cfg(not(feature = "analytics"))]
async fn sessions(
    State(_state): State<AppState>,
    Query(_query): Query<SessionsQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("sessionize")
}

#[cfg(feature = "analytics")]
async fn retention(
    State(state): State<AppState>,
    Query(query): Query<RetentionQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("retention");
    }

    let params = birdnet_behavioral::types::RetentionParams {
        min_detections: query.min_detections.unwrap_or(5),
        ..birdnet_behavioral::types::RetentionParams::default()
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.retention(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(retention_data)) => {
            let total = retention_data.len();
            (
                StatusCode::OK,
                Json(json!({
                    "retention": retention_data,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => extension_error("retention", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn retention(
    State(_state): State<AppState>,
    Query(_query): Query<RetentionQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("retention")
}

#[cfg(feature = "analytics")]
async fn funnel(
    State(state): State<AppState>,
    Query(query): Query<FunnelQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("window_funnel");
    }

    let default = birdnet_behavioral::types::FunnelParams::default();
    let species_sequence = parse_species_sequence(query.species, default.species_sequence);

    let params = birdnet_behavioral::types::FunnelParams {
        species_sequence,
        window_minutes: query.window.unwrap_or(default.window_minutes),
        hour_start: query.hour_start.unwrap_or(default.hour_start),
        hour_end: query.hour_end.unwrap_or(default.hour_end),
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.funnel(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(funnel_data)) => {
            let total = funnel_data.len();
            (
                StatusCode::OK,
                Json(json!({
                    "funnel": funnel_data,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => extension_error("window_funnel", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn funnel(
    State(_state): State<AppState>,
    Query(_query): Query<FunnelQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("window_funnel")
}

#[cfg(feature = "analytics")]
async fn funnel_events(
    State(state): State<AppState>,
    Query(query): Query<FunnelQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("window_funnel_events");
    }

    let default = birdnet_behavioral::types::FunnelParams::default();
    let species_sequence = parse_species_sequence(query.species, default.species_sequence);

    let params = birdnet_behavioral::types::FunnelParams {
        species_sequence,
        window_minutes: query.window.unwrap_or(default.window_minutes),
        hour_start: query.hour_start.unwrap_or(default.hour_start),
        hour_end: query.hour_end.unwrap_or(default.hour_end),
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.funnel_events(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(events)) => {
            let total = events.len();
            (
                StatusCode::OK,
                Json(json!({
                    "funnel_events": events,
                    "total": total,
                })),
            )
        }
        Ok(Err(birdnet_behavioral::connection::AnalyticsError::InvalidData(msg))) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "invalid_request",
                "function": "window_funnel_events",
                "error": msg,
            })),
        ),
        Ok(Err(e)) => extension_error("window_funnel_events", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn funnel_events(
    State(_state): State<AppState>,
    Query(_query): Query<FunnelQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("window_funnel_events")
}

#[cfg(feature = "analytics")]
async fn patterns(
    State(state): State<AppState>,
    Query(query): Query<PatternsQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("sequence_match");
    }

    let default = birdnet_behavioral::types::PatternParams::default();
    let species_sequence = parse_species_sequence(query.species, default.species_sequence);

    let params = birdnet_behavioral::types::PatternParams {
        species_sequence,
        max_gap_minutes: query.max_gap,
        hour_start: query.hour_start.unwrap_or(default.hour_start),
        hour_end: query.hour_end.unwrap_or(default.hour_end),
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.sequence_match(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(matches)) => {
            let total = matches.len();
            let matched_days = matches.iter().filter(|m| m.matched).count();
            (
                StatusCode::OK,
                Json(json!({
                    "patterns": matches,
                    "total": total,
                    "matched_days": matched_days,
                })),
            )
        }
        // A bad species count is a client error, not an extension fault.
        Ok(Err(birdnet_behavioral::connection::AnalyticsError::InvalidData(msg))) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "invalid_request",
                "function": "sequence_match",
                "error": msg,
            })),
        ),
        Ok(Err(e)) => extension_error("sequence_match", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(feature = "analytics")]
async fn sequence_count(
    State(state): State<AppState>,
    Query(query): Query<PatternsQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("sequence_count");
    }

    let default = birdnet_behavioral::types::PatternParams::default();
    let species_sequence = parse_species_sequence(query.species, default.species_sequence);

    let params = birdnet_behavioral::types::PatternParams {
        species_sequence,
        max_gap_minutes: query.max_gap,
        hour_start: query.hour_start.unwrap_or(default.hour_start),
        hour_end: query.hour_end.unwrap_or(default.hour_end),
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.sequence_count(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(counts)) => {
            let total = counts.len();
            let total_occurrences: u64 = counts.iter().map(|c| c.count).sum();
            (
                StatusCode::OK,
                Json(json!({
                    "sequence_count": counts,
                    "total": total,
                    "total_occurrences": total_occurrences,
                })),
            )
        }
        Ok(Err(birdnet_behavioral::connection::AnalyticsError::InvalidData(msg))) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "invalid_request",
                "function": "sequence_count",
                "error": msg,
            })),
        ),
        Ok(Err(e)) => extension_error("sequence_count", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn sequence_count(
    State(_state): State<AppState>,
    Query(_query): Query<PatternsQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("sequence_count")
}

#[cfg(feature = "analytics")]
async fn sequence_match_events(
    State(state): State<AppState>,
    Query(query): Query<PatternsQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("sequence_match_events");
    }

    let default = birdnet_behavioral::types::PatternParams::default();
    let species_sequence = parse_species_sequence(query.species, default.species_sequence);

    let params = birdnet_behavioral::types::PatternParams {
        species_sequence,
        max_gap_minutes: query.max_gap,
        hour_start: query.hour_start.unwrap_or(default.hour_start),
        hour_end: query.hour_end.unwrap_or(default.hour_end),
    };

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.sequence_match_events(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(events)) => {
            let total = events.len();
            // A full match has one timestamp per step; shorter lists are the
            // longest in-order prefix a day reached. Count only full matches so
            // this aligns with sequence_count's occurrence days.
            let matched_days = events
                .iter()
                .filter(|e| e.step_times.len() == e.species_sequence.len())
                .count();
            (
                StatusCode::OK,
                Json(json!({
                    "sequence_match_events": events,
                    "total": total,
                    "matched_days": matched_days,
                })),
            )
        }
        Ok(Err(birdnet_behavioral::connection::AnalyticsError::InvalidData(msg))) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "invalid_request",
                "function": "sequence_match_events",
                "error": msg,
            })),
        ),
        Ok(Err(e)) => extension_error("sequence_match_events", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn sequence_match_events(
    State(_state): State<AppState>,
    Query(_query): Query<PatternsQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("sequence_match_events")
}

#[cfg(not(feature = "analytics"))]
async fn patterns(
    State(_state): State<AppState>,
    Query(_query): Query<PatternsQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("sequence_match")
}

#[cfg(feature = "analytics")]
async fn next_species(
    State(state): State<AppState>,
    Query(query): Query<NextSpeciesQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("sequence_next_node");
    }

    let Some(trigger) = query.after else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "missing required query parameter: after",
                "usage": "/analytics/next-species?after=European+Robin&window=60&limit=10",
            })),
        );
    };

    let window = query.window.unwrap_or(60);
    let limit = query.limit.unwrap_or(10).min(MAX_NEXT_SPECIES_LIMIT);

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.next_species(&trigger, window, limit))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(predictions)) => {
            let total = predictions.len();
            (
                StatusCode::OK,
                Json(json!({
                    "predictions": predictions,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => extension_error("sequence_next_node", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn next_species(
    State(_state): State<AppState>,
    Query(_query): Query<NextSpeciesQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("sequence_next_node")
}

/// `GET /analytics/previous-species?before=…` — what is heard immediately
/// before the first detection of a species in an activity session
/// (`sequence_next_node` run `backward`). The mirror of `next-species`.
#[cfg(feature = "analytics")]
async fn previous_species(
    State(state): State<AppState>,
    Query(query): Query<PreviousSpeciesQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return unavailable("sequence_next_node");
    }
    let Some(trigger) = query.before else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "missing required query parameter: before",
                "usage": "/analytics/previous-species?before=European+Robin&window=60&limit=10",
            })),
        );
    };
    let window = query.window.unwrap_or(60);
    let limit = query.limit.unwrap_or(10).min(MAX_NEXT_SPECIES_LIMIT);

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.previous_species(&trigger, window, limit))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(predictions)) => {
            let total = predictions.len();
            (
                StatusCode::OK,
                Json(json!({
                    "predictions": predictions,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => extension_error("sequence_next_node", &e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn previous_species(
    State(_state): State<AppState>,
    Query(_query): Query<PreviousSpeciesQuery>,
) -> (StatusCode, Json<Value>) {
    unavailable("sequence_next_node")
}

/// Analytics status endpoint -- reports what capabilities are available.
///
/// Reports the *store*, not just the build flags. `analytics_compiled` and
/// `analytics_configured` describe intent — whether the binary has DuckDB in it
/// and whether a database was wired up — and both stay `true` through every way
/// the dashboards actually fail: an extension that never loaded, an OLAP copy
/// that never synced, or rows no query can place in time. A station reporting
/// "the analytics dashboards are broken" could not be told apart from a healthy
/// one here, which is why that report arrived with no detail attached. The
/// fields below are the ones that differ.
/// Detections per hour of listening, per species per week.
///
/// The analytic a raw count cannot substitute for. A detection count is a
/// numerator over a denominator nobody was recording: a solar window is six
/// hours longer in June than December, a week of downtime removes a week of
/// listening, a failed microphone halves the channels — each moves the count
/// without moving a single bird. Dividing by recorded effort
/// (`recording_effort`, migration 27) is the standard correction, and it is what
/// makes a between-season or between-year comparison mean anything.
///
/// `effort_hours` is returned beside `detections_per_hour` so a week with little
/// or no listening is visible as such rather than as a wild rate.
#[cfg(feature = "analytics")]
async fn abundance(
    State(state): State<AppState>,
    Query(q): Query<PhenologyQuery>,
) -> (StatusCode, Json<Value>) {
    let params = birdnet_behavioral::phenology::AbundanceParams {
        year: q.year.unwrap_or_else(current_year),
        species: q.species,
        min_weekly_count: q.min_detections.unwrap_or(1),
    };
    let sql = birdnet_behavioral::phenology::effort_corrected_abundance_sql(&params);
    run_phenology_query(&state, &sql, "abundance").await
}

/// Per-species arrival, departure and presence for a calendar year.
///
/// `year_crossing` marks the species for which the calendar-year window is not a
/// migration window — a resident or overwintering visitor detected in both the
/// first and last fortnight, whose "arrival" would otherwise read as 1 January.
/// `detected_days` is returned beside `presence_days` because the latter is a
/// span, not an occupancy.
#[cfg(feature = "analytics")]
async fn phenology(
    State(state): State<AppState>,
    Query(q): Query<PhenologyQuery>,
) -> (StatusCode, Json<Value>) {
    let params = birdnet_behavioral::phenology::PhenologyParams {
        species: q.species,
        year_start: q.year,
        year_end: q.year,
        min_detections: q.min_detections.unwrap_or(5),
        limit: q.limit.unwrap_or(500),
    };
    let sql = birdnet_behavioral::phenology::phenology_timing_sql(&params);
    run_phenology_query(&state, &sql, "phenology").await
}

/// The current calendar year, from the station's local clock.
#[cfg(feature = "analytics")]
fn current_year() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
        + birdnet_db::clock::local_utc_offset_secs();
    birdnet_core::civil::civil_from_unix_secs(secs).year
}

/// Execute a phenology SQL string and return its rows as JSON objects.
///
/// These queries are pure SQL over `detections_ts` and `recording_effort` — no
/// behavioural extension involved — so they work on an air-gapped station where
/// the community extension never loaded.
#[cfg(feature = "analytics")]
async fn run_phenology_query(
    state: &AppState,
    sql: &str,
    label: &str,
) -> (StatusCode, Json<Value>) {
    let (state, sql, label) = (state.clone(), sql.to_owned(), label.to_owned());
    let rows = tokio::task::spawn_blocking(move || {
        state.with_analytics(|adb| {
            let mut stmt = adb.conn().prepare(&sql)?;
            let names: Vec<String> = stmt
                .column_names()
                .iter()
                .map(ToString::to_string)
                .collect();
            let mut out = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let mut obj = serde_json::Map::new();
                for (i, name) in names.iter().enumerate() {
                    obj.insert(name.clone(), duckdb_value_to_json(row, i));
                }
                out.push(Value::Object(obj));
            }
            Ok::<_, birdnet_behavioral::duckdb::Error>(out)
        })
    })
    .await;

    match rows {
        Ok(Some(Ok(rows))) => (StatusCode::OK, Json(json!({ label: rows }))),
        Ok(Some(Err(e))) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "analytics database not configured" })),
        ),
    }
}

/// Convert one `DuckDB` column to JSON, degrading to null rather than failing.
///
/// Column types vary by query (and `percentile_cont` returns DOUBLE where the
/// input was INTEGER), so this dispatches on the type DuckDB actually returned.
///
/// It used to probe instead, trying `Option<i64>` first. duckdb-rs's integer
/// `FromSql` accepts a DOUBLE, DECIMAL, VARCHAR, DATE or TIMESTAMP and casts it
/// (`duckdb` 1.10506 `src/types/from_sql.rs`), so the first probe always won:
/// `detections_per_hour` of 0.43 reached the abundance JSON as `0`, and a
/// numeric-looking string as a number.
#[cfg(feature = "analytics")]
fn duckdb_value_to_json(row: &birdnet_behavioral::duckdb::Row<'_>, i: usize) -> Value {
    use birdnet_behavioral::duckdb::types::ValueRef;
    let Ok(v) = row.get_ref(i) else {
        return Value::Null;
    };
    match v {
        ValueRef::Boolean(b) => json!(b),
        ValueRef::TinyInt(n) => json!(n),
        ValueRef::SmallInt(n) => json!(n),
        ValueRef::Int(n) => json!(n),
        ValueRef::BigInt(n) => json!(n),
        ValueRef::UTinyInt(n) => json!(n),
        ValueRef::USmallInt(n) => json!(n),
        ValueRef::UInt(n) => json!(n),
        ValueRef::UBigInt(n) => json!(n),
        // `SUM` over an integer column is HUGEINT in DuckDB.
        ValueRef::HugeInt(n) => {
            i64::try_from(n).map_or_else(|_| json!(n.to_string()), |n| json!(n))
        }
        // Through the f32's own shortest decimal form: widening it to f64
        // prints `ROUND(CAST(3 AS REAL) / 7, 4)` as 0.428600013256073.
        ValueRef::Float(f) => f
            .to_string()
            .parse::<f64>()
            .map_or(Value::Null, |f| json!(f)),
        ValueRef::Double(f) => json!(f),
        ValueRef::Decimal(_) => row.get::<_, f64>(i).map_or(Value::Null, |f| json!(f)),
        ValueRef::Text(_) => row.get::<_, String>(i).map_or(Value::Null, |s| json!(s)),
        _ => Value::Null,
    }
}

#[cfg(feature = "analytics")]
async fn analytics_status(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();

    // Each of these is a way the dashboards go empty while the rest of the app
    // stays healthy, so each is reported separately rather than collapsed into
    // one "ok" flag.
    //
    // On the blocking pool: the analytics handle sits behind one mutex shared
    // with every analytics query, so waiting for it here held a runtime worker
    // for the length of whatever query was running.
    let store = tokio::task::spawn_blocking(move || {
        state.with_analytics(|db| {
            // `null` when a count cannot be read: a defaulted 0 here reads as
            // "the sync has not run", the wrong diagnosis for a failing store.
            let detections = db.detection_count().ok();
            let unplaceable = db.unplaceable_detection_count().ok();
            json!({
                // The behavioural functions (sessionize, retention, window_funnel,
                // sequence_*) need this; the extension is optional to *open* the
                // database, so a false here is invisible until a query runs.
                "extension_loaded": db.extension_loaded(),
                // Loaded is not the same as answering correctly: every function is
                // run on fixed events and compared with the extension version this
                // build was written for. `null` when the extension is not loaded,
                // otherwise `ok` or the first disagreement.
                "self_test": db.extension_loaded().then(|| {
                    db.verify_behavioral_functions()
                        .map_or_else(|e| e.to_string(), |()| "ok".to_owned())
                }),
                "detections": detections,
                // Rows whose Date/Time name no point in time: present in the total,
                // absent from every time-bucketed analytic. A non-zero value here
                // is why a dashboard total can sit below the station's own count.
                "unplaceable_detections": unplaceable,
                "detections_placeable": detections
                    .zip(unplaceable)
                    .map(|(d, u)| d.saturating_sub(u)),
                // What the engine itself is, so a mismatch below can be read
                // without knowing how this binary was built.
                "engine_duckdb_version": db.duckdb_version(),
                "engine_platform": db.engine_platform(),
                "embedded_extension": {
                    "version": AnalyticsDb::embedded_extension_version(),
                    // The footer's version target, read according to `abi`: a
                    // DuckDB version for `CPP` / `C_STRUCT_UNSTABLE`, a minimum C
                    // API version for `C_STRUCT` (behavioral v0.10.0 and later,
                    // which loads into any engine at or above it).
                    "duckdb_version": AnalyticsDb::embedded_extension_duckdb_version(),
                    "abi": AnalyticsDb::embedded_extension_abi(),
                    "platform": AnalyticsDb::embedded_extension_platform(),
                    // Some(..) means the embedded copy can never load, so an
                    // offline station has no behavioural analytics at all. An
                    // extension is locked to a platform as well as a version and
                    // the two fail identically at LOAD, so `property` names which
                    // one is wrong rather than leaving it to be inferred.
                    "mismatch": db.embedded_extension_mismatch().map(|m| {
                        json!({
                            "property": m.kind.to_string(),
                            "embedded_for": m.embedded_for,
                            "embedded_abi": m.embedded_abi,
                            "engine": m.engine,
                            "embedded_platform": m.embedded_platform,
                            "engine_platform": m.engine_platform,
                        })
                    }),
                },
            })
        })
    })
    .await
    .unwrap_or_else(|e| {
        // Not `None`: a null `store` means "built without analytics".
        tracing::warn!(error = %e, "analytics status: task failed");
        Some(json!({ "error": "the status check itself failed; see the log" }))
    });

    (
        StatusCode::OK,
        Json(json!({
            "analytics_compiled": compiled,
            "analytics_configured": configured,
            "store": store,
            "endpoints": {
                "sessions": "/analytics/sessions?species=...&gap=30&limit=100",
                "retention": "/analytics/retention?min_detections=5",
                "funnel": "/analytics/funnel?species=Robin,Blackbird&window=120&hour_start=4&hour_end=8",
                "next_species": "/analytics/next-species?after=European+Robin&window=60&limit=10",
                "previous_species": "/analytics/previous-species?before=European+Robin&window=60&limit=10",
                "patterns": "/analytics/patterns?species=Robin,Blackbird,Wren&max_gap=60&hour_start=4&hour_end=8",
            },
        })),
    )
}

/// Analytics status endpoint -- reports what capabilities are available.
///
/// Slim build: there is no store to report on, so `store` is null rather than
/// absent — a caller can tell "built without analytics" from "analytics present
/// but broken" without special-casing a missing key.
#[cfg(not(feature = "analytics"))]
async fn analytics_status(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();

    (
        StatusCode::OK,
        Json(json!({
            "analytics_compiled": compiled,
            "analytics_configured": configured,
            "store": Value::Null,
            "endpoints": {
                "sessions": "/analytics/sessions?species=...&gap=30&limit=100",
                "retention": "/analytics/retention?min_detections=5",
                "funnel": "/analytics/funnel?species=Robin,Blackbird&window=120&hour_start=4&hour_end=8",
                "next_species": "/analytics/next-species?after=European+Robin&window=60&limit=10",
                "previous_species": "/analytics/previous-species?before=European+Robin&window=60&limit=10",
                "patterns": "/analytics/patterns?species=Robin,Blackbird,Wren&max_gap=60&hour_start=4&hour_end=8",
            },
        })),
    )
}

/// Response when `DuckDB` analytics is not configured or compiled.
fn unavailable(function: &str) -> (StatusCode, Json<Value>) {
    let message = if cfg!(feature = "analytics") {
        "DuckDB analytics not configured. Start with --analytics-db to enable."
    } else {
        "DuckDB analytics not compiled. Rebuild with --features analytics to enable."
    };

    (
        StatusCode::OK,
        Json(json!({
            "status": "unavailable",
            "message": message,
            "function": function,
        })),
    )
}

/// Response when the behavioral extension is required but not loaded.
#[cfg(feature = "analytics")]
fn extension_error(function: &str, error: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "status": "extension_required",
            "message": "The duckdb-behavioral extension is required for this query.",
            "function": function,
            "error": error,
        })),
    )
}

#[cfg(all(test, feature = "analytics"))]
mod tests {
    use super::{MAX_SPECIES_SEQUENCE, parse_species_sequence};

    #[test]
    fn parse_species_sequence_uses_default_when_absent() {
        let def = vec!["Robin".to_string(), "Wren".to_string()];
        assert_eq!(parse_species_sequence(None, def.clone()), def);
    }

    #[test]
    fn parse_species_sequence_splits_and_trims() {
        assert_eq!(
            parse_species_sequence(Some(" Robin , Blackbird ,Wren".to_string()), vec![]),
            vec![
                "Robin".to_string(),
                "Blackbird".to_string(),
                "Wren".to_string(),
            ]
        );
    }

    #[test]
    fn parse_species_sequence_caps_element_count() {
        // A pathological `?species=sp,sp,sp,…` (5000 entries) is capped so a
        // single public request can't push an oversized sequence into the
        // analytics query builder.
        let raw = vec!["sp"; 5000].join(",");
        let parsed = parse_species_sequence(Some(raw), vec![]);
        assert_eq!(parsed.len(), MAX_SPECIES_SEQUENCE);
    }
}

#[cfg(all(test, feature = "analytics"))]
mod json_value_tests {
    use super::{PreviousSpeciesQuery, analytics_status, duckdb_value_to_json, previous_species};
    use axum::extract::{Query, State};

    /// `/analytics/previous-species` answers from the store, names the
    /// direction in its JSON, and rejects a request without a trigger.
    #[tokio::test]
    async fn previous_species_reports_what_came_before() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new_with_analytics(
            dir.path().join("birds.db"),
            &dir.path().join("analytics.duckdb"),
        )
        .unwrap();
        let loaded = state.with_analytics(|db| db.extension_loaded()).unwrap();
        if !loaded {
            birdnet_behavioral::gating::skip_or_fail(
                "the behavioral extension",
                "it did not load into the test store",
            );
            return;
        }
        state
            .with_analytics(|db| {
                db.conn().execute_batch(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) \
                     VALUES ('2024-05-01', '05:00:00', 'x', 'Eurasian Wren', 0.9, 1714539600), \
                            ('2024-05-01', '05:01:00', 'x', 'European Robin', 0.9, 1714539660);",
                )
            })
            .unwrap()
            .unwrap();

        let (status, body) = previous_species(
            State(state.clone()),
            Query(PreviousSpeciesQuery {
                before: Some("European Robin".into()),
                window: None,
                limit: None,
            }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{}", body.0);
        assert_eq!(body.0["total"], json!(1), "{}", body.0);
        let p = &body.0["predictions"][0];
        assert_eq!(p["before_species"], json!("European Robin"), "{}", body.0);
        assert_eq!(p["predicted_species"], json!("Eurasian Wren"), "{}", body.0);

        let (status, _) = previous_species(
            State(state),
            Query(PreviousSpeciesQuery {
                before: None,
                window: None,
                limit: None,
            }),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    }
    use serde_json::{Value, json};

    /// Waiting for the analytics handle does not stall the async runtime.
    ///
    /// On a one-thread runtime, another thread holds the handle for 500 ms
    /// while a status request waits for it, and a second task's 50 ms timer
    /// expires meanwhile. That task runs only if the request yielded the
    /// thread; done on the runtime, the request blocked it for the full
    /// 500 ms and the task was still waiting when it returned.
    #[tokio::test(flavor = "current_thread")]
    async fn status_waits_for_the_handle_off_the_runtime() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new_with_analytics(
            dir.path().join("birds.db"),
            &dir.path().join("analytics.duckdb"),
        )
        .unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let holder = {
            let state = state.clone();
            std::thread::spawn(move || {
                state.with_analytics(|_| {
                    held_tx.send(()).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                });
            })
        };
        held_rx.recv().unwrap();
        let ticked = Arc::new(AtomicBool::new(false));
        let ticker = {
            let ticked = Arc::clone(&ticked);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                ticked.store(true, Ordering::SeqCst);
            })
        };
        let _ = analytics_status(State(state)).await;
        assert!(
            ticked.load(Ordering::SeqCst),
            "the runtime's only thread was blocked while the request waited for the handle"
        );
        ticker.await.unwrap();
        holder.join().unwrap();
    }

    /// A store whose counts cannot be read says so, rather than reporting a
    /// zero that the docs tell the operator means "the sync has not run".
    #[tokio::test]
    async fn status_reports_an_unreadable_count_as_null_not_zero() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new_with_analytics(
            dir.path().join("birds.db"),
            &dir.path().join("analytics.duckdb"),
        )
        .unwrap();
        let (_, healthy) = analytics_status(State(state.clone())).await;
        // Counterpart: a readable empty store still reports a real zero.
        assert_eq!(healthy.0["store"]["detections"], json!(0), "{}", healthy.0);
        let expected_self_test = if healthy.0["store"]["extension_loaded"] == json!(true) {
            json!("ok")
        } else {
            Value::Null
        };
        assert_eq!(
            healthy.0["store"]["self_test"], expected_self_test,
            "{}",
            healthy.0
        );
        state
            .with_analytics(|db| {
                db.conn()
                    .execute_batch("DROP VIEW detections_ts; DROP TABLE detections;")
            })
            .unwrap()
            .unwrap();
        let (_, broken) = analytics_status(State(state)).await;
        assert_eq!(broken.0["store"]["detections"], Value::Null, "{}", broken.0);
        assert_eq!(
            broken.0["store"]["detections_placeable"],
            Value::Null,
            "{}",
            broken.0
        );
    }

    /// Each column reaches the JSON as the type DuckDB returned, not as
    /// whatever the first probe could coerce it into.
    #[test]
    fn json_keeps_each_duckdb_type() {
        let conn = birdnet_behavioral::duckdb::Connection::open_in_memory().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT 0.43::DOUBLE, ROUND(CAST(3 AS REAL) / 7, 4), 2.5::DECIMAL(9,2), \
                 SUM(x)::HUGEINT, COUNT(*), '123', NULL::DOUBLE, TRUE, 1.5::FLOAT \
                 FROM (VALUES (40), (2)) t(x)",
            )
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let got: Vec<Value> = (0..9).map(|i| duckdb_value_to_json(row, i)).collect();
        assert_eq!(
            got,
            vec![
                json!(0.43),
                json!(0.4286),
                json!(2.5),
                json!(42),
                json!(2),
                json!("123"),
                Value::Null,
                json!(true),
                json!(1.5),
            ]
        );
    }
}
