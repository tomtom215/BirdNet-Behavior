//! Time-series analytics API endpoints.
//!
//! Provides REST endpoints for temporal analysis of bird detections:
//! tumbling windows (hourly/daily/weekly), moving averages, peak windows,
//! activity sessions, diversity indices, and anomaly detection.
//!
//! All endpoints use `birdnet-timeseries` which runs standard DuckDB SQL
//! window functions — **no behavioral extension required**.
//!
//! Base path: `/api/v2/timeseries/`

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Time-series route definitions.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/timeseries/hourly", get(hourly))
        .route("/timeseries/daily", get(daily))
        .route("/timeseries/weekly", get(weekly))
        .route("/timeseries/heatmap", get(heatmap))
        .route("/timeseries/trend", get(trend))
        .route("/timeseries/anomalies", get(anomalies))
        .route("/timeseries/year-over-year", get(year_over_year))
        .route("/timeseries/diversity", get(diversity))
        .route("/timeseries/accumulation", get(accumulation))
        .route("/timeseries/peak-windows", get(peak_windows))
        .route("/timeseries/sessions", get(sessions))
        .route("/timeseries/gaps", get(gaps))
        .route("/timeseries/status", get(status))
}

// ---------------------------------------------------------------------------
// Query parameter types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[allow(dead_code)]
struct HourlyQuery {
    days: Option<u32>,
    species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DailyQuery {
    days: Option<u32>,
    species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct WeeklyQuery {
    weeks: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct TrendQuery {
    window: Option<u32>,
    from: Option<String>,
    to: Option<String>,
    species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AnomalyQuery {
    z: Option<f64>,
    window: Option<u32>,
    days: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DiversityQuery {
    days: Option<u32>,
    shannon: Option<bool>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AccumulationQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct PeakQuery {
    window: Option<u32>,
    hop: Option<u32>,
    days: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SessionQuery {
    gap: Option<u32>,
    date: Option<String>,
    days: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct GapsQuery {
    date: Option<String>,
    threshold: Option<u32>,
    days: Option<u32>,
}

// ---------------------------------------------------------------------------
// Handlers (analytics feature enabled)
// ---------------------------------------------------------------------------

#[cfg(feature = "analytics")]
async fn hourly(
    State(state): State<AppState>,
    Query(q): Query<HourlyQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("hourly activity");
    }
    let params = birdnet_timeseries::types::params::HourlyParams {
        lookback_days: q.days.unwrap_or(7),
        species: q.species,
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.hourly_activity(&params))
    })
    .await;
    handle_ts_result(result, "hourly")
}

#[cfg(feature = "analytics")]
async fn daily(
    State(state): State<AppState>,
    Query(q): Query<DailyQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("daily activity");
    }
    let params = birdnet_timeseries::types::params::DailyParams {
        lookback_days: q.days.unwrap_or(30),
        species: q.species,
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.daily_activity(&params))
    })
    .await;
    handle_ts_result(result, "daily")
}

#[cfg(feature = "analytics")]
async fn weekly(
    State(state): State<AppState>,
    Query(q): Query<WeeklyQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("weekly activity");
    }
    let params = birdnet_timeseries::types::params::WeeklyParams {
        lookback_weeks: q.weeks.unwrap_or(52),
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.weekly_activity(&params))
    })
    .await;
    handle_ts_result(result, "weekly")
}

#[cfg(feature = "analytics")]
async fn heatmap(
    State(state): State<AppState>,
    Query(q): Query<HourlyQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("hourly heatmap");
    }
    let params = birdnet_timeseries::types::params::HourlyParams {
        lookback_days: q.days.unwrap_or(90),
        species: q.species,
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.hourly_heatmap(&params))
    })
    .await;
    handle_ts_result(result, "heatmap")
}

#[cfg(feature = "analytics")]
async fn trend(
    State(state): State<AppState>,
    Query(q): Query<TrendQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("moving average trend");
    }
    let params = birdnet_timeseries::types::params::TrendParams {
        window_days: q.window.unwrap_or(7),
        from_date: q.from.or_else(|| {
            Some("CURRENT_DATE - INTERVAL 90 DAYS".into())
        }),
        to_date: q.to,
        species: q.species,
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.moving_average(&params))
    })
    .await;
    handle_ts_result(result, "trend")
}

#[cfg(feature = "analytics")]
async fn anomalies(
    State(state): State<AppState>,
    Query(q): Query<AnomalyQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("anomaly detection");
    }
    let params = birdnet_timeseries::types::params::AnomalyParams {
        z_threshold: q.z.unwrap_or(2.0),
        window_days: q.window.unwrap_or(30),
        lookback_days: q.days.unwrap_or(180),
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.anomalies(&params))
    })
    .await;
    handle_ts_result(result, "anomalies")
}

#[cfg(feature = "analytics")]
async fn year_over_year(
    State(state): State<AppState>,
    Query(q): Query<WeeklyQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("year-over-year");
    }
    let params = birdnet_timeseries::types::params::WeeklyParams {
        lookback_weeks: q.weeks.unwrap_or(52),
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.year_over_year(&params))
    })
    .await;
    handle_ts_result(result, "year_over_year")
}

#[cfg(feature = "analytics")]
async fn diversity(
    State(state): State<AppState>,
    Query(q): Query<DiversityQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("diversity");
    }
    let params = birdnet_timeseries::types::params::DiversityParams {
        lookback_days: q.days.unwrap_or(90),
        include_shannon: q.shannon.unwrap_or(true),
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.daily_richness(&params))
    })
    .await;
    handle_ts_result(result, "diversity")
}

