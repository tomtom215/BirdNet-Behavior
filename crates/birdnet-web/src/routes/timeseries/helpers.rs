//! Shared response helpers for time-series endpoints.

use axum::Json;
use axum::http::StatusCode;
use serde_json::{Value, json};

/// Return a standard "unavailable" response for a time-series endpoint.
pub(super) fn ts_unavailable(endpoint: &str) -> (StatusCode, Json<Value>) {
    let message = if cfg!(feature = "analytics") {
        "DuckDB not configured. Start with --analytics-db to enable time-series analytics."
    } else {
        "Time-series analytics not compiled. Rebuild with --features analytics."
    };
    (
        StatusCode::OK,
        Json(json!({
            "status": "unavailable",
            "endpoint": endpoint,
            "message": message,
        })),
    )
}

/// Unwrap a `spawn_blocking` result containing a time-series query result.
#[cfg(feature = "analytics")]
pub(super) fn handle_ts_result<T: serde::Serialize>(
    join_result: Result<
        Option<Result<T, birdnet_timeseries::TimeSeriesError>>,
        tokio::task::JoinError,
    >,
    key: &str,
) -> (StatusCode, Json<Value>) {
    match join_result {
        Ok(Some(Ok(data))) => {
            let v = serde_json::to_value(&data).unwrap_or(Value::Null);
            let total = v.as_array().map_or(0, Vec::len);
            (StatusCode::OK, Json(json!({ key: v, "total": total })))
        }
        Ok(Some(Err(e))) => {
            // Log server-side so intermittent query failures are diagnosable —
            // previously the detail only went into the JSON body, leaving an
            // unexplained 500 in the access log.
            tracing::error!(endpoint = key, error = %e, "time-series query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal server error" })),
            )
        }
        Ok(None) => ts_unavailable(key),
        Err(e) => {
            tracing::error!(endpoint = key, error = %e, "time-series task join failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal server error" })),
            )
        }
    }
}
