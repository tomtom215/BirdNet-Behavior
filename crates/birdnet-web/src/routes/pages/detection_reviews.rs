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
//! | POST   | `/pages/detection-review-dismiss` | Dismiss a verdict attached to no detection |
//!
//! A verdict is about one detection, named by its date, time, species **and
//! clip**: two microphones that hear one bird in one second have two. Every form
//! here carries the clip; one that does not (an older page left open) is
//! accepted only when its date, time and species name a single detection.

use std::fmt::Write as _;

use crate::routes::form::HtmxForm;
use axum::extract::{Query, State};
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
        .route(
            "/pages/detection-review-dismiss",
            axum::routing::post(detection_review_dismiss),
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
    /// The detection's clip, `""` for one with no clip; absent from a form that
    /// predates it, which is then accepted only if the date, time and species
    /// name one detection.
    #[serde(default)]
    pub file_name: Option<String>,
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
    /// The detection's clip; see [`ReviewForm::file_name`].
    #[serde(default)]
    pub file_name: Option<String>,
}

/// Form for dismissing a verdict that is attached to no detection.
#[derive(Debug, Deserialize)]
pub struct DismissForm {
    /// The review row's id.
    pub id: i64,
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
    super::render_page_for_request("Detection reviews", content, "species", &headers)
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
    let names = state.names_handle();

    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let pending = birdnet_db::sqlite::unreviewed_recent_detections(conn, QUEUE_LIMIT)?;
            let recent =
                birdnet_db::sqlite::detection_reviews_page(conn, status, RECENT_LIMIT, offset)?;
            let matching = birdnet_db::sqlite::detection_review_total(conn, status)?;
            let counts = birdnet_db::sqlite::detection_review_counts(conn)?;
            let detached = birdnet_db::sqlite::detached_review_count(conn)?;
            let floor = super::confidence::station_floor(conn);
            Ok::<_, birdnet_db::sqlite::DbError>((
                pending, recent, matching, counts, detached, floor,
            ))
        })
    })
    .await;

    match result {
        Ok(Ok((pending, recent, matching, (confirmed, rejected), detached, floor))) => {
            let html = render_queue(
                &pending,
                &recent,
                confirmed,
                rejected,
                detached,
                QueueView {
                    status: q
                        .status
                        .as_deref()
                        .and_then(birdnet_db::sqlite::ReviewStatus::parse),
                    offset,
                    matching,
                },
                &names,
                floor,
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        // See `error_states::failed_partial` for why this is a 200.
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "review queue: query failed");
            super::error_states::failed_partial("the review queue")
        }
        Err(e) => {
            tracing::warn!(error = %e, "review queue: task failed");
            super::error_states::failed_partial("the review queue")
        }
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
    detached: i64,
    view: QueueView,
    names: &birdnet_core::i18n::NameLocalizer,
    floor: f64,
) -> String {
    let mut html = String::with_capacity(8192);

    let _ = write!(
        html,
        "<div class=\"dr-counts\">\
  <span class=\"bnb-pill moss\">&#10003; {confirmed} confirmed</span>\
  <span class=\"bnb-pill rare\">&#10007; {rejected} rejected</span>\
  <span class=\"bnb-pill\">{} awaiting review</span>{not_attached}\
</div>",
        pending.len(),
        not_attached = if detached > 0 {
            format!("<span class=\"bnb-pill\">{detached} not attached to a detection</span>")
        } else {
            String::new()
        },
    );

    html.push_str("<div class=\"bnb-card pad dr-card-mb\">");
    html.push_str("<h2 class=\"dr-h2\">Awaiting review</h2>");
    if pending.is_empty() {
        html.push_str(
            "<p class=\"dr-empty\">Every recent detection has a verdict. Nice and tidy.</p>",
        );
    } else {
        html.push_str(&super::confidence::legend(floor));
        for d in pending {
            render_pending_row(&mut html, d, names, floor);
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
            render_verdict_row(&mut html, r, names);
        }
    }
    html.push_str(&render_pager(view));
    html.push_str("</div>");

    html
}

