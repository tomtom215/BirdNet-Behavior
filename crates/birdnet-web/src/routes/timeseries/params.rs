//! Query parameter types for time-series endpoints.

use serde::Deserialize;

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct HourlyQuery {
    pub(super) days: Option<u32>,
    pub(super) species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct DailyQuery {
    pub(super) days: Option<u32>,
    pub(super) species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct WeeklyQuery {
    pub(super) weeks: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct TrendQuery {
    pub(super) window: Option<u32>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct AnomalyQuery {
    pub(super) z: Option<f64>,
    pub(super) window: Option<u32>,
    pub(super) days: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct DiversityQuery {
    pub(super) days: Option<u32>,
    pub(super) shannon: Option<bool>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct AccumulationQuery {
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct PeakQuery {
    pub(super) window: Option<u32>,
    pub(super) hop: Option<u32>,
    pub(super) days: Option<u32>,
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct SessionQuery {
    pub(super) gap: Option<u32>,
    pub(super) date: Option<String>,
    pub(super) days: Option<u32>,
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct GapsQuery {
    pub(super) date: Option<String>,
    pub(super) threshold: Option<u32>,
    pub(super) days: Option<u32>,
}
