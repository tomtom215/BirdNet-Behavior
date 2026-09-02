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

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, simple_url_encode};
use crate::state::AppState;

const QUEUE_LIMIT: u32 = 40;
const RECENT_LIMIT: u32 = 25;

/// Mount the detection-reviews page and its HTMX action routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detection-reviews", get(detection_reviews_page))
        .route(
            "/pages/detection-reviews-queue",
            get(detection_reviews_queue_partial),
        )
}

/// The state-changing half, mounted behind the admin auth middleware.
///
/// A verdict is a curator's claim about the record, and migration 27 carries it
/// into every analytic — so an unauthenticated write here silently rewrites the
/// station's numbers.
pub fn mutating_router() -> Router<AppState> {
    Router::new()
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
    /// Detection date in `YYYY-MM-DD` form.
    pub date: String,
    /// Detection time in `HH:MM:SS` form.
    pub time: String,
    /// Scientific name used with `date`/`time` to identify the detection row.
    pub sci_name: String,
    /// Common name stored alongside the verdict for display purposes.
    pub com_name: String,
    /// `confirmed` or `rejected`; anything else is rejected as bad input.
    pub status: String,
}

/// Form for undoing a verdict.
#[derive(Debug, Deserialize)]
pub struct ClearForm {
    /// Detection date in `YYYY-MM-DD` form.
    pub date: String,
    /// Detection time in `HH:MM:SS` form.
    pub time: String,
    /// Scientific name used with `date`/`time` to locate the verdict to clear.
    pub sci_name: String,
}

async fn detection_reviews_page(headers: HeaderMap) -> Html<String> {
    let content = "<div class=\"dr-head\">\
  <div class=\"bnb-eyebrow\">Quality control</div>\
  <h1 class=\"display dr-h1\">Detection reviews</h1>\
  <p class=\"dr-lede\">Confirm detections that look right or reject likely misidentifications. \
  Verdicts are annotations — nothing is deleted. For uncertain rare birds held out of the log, see \
  <a href=\"/quarantine\" class=\"dr-link\">Quarantine</a>.</p>\
</div>\
<div id=\"dr-queue\" hx-get=\"/pages/detection-reviews-queue\" hx-trigger=\"load\" hx-swap=\"innerHTML\">\
  <p class=\"dr-loading\">Loading review queue…</p>\
</div>";
    super::render_page_for_request("Detection reviews", content, "", &headers)
}

/// How the operator has narrowed the verdict list.
///
/// `status` is `confirmed` / `rejected` / absent (both). `offset` pages through
/// it. Both are query parameters rather than session state so a view is a URL —
/// which is the whole point of this change: a verdict has to be *findable*, and
/// a link to page three of the rejections is a thing you can keep.
#[derive(Debug, Default, Deserialize)]
pub struct QueueQuery {
    /// `confirmed`, `rejected`, or absent for both.
    #[serde(default)]
    pub status: Option<String>,
    /// How many verdicts to skip. Clamped by the handler.
    #[serde(default)]
    pub offset: Option<u32>,
}

async fn detection_reviews_queue_partial(
    State(state): State<AppState>,
    Query(q): Query<QueueQuery>,
) -> impl IntoResponse {
    // An unrecognised status is treated as "both" rather than rejected: this is
    // a view filter, and answering a mistyped URL with an error page helps
    // nobody find their verdict.
    let status = q
        .status
        .as_deref()
        .and_then(birdnet_db::sqlite::ReviewStatus::parse);
    let offset = q.offset.unwrap_or(0);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let pending = birdnet_db::sqlite::unreviewed_recent_detections(conn, QUEUE_LIMIT)?;
            let recent =
                birdnet_db::sqlite::detection_reviews_page(conn, status, RECENT_LIMIT, offset)?;
            let matching = birdnet_db::sqlite::detection_review_total(conn, status)?;
            let counts = birdnet_db::sqlite::detection_review_counts(conn)?;
            Ok::<_, birdnet_db::sqlite::DbError>((pending, recent, matching, counts))
        })
    })
    .await;

    match result {
        Ok(Ok((pending, recent, matching, (confirmed, rejected)))) => {
            let html = render_queue(
                &pending,
                &recent,
                confirmed,
                rejected,
                QueueView {
                    status: q
                        .status
                        .as_deref()
                        .and_then(birdnet_db::sqlite::ReviewStatus::parse),
                    offset,
                    matching,
                },
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p class=\"dr-error\">Error loading the review queue.</p>".to_string(),
        ),
    }
}

/// Which slice of the verdict history is on screen.
#[derive(Debug, Clone, Copy)]
struct QueueView {
    status: Option<birdnet_db::sqlite::ReviewStatus>,
    offset: u32,
    /// Verdicts matching `status` in total, so the page can say whether there
    /// is more. A list that just stops is the failure this replaces.
    matching: i64,
}

