//! Dashboard stats bar HTMX partial.

use axum::extract::State;
use axum::http::{StatusCode, header};

use crate::routes::pages::{escape_html, today_count};
use crate::state::AppState;

/// HTMX partial: summary stat cards (total, species, today, last hour, last detection).
pub(super) async fn stats_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let today = today_count(conn);
            let last_hour = birdnet_db::sqlite::last_hour_count(conn).unwrap_or(0);
            let latest = birdnet_db::sqlite::latest_detection(conn).ok().flatten();
            (total, species, today, last_hour, latest)
        })
    })
    .await;

    match result {
        Ok((total, species, today, last_hour, latest)) => {
            let latest_html = if let Some((_, time, name)) = latest {
                format!(
                    "<div class=\"stat-card\">\
                      <div class=\"value\" style=\"font-size:1.2rem;\">{time}</div>\
                      <div class=\"label\">Last: {name}</div>\
                    </div>",
                    time = escape_html(&time),
                    name = escape_html(&name),
                )
            } else {
                "<div class=\"stat-card\"><div class=\"value\">--</div>\
                 <div class=\"label\">No Detections</div></div>"
                    .to_string()
            };

            let html = format!(
                "<div class=\"stat-card\">\
                   <div class=\"value\">{total}</div>\
                   <div class=\"label\">Total Detections</div>\
                 </div>\
                 <div class=\"stat-card\">\
                   <div class=\"value\">{species}</div>\
                   <div class=\"label\">Unique Species</div>\
                 </div>\
                 <div class=\"stat-card\">\
                   <div class=\"value\">{today}</div>\
                   <div class=\"label\">Today</div>\
                 </div>\
                 <div class=\"stat-card\">\
                   <div class=\"value\" style=\"color:var(--warning);\">{last_hour}</div>\
                   <div class=\"label\">Last Hour</div>\
                 </div>\
                 {latest_html}",
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading stats</p>".to_string(),
        ),
    }
}
