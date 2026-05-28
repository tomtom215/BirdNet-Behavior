//! Detection-review triage page and HTMX actions.
//!
//! A lightweight queue for confirming or rejecting individual detections. A
//! verdict is a non-destructive annotation (stored in `detection_reviews`),
//! distinct from quarantine — which gates uncertain rows *out* of `detections`
//! before they are admitted. Reviewers reach this page from the quarantine
//! page's tab strip and from each detection-detail page's Confirm/Reject
//! buttons.
//!
//! # Routes
//!
//! | Method | Path                              | Description                     |
//! |--------|-----------------------------------|---------------------------------|
//! | GET    | `/detection-reviews`              | Full triage page                |
//! | GET    | `/pages/detection-reviews-queue`  | HTMX partial: queue + verdicts  |
//! | POST   | `/pages/detection-review`         | Record a confirm/reject verdict |
//! | POST   | `/pages/detection-review-clear`   | Undo a verdict                  |

use std::fmt::Write as _;

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, simple_url_encode};
use crate::state::AppState;

const QUEUE_LIMIT: u32 = 40;
const RECENT_LIMIT: u32 = 25;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detection-reviews", get(detection_reviews_page))
        .route(
            "/pages/detection-reviews-queue",
            get(detection_reviews_queue_partial),
        )
        .route(
            "/pages/detection-review",
            axum::routing::post(detection_review_set),
        )
        .route(
            "/pages/detection-review-clear",
            axum::routing::post(detection_review_clear),
        )
        .route(
            "/pages/detection-review-inline",
            axum::routing::post(detection_review_inline),
        )
}

/// Form for recording a verdict.
#[derive(Debug, Deserialize)]
pub struct ReviewForm {
    pub date: String,
    pub time: String,
    pub sci_name: String,
    pub com_name: String,
    /// `confirmed` or `rejected`; anything else is rejected as bad input.
    pub status: String,
}

/// Form for undoing a verdict.
#[derive(Debug, Deserialize)]
pub struct ClearForm {
    pub date: String,
    pub time: String,
    pub sci_name: String,
}

async fn detection_reviews_page(headers: HeaderMap) -> Html<String> {
    let content = "<div style=\"margin-bottom:1.25rem;\">\
  <div class=\"bnb-eyebrow\">Quality control</div>\
  <h1 class=\"display\" style=\"font-size:32px;margin:0.1rem 0 0.35rem;\">Detection reviews</h1>\
  <p style=\"color:var(--fg-2);max-width:60ch;margin:0;\">Confirm detections that look right or reject likely misidentifications. \
  Verdicts are annotations — nothing is deleted. For uncertain rare birds held out of the log, see \
  <a href=\"/quarantine\" style=\"color:var(--primary);\">Quarantine</a>.</p>\
</div>\
<div id=\"dr-queue\" hx-get=\"/pages/detection-reviews-queue\" hx-trigger=\"load\" hx-swap=\"innerHTML\">\
  <p style=\"color:var(--fg-3);padding:2rem;text-align:center;\">Loading review queue…</p>\
</div>";
    super::render_page_for_request("Detection reviews", content, "", &headers)
}

async fn detection_reviews_queue_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let pending = birdnet_db::sqlite::unreviewed_recent_detections(conn, QUEUE_LIMIT)?;
            let recent = birdnet_db::sqlite::recent_detection_reviews(conn, RECENT_LIMIT)?;
            let counts = birdnet_db::sqlite::detection_review_counts(conn)?;
            Ok::<_, birdnet_db::sqlite::DbError>((pending, recent, counts))
        })
    })
    .await;

    match result {
        Ok(Ok((pending, recent, (confirmed, rejected)))) => {
            let html = render_queue(&pending, &recent, confirmed, rejected);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p style=\"color:var(--danger);\">Error loading the review queue.</p>".to_string(),
        ),
    }
}

