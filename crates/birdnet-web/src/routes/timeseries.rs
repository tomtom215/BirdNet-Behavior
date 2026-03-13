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

mod helpers;
mod params;
#[cfg(not(feature = "analytics"))]
mod stubs;

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

#[cfg(feature = "analytics")]
use axum::extract::Query;
#[cfg(feature = "analytics")]
use helpers::{handle_ts_result, ts_unavailable};
#[cfg(feature = "analytics")]
use params::{
    AccumulationQuery, AnomalyQuery, DailyQuery, DiversityQuery, GapsQuery, HourlyQuery,
    PeakQuery, SessionQuery, TrendQuery, WeeklyQuery,
};

// Stub handlers when analytics feature is not compiled.
#[cfg(not(feature = "analytics"))]
use stubs::{
    accumulation, anomalies, daily, diversity, gaps, heatmap, hourly, peak_windows, sessions,
    trend, weekly, year_over_year,
};

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
