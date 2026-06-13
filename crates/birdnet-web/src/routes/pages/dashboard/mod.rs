//! Detection/stats/species HTMX partials and kiosk mode.
//!
//! The full dashboard page merged into the Today home (`pages::today`) with
//! the v3 spine; this module keeps the partial fleet that home (and the
//! kiosk wall display) is composed from.
//!
//! | Module         | Responsibility                            |
//! |----------------|-------------------------------------------|
//! | `stats`        | Stats bar partial                         |
//! | `partials`     | Detections table, top species, charts     |
//! | `kiosk`        | Kiosk mode page and content partial       |
//! | `heatmap_widget` | Species × hour activity heatmap widget  |

mod heatmap_widget;
mod kiosk;
mod partials;
mod stats;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Mount the kiosk page and all HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/kiosk", get(kiosk::kiosk_page))
        .route("/pages/stats", get(stats::stats_partial))
        .route("/pages/hero-status", get(stats::hero_status_partial))
        .route("/pages/detections", get(partials::detections_partial))
        .route(
            "/pages/best-detections",
            get(partials::best_detections_partial),
        )
        .route("/pages/top-species", get(partials::top_species_partial))
        .route("/pages/species-list", get(partials::species_list_partial))
        .route("/pages/hourly-chart", get(partials::hourly_chart_partial))
        .route("/pages/daily-chart", get(partials::daily_chart_partial))
        .route(
            "/pages/confidence-chart",
            get(partials::confidence_chart_partial),
        )
        .route("/pages/kiosk-content", get(kiosk::kiosk_content_partial))
        .route("/pages/most-recent", get(partials::most_recent_partial))
        .route(
            "/pages/activity-heatmap",
            get(heatmap_widget::activity_heatmap_partial),
        )
}

/// Confidence class for badge coloring.
pub(crate) fn conf_class(pct: f64) -> &'static str {
    if pct >= 80.0 {
        "high"
    } else if pct >= 50.0 {
        "mid"
    } else {
        "low"
    }
}
