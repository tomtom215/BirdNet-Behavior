//! Recording Browser page and HTMX partials.
//!
//! Provides two views for browsing recordings:
//! - By Species: list species, click to see their recordings
//! - By Date: list dates, click to see recordings for that day

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, response::Html, routing::get};
use serde::Deserialize;

use super::atoms::avatar;
use super::{RECORDINGS_PAGE_HTML, escape_html, simple_url_encode};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/recordings", get(recordings_page))
        .route("/pages/recordings-species-list", get(species_list_partial))
        .route("/pages/recordings-date-list", get(date_list_partial))
        .route(
            "/pages/recordings-by-species",
            get(recordings_by_species_partial),
        )
        .route("/pages/recordings-by-date", get(recordings_by_date_partial))
        .route(
            "/pages/recordings-relabel",
            axum::routing::post(relabel_recording),
        )
        .route(
            "/pages/recordings-delete",
            axum::routing::post(delete_recording),
        )
}

async fn recordings_page(headers: HeaderMap) -> Html<String> {
    // O-20 help link in the page-head eyebrow.
    let body = RECORDINGS_PAGE_HTML.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Recordings),
    );
    super::render_page_for_request("Recordings", &body, "recordings", &headers)
}

/// HTMX partial: list of species with detection counts.
async fn species_list_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::top_species(conn, 500))
    })
    .await;

    match result {
        Ok(Ok(species)) => {
            if species.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="rec-muted">No species detected yet.</p>"#.to_string(),
                );
            }
            let mut html = String::with_capacity(2048);
            for s in &species {
                let enc = simple_url_encode(&s.com_name);
                let _ = write!(
                    html,
                    r##"<div class="species-item rec-item"
                         hx-get="/pages/recordings-by-species?name={enc}"
                         hx-target="#recordings-detail-content"
                         hx-swap="innerHTML"
                         onclick="document.getElementById(&quot;recordings-detail&quot;).style.display=&quot;&quot;">
  {av}
  <div class="rec-item-main">
    <span class="species-name">{name}</span>
    <span class="bnb-meta rec-sci">{sci}</span>
  </div>
  <span class="species-count">{count}</span>
</div>"##,
                    av = avatar(&s.com_name, ""),
                    name = escape_html(&s.com_name),
                    sci = escape_html(&s.sci_name),
                    count = s.count,
                );
            }
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading species</p>".to_string(),
        ),
    }
}

/// HTMX partial: list of dates with detection counts.
async fn date_list_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::detection_dates(conn, 90))
    })
    .await;

    match result {
        Ok(Ok(dates)) => {
            if dates.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="rec-muted">No detection dates found.</p>"#.to_string(),
                );
            }
            let mut html = String::with_capacity(1024);
            for date in &dates {
                let enc = simple_url_encode(date);
                let _ = write!(
                    html,
                    r##"<div class="species-item rec-item"
                         hx-get="/pages/recordings-by-date?date={enc}"
                         hx-target="#recordings-detail-content"
                         hx-swap="innerHTML"
                         onclick="document.getElementById(&quot;recordings-detail&quot;).style.display=&quot;&quot;">
  <span class="species-name">{date}</span>
</div>"##,
                    date = escape_html(date),
                );
            }
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading dates</p>".to_string(),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct BySpeciesQuery {
    name: String,
}

/// HTMX partial: recordings for a specific species.
async fn recordings_by_species_partial(
    State(state): State<AppState>,
    Query(query): Query<BySpeciesQuery>,
) -> impl IntoResponse {
    let name = query.name;

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::detections_by_species(conn, &name, 100))
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            let html = render_detection_list(&detections, true);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading recordings</p>".to_string(),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ByDateQuery {
    date: String,
}

/// HTMX partial: recordings for a specific date.
async fn recordings_by_date_partial(
    State(state): State<AppState>,
    Query(query): Query<ByDateQuery>,
) -> impl IntoResponse {
    let date = query.date;

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::detections_by_date(conn, &date))
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            let html = render_detection_list(&detections, false);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading recordings</p>".to_string(),
        ),
    }
}

/// Render a list of detections with audio players and actions.
fn render_detection_list(
    detections: &[birdnet_db::sqlite::DetectionRow],
    show_date: bool,
) -> String {
    if detections.is_empty() {
        return r#"<p class="rec-muted">No recordings found.</p>"#.to_string();
    }

    let mut html = String::with_capacity(4096);
    let _ = write!(
        html,
        r#"<p class="rec-count-note">{count} recordings</p>"#,
        count = detections.len(),
    );

    for d in detections {
        let conf_pct = d.confidence * 100.0;
        let cls = conf_class(conf_pct);
        let enc_name = simple_url_encode(&d.com_name);

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
                    r#"<audio controls preload="none" class="rec-audio">
                      <source src="/api/v2/recordings/{safe}" type="audio/wav">
                    </audio>"#
                )
            })
            .unwrap_or_default();

        let date_display = if show_date {
            format!(r#"<span class="rec-date">{}</span>"#, escape_html(&d.date))
        } else {
            String::new()
        };

        let _ = write!(
            html,
            r#"<div class="rec-row">
  {av}
  <div class="rec-row-main">
    <div class="rec-row-head">
      <a href="/species/detail?name={enc_name}" class="rec-name-link">{com_name}</a>
      <span class="conf {cls}">{conf_pct:.0}%</span>
      {date_display}
    </div>
    <div class="rec-sub">{time} &middot; <i>{sci_name}</i></div>
    {audio}
  </div>
  <div class="rec-actions">
    <button hx-post="/pages/recordings-delete"
            hx-vals='{{"date":"{date_raw}","time":"{time_raw}","sci_name":"{sci_raw}"}}'
            hx-target="closest .rec-row"
            hx-swap="outerHTML"
            hx-confirm="Delete this recording?"
            data-confirm-action="hx-post"
            data-confirm-url="/pages/recordings-delete"
            data-confirm-title="Delete recording"
            data-confirm-body="Delete this recording?"
            data-confirm-confirm-label="Delete"
            data-confirm-style="danger"
            class="rec-del-btn">
      Delete
    </button>
  </div>
</div>"#,
            av = avatar(&d.com_name, ""),
            com_name = escape_html(&d.com_name),
            sci_name = escape_html(&d.sci_name),
            time = escape_html(&d.time),
            date_raw = escape_html(&d.date),
            time_raw = escape_html(&d.time),
            sci_raw = escape_html(&d.sci_name),
        );
    }

    html
}

#[derive(Debug, Deserialize)]
struct RecordingDeleteForm {
    date: String,
    time: String,
    sci_name: String,
}

async fn delete_recording(
    State(state): State<AppState>,
    Form(form): Form<RecordingDeleteForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::delete_detection(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;

    // Return empty string to remove the element via hx-swap="outerHTML"
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        String::new(),
    )
}

#[derive(Debug, Deserialize)]
struct RecordingRelabelForm {
    date: String,
    time: String,
    old_sci_name: String,
    new_sci_name: String,
    new_com_name: String,
}

async fn relabel_recording(
    State(state): State<AppState>,
    Form(form): Form<RecordingRelabelForm>,
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
        r#"<p class="rec-relabel-ok">Re-labeled successfully. Refresh to see changes.</p>"#
            .to_string(),
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
