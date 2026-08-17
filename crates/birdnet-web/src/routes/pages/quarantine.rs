//! Rare-bird quarantine review page and HTMX partials.
//!
//! Detections that pass the global confidence threshold but fail a stricter
//! per-species threshold are held in the `quarantine` table for manual review
//! before being admitted into `detections` (approved) or discarded (rejected).
//!
//! # Routes
//!
//! | Method | Path                        | Description                        |
//! |--------|-----------------------------|------------------------------------|
//! | GET    | `/quarantine`               | Full quarantine review page        |
//! | GET    | `/pages/quarantine-list`    | HTMX partial: paginated row list   |
//! | GET    | `/pages/quarantine-stats`   | HTMX partial: stats badges         |
//! | POST   | `/pages/quarantine-approve` | Approve — copy to detections table |
//! | POST   | `/pages/quarantine-reject`  | Reject — mark reviewed             |
//! | POST   | `/pages/quarantine-delete`  | Delete permanently                 |

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::toast::Toast;
use super::{escape_html, simple_url_encode};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build all quarantine page routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/quarantine", get(quarantine_page))
        .route("/pages/quarantine-list", get(quarantine_list_partial))
        .route("/pages/quarantine-stats", get(quarantine_stats_partial))
        .route(
            "/pages/quarantine-approve",
            axum::routing::post(quarantine_approve),
        )
        .route(
            "/pages/quarantine-reject",
            axum::routing::post(quarantine_reject),
        )
        .route(
            "/pages/quarantine-delete",
            axum::routing::post(quarantine_delete),
        )
        .route(
            "/pages/quarantine-pending-count",
            get(quarantine_pending_count_partial),
        )
}

// ---------------------------------------------------------------------------
// Query / form types
// ---------------------------------------------------------------------------

/// Query parameters for the list partial.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Status filter: `pending` (default), `approved`, `rejected`, `all`.
    pub filter: Option<String>,
    /// Page offset.
    pub offset: Option<u32>,
    /// Items per page (default 30, max 100).
    pub limit: Option<u32>,
}

/// Form for approve / reject / delete actions.
#[derive(Debug, Deserialize)]
pub struct ActionForm {
    /// Quarantine row primary key.
    pub id: i64,
    /// Current filter (forwarded for list re-render).
    pub filter: Option<String>,
    /// Current offset (forwarded for list re-render).
    pub offset: Option<u32>,
}

// ---------------------------------------------------------------------------
// Full page
// ---------------------------------------------------------------------------

/// Render the full Quarantine Review page (server-side HTML, HTMX-enhanced).
///
/// Accepts an optional `filter` query parameter so that direct links like
/// `/quarantine?filter=all` correctly pre-select the active filter and load
/// the matching list via the initial HTMX trigger.
async fn quarantine_page(Query(params): Query<ListParams>, headers: HeaderMap) -> Html<String> {
    let filter = params.filter.as_deref().unwrap_or("pending");
    let content = build_page_html(filter);
    super::render_page_for_request("Quarantine Review", &content, "quarantine", &headers)
}

