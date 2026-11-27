//! The detection log's own rows, as CSV (UIC-10).
//!
//! `GET /api/v2/detections/export/search?<the /search query string>`
//!
//! The search page composes nine criteria; this takes the same query string,
//! translates it with the page's own [`SearchParams::to_filter`] and the same
//! name resolution, and writes every matching row (not one page of them) in
//! the detections CSV's columns, in the page's sort order. Unlike
//! `/api/v2/detections/export`, it reads the raw table, so a search for
//! rejected rows exports rejected rows, as the page lists them.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::{MAX_EXPORT_ROWS, export_too_large};
use crate::routes::pages::search::SearchParams;
use crate::state::AppState;

/// `GET /api/v2/detections/export/search`.
pub(super) async fn export_search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Response {
    let filter = crate::routes::pages::search::resolved_filter(&params, &state.names_handle());
    let result = state
        .run_blocking_heavy(move |s| {
            s.with_read_db(|conn| {
                let rows = birdnet_db::sqlite::search_detections(
                    conn,
                    &filter,
                    MAX_EXPORT_ROWS.saturating_add(1),
                    0,
                )?;
                let runs = birdnet_db::sqlite::model_attribution(conn)?;
                // Station, deployment, device and clock, as the detections
                // export names them (RES-12, RES-22, MSC-13).
                let provenance = super::csv::RowProvenance::load(conn)?;
                Ok::<_, birdnet_db::sqlite::DbError>((rows, (runs, provenance)))
            })
        })
        .await;
    match result {
        Ok((rows, _)) if rows.len() > MAX_EXPORT_ROWS as usize => export_too_large(),
        Ok((rows, (runs, provenance))) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"detections-search.csv\"",
                ),
            ],
            super::csv::detections_to_csv_with(&rows, &runs, &provenance),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({ "error": crate::routes::log_internal("search export", &e) }).to_string(),
        )
            .into_response(),
    }
}