#[cfg(feature = "analytics")]
async fn accumulation(
    State(state): State<AppState>,
    Query(q): Query<AccumulationQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("accumulation curve");
    }
    let from = q.from;
    let to = q.to;
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.accumulation_curve(from, to))
    })
    .await;
    handle_ts_result(result, "accumulation")
}

#[cfg(feature = "analytics")]
async fn peak_windows(
    State(state): State<AppState>,
    Query(q): Query<PeakQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("peak windows");
    }
    let params = birdnet_timeseries::types::params::PeakParams {
        window_minutes: q.window.unwrap_or(15),
        hop_minutes: q.hop.unwrap_or(5),
        lookback_days: q.days.unwrap_or(1),
        limit: q.limit.unwrap_or(10),
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.peak_windows(&params))
    })
    .await;
    handle_ts_result(result, "peak_windows")
}

#[cfg(feature = "analytics")]
async fn sessions(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("activity sessions");
    }
    let params = birdnet_timeseries::types::params::SessionParams {
        gap_minutes: q.gap.unwrap_or(30),
        date_filter: q.date,
        lookback_days: q.days.unwrap_or(7),
        limit: q.limit.unwrap_or(100),
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.activity_sessions(&params))
    })
    .await;
    handle_ts_result(result, "sessions")
}

#[cfg(feature = "analytics")]
async fn gaps(
    State(state): State<AppState>,
    Query(q): Query<GapsQuery>,
) -> (StatusCode, Json<Value>) {
    if !state.has_analytics() {
        return ts_unavailable("gap detection");
    }
    let threshold = q.threshold.unwrap_or(30);
    let lookback = q.days.unwrap_or(7);

    if let Some(date) = q.date {
        let result = tokio::task::spawn_blocking(move || {
            state.with_timeseries(|ts| ts.intraday_gaps(&date, threshold))
        })
        .await;
        return handle_ts_result(result, "gaps");
    }

    let result = tokio::task::spawn_blocking(move || {
        state.with_timeseries(|ts| ts.daily_max_gaps(lookback, threshold))
    })
    .await;
    handle_ts_result(result, "gaps")
}

// ---------------------------------------------------------------------------
// Stub handlers (feature not compiled)
// ---------------------------------------------------------------------------

#[cfg(not(feature = "analytics"))]
async fn hourly(
    State(_): State<AppState>,
    Query(_): Query<HourlyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("hourly activity")
}

#[cfg(not(feature = "analytics"))]
async fn daily(
    State(_): State<AppState>,
    Query(_): Query<DailyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("daily activity")
}

#[cfg(not(feature = "analytics"))]
async fn weekly(
    State(_): State<AppState>,
    Query(_): Query<WeeklyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("weekly activity")
}

#[cfg(not(feature = "analytics"))]
async fn heatmap(
    State(_): State<AppState>,
    Query(_): Query<HourlyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("hourly heatmap")
}

#[cfg(not(feature = "analytics"))]
async fn trend(
    State(_): State<AppState>,
    Query(_): Query<TrendQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("moving average trend")
}

#[cfg(not(feature = "analytics"))]
async fn anomalies(
    State(_): State<AppState>,
    Query(_): Query<AnomalyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("anomaly detection")
}

#[cfg(not(feature = "analytics"))]
async fn year_over_year(
    State(_): State<AppState>,
    Query(_): Query<WeeklyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("year-over-year")
}

#[cfg(not(feature = "analytics"))]
async fn diversity(
    State(_): State<AppState>,
    Query(_): Query<DiversityQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("diversity")
}

#[cfg(not(feature = "analytics"))]
async fn accumulation(
    State(_): State<AppState>,
    Query(_): Query<AccumulationQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("accumulation curve")
}

#[cfg(not(feature = "analytics"))]
async fn peak_windows(
    State(_): State<AppState>,
    Query(_): Query<PeakQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("peak windows")
}

#[cfg(not(feature = "analytics"))]
async fn sessions(
    State(_): State<AppState>,
    Query(_): Query<SessionQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("activity sessions")
}

#[cfg(not(feature = "analytics"))]
async fn gaps(
    State(_): State<AppState>,
    Query(_): Query<GapsQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("gap detection")
}

// ---------------------------------------------------------------------------
// Status (always available)
// ---------------------------------------------------------------------------

async fn status(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();
    (
        StatusCode::OK,
        Json(json!({
            "timeseries_compiled": compiled,
            "timeseries_configured": configured,
            "note": "Time-series analytics use standard DuckDB SQL — no extension required",
            "endpoints": {
                "hourly":        "/timeseries/hourly?days=7&species=...",
                "daily":         "/timeseries/daily?days=30&species=...",
                "weekly":        "/timeseries/weekly?weeks=52",
                "heatmap":       "/timeseries/heatmap?days=90",
                "trend":         "/timeseries/trend?window=7&from=2026-01-01&species=...",
                "anomalies":     "/timeseries/anomalies?z=2.0&window=30&days=180",
                "year_over_year":"/timeseries/year-over-year?weeks=52",
                "diversity":     "/timeseries/diversity?days=90&shannon=true",
                "accumulation":  "/timeseries/accumulation?from=2026-01-01",
                "peak_windows":  "/timeseries/peak-windows?window=15&hop=5&days=1&limit=10",
                "sessions":      "/timeseries/sessions?gap=30&date=2026-03-12&limit=100",
                "gaps":          "/timeseries/gaps?date=2026-03-12&threshold=30",
            }
        })),
    )
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn ts_unavailable(endpoint: &str) -> (StatusCode, Json<Value>) {
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

#[cfg(feature = "analytics")]
fn handle_ts_result<T: serde::Serialize>(
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
            (
                StatusCode::OK,
                Json(json!({ key: v, "total": total })),
            )
        }
        Ok(Some(Err(e))) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
        Ok(None) => ts_unavailable(key),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("task error: {e}") })),
        ),
    }
}