fn build_page_html(active_filter: &str) -> String {
    // Active filter tab → highlighted vs plain, as enumerable classes.
    let cls = |f: &str| {
        if active_filter == f {
            "qz-filter-link active"
        } else {
            "qz-filter-link"
        }
    };
    let s_pending = cls("pending");
    let s_approved = cls("approved");
    let s_rejected = cls("rejected");
    let s_all = cls("all");

    // O-20 help link for the quarantine review surface.
    let help_link = super::help::help_link(super::help::Topic::Reviews);

    // Initial HTMX load passes the active filter so the list matches the URL.
    format!(
        "<div class=\"qz-head\">\
  <div class=\"qz-title-row\">\
    <h1 class=\"qz-h1\">\
      &#128269; Rare Bird Quarantine\
    </h1>\
    {help_link}\
  </div>\
  <p class=\"qz-lede\">\
    Detections that passed the global confidence threshold but failed a stricter \
    per-species threshold are held here for manual review. Approve to admit into \
    the detection log; reject or delete to discard. To confirm or reject \
    detections already in the log, use \
    <a href=\"/detection-reviews\">Detection reviews</a>.\
  </p>\
</div>\
<div id=\"quarantine-stats\" \
     hx-get=\"/pages/quarantine-stats\" \
     hx-trigger=\"load\" \
     hx-swap=\"innerHTML\">\
  <p class=\"qz-loading\">Loading stats\u{2026}</p>\
</div>\
<div class=\"card qz-filter-card\">\
  <div class=\"qz-filter-row\">\
    <strong class=\"qz-filter-label\">Filter</strong>\
    <a href=\"/quarantine\" class=\"{s_pending}\">Pending</a>\
    <a href=\"/quarantine?filter=approved\" class=\"{s_approved}\">Approved</a>\
    <a href=\"/quarantine?filter=rejected\" class=\"{s_rejected}\">Rejected</a>\
    <a href=\"/quarantine?filter=all\" class=\"{s_all}\">All</a>\
  </div>\
  <div id=\"quarantine-list\" \
       hx-get=\"/pages/quarantine-list?filter={active_filter}\" \
       hx-trigger=\"load\" \
       hx-swap=\"innerHTML\">\
    <p class=\"qz-list-loading\">Loading\u{2026}</p>\
  </div>\
</div>"
    )
}

// ---------------------------------------------------------------------------
// Stats partial
// ---------------------------------------------------------------------------

async fn quarantine_stats_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result =
        tokio::task::spawn_blocking(move || state.with_db(birdnet_db::sqlite::quarantine_stats))
            .await;

    match result {
        Ok(Ok(qstats)) => {
            let mut html = String::with_capacity(512);
            let _ = write!(
                html,
                r#"<div class="stats-grid qz-stats-flush">
  <div class="stat-card">
    <div class="value qz-stat warn">{pending}</div>
    <div class="label">Pending Review</div>
  </div>
  <div class="stat-card">
    <div class="value qz-stat success">{approved}</div>
    <div class="label">Approved</div>
  </div>
  <div class="stat-card">
    <div class="value qz-stat danger">{rejected}</div>
    <div class="label">Rejected</div>
  </div>
  <div class="stat-card">
    <div class="value">{total}</div>
    <div class="label">Total</div>
  </div>
</div>"#,
                pending = qstats.pending,
                approved = qstats.approved,
                rejected = qstats.rejected,
                total = qstats.total,
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p class=\"qz-err\">Error loading stats</p>".to_string(),
        ),
    }
}

/// Tiny partial used by the nav badge to show pending count.
async fn quarantine_pending_count_partial(State(state): State<AppState>) -> impl IntoResponse {
    let count = tokio::task::spawn_blocking(move || {
        state.with_db(birdnet_db::sqlite::quarantine_pending_count)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(0);

    let html = if count > 0 {
        format!(
            r#"<span class="qz-pending-badge">
               {count}
            </span>"#
        )
    } else {
        String::new()
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// List partial
// ---------------------------------------------------------------------------

async fn quarantine_list_partial(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let filter = parse_filter(params.filter.as_deref());
    let limit = params.limit.unwrap_or(30).min(100);
    let offset = params.offset.unwrap_or(0);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let rows = birdnet_db::sqlite::list_quarantine(conn, filter, limit, offset)?;
            let total = birdnet_db::sqlite::count_quarantine(conn, filter)?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, total))
        })
    })
    .await;

    match result {
        Ok(Ok((rows, total))) => {
            let mut html = String::with_capacity(4096);

            if rows.is_empty() && offset == 0 {
                html.push_str(&super::empty_states::no_rare_yet());
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html);
            }

            render_table_header(&mut html);
            for row in &rows {
                render_quarantine_row(&mut html, row, &filter_str(filter));
            }
            html.push_str("</tbody></table>");

            // Pagination
            let shown = offset + u32::try_from(rows.len()).unwrap_or(limit);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let total_u = total as u32;
            if shown < total_u {
                let filter_param = filter_str(filter);
                let remaining = total_u.saturating_sub(shown);
                // hx-target="#quarantine-list" — use a variable so "# doesn't end an r# literal.
                let target = "#quarantine-list";
                let _ = write!(
                    html,
                    "<div class=\"qz-loadmore-row\">\
                    <button \
                    hx-get=\"/pages/quarantine-list?filter={filter_param}&offset={shown}&limit={limit}\" \
                    hx-target=\"{target}\" hx-swap=\"innerHTML\" \
                    class=\"qz-loadmore\">\
                      Load {limit} more ({remaining} remaining)\
                    </button></div>",
                );
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p class=\"qz-err\">Error loading quarantine list</p>".to_string(),
        ),
    }
}

