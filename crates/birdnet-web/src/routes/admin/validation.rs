//! Validation samples (RES-07): draw one by a recorded seed, review it blind,
//! export the verdicts.
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET  | `/admin/validation/samples` | every sample's header |
//! | POST | `/admin/validation/samples` | draw one: a JSON [`SampleProtocol`] |
//! | GET  | `/admin/validation/samples/{id}` | the protocol and the items, blind when it says so |
//! | POST | `/admin/validation/samples/{id}/verdicts` | `{"position": n, "verdict": "present"}` |
//! | GET  | `/admin/validation/samples/{id}/export.csv` | items, weights, scores and latest verdicts |
//!
//! Under `/admin`, so a write needs the administrator and a read a signed-in
//! account. The rule is `birdnet_db::sqlite::queries::validation`; nothing
//! here computes an accuracy figure.

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json, Router, routing::get};
use birdnet_db::sqlite::queries::validation::{self, SampleError, SampleProtocol, VERDICTS};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth_middleware::RequestUser;
use crate::routes::export::escape_csv;
use crate::state::AppState;

/// Mount the routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/validation/samples",
            get(list_samples).post(create_sample),
        )
        .route("/admin/validation/samples/{id}", get(show_sample))
        .route(
            "/admin/validation/samples/{id}/verdicts",
            axum::routing::post(add_verdict),
        )
        .route(
            "/admin/validation/samples/{id}/export.csv",
            get(export_sample),
        )
}

/// What every response says the sample is not.
const NOT_ACCURACY: &str = "A validation sample's verdicts estimate precision only for the frame \
it was drawn from, weighted by 1 / inclusion_probability, and say nothing about missed birds. \
The station computes no accuracy figure from them. A score is not a probability.";

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn internal<E: std::fmt::Display>(e: &E) -> Response {
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        crate::routes::log_internal("validation sample", e),
    )
}

async fn list_samples(State(state): State<AppState>) -> Response {
    match state
        .run_blocking(|s| s.with_read_db(validation::samples))
        .await
    {
        Ok(samples) => Json(json!({ "samples": samples, "note": NOT_ACCURACY })).into_response(),
        Err(e) => internal(&e),
    }
}

async fn create_sample(
    State(state): State<AppState>,
    Json(protocol): Json<SampleProtocol>,
) -> Response {
    if let Err(e) = protocol.validate() {
        return json_error(StatusCode::BAD_REQUEST, &e.0);
    }
    match state
        .run_blocking(move |s| s.with_db(|conn| validation::create_sample(conn, &protocol)))
        .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({ "id": id, "href": format!("/admin/validation/samples/{id}") })),
        )
            .into_response(),
        Err(SampleError::Protocol(e)) => json_error(StatusCode::BAD_REQUEST, &e.0),
        Err(e @ SampleError::TooLarge(_)) => {
            json_error(StatusCode::PAYLOAD_TOO_LARGE, &e.to_string())
        }
        Err(SampleError::Db(e)) => internal(&e),
    }
}

async fn show_sample(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let read = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| {
                let Some(sample) = validation::sample(conn, id)? else {
                    return Ok(None);
                };
                let items = if sample.blind {
                    // Blind: the items only, which carry no score and no model.
                    validation::sample_items(conn, id)?
                        .into_iter()
                        .map(|i| serde_json::to_value(i).unwrap_or(Value::Null))
                        .collect::<Vec<_>>()
                } else {
                    validation::sample_export(conn, id)?
                        .into_iter()
                        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                        .collect()
                };
                Ok::<_, birdnet_db::sqlite::DbError>(Some((sample, items)))
            })
        })
        .await;
    match read {
        Ok(Some((sample, items))) => Json(json!({
            "sample": sample,
            "items": items,
            "verdicts": VERDICTS,
            "note": NOT_ACCURACY,
        }))
        .into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "no such sample"),
        Err(e) => internal(&e),
    }
}

#[derive(Debug, Deserialize)]
struct VerdictBody {
    position: i64,
    verdict: String,
}

async fn add_verdict(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    user: Option<Extension<RequestUser>>,
    Json(body): Json<VerdictBody>,
) -> Response {
    if !VERDICTS.contains(&body.verdict.as_str()) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "verdict is one of present, absent, uncertain, not_identifiable",
        );
    }
    let reviewer = user.map(|Extension(u)| u.user.username);
    match state
        .run_blocking(move |s| {
            s.with_db(|conn| {
                validation::record_verdict(
                    conn,
                    id,
                    body.position,
                    &body.verdict,
                    reviewer.as_deref(),
                )
            })
        })
        .await
    {
        Ok(true) => (StatusCode::CREATED, Json(json!({ "recorded": true }))).into_response(),
        Ok(false) => json_error(StatusCode::NOT_FOUND, "no such item in that sample"),
        Err(e) => internal(&e),
    }
}

async fn export_sample(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let read = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| {
                let Some(sample) = validation::sample(conn, id)? else {
                    return Ok(None);
                };
                Ok::<_, birdnet_db::sqlite::DbError>(Some((
                    sample,
                    validation::sample_export(conn, id)?,
                )))
            })
        })
        .await;
    let (sample, rows) = match read {
        Ok(Some(v)) => v,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "no such sample"),
        Err(e) => return internal(&e),
    };
    let mut csv = String::from(
        "sample_id,protocol_sha256,seed,position,date,time,sci_name,file_name,stratum,\
         stratum_size,inclusion_probability,weight,confidence,model_id,run_id,verdict,\
         reviewer_name,verdict_at,verdict_count\n",
    );
    for r in rows {
        let i = &r.item;
        let line = [
            sample.id.to_string(),
            sample.protocol_sha256.clone(),
            sample.seed.to_string(),
            i.position.to_string(),
            escape_csv(&i.date),
            escape_csv(&i.time),
            escape_csv(&i.sci_name),
            escape_csv(&i.file_name),
            escape_csv(&i.stratum),
            i.stratum_size.to_string(),
            format!("{}", i.inclusion_probability),
            format!("{}", 1.0 / i.inclusion_probability),
            r.confidence.map_or_default(|c| format!("{c}")),
            r.model_id.as_deref().map_or_default(escape_csv),
            r.run_id.map_or_default(|v| v.to_string()),
            r.verdict.as_deref().map_or_default(escape_csv),
            r.reviewer_name.as_deref().map_or_default(escape_csv),
            r.verdict_at.as_deref().map_or_default(escape_csv),
            r.verdict_count.to_string(),
        ]
        .join(",");
        csv.push_str(&line);
        csv.push('\n');
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"validation-sample-{id}.csv\""),
            ),
        ],
        csv,
    )
        .into_response()
}