fn render_queue(
    pending: &[birdnet_db::sqlite::UnreviewedDetection],
    recent: &[birdnet_db::sqlite::DetectionReview],
    confirmed: i64,
    rejected: i64,
) -> String {
    let mut html = String::with_capacity(8192);

    let _ = write!(
        html,
        "<div style=\"display:flex;gap:0.75rem;flex-wrap:wrap;margin-bottom:1rem;\">\
  <span class=\"bnb-pill moss\">&#10003; {confirmed} confirmed</span>\
  <span class=\"bnb-pill rare\">&#10007; {rejected} rejected</span>\
  <span class=\"bnb-pill\">{} awaiting review</span>\
</div>",
        pending.len()
    );

    html.push_str("<div class=\"bnb-card pad\" style=\"margin-bottom:1.25rem;\">");
    html.push_str("<h2 style=\"font-size:1.1rem;margin:0 0 0.75rem;\">Awaiting review</h2>");
    if pending.is_empty() {
        html.push_str(
            "<p style=\"color:var(--fg-3);margin:0;\">Every recent detection has a verdict. Nice and tidy.</p>",
        );
    } else {
        for d in pending {
            render_pending_row(&mut html, d);
        }
    }
    html.push_str("</div>");

    html.push_str("<div class=\"bnb-card pad\">");
    html.push_str("<h2 style=\"font-size:1.1rem;margin:0 0 0.75rem;\">Recent verdicts</h2>");
    if recent.is_empty() {
        html.push_str("<p style=\"color:var(--fg-3);margin:0;\">No verdicts recorded yet.</p>");
    } else {
        for r in recent {
            render_verdict_row(&mut html, r);
        }
    }
    html.push_str("</div>");

    html
}

fn render_pending_row(html: &mut String, d: &birdnet_db::sqlite::UnreviewedDetection) {
    let com = escape_html(&d.com_name);
    let sci = escape_html(&d.sci_name);
    let date = escape_html(&d.date);
    let time = escape_html(&d.time);
    let enc_com = simple_url_encode(&d.com_name);
    let conf_pct = d.confidence * 100.0;
    let conf_cls = if conf_pct >= 80.0 {
        "high"
    } else if conf_pct >= 50.0 {
        "mid"
    } else {
        "low"
    };
    // Per-row form: hidden inputs carry the (possibly apostrophe-bearing)
    // identity safely as escaped attribute values; the clicked submit button's
    // name=status value selects the verdict.
    let _ = write!(
        html,
        "<form hx-post=\"/pages/detection-review\" hx-target=\"#dr-queue\" hx-swap=\"innerHTML\" \
          style=\"display:flex;align-items:center;gap:0.75rem;flex-wrap:wrap;padding:0.6rem 0;border-top:0.5px solid var(--hairline);\">\
  <input type=\"hidden\" name=\"date\" value=\"{date}\">\
  <input type=\"hidden\" name=\"time\" value=\"{time}\">\
  <input type=\"hidden\" name=\"sci_name\" value=\"{sci}\">\
  <input type=\"hidden\" name=\"com_name\" value=\"{com}\">\
  <div style=\"flex:1 1 200px;min-width:0;\">\
    <a href=\"/species/detail?name={enc_com}\" style=\"font-weight:600;color:var(--fg);\">{com}</a>\
    <div style=\"color:var(--fg-3);font-size:0.8rem;font-style:italic;\">{sci}</div>\
    <div style=\"color:var(--fg-3);font-size:0.8rem;\">{date} · {time}</div>\
  </div>\
  <span class=\"conf {conf_cls}\">{conf_pct:.0}%</span>\
  <button type=\"submit\" name=\"status\" value=\"confirmed\" class=\"bnb-btn\" \
    style=\"background:var(--moss);color:var(--bg);border:none;white-space:nowrap;\">&#10003; Confirm</button>\
  <button type=\"submit\" name=\"status\" value=\"rejected\" class=\"bnb-btn ghost\" \
    style=\"white-space:nowrap;\">&#10007; Reject</button>\
</form>"
    );
}

fn render_verdict_row(html: &mut String, r: &birdnet_db::sqlite::DetectionReview) {
    let com = escape_html(&r.com_name);
    let date = escape_html(&r.date);
    let time = escape_html(&r.time);
    let sci = escape_html(&r.sci_name);
    let (badge, badge_cls) = if r.status == "confirmed" {
        ("&#10003; Confirmed", "moss")
    } else {
        ("&#10007; Rejected", "rare")
    };
    let _ = write!(
        html,
        "<form hx-post=\"/pages/detection-review-clear\" hx-target=\"#dr-queue\" hx-swap=\"innerHTML\" \
          style=\"display:flex;align-items:center;gap:0.75rem;flex-wrap:wrap;padding:0.5rem 0;border-top:0.5px solid var(--hairline);\">\
  <input type=\"hidden\" name=\"date\" value=\"{date}\">\
  <input type=\"hidden\" name=\"time\" value=\"{time}\">\
  <input type=\"hidden\" name=\"sci_name\" value=\"{sci}\">\
  <span class=\"bnb-pill {badge_cls}\">{badge}</span>\
  <div style=\"flex:1 1 180px;min-width:0;font-weight:500;\">{com}\
    <span style=\"color:var(--fg-3);font-weight:400;font-size:0.8rem;\"> · {date} {time}</span></div>\
  <button type=\"submit\" class=\"bnb-btn ghost\" style=\"white-space:nowrap;font-size:0.8rem;\">Undo</button>\
</form>"
    );
}

