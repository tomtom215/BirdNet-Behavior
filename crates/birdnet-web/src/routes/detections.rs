//! Detection API endpoints.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::{DailyCount, DbError, DetectionRow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Detection routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections", get(list_detections))
        .route("/detections/recent", get(recent_detections))
        .route("/detections/daily", get(daily_detections))
}

#[derive(Deserialize)]
struct DetectionQuery {
    date: Option<String>,
    species: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list_detections(
    State(state): State<AppState>,
    Query(query): Query<DetectionQuery>,
) -> (StatusCode, Json<Value>) {
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);

    let result: Result<Result<Vec<DetectionRow>, DbError>, _> = if let Some(species) =
        &query.species
    {
        let species = species.clone();
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::detections_by_species(conn, &species, limit))
        })
        .await
    } else if let Some(date) = &query.date {
        let date = date.clone();
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::detections_by_date(conn, &date))
        })
        .await
    } else {
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::recent_detections_page(conn, limit, offset))
        })
        .await
    };

    match result {
        Ok(Ok(detections)) => {
            let count = detections.len();
            (
                StatusCode::OK,
                Json(json!({
                    "detections": detections,
                    "count": count,
                    "limit": limit,
                    "offset": offset,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": e.to_string(),
                "detections": [],
                "count": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("internal error: {e}"),
                "detections": [],
                "count": 0,
            })),
        ),
    }
}

async fn recent_detections(
    State(state): State<AppState>,
    Query(query): Query<DetectionQuery>,
) -> (StatusCode, Json<Value>) {
    let limit = query.limit.unwrap_or(20);

    let result: Result<Result<Vec<DetectionRow>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::recent_detections(conn, limit))
        })
        .await;

    match result {
        Ok(Ok(detections)) => {
            let total = detections.len();
            (
                StatusCode::OK,
                Json(json!({
                    "detections": detections,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

#[derive(Deserialize)]
struct DailyQuery {
    days: Option<u32>,
}

/// Daily detection counts for trend analysis.
async fn daily_detections(
    State(state): State<AppState>,
    Query(query): Query<DailyQuery>,
) -> (StatusCode, Json<Value>) {
    let days = query.days.unwrap_or(7);

    let result: Result<Result<Vec<DailyCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::daily_counts(conn, days))
        })
        .await;

    match result {
        Ok(Ok(daily)) => {
            let total = daily.len();
            (
                StatusCode::OK,
                Json(json!({
                    "daily": daily,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}