fn render_queue(
    pending: &[birdnet_db::sqlite::UnreviewedDetection],
    recent: &[birdnet_db::sqlite::DetectionReview],
    confirmed: i64,
    rejected: i64,
    view: QueueView,
) -> String {
    let mut html = String::with_capacity(8192);

    let _ = write!(
        html,
        "<div class=\"dr-counts\">\
  <span class=\"bnb-pill moss\">&#10003; {confirmed} confirmed</span>\
  <span class=\"bnb-pill rare\">&#10007; {rejected} rejected</span>\
  <span class=\"bnb-pill\">{} awaiting review</span>\
</div>",
        pending.len()
    );

    html.push_str("<div class=\"bnb-card pad dr-card-mb\">");
    html.push_str("<h2 class=\"dr-h2\">Awaiting review</h2>");
    if pending.is_empty() {
        html.push_str(
            "<p class=\"dr-empty\">Every recent detection has a verdict. Nice and tidy.</p>",
        );
    } else {
        for d in pending {
            render_pending_row(&mut html, d);
        }
    }
    html.push_str("</div>");

    html.push_str("<div class=\"bnb-card pad\">");
    let heading = match view.status {
        Some(birdnet_db::sqlite::ReviewStatus::Confirmed) => "Confirmed",
        Some(birdnet_db::sqlite::ReviewStatus::Rejected) => "Rejected",
        None => "All verdicts",
    };
    let _ = write!(
        html,
        "<h2 class=\"dr-h2\">{heading} <span class=\"bnb-meta\">({} recorded)</span></h2>",
        view.matching
    );
    html.push_str(&render_status_filter(view));
    if recent.is_empty() {
        html.push_str(if view.offset > 0 {
            "<p class=\"dr-empty\">Nothing further back than this.</p>"
        } else {
            "<p class=\"dr-empty\">No verdicts recorded yet.</p>"
        });
    } else {
        for r in recent {
            render_verdict_row(&mut html, r);
        }
    }
    html.push_str(&render_pager(view));
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
          class=\"dr-prow\">\
  <input type=\"hidden\" name=\"date\" value=\"{date}\">\
  <input type=\"hidden\" name=\"time\" value=\"{time}\">\
  <input type=\"hidden\" name=\"sci_name\" value=\"{sci}\">\
  <input type=\"hidden\" name=\"com_name\" value=\"{com}\">\
  <div class=\"dr-row-main\">\
    <a href=\"/species/detail?name={enc_com}\" class=\"dr-row-name\">{com}</a>\
    <div class=\"dr-row-sci\">{sci}</div>\
    <div class=\"dr-row-meta\">{date} · {time}</div>\
  </div>\
  <span class=\"conf {conf_cls}\">{conf_pct:.0}%</span>\
  <button type=\"submit\" name=\"status\" value=\"confirmed\" class=\"bnb-btn dr-confirm-btn\">&#10003; Confirm</button>\
  <button type=\"submit\" name=\"status\" value=\"rejected\" class=\"bnb-btn ghost dr-nowrap\">&#10007; Reject</button>\
</form>"
    );
}

