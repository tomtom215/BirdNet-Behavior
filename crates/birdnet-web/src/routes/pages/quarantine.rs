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
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

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
async fn quarantine_page(Query(params): Query<ListParams>) -> Html<String> {
    let filter = params.filter.as_deref().unwrap_or("pending");
    let content = build_page_html(filter);
    super::render_page("Quarantine Review", &content, "quarantine")
}

fn build_page_html(active_filter: &str) -> String {
    // Active filter tab style — highlighted vs plain.
    let active_style = "font-size:0.9rem;font-weight:700;color:var(--primary);\
                        border-bottom:2px solid var(--primary);padding-bottom:0.1rem;";
    let plain_style = "font-size:0.9rem;color:var(--text-muted);";

    let s_pending = if active_filter == "pending" {
        active_style
    } else {
        plain_style
    };
    let s_approved = if active_filter == "approved" {
        active_style
    } else {
        plain_style
    };
    let s_rejected = if active_filter == "rejected" {
        active_style
    } else {
        plain_style
    };
    let s_all = if active_filter == "all" {
        active_style
    } else {
        plain_style
    };

    // Initial HTMX load passes the active filter so the list matches the URL.
    format!(
        "<div style=\"margin-bottom:1.5rem;\">\
  <h1 style=\"font-size:1.5rem;font-weight:700;margin-bottom:0.25rem;\">\
    &#128269; Rare Bird Quarantine\
  </h1>\
  <p style=\"color:var(--text-muted);font-size:0.9rem;\">\
    Detections that passed the global confidence threshold but failed a stricter \
    per-species threshold are held here for manual review. Approve to admit into \
    the detection log; reject or delete to discard.\
  </p>\
</div>\
<div id=\"quarantine-stats\" \
     hx-get=\"/pages/quarantine-stats\" \
     hx-trigger=\"load\" \
     hx-swap=\"innerHTML\">\
  <p style=\"color:var(--text-muted);\">Loading stats\u{2026}</p>\
</div>\
<div class=\"card\" style=\"margin-top:1rem;\">\
  <div style=\"display:flex;align-items:center;gap:0.75rem;flex-wrap:wrap;\
               margin-bottom:1rem;border-bottom:1px solid var(--border);padding-bottom:0.75rem;\">\
    <strong style=\"color:var(--text-muted);font-size:0.85rem;text-transform:uppercase;\
                    letter-spacing:0.05em;\">Filter</strong>\
    <a href=\"/quarantine\" style=\"{s_pending}\">Pending</a>\
    <a href=\"/quarantine?filter=approved\" style=\"{s_approved}\">Approved</a>\
    <a href=\"/quarantine?filter=rejected\" style=\"{s_rejected}\">Rejected</a>\
    <a href=\"/quarantine?filter=all\" style=\"{s_all}\">All</a>\
  </div>\
  <div id=\"quarantine-list\" \
       hx-get=\"/pages/quarantine-list?filter={active_filter}\" \
       hx-trigger=\"load\" \
       hx-swap=\"innerHTML\">\
    <p style=\"color:var(--text-muted);text-align:center;padding:2rem;\">Loading\u{2026}</p>\
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
                r#"<div class="stats-grid" style="margin-bottom:0;">
  <div class="stat-card">
    <div class="value" style="color:var(--warning);">{pending}</div>
    <div class="label">Pending Review</div>
  </div>
  <div class="stat-card">
    <div class="value" style="color:var(--success);">{approved}</div>
    <div class="label">Approved</div>
  </div>
  <div class="stat-card">
    <div class="value" style="color:var(--danger);">{rejected}</div>
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
            "<p style=\"color:var(--danger);\">Error loading stats</p>".to_string(),
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
            r#"<span style="background:var(--warning);color:#000;border-radius:9999px;
               padding:0.1rem 0.45rem;font-size:0.7rem;font-weight:700;margin-left:0.25rem;">
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
                html.push_str(
                    r#"<p style="color:var(--text-muted);text-align:center;padding:2rem;">
                    No entries found.</p>"#,
                );
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
                    "<div style=\"text-align:center;padding:1rem;\">\
                    <button \
                    hx-get=\"/pages/quarantine-list?filter={filter_param}&offset={shown}&limit={limit}\" \
                    hx-target=\"{target}\" hx-swap=\"innerHTML\" \
                    style=\"background:var(--bg-hover);border:1px solid var(--border);\
                           color:var(--text);padding:0.5rem 1.5rem;\
                           border-radius:var(--radius);cursor:pointer;font-size:0.9rem;\">\
                      Load {limit} more ({remaining} remaining)\
                    </button></div>",
                );
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p style=\"color:var(--danger);\">Error loading quarantine list</p>".to_string(),
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
        .map(|p| {
            format!(
                "<div style=\"color:var(--text-muted);font-size:0.75rem;\">SF prob: {:.1}%</div>",
                p * 100.0
            )
        })
        .unwrap_or_default();
    let enc_species = simple_url_encode(&row.com_name);
    let status = if row.reviewed {
        if row.approved {
            r#"<span style="color:var(--success);">&#10003; Approved</span>"#
        } else {
            r#"<span style="color:var(--danger);">&#10007; Rejected</span>"#
        }
    } else {
        r#"<span style="color:var(--warning);">&#9679; Pending</span>"#
    };
    let id = row.id;
    let actions = if row.reviewed {
        row_delete_button(id, filter_param)
    } else {
        row_action_buttons(id, filter_param, &com_name)
    };
    let audio = row_audio_player(row.file_name.as_deref());
    let _ = write!(
        html,
        r#"<tr>
          <td>
            <div>
              <a href="/species/detail?name={enc_species}"
                 style="font-weight:600;color:var(--text);">{com_name}</a>
            </div>
            <div style="color:var(--text-muted);font-size:0.8rem;font-style:italic;">{sci_name}</div>
            {sf_info}
            {audio}
          </td>
          <td><span class="conf {conf_cls}">{conf_pct:.0}%</span></td>
          <td style="color:var(--text-muted);font-size:0.85rem;">{reason_label}</td>
          <td style="font-size:0.85rem;">{date}<br><span style="color:var(--text-muted);">{time}</span></td>
          <td style="font-size:0.85rem;">{status}</td>
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
        "<div style=\"display:flex;gap:0.4rem;flex-wrap:wrap;\">\
          <button hx-post=\"/pages/quarantine-approve\" \
            hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
            hx-target=\"{target}\" hx-swap=\"innerHTML\" \
            hx-confirm=\"Approve {com_name} and admit to detections?\" \
            style=\"background:var(--success);color:#fff;border:none;\
                   padding:0.25rem 0.6rem;border-radius:var(--radius);\
                   cursor:pointer;font-size:0.8rem;white-space:nowrap;\">\
            &#10003; Approve\
          </button>\
          <button hx-post=\"/pages/quarantine-reject\" \
            hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
            hx-target=\"{target}\" hx-swap=\"innerHTML\" \
            style=\"background:none;border:1px solid var(--warning);\
                   color:var(--warning);padding:0.25rem 0.6rem;\
                   border-radius:var(--radius);cursor:pointer;\
                   font-size:0.8rem;white-space:nowrap;\">\
            Reject\
          </button>\
          <button hx-post=\"/pages/quarantine-delete\" \
            hx-vals='{{\"id\":{id},\"filter\":\"{filter_param}\"}}' \
            hx-target=\"{target}\" hx-swap=\"innerHTML\" \
            hx-confirm=\"Permanently delete this quarantine entry?\" \
            style=\"background:none;border:1px solid var(--danger);\
                   color:var(--danger);padding:0.25rem 0.6rem;\
                   border-radius:var(--radius);cursor:pointer;\
                   font-size:0.8rem;white-space:nowrap;\">\
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
           style=\"background:none;border:1px solid var(--danger);\
                  color:var(--danger);padding:0.25rem 0.6rem;\
                  border-radius:var(--radius);cursor:pointer;\
                  font-size:0.8rem;\">\
           Delete\
        </button>",
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
                "<audio controls preload=\"none\" \
                    style=\"width:100%;height:28px;margin-top:0.4rem;\">\
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

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::approve_quarantine(conn, id))
    })
    .await;

    match result {
        Ok(Ok(newly_inserted)) => {
            tracing::info!(id, newly_inserted, "quarantine entry approved");
        }
        Ok(Err(e)) => tracing::warn!(id, error = %e, "failed to approve quarantine entry"),
        Err(e) => tracing::warn!(id, error = %e, "task panic approving quarantine entry"),
    }

    reload_list_response(&filter_param, offset)
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

    reload_list_response(&filter_param, offset)
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

    reload_list_response(&filter_param, offset)
}

/// Return an HTMX-trigger div that reloads the quarantine list.
///
/// The `+ use<>` bound on the return type tells Rust 2024 not to capture the
/// `filter_param` lifetime, allowing callers to pass short-lived borrows from
/// local variables without causing `E0515` lifetime errors.
fn reload_list_response(filter_param: &str, offset: u32) -> impl IntoResponse + use<> {
    // hx-target uses a CSS ID selector (#quarantine-list).  A local variable
    // prevents the "# sequence from terminating an r#"..."# raw-string literal.
    let target = "#quarantine-list";
    let html = format!(
        "<div hx-get=\"/pages/quarantine-list?filter={filter_param}&offset={offset}\" \
         hx-trigger=\"load\" \
         hx-target=\"{target}\" \
         hx-swap=\"innerHTML\"></div>"
    );
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
