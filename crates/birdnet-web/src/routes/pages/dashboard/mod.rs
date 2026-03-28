//! Dashboard page and stats/detection/species HTMX partials.
//!
//! Split into focused sub-modules:
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

use axum::response::Html;
use axum::{Router, routing::get};

use super::DASHBOARD_HTML;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard_page))
        .route("/kiosk", get(kiosk::kiosk_page))
        .route("/pages/stats", get(stats::stats_partial))
        .route("/pages/detections", get(partials::detections_partial))
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

async fn dashboard_page() -> Html<String> {
    super::render_page("Dashboard", DASHBOARD_HTML, "dashboard")
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
