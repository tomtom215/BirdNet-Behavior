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
        .route("/analytics/patterns", get(patterns))
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
        limit: query.limit.unwrap_or(100),
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
    let species_sequence = query
        .species
        .map(|s| s.split(',').map(|part| part.trim().to_string()).collect())
        .unwrap_or(default.species_sequence);

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

async fn patterns(State(_state): State<AppState>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "status": "planned",
            "message": "Pattern matching (sequence_match) endpoint is not yet implemented.",
            "function": "sequence_match",
        })),
    )
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
    let limit = query.limit.unwrap_or(10);

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
                "patterns": "planned",
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
