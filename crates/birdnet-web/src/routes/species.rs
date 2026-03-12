//! Species API endpoints.

use axum::extract::{Query, State};
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::{DbError, HourlyCount, SpeciesCount};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Species routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/species/top", get(top_species))
        .route("/species/activity", get(hourly_activity))
}

#[derive(Deserialize)]
struct TopSpeciesQuery {
    limit: Option<u32>,
}

async fn top_species(
    State(state): State<AppState>,
    Query(query): Query<TopSpeciesQuery>,
) -> Json<Value> {
    let limit = query.limit.unwrap_or(20);

    let result: Result<Result<Vec<SpeciesCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::top_species(conn, limit))
        })
        .await;

    match result {
        Ok(Ok(species)) => {
            let total = species.len();
            Json(json!({
                "species": species,
                "total": total,
            }))
        }
        Ok(Err(e)) => Json(json!({ "error": e.to_string() })),
        Err(e) => Json(json!({ "error": format!("internal error: {e}") })),
    }
}

#[derive(Deserialize)]
struct ActivityQuery {
    date: String,
}

async fn hourly_activity(
    State(state): State<AppState>,
    Query(query): Query<ActivityQuery>,
) -> Json<Value> {
    let date = query.date;

    let result: Result<Result<Vec<HourlyCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::hourly_activity(conn, &date))
        })
        .await;

    match result {
        Ok(Ok(hours)) => Json(json!({
            "activity": hours,
        })),
        Ok(Err(e)) => Json(json!({ "error": e.to_string() })),
        Err(e) => Json(json!({ "error": format!("internal error: {e}") })),
    }
}
