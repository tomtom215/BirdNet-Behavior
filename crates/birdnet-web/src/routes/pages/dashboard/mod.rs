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

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Router, routing::get};

use super::DASHBOARD_HTML;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard_page))
        .route("/kiosk", get(kiosk::kiosk_page))
        .route("/pages/stats", get(stats::stats_partial))
        .route("/pages/hero-status", get(stats::hero_status_partial))
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

async fn dashboard_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // First run: a station with no detections that hasn't completed onboarding is
    // bounced to the setup wizard instead of being shown an empty dashboard.
    if first_run_needs_onboarding(&state) {
        return Redirect::to("/onboarding").into_response();
    }

    // O-16 — feed_rows skeleton on first paint. The htmx response from
    // `/pages/detections` swaps to either rendered rows or
    // `empty_states::quiet_yard()` for the 0-row case, resolving the
    // contradictory note in the DIFF: skeleton WHILE loading, quiet_yard
    // ONCE we know the yard is quiet.
    // O-20 — help link wires the hero eyebrow to the dashboard mdBook page.
    let content = DASHBOARD_HTML
        .replace("{{skel_detections}}", &super::skeletons::feed_rows(8))
        .replace(
            "{{help_link}}",
            &super::help::help_link(super::help::Topic::Dashboard),
        );
    super::render_page_for_request("Dashboard", &content, "dashboard", &headers).into_response()
}

/// Whether to bounce a fresh station to the onboarding wizard: no detections yet
/// **and** onboarding not marked complete. Fails safe — any DB error is treated
/// as "already set up" so a hiccup never traps the operator on `/onboarding`.
fn first_run_needs_onboarding(state: &AppState) -> bool {
    state.with_db(|conn| {
        let onboarded = birdnet_db::settings::get_or(conn, "onboarding_complete", "false")
            .map_or(true, |v| v == "true");
        let has_detections = birdnet_db::sqlite::detection_count(conn).map_or(true, |n| n > 0);
        !onboarded && !has_detections
    })
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
