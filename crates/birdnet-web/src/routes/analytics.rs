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
        .route("/analytics/status", get(analytics_status))
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

/// Upper bound on `?limit=` for `/analytics/next-species`.
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
            let matched_days = events.iter().filter(|e| !e.step_times.is_empty()).count();
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

/// Analytics status endpoint -- reports what capabilities are available.
async fn analytics_status(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();

    (
        StatusCode::OK,
        Json(json!({
            "analytics_compiled": compiled,
            "analytics_configured": configured,
            "endpoints": {
                "sessions": "/analytics/sessions?species=...&gap=30&limit=100",
                "retention": "/analytics/retention?min_detections=5",
                "funnel": "/analytics/funnel?species=Robin,Blackbird&window=120&hour_start=4&hour_end=8",
                "next_species": "/analytics/next-species?after=European+Robin&window=60&limit=10",
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
