//! Today's Detections page and HTMX partials.
//!
//! The primary daily-use page showing today's detections in a searchable,
//! paginated list with delete support and auto-refresh.

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{TODAY_PAGE_HTML, escape_html, simple_url_encode, today_date_string};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/today", get(today_page))
        .route("/pages/today-list", get(today_partial))
        .route("/pages/today-count", get(today_count_partial))
        .route(
            "/pages/today-delete",
            axum::routing::post(delete_detection),
        )
        .route(
            "/pages/today-relabel",
            axum::routing::post(relabel_detection),
        )
}

/// Query parameters for the today list partial.
#[derive(Debug, Deserialize)]
pub struct TodayParams {
    /// Search filter. Prefix with "NOT " for exclusion.
    pub search: Option<String>,
    /// Pagination offset.
    pub offset: Option<u32>,
    /// Items per page (default 40).
    pub limit: Option<u32>,
}

/// Form data for deleting a detection.
#[derive(Debug, Deserialize)]
pub struct DeleteForm {
    pub date: String,
    pub time: String,
    pub sci_name: String,
}

/// Form data for re-labeling a detection.
#[derive(Debug, Deserialize)]
pub struct RelabelForm {
    pub date: String,
    pub time: String,
    pub old_sci_name: String,
    pub new_sci_name: String,
    pub new_com_name: String,
}

/// Render the full Today page.
async fn today_page() -> Html<String> {
    super::render_page("Today", TODAY_PAGE_HTML, "today")
}

/// HTMX partial: today's detection count (for the header badge).
async fn today_count_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
) -> impl IntoResponse {
    let today = today_date_string();
    let search = params.search.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::todays_detection_count(conn, &today, search.as_deref())
        })
    })
    .await;

    match result {
        Ok(Ok(count)) => {
            let label = if params.search.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                format!("{count} matching detections")
            } else {
                format!("{count} detections today")
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                label,
            )
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "Error loading count".to_string(),
        ),
    }
}