fn render_table_header(html: &mut String) {
    html.push_str(
        "<table>\n\
         <thead>\n\
         <tr>\n\
         <th>Species</th>\n\
         <th>Confidence</th>\n\
         <th>Reason</th>\n\
         <th>Date / Time</th>\n\
         <th>Status</th>\n\
         <th>Actions</th>\n\
         </tr>\n\
         </thead>\n\
         <tbody>",
    );
}

fn render_quarantine_row(
    html: &mut String,
    row: &birdnet_db::sqlite::QuarantineRow,
    filter_param: &str,
) {
    let conf_pct = row.confidence * 100.0;
    let conf_cls = if conf_pct >= 80.0 {
        "high"
    } else if conf_pct >= 50.0 {
        "mid"
    } else {
        "low"
    };
    let com_name = escape_html(&row.com_name);
    let sci_name = escape_html(&row.sci_name);
    let date = escape_html(&row.date);
    let time = escape_html(&row.time);
    let reason_label =
        escape_html(birdnet_db::sqlite::QuarantineReason::from_db_str(&row.reason).label());
    let sf_info = row
        .sf_probability
        .map(|p| format!("<div class=\"qz-sf\">SF prob: {:.1}%</div>", p * 100.0))
        .unwrap_or_default();
    let enc_species = simple_url_encode(&row.com_name);
    let status = if row.reviewed {
        if row.approved {
            r#"<span class="qz-status approved">&#10003; Approved</span>"#
        } else {
            r#"<span class="qz-status rejected">&#10007; Rejected</span>"#
        }
    } else {
        r#"<span class="qz-status pending">&#9679; Pending</span>"#
    };
    let id = row.id;
    // O-07: a public, HMAC-signed share link for this rare-bird row. The share
    // page falls back to the quarantine table, so pending rows resolve too.
    let token = crate::routes::share::issue_token_for(&row.date, &row.time, &row.com_name);
    let base_actions = if row.reviewed {
        row_delete_button(id, filter_param)
    } else {
        row_action_buttons(id, filter_param, &com_name)
    };
    let actions = format!(
        "<div class=\"qz-actions\">{base_actions}{}</div>",
        row_share_button(&token)
    );
    let audio = row_audio_player(row.file_name.as_deref());
    let _ = write!(
        html,
        r#"<tr>
          <td>
            <div>
              <a href="/species/detail?name={enc_species}" class="qz-name">{com_name}</a>
            </div>
            <div class="qz-sci">{sci_name}</div>
            {sf_info}
            {audio}
          </td>
          <td><span class="conf {conf_cls}">{conf_pct:.0}%</span></td>
          <td class="qz-reason">{reason_label}</td>
          <td class="qz-datetime">{date}<br><span class="qz-time">{time}</span></td>
          <td class="qz-status">{status}</td>
          <td>{actions}</td>
        </tr>"#,
    );
}

