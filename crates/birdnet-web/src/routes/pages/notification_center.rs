//! Notification center page: recent notification history and channel status.
//!
//! | Path                             | Purpose                              |
//! |----------------------------------|--------------------------------------|
//! | `GET /notifications`             | Full notification center page        |
//! | `GET /pages/notif-history`       | Notification log table (HTMX)        |
//! | `GET /pages/notif-stats`         | Channel stats partial (HTMX)         |

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, render_page};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/notifications", get(notification_page))
        .route("/pages/notif-history", get(notif_history_partial))
        .route("/pages/notif-stats", get(notif_stats_partial))
}

async fn notification_page() -> Html<String> {
    render_page("Notifications", NOTIFICATION_HTML, "notifications")
}

#[derive(Deserialize)]
struct NotifQuery {
    channel: Option<String>,
    limit: Option<u32>,
}

/// HTMX partial: notification log table.
async fn notif_history_partial(
    State(state): State<AppState>,
    Query(params): Query<NotifQuery>,
) -> impl axum::response::IntoResponse {
    let channel = params.channel.clone();
    let limit = params.limit.unwrap_or(50).min(200);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            channel.as_deref().map_or_else(
                || birdnet_db::notifications::recent_notifications(conn, limit, 0),
                |ch| birdnet_db::notifications::notifications_by_channel(conn, ch, limit),
            )
        })
    })
    .await;

    match result {
        Ok(Ok(entries)) => {
            if entries.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p style="color:var(--text-muted);text-align:center;padding:2rem;">No notifications sent yet.</p>"#.to_string(),
                );
            }

            let mut html = String::with_capacity(entries.len() * 200);
            html.push_str(
                "<table><thead><tr>\
                 <th>Time</th>\
                 <th>Channel</th>\
                 <th>Species</th>\
                 <th>Status</th>\
                 <th>Message</th>\
                 </tr></thead><tbody>",
            );

            for e in &entries {
                let status_cls = match e.status.as_str() {
                    "sent" => "high",
                    "failed" => "low",
                    _ => "mid",
                };
                let species = e.species_com_name.as_deref().unwrap_or("\u{2014}");
                let msg = e.message.as_deref().unwrap_or("\u{2014}");
                let _ = write!(
                    html,
                    "<tr>\
                     <td style=\"font-size:0.85rem;color:var(--text-muted);white-space:nowrap;\">{time}</td>\
                     <td><span style=\"background:var(--bg-hover);padding:0.1rem 0.5rem;border-radius:4px;font-size:0.8rem;\">{channel}</span></td>\
                     <td>{species}</td>\
                     <td><span class=\"conf {status_cls}\">{status}</span></td>\
                     <td style=\"font-size:0.85rem;max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;\">{msg}</td>\
                     </tr>",
                    time = escape_html(&e.sent_at),
                    channel = escape_html(&e.channel),
                    species = escape_html(species),
                    status = escape_html(&e.status),
                    msg = escape_html(msg),
                );
            }

            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(_)) | Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading notification history</p>".to_string(),
        ),
    }
}

/// HTMX partial: notification channel summary stats.
async fn notif_stats_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::notifications::notification_stats(conn, 30))
    })
    .await;

    match result {
        Ok(Ok((sent, failed, skipped))) => {
            let total = sent + failed + skipped;
            let html = format!(
                "<div class=\"stat-card\">\
                  <div class=\"value\">{total}</div>\
                  <div class=\"label\">Total (30 days)</div>\
                </div>\
                <div class=\"stat-card\">\
                  <div class=\"value\" style=\"color:var(--success);\">{sent}</div>\
                  <div class=\"label\">Sent</div>\
                </div>\
                <div class=\"stat-card\">\
                  <div class=\"value\" style=\"color:var(--danger);\">{failed}</div>\
                  <div class=\"label\">Failed</div>\
                </div>\
                <div class=\"stat-card\">\
                  <div class=\"value\" style=\"color:var(--warning);\">{skipped}</div>\
                  <div class=\"label\">Skipped</div>\
                </div>",
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(_)) | Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading stats</p>".to_string(),
        ),
    }
}

const NOTIFICATION_HTML: &str = r#"<h1 style="margin-bottom:1.5rem;">Notification Center</h1>

<div class="stats-grid" hx-get="/pages/notif-stats" hx-trigger="load, every 60s" hx-swap="innerHTML">
    <div class="stat-card"><div class="value">--</div><div class="label">Loading...</div></div>
</div>

<div class="card">
    <h2>Notification History</h2>
    <p style="color:var(--text-muted);font-size:0.85rem;margin-bottom:1rem;">
        Recent notifications across all channels (last 30 days).
    </p>
    <div hx-get="/pages/notif-history" hx-trigger="load, every 30s" hx-swap="innerHTML">
        <p style="color:var(--text-muted);">Loading history...</p>
    </div>
</div>"#;