/// HTMX partial: paginated list of today's detections as cards.
async fn today_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
) -> impl IntoResponse {
    let today = today_date_string();
    let limit = params.limit.unwrap_or(40).min(200);
    let offset = params.offset.unwrap_or(0);
    let search = params.search.clone();
    let search2 = params.search.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let rows =
                birdnet_db::sqlite::todays_detections(conn, &today, search.as_deref(), limit, offset)?;
            let total =
                birdnet_db::sqlite::todays_detection_count(conn, &today, search.as_deref())?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, total))
        })
    })
    .await;

    match result {
        Ok(Ok((detections, total))) => {
            let mut html = String::with_capacity(4096);

            if detections.is_empty() && offset == 0 {
                html.push_str(
                    r#"<p style="color:var(--text-muted);text-align:center;padding:2rem;">No detections found today.</p>"#,
                );
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html);
            }

            for d in &detections {
                let conf_pct = d.confidence * 100.0;
                let cls = conf_class(conf_pct);
                let enc_name = simple_url_encode(&d.com_name);
                let enc_sci = simple_url_encode(&d.sci_name);

                // Audio player
                let audio = d
                    .file_name
                    .as_deref()
                    .filter(|f| !f.is_empty())
                    .map(|f| {
                        let basename = std::path::Path::new(f)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let safe = escape_html(&basename);
                        format!(
                            r#"<audio controls preload="none" style="width:100%;height:32px;margin-top:0.5rem;">
                              <source src="/api/v2/recordings/{safe}" type="audio/wav">
                            </audio>"#
                        )
                    })
                    .unwrap_or_default();

                let _ = write!(
                    html,
                    r##"<div class="card" style="display:flex;gap:1rem;align-items:flex-start;padding:0.75rem 1rem;">
  <div style="flex:1;min-width:0;">
    <div style="display:flex;align-items:center;gap:0.5rem;flex-wrap:wrap;">
      <a href="/species/detail?name={enc_name}" style="font-weight:600;color:var(--text);text-decoration:none;font-size:1rem;">{com_name}</a>
      <span class="conf {cls}">{conf_pct:.0}%</span>
    </div>
    <div style="color:var(--text-muted);font-size:0.8rem;font-style:italic;">{sci_name}</div>
    <div style="color:var(--text-muted);font-size:0.8rem;margin-top:0.25rem;">
      <a href="/detections/detail?date={date_enc}&time={time_enc}&name={enc_name}" style="color:var(--text-muted);text-decoration:none;">{time}</a>
    </div>
    {audio}
  </div>
  <div style="display:flex;flex-direction:column;gap:0.25rem;flex-shrink:0;">
    <button hx-post="/pages/today-delete"
            hx-vals='{{"date":"{date_raw}","time":"{time_raw}","sci_name":"{sci_name_raw}"}}'
            hx-target="#today-results"
            hx-swap="innerHTML"
            hx-include="#today-search"
            hx-confirm="Delete detection of {com_name} at {time}?"
            style="background:none;border:1px solid var(--danger);color:var(--danger);padding:0.2rem 0.5rem;border-radius:var(--radius);cursor:pointer;font-size:0.75rem;"
            title="Delete this detection">
      Delete
    </button>
  </div>
</div>"##,
                    com_name = escape_html(&d.com_name),
                    sci_name = escape_html(&d.sci_name),
                    time = escape_html(&d.time),
                    date_enc = simple_url_encode(&d.date),
                    time_enc = simple_url_encode(&d.time),
                    date_raw = escape_html(&d.date),
                    time_raw = escape_html(&d.time),
                    sci_name_raw = escape_html(&d.sci_name),
                );
            }

            // "Load more" button if there are more results
            let shown = offset + limit;
            #[allow(clippy::cast_sign_loss)]
            let total_u = total as u32;
            if shown < total_u {
                let search_param = search2
                    .as_ref()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| format!("&search={}", simple_url_encode(s)))
                    .unwrap_or_default();
                let _ = write!(
                    html,
                    r#"<div style="text-align:center;padding:1rem;">
  <button hx-get="/pages/today-list?offset={next_offset}&limit={limit}{search_param}"
          hx-target="#today-results"
          hx-swap="innerHTML"
          style="background:var(--bg-hover);border:1px solid var(--border);color:var(--text);padding:0.5rem 1.5rem;border-radius:var(--radius);cursor:pointer;font-size:0.9rem;">
    Load {limit} more ({remaining} remaining)
  </button>
</div>"#,
                    next_offset = shown,
                    remaining = total_u.saturating_sub(shown),
                );
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading detections</p>".to_string(),
        ),
    }
}

/// Delete a detection and re-render the list.
async fn delete_detection(
    State(state): State<AppState>,
    Form(form): Form<DeleteForm>,
) -> impl IntoResponse {
    let date = form.date;
    let time = form.time;
    let sci_name = form.sci_name;

    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::delete_detection(conn, &date, &time, &sci_name))
    })
    .await;

    // Return an HTMX redirect header to reload the today list
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html"),
        ],
        r#"<div hx-get="/pages/today-list" hx-trigger="load" hx-target="#today-results" hx-swap="innerHTML" hx-include="#today-search"></div>"#.to_string(),
    )
}

/// Re-label a detection and re-render the list.
async fn relabel_detection(
    State(state): State<AppState>,
    Form(form): Form<RelabelForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::relabel_detection(
                conn,
                &form.date,
                &form.time,
                &form.old_sci_name,
                &form.new_sci_name,
                &form.new_com_name,
            )
        })
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        r#"<div hx-get="/pages/today-list" hx-trigger="load" hx-target="#today-results" hx-swap="innerHTML" hx-include="#today-search"></div>"#.to_string(),
    )
}

fn conf_class(pct: f64) -> &'static str {
    if pct >= 80.0 {
        "high"
    } else if pct >= 50.0 {
        "mid"
    } else {
        "low"
    }
}