async fn detection_review_set(
    State(state): State<AppState>,
    Form(form): Form<ReviewForm>,
) -> impl IntoResponse {
    if let Some(status) = birdnet_db::sqlite::ReviewStatus::parse(&form.status) {
        let _ = tokio::task::spawn_blocking(move || {
            state.with_db(|conn| {
                birdnet_db::sqlite::set_detection_review(
                    conn,
                    &form.date,
                    &form.time,
                    &form.sci_name,
                    &form.com_name,
                    status,
                    None,
                )
            })
        })
        .await;
    } else {
        tracing::warn!(status = %form.status, "ignoring detection review with unknown status");
    }
    reload_queue()
}

async fn detection_review_clear(
    State(state): State<AppState>,
    Form(form): Form<ClearForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::clear_detection_review(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;
    reload_queue()
}

/// Inline verdict handler for the detection-detail page. Writes the verdict
/// and returns the self-replacing review widget reflecting the new state.
async fn detection_review_inline(
    State(state): State<AppState>,
    Form(form): Form<ReviewForm>,
) -> impl IntoResponse {
    let current = if let Some(status) = birdnet_db::sqlite::ReviewStatus::parse(&form.status) {
        let (date, time, sci, com) = (
            form.date.clone(),
            form.time.clone(),
            form.sci_name.clone(),
            form.com_name.clone(),
        );
        let _ = tokio::task::spawn_blocking(move || {
            state.with_db(|conn| {
                birdnet_db::sqlite::set_detection_review(
                    conn, &date, &time, &sci, &com, status, None,
                )
            })
        })
        .await;
        Some(status.as_str())
    } else {
        None
    };
    let widget = render_review_widget(
        &form.date,
        &form.time,
        &form.sci_name,
        &form.com_name,
        current,
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        widget,
    )
}

/// The self-replacing "Review this detection" widget shown on the
/// detection-detail page. `current` is the existing verdict, if any.
#[must_use]
pub(crate) fn render_review_widget(
    date: &str,
    time: &str,
    sci_name: &str,
    com_name: &str,
    current: Option<&str>,
) -> String {
    let date_e = escape_html(date);
    let time_e = escape_html(time);
    let sci_e = escape_html(sci_name);
    let com_e = escape_html(com_name);
    let badge = match current {
        Some("confirmed") => "<span class=\"bnb-pill moss\">&#10003; Confirmed</span>",
        Some("rejected") => "<span class=\"bnb-pill rare\">&#10007; Rejected</span>",
        _ => "<span class=\"bnb-pill\">Unreviewed</span>",
    };
    let confirm_cls = if current == Some("confirmed") {
        "bnb-btn"
    } else {
        "bnb-btn ghost"
    };
    let reject_cls = if current == Some("rejected") {
        "bnb-btn"
    } else {
        "bnb-btn ghost"
    };
    format!(
        "<div id=\"dr-review-widget\" class=\"bnb-card pad\" style=\"margin-top:16px;\">\
  <div class=\"section-header\"><div><div class=\"bnb-eyebrow\">Quality control</div><h3>Review this detection</h3></div>{badge}</div>\
  <form hx-post=\"/pages/detection-review-inline\" hx-target=\"#dr-review-widget\" hx-swap=\"outerHTML\" \
        style=\"display:flex;gap:8px;flex-wrap:wrap;margin-top:10px;\">\
    <input type=\"hidden\" name=\"date\" value=\"{date_e}\">\
    <input type=\"hidden\" name=\"time\" value=\"{time_e}\">\
    <input type=\"hidden\" name=\"sci_name\" value=\"{sci_e}\">\
    <input type=\"hidden\" name=\"com_name\" value=\"{com_e}\">\
    <button type=\"submit\" name=\"status\" value=\"confirmed\" class=\"{confirm_cls}\" \
      style=\"white-space:nowrap;\">&#10003; Confirm</button>\
    <button type=\"submit\" name=\"status\" value=\"rejected\" class=\"{reject_cls}\" \
      style=\"white-space:nowrap;\">&#10007; Reject</button>\
  </form>\
  <p class=\"bnb-meta\" style=\"margin-top:8px;\">Records a verdict in the <a href=\"/detection-reviews\" style=\"color:var(--primary);\">review queue</a>. Nothing is deleted.</p>\
</div>"
    )
}

/// Return an HTMX-trigger div that reloads the queue partial after a mutation.
fn reload_queue() -> impl IntoResponse {
    let target = "#dr-queue";
    let html = format!(
        "<div hx-get=\"/pages/detection-reviews-queue\" hx-trigger=\"load\" \
         hx-target=\"{target}\" hx-swap=\"innerHTML\"></div>"
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
