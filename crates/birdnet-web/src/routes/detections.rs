//! Detection API endpoints.

use axum::extract::{Query, State};
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::{DbError, DetectionRow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Detection routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections", get(list_detections))
        .route("/detections/recent", get(recent_detections))
}

#[derive(Deserialize)]
struct DetectionQuery {
    date: Option<String>,
    limit: Option<u32>,
}

async fn list_detections(
    State(state): State<AppState>,
    Query(query): Query<DetectionQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(100);

    let result: Result<Result<Vec<DetectionRow>, DbError>, _> = if let Some(date) = &query.date {
        let date = date.clone();
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::detections_by_date(conn, &date))
        })
        .await
    } else {
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::recent_detections(conn, limit))
        })
        .await
    };

    match result {
        Ok(Ok(detections)) => {
            let total = detections.len();
            Json(json!({
                "detections": detections,
                "total": total,
            }))
        }
        Ok(Err(e)) => Json(json!({
            "error": e.to_string(),
            "detections": [],
            "total": 0,
        })),
        Err(e) => Json(json!({
            "error": format!("internal error: {e}"),
            "detections": [],
            "total": 0,
        })),
    }
}

async fn recent_detections(
    State(state): State<AppState>,
    Query(query): Query<DetectionQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(20);

    let result: Result<Result<Vec<DetectionRow>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::recent_detections(conn, limit))
        })
        .await;

    match result {
        Ok(Ok(detections)) => {
            let total = detections.len();
            Json(json!({
                "detections": detections,
                "total": total,
            }))
        }
        Ok(Err(e)) => Json(json!({ "error": e.to_string() })),
        Err(e) => Json(json!({ "error": format!("internal error: {e}") })),
    }
}