fn render_pending_row(
    html: &mut String,
    d: &birdnet_db::sqlite::UnreviewedDetection,
    names: &birdnet_core::i18n::NameLocalizer,
    floor: f64,
) {
    // `com` is the identity the form posts back; `shown` is what is read.
    let com = escape_html(&d.com_name);
    let shown = escape_html(names.name(&d.sci_name, &d.com_name));
    let sci = escape_html(&d.sci_name);
    let date = escape_html(&d.date);
    let time = escape_html(&d.time);
    let time_clk = crate::display::clock(&d.time);
    let enc_com = simple_url_encode(&d.com_name);
    // `""` for a detection with no clip: said, not absent, so the form names it.
    let clip = d.file_name.as_deref().unwrap_or_default();
    let file = escape_html(clip);
    // The page for this detection, not for whichever of two same-second
    // detections comes first: the reviewer should be able to listen to the
    // one they are about to judge.
    let open = format!(
        "/detections/detail?date={}&time={}&name={enc_com}&file={}",
        simple_url_encode(&d.date),
        simple_url_encode(&d.time),
        simple_url_encode(clip)
    );
    let conf = super::atoms::conf_pill(d.confidence * 100.0, floor);
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
  <input type=\"hidden\" name=\"file_name\" value=\"{file}\">\
  <input type=\"hidden\" name=\"com_name\" value=\"{com}\">\
  <div class=\"dr-row-main\">\
    <a href=\"/species/detail?name={enc_com}\" class=\"dr-row-name\">{shown}</a>\
    <div class=\"dr-row-sci\">{sci}</div>\
    <div class=\"dr-row-meta\"><a href=\"{open_href}\" class=\"dr-link\">{date} · {time_clk}</a></div>\
  </div>\
  {conf}\
  <button type=\"submit\" name=\"status\" value=\"confirmed\" class=\"bnb-btn dr-confirm-btn\">&#10003; Confirm</button>\
  <button type=\"submit\" name=\"status\" value=\"rejected\" class=\"bnb-btn ghost dr-nowrap\">&#10007; Reject</button>\
</form>",
        open_href = escape_html(&open),
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

fn render_verdict_row(
    html: &mut String,
    r: &birdnet_db::sqlite::DetectionReview,
    names: &birdnet_core::i18n::NameLocalizer,
) {
    let com = escape_html(names.name(&r.sci_name, &r.com_name));
    let date = escape_html(&r.date);
    let time = escape_html(&r.time);
    let time_clk = crate::display::clock(&r.time);
    let sci = escape_html(&r.sci_name);
    let (badge, badge_cls) = if r.status == "confirmed" {
        ("&#10003; Confirmed", "moss")
    } else {
        ("&#10007; Rejected", "rare")
    };
    let Some(why) = r.detached else {
        // Attached: it is this detection's verdict, and undoing it is naming
        // the same detection, clip included.
        let file = escape_html(&r.file_name);
        let _ = write!(
            html,
            "<form hx-post=\"/pages/detection-review-clear\" hx-target=\"#dr-queue\" hx-swap=\"innerHTML\" \
          class=\"dr-vrow\">\
  <input type=\"hidden\" name=\"date\" value=\"{date}\">\
  <input type=\"hidden\" name=\"time\" value=\"{time}\">\
  <input type=\"hidden\" name=\"sci_name\" value=\"{sci}\">\
  <input type=\"hidden\" name=\"file_name\" value=\"{file}\">\
  <span class=\"bnb-pill {badge_cls}\">{badge}</span>\
  <div class=\"dr-vrow-main\">{com}\
    <span class=\"dr-vrow-meta\"> · {date} {time_clk}</span></div>\
  <button type=\"submit\" class=\"bnb-btn ghost dr-undo-btn\">Undo</button>\
</form>"
        );
        return;
    };
    // Detached: kept because it records what a person decided, attached to no
    // detection and in no count. It can be read and dismissed, not undone.
    let note = match why {
        birdnet_db::sqlite::Detached::Ambiguous => {
            "Recorded when more than one detection shared this date, time and species, \
             so it cannot be said which was meant. It is attached to none of them, and \
             each is back in the queue above."
        }
        birdnet_db::sqlite::Detached::Orphaned => "Its detection no longer exists.",
        birdnet_db::sqlite::Detached::Relabelled => {
            "About the species this detection had before it was relabelled."
        }
    };
    let id = r.id;
    let _ = write!(
        html,
        "<form hx-post=\"/pages/detection-review-dismiss\" hx-target=\"#dr-queue\" hx-swap=\"innerHTML\" \
          class=\"dr-vrow\">\
  <input type=\"hidden\" name=\"id\" value=\"{id}\">\
  <span class=\"bnb-pill {badge_cls}\">{badge}</span>\
  <div class=\"dr-vrow-main\">{com}\
    <span class=\"dr-vrow-meta\"> · {date} {time_clk}</span>\
    <div class=\"bnb-meta\"><span class=\"bnb-pill\">Not attached</span> {note}</div></div>\
  <button type=\"submit\" class=\"bnb-btn ghost dr-undo-btn\">Dismiss</button>\
</form>"
    );
}

/// The detection a form names.
fn row_of(
    date: &str,
    time: &str,
    sci_name: &str,
    file_name: Option<&String>,
) -> crate::state::RowRef {
    crate::state::RowRef {
        date: date.to_owned(),
        time: time.to_owned(),
        sci_name: sci_name.to_owned(),
        file_name: file_name.cloned(),
    }
}

/// The clip for the audit row: what the form said, `-` when it did not.
fn clip_for_audit(row: &crate::state::RowRef) -> &str {
    row.file_name.as_deref().unwrap_or("-")
}

async fn detection_review_set(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(form): HtmxForm<ReviewForm>,
) -> axum::response::Response {
    if let Some(status) = birdnet_db::sqlite::ReviewStatus::parse(&form.status) {
        // `state.set_detection_review`, not `with_db(set_detection_review)`:
        // both `detections_analytic` and `detections_ts` filter on the verdict,
        // so one that reached only SQLite would change the species totals and
        // leave every behavioural dashboard still counting the reject.
        let row = row_of(
            &form.date,
            &form.time,
            &form.sci_name,
            form.file_name.as_ref(),
        );
        let com_name = form.com_name;
        let result = tokio::task::spawn_blocking(move || {
            let result = state.set_detection_review(&row, &com_name, status, None);
            // A rejection withdraws a detection from every count and every
            // share link; a verdict nobody can attribute is a detection that
            // vanished with no one having done it.
            if matches!(result, Ok(crate::state::ReviewOutcome::Done)) {
                crate::audit::audit(
                    &state,
                    Some(&request_user),
                    "detection.review",
                    Some(&crate::audit::detection_target(
                        &row.date,
                        &row.time,
                        &row.sci_name,
                    )),
                    Some(&format!(
                        "via=review verdict={} clip={}",
                        status.as_str(),
                        clip_for_audit(&row)
                    )),
                );
            }
            result
        })
        .await;
        if let Some(refused) = refused_review(result, "save that verdict") {
            return refused;
        }
    } else {
        tracing::warn!(status = %form.status, "ignoring detection review with unknown status");
    }
    reload_queue().into_response()
}

/// The page left as it was and a toast saying why, when a review write did not
/// happen: the database refused it, or the detection it names is not one.
///
/// These handlers discarded the result and answered as if it had worked, so a
/// verdict the database refused showed as recorded — and a rejection is what
/// withdraws a detection from every count and every share link. A 4xx would not
/// do either: htmx never swaps one, so the page would change nothing and say
/// nothing. The answer is a 200 carrying the toast.
fn refused_review<E: std::fmt::Display>(
    result: Result<Result<crate::state::ReviewOutcome, E>, tokio::task::JoinError>,
    what: &str,
) -> Option<axum::response::Response> {
    use crate::state::ReviewOutcome;
    let detail = match result {
        Ok(Ok(ReviewOutcome::Done)) => return None,
        Ok(Ok(ReviewOutcome::NoSuchDetection)) => {
            return Some(refusal(
                "That detection is no longer in the log, so there is nothing to review. \
                 Nothing was changed.",
            ));
        }
        Ok(Ok(ReviewOutcome::Ambiguous { detections })) => {
            return Some(refusal(&format!(
                "{detections} sources heard this species at that second, and this page \
                 does not say which of them you mean. Open the one you want from the list \
                 and review it there. Nothing was changed."
            )));
        }
        Ok(Err(e)) => e.to_string(),
        Err(e) => e.to_string(),
    };
    Some(write_failed(&detail, what))
}

/// A refusal the reader should see, as a 200 with a toast.
fn refusal(message: &str) -> axum::response::Response {
    super::toast::not_applied(&super::toast::Toast::error(message))
}

/// The toast for a write that failed, with the detail kept in the log.
fn write_failed(detail: &str, what: &str) -> axum::response::Response {
    tracing::warn!(error = %detail, "detection review write failed: {what}");
    refusal(&format!(
        "The station could not {what}. Nothing was changed."
    ))
}

async fn detection_review_clear(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(form): HtmxForm<ClearForm>,
) -> axum::response::Response {
    // Paired write — see `detection_review_set`. Clearing must reach both
    // stores or the exclusion outlives the verdict that justified it.
    let row = row_of(
        &form.date,
        &form.time,
        &form.sci_name,
        form.file_name.as_ref(),
    );
    let result = tokio::task::spawn_blocking(move || {
        let result = state.clear_detection_review(&row);
        if matches!(result, Ok(crate::state::ReviewOutcome::Done)) {
            crate::audit::audit(
                &state,
                Some(&request_user),
                "detection.review",
                Some(&crate::audit::detection_target(
                    &row.date,
                    &row.time,
                    &row.sci_name,
                )),
                Some(&format!(
                    "via=review verdict=cleared clip={}",
                    clip_for_audit(&row)
                )),
            );
        }
        result
    })
    .await;
    if let Some(refused) = refused_review(result, "undo that verdict") {
        return refused;
    }
    reload_queue().into_response()
}

/// Dismiss a verdict that is attached to no detection: read, and let go.
async fn detection_review_dismiss(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(form): HtmxForm<DismissForm>,
) -> axum::response::Response {
    let result = tokio::task::spawn_blocking(move || {
        let result =
            state.with_db(|conn| birdnet_db::sqlite::dismiss_detached_review(conn, form.id));
        if matches!(result, Ok(true)) {
            crate::audit::audit(
                &state,
                Some(&request_user),
                "detection.review",
                Some(&format!("review {}", form.id)),
                Some("via=review verdict=dismissed"),
            );
        }
        result
    })
    .await;
    match result {
        Ok(Ok(_)) => reload_queue().into_response(),
        Ok(Err(e)) => write_failed(&e.to_string(), "dismiss that verdict"),
        Err(e) => write_failed(&e.to_string(), "dismiss that verdict"),
    }
}

/// Inline verdict handler for the detection-detail page. Writes the verdict
/// and returns the self-replacing review widget reflecting the new state.
async fn detection_review_inline(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(form): HtmxForm<ReviewForm>,
) -> axum::response::Response {
    let current = if let Some(status) = birdnet_db::sqlite::ReviewStatus::parse(&form.status) {
        let row = row_of(
            &form.date,
            &form.time,
            &form.sci_name,
            form.file_name.as_ref(),
        );
        let com = form.com_name.clone();
        // Paired write — see `detection_review_set`.
        let result = tokio::task::spawn_blocking(move || {
            let result = state.set_detection_review(&row, &com, status, None);
            if matches!(result, Ok(crate::state::ReviewOutcome::Done)) {
                crate::audit::audit(
                    &state,
                    Some(&request_user),
                    "detection.review",
                    Some(&crate::audit::detection_target(
                        &row.date,
                        &row.time,
                        &row.sci_name,
                    )),
                    Some(&format!(
                        "via=detection verdict={} clip={}",
                        status.as_str(),
                        clip_for_audit(&row)
                    )),
                );
            }
            result
        })
        .await;
        if let Some(refused) = refused_review(result, "save that verdict") {
            return refused;
        }
        Some(status.as_str())
    } else {
        None
    };
    let widget = render_review_widget(
        &form.date,
        &form.time,
        &form.sci_name,
        form.file_name.as_deref().unwrap_or_default(),
        &form.com_name,
        current,
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        widget,
    )
        .into_response()
}

/// The self-replacing "Review this detection" widget shown on the
/// detection-detail page. `current` is the existing verdict, if any.
#[must_use]
pub(crate) fn render_review_widget(
    date: &str,
    time: &str,
    sci_name: &str,
    file_name: &str,
    com_name: &str,
    current: Option<&str>,
) -> String {
    let date_e = escape_html(date);
    let time_e = escape_html(time);
    let sci_e = escape_html(sci_name);
    // The clip of the detection on this page, `""` for one with none: said, so
    // the verdict goes to this detection and not to its neighbour.
    let file_e = escape_html(file_name);
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
  <div class=\"section-header\"><div><div class=\"bnb-eyebrow\">Quality control</div><h2 class=\"sh-h\">Review this detection</h2></div>{badge}</div>\
  <form hx-post=\"/pages/detection-review-inline\" hx-target=\"#dr-review-widget\" hx-swap=\"outerHTML\" \
        class=\"dr-widget-form\">\
    <input type=\"hidden\" name=\"date\" value=\"{date_e}\">\
    <input type=\"hidden\" name=\"time\" value=\"{time_e}\">\
    <input type=\"hidden\" name=\"sci_name\" value=\"{sci_e}\">\
    <input type=\"hidden\" name=\"file_name\" value=\"{file_e}\">\
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