/// The `All / Confirmed / Rejected` filter.
///
/// Plain links, not htmx buttons, so each view has a URL an operator can keep
/// or share. "Where is the detection I rejected in March" has to be answerable,
/// and before this it was not answerable at all past the 25th verdict.
fn render_status_filter(view: QueueView) -> String {
    let mut out = String::from(r#"<nav class="dr-filter" aria-label="Filter verdicts">"#);
    for (key, label) in [
        ("", "All"),
        ("confirmed", "Confirmed"),
        ("rejected", "Rejected"),
    ] {
        let selected = view
            .status
            .map_or("", birdnet_db::sqlite::ReviewStatus::as_str)
            == key;
        let href = if key.is_empty() {
            "/pages/detection-reviews-queue".to_string()
        } else {
            format!("/pages/detection-reviews-queue?status={key}")
        };
        let cls = if selected {
            "bnb-btn ghost dr-filter__btn is-selected"
        } else {
            "bnb-btn ghost dr-filter__btn"
        };
        let aria = if selected {
            r#" aria-current="page""#
        } else {
            ""
        };
        let _ = write!(
            out,
            r##"<a class="{cls}" href="{href}" hx-get="{href}" hx-target="#dr-queue" hx-swap="innerHTML"{aria}>{label}</a>"##
        );
    }
    out.push_str("</nav>");
    out
}

/// Older / newer links, shown only when there is somewhere to go.
///
/// The count comes from `detection_review_total`, so "Older" appears exactly
/// when more verdicts exist — a pager that offers a page which turns out to be
/// empty is the same lie as a list that silently ends.
fn render_pager(view: QueueView) -> String {
    let per_page = i64::from(RECENT_LIMIT);
    let offset = i64::from(view.offset);
    let has_older = offset + per_page < view.matching;
    let has_newer = offset > 0;
    if !has_older && !has_newer {
        return String::new();
    }
    let status_q = view
        .status
        .map(|s| format!("status={}&", s.as_str()))
        .unwrap_or_default();
    let mut out = String::from(r#"<nav class="dr-pager" aria-label="Verdict history pages">"#);
    if has_newer {
        let prev = offset.saturating_sub(per_page).max(0);
        let href = format!("/pages/detection-reviews-queue?{status_q}offset={prev}");
        let _ = write!(
            out,
            r##"<a class="bnb-btn ghost" href="{href}" hx-get="{href}" hx-target="#dr-queue" hx-swap="innerHTML">&larr; Newer</a>"##
        );
    }
    let shown_to = (offset + per_page).min(view.matching);
    let _ = write!(
        out,
        r#"<span class="bnb-meta">{}&ndash;{shown_to} of {}</span>"#,
        offset + 1,
        view.matching
    );
    if has_older {
        let next = offset + per_page;
        let href = format!("/pages/detection-reviews-queue?{status_q}offset={next}");
        let _ = write!(
            out,
            r##"<a class="bnb-btn ghost" href="{href}" hx-get="{href}" hx-target="#dr-queue" hx-swap="innerHTML">Older &rarr;</a>"##
        );
    }
    out.push_str("</nav>");
    out
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
          class=\"dr-vrow\">\
  <input type=\"hidden\" name=\"date\" value=\"{date}\">\
  <input type=\"hidden\" name=\"time\" value=\"{time}\">\
  <input type=\"hidden\" name=\"sci_name\" value=\"{sci}\">\
  <span class=\"bnb-pill {badge_cls}\">{badge}</span>\
  <div class=\"dr-vrow-main\">{com}\
    <span class=\"dr-vrow-meta\"> · {date} {time}</span></div>\
  <button type=\"submit\" class=\"bnb-btn ghost dr-undo-btn\">Undo</button>\
</form>"
    );
}

async fn detection_review_set(
    State(state): State<AppState>,
    Form(form): Form<ReviewForm>,
) -> impl IntoResponse {
    if let Some(status) = birdnet_db::sqlite::ReviewStatus::parse(&form.status) {
        // `state.set_detection_review`, not `with_db(set_detection_review)`:
        // both `detections_analytic` and `detections_ts` filter on the verdict,
        // so one that reached only SQLite would change the species totals and
        // leave every behavioural dashboard still counting the reject.
        let _ = tokio::task::spawn_blocking(move || {
            state.set_detection_review(
                &form.date,
                &form.time,
                &form.sci_name,
                &form.com_name,
                status,
                None,
            )
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
    // Paired write — see `detection_review_set`. Clearing must reach both
    // stores or the exclusion outlives the verdict that justified it.
    let _ = tokio::task::spawn_blocking(move || {
        state.clear_detection_review(&form.date, &form.time, &form.sci_name)
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
        // Paired write — see `detection_review_set`.
        let _ = tokio::task::spawn_blocking(move || {
            state.set_detection_review(&date, &time, &sci, &com, status, None)
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
        "<div id=\"dr-review-widget\" class=\"bnb-card pad dr-widget\">\
  <div class=\"section-header\"><div><div class=\"bnb-eyebrow\">Quality control</div><h3>Review this detection</h3></div>{badge}</div>\
  <form hx-post=\"/pages/detection-review-inline\" hx-target=\"#dr-review-widget\" hx-swap=\"outerHTML\" \
        class=\"dr-widget-form\">\
    <input type=\"hidden\" name=\"date\" value=\"{date_e}\">\
    <input type=\"hidden\" name=\"time\" value=\"{time_e}\">\
    <input type=\"hidden\" name=\"sci_name\" value=\"{sci_e}\">\
    <input type=\"hidden\" name=\"com_name\" value=\"{com_e}\">\
    <button type=\"submit\" name=\"status\" value=\"confirmed\" class=\"{confirm_cls} dr-nowrap\">&#10003; Confirm</button>\
    <button type=\"submit\" name=\"status\" value=\"rejected\" class=\"{reject_cls} dr-nowrap\">&#10007; Reject</button>\
  </form>\
  <p class=\"bnb-meta dr-widget-note\">Records a verdict in the <a href=\"/detection-reviews\" class=\"dr-link\">review queue</a>. Nothing is deleted.</p>\
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
