//! Stub handlers returned when the `analytics` feature is not compiled.
//!
//! Each stub returns the standard "unavailable" JSON response with a hint
//! to rebuild with `--features analytics`.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde_json::Value;

use crate::state::AppState;

use super::helpers::ts_unavailable;
use super::params::{
    AccumulationQuery, AnomalyQuery, DailyQuery, DiversityQuery, GapsQuery, HourlyQuery, PeakQuery,
    SessionQuery, TrendQuery, WeeklyQuery,
};

pub(super) async fn hourly(
    State(_): State<AppState>,
    Query(_): Query<HourlyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("hourly activity")
}

pub(super) async fn daily(
    State(_): State<AppState>,
    Query(_): Query<DailyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("daily activity")
}

pub(super) async fn weekly(
    State(_): State<AppState>,
    Query(_): Query<WeeklyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("weekly activity")
}

pub(super) async fn heatmap(
    State(_): State<AppState>,
    Query(_): Query<HourlyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("hourly heatmap")
}

pub(super) async fn trend(
    State(_): State<AppState>,
    Query(_): Query<TrendQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("moving average trend")
}

pub(super) async fn anomalies(
    State(_): State<AppState>,
    Query(_): Query<AnomalyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("anomaly detection")
}

pub(super) async fn year_over_year(
    State(_): State<AppState>,
    Query(_): Query<WeeklyQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("year-over-year")
}

pub(super) async fn diversity(
    State(_): State<AppState>,
    Query(_): Query<DiversityQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("diversity")
}

pub(super) async fn accumulation(
    State(_): State<AppState>,
    Query(_): Query<AccumulationQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("accumulation curve")
}

pub(super) async fn peak_windows(
    State(_): State<AppState>,
    Query(_): Query<PeakQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("peak windows")
}

pub(super) async fn sessions(
    State(_): State<AppState>,
    Query(_): Query<SessionQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("activity sessions")
}

pub(super) async fn gaps(
    State(_): State<AppState>,
    Query(_): Query<GapsQuery>,
) -> (StatusCode, Json<Value>) {
    ts_unavailable("gap detection")
}