/// Render the approve / reject / delete button group for a pending quarantine row.
///
/// Uses a local `target` variable for `hx-target="#quarantine-list"` to prevent
/// the `"#` sequence from terminating a raw-string literal.
fn row_action_buttons(id: i64, filter_param: &str, com_name: &str) -> String {
    let target = "#quarantine-list";
    format!(
        "<div class=\"qz-btn-group\">\
          <button hx-post=\"/pages/quarantine-approve\" \
            hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
            hx-target=\"{target}\" hx-swap=\"innerHTML\" \
            hx-confirm=\"Approve {com_name} and admit to detections?\" \
            data-confirm-action=\"hx-post\" \
            data-confirm-url=\"/pages/quarantine-approve\" \
            data-confirm-title=\"Approve detection\" \
            data-confirm-body=\"Approve {com_name} and admit to detections?\" \
            data-confirm-confirm-label=\"Approve\" \
            data-confirm-style=\"moss\" \
            class=\"qz-btn approve\">\
            &#10003; Approve\
          </button>\
          <button hx-post=\"/pages/quarantine-reject\" \
            hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
            hx-target=\"{target}\" hx-swap=\"innerHTML\" \
            class=\"qz-btn reject\">\
            Reject\
          </button>\
          <button hx-post=\"/pages/quarantine-delete\" \
            hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
            hx-target=\"{target}\" hx-swap=\"innerHTML\" \
            hx-confirm=\"Permanently delete this quarantine entry?\" \
            data-confirm-action=\"hx-post\" \
            data-confirm-url=\"/pages/quarantine-delete\" \
            data-confirm-title=\"Delete quarantine entry\" \
            data-confirm-body=\"Permanently delete this quarantine entry?\" \
            data-confirm-confirm-label=\"Delete\" \
            data-confirm-style=\"danger\" \
            class=\"qz-btn delete\">\
            Delete\
          </button>\
        </div>",
    )
}

/// Render a delete-only button for already-reviewed quarantine rows.
fn row_delete_button(id: i64, filter_param: &str) -> String {
    let target = "#quarantine-list";
    format!(
        "<button hx-post=\"/pages/quarantine-delete\" \
           hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
           hx-target=\"{target}\" hx-swap=\"innerHTML\" \
           hx-confirm=\"Permanently delete this quarantine entry?\" \
           class=\"qz-btn delete\">\
           Delete\
        </button>",
    )
}

/// Render a "Share" button that copies a public `/r/<token>` link for the row
/// (O-07). The copy runs through the global delegated `data-copy-url` handler
/// (layout.html); the base64url token is safe inside the HTML attribute.
fn row_share_button(token: &str) -> String {
    format!(
        "<button type=\"button\" title=\"Copy a public share link\" \
           class=\"qz-btn share\" \
           data-copy-url=\"/r/{token}\" data-copied-label=\"Copied\">Share</button>"
    )
}

/// Render an inline `<audio>` player for a quarantine row's source recording, if any.
fn row_audio_player(file_name: Option<&str>) -> String {
    file_name
        .filter(|f| !f.is_empty())
        .map(|f| {
            let basename = std::path::Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let safe = escape_html(&basename);
            format!(
                "<audio controls preload=\"none\" class=\"qz-audio\">\
                  <source src=\"/api/v2/recordings/{safe}\" type=\"audio/wav\">\
                  </audio>",
            )
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Action handlers — each re-renders the list after mutation
// ---------------------------------------------------------------------------

async fn quarantine_approve(
    State(state): State<AppState>,
    Form(form): Form<ActionForm>,
) -> impl IntoResponse {
    let id = form.id;
    let filter_param = form.filter.as_deref().unwrap_or("pending").to_owned();
    let offset = form.offset.unwrap_or(0);

    // `state.approve_quarantine`, not `with_db(approve_quarantine)`: the row is
    // back-dated by construction, so the incremental analytics sync would skip
    // it on every future start. Without the paired write an approved detection
    // never reached the analytics dashboards at all.
    let result = tokio::task::spawn_blocking(move || state.approve_quarantine(id)).await;

    // O-18: outcome toast.
    let toast = match &result {
        Ok(Ok(newly_inserted)) => {
            tracing::info!(id, newly_inserted, "quarantine entry approved");
            Some(Toast::success("Approved."))
        }
        Ok(Err(e)) => {
            tracing::warn!(id, error = %e, "failed to approve quarantine entry");
            Some(Toast::error(format!("Approve failed: {e}")))
        }
        Err(e) => {
            tracing::warn!(id, error = %e, "task panic approving quarantine entry");
            Some(Toast::error("Approve failed."))
        }
    };

    reload_list_response(&filter_param, offset, toast)
}

async fn quarantine_reject(
    State(state): State<AppState>,
    Form(form): Form<ActionForm>,
) -> impl IntoResponse {
    let id = form.id;
    let filter_param = form.filter.as_deref().unwrap_or("pending").to_owned();
    let offset = form.offset.unwrap_or(0);

    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::reject_quarantine(conn, id))
    })
    .await;

    // O-18: outcome toast (reject is best-effort; surface the action either way).
    reload_list_response(&filter_param, offset, Some(Toast::success("Rejected.")))
}

async fn quarantine_delete(
    State(state): State<AppState>,
    Form(form): Form<ActionForm>,
) -> impl IntoResponse {
    let id = form.id;
    let filter_param = form.filter.as_deref().unwrap_or("pending").to_owned();
    let offset = form.offset.unwrap_or(0);

    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::delete_quarantine(conn, id))
    })
    .await;

    // O-18: outcome toast.
    reload_list_response(
        &filter_param,
        offset,
        Some(Toast::success("Quarantine entry deleted.")),
    )
}

/// Return an HTMX-trigger div that reloads the quarantine list.
///
/// The `+ use<>` bound on the return type tells Rust 2024 not to capture the
/// `filter_param` lifetime, allowing callers to pass short-lived borrows from
/// local variables without causing `E0515` lifetime errors.
fn reload_list_response(
    filter_param: &str,
    offset: u32,
    toast: Option<Toast>,
) -> impl IntoResponse + use<> {
    // hx-target uses a CSS ID selector (#quarantine-list).  A local variable
    // prevents the "# sequence from terminating an r#"..."# raw-string literal.
    let target = "#quarantine-list";
    let mut html = format!(
        "<div hx-get=\"/pages/quarantine-list?filter={filter_param}&offset={offset}\" \
         hx-trigger=\"load\" \
         hx-target=\"{target}\" \
         hx-swap=\"innerHTML\"></div>"
    );
    // O-18: append the OOB toast fragment so htmx swaps it into #bnb-toasts
    // alongside the list-reload trigger.
    if let Some(t) = toast {
        html.push_str(&t.render_oob());
    }
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_filter(s: Option<&str>) -> birdnet_db::sqlite::QuarantineFilter {
    match s {
        Some("approved") => birdnet_db::sqlite::QuarantineFilter::Approved,
        Some("rejected") => birdnet_db::sqlite::QuarantineFilter::Rejected,
        Some("all") => birdnet_db::sqlite::QuarantineFilter::All,
        _ => birdnet_db::sqlite::QuarantineFilter::Pending,
    }
}

fn filter_str(filter: birdnet_db::sqlite::QuarantineFilter) -> String {
    match filter {
        birdnet_db::sqlite::QuarantineFilter::Approved => "approved",
        birdnet_db::sqlite::QuarantineFilter::Rejected => "rejected",
        birdnet_db::sqlite::QuarantineFilter::All => "all",
        birdnet_db::sqlite::QuarantineFilter::Pending => "pending",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_filter_defaults_to_pending() {
        assert_eq!(
            parse_filter(None),
            birdnet_db::sqlite::QuarantineFilter::Pending
        );
        assert_eq!(
            parse_filter(Some("garbage")),
            birdnet_db::sqlite::QuarantineFilter::Pending
        );
    }

    #[test]
    fn parse_filter_all_variants() {
        assert_eq!(
            parse_filter(Some("approved")),
            birdnet_db::sqlite::QuarantineFilter::Approved
        );
        assert_eq!(
            parse_filter(Some("rejected")),
            birdnet_db::sqlite::QuarantineFilter::Rejected
        );
        assert_eq!(
            parse_filter(Some("all")),
            birdnet_db::sqlite::QuarantineFilter::All
        );
    }

    #[test]
    fn filter_str_round_trips() {
        for (f, s) in [
            (birdnet_db::sqlite::QuarantineFilter::Pending, "pending"),
            (birdnet_db::sqlite::QuarantineFilter::Approved, "approved"),
            (birdnet_db::sqlite::QuarantineFilter::Rejected, "rejected"),
            (birdnet_db::sqlite::QuarantineFilter::All, "all"),
        ] {
            assert_eq!(filter_str(f), s);
        }
    }

    #[test]
    fn build_page_html_contains_key_elements() {
        let html = build_page_html("pending");
        assert!(html.contains("quarantine-stats"));
        assert!(html.contains("quarantine-list"));
        assert!(html.contains("Pending Review") || html.contains("Filter"));
    }
}
