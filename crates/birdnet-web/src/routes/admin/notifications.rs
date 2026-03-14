//! Admin notification history routes.
//!
//! `GET /admin/notifications` — full HTML page showing the notification log.
//! `GET /admin/notifications/partial` — HTMX partial (table rows only) for polling.
//! `DELETE /admin/notifications/prune` — prune entries older than 90 days.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};

use birdnet_db::notifications::{NotifEntry, notification_stats, recent_notifications};

use crate::state::AppState;

/// Mount notification log routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/notifications", get(notifications_page))
        .route("/admin/notifications/partial", get(notifications_partial))
        .route(
            "/admin/notifications/prune",
            axum::routing::delete(prune_handler),
        )
}

// ---------------------------------------------------------------------------
// GET /admin/notifications
// ---------------------------------------------------------------------------

async fn notifications_page(State(state): State<AppState>) -> Html<String> {
    let (entries, stats) = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let entries = recent_notifications(conn, 100, 0).unwrap_or_default();
            let stats = notification_stats(conn, 30).unwrap_or((0, 0, 0));
            (entries, stats)
        })
    })
    .await
    .unwrap_or_default();

    Html(render_page(&entries, stats))
}

// ---------------------------------------------------------------------------
// GET /admin/notifications/partial  (HTMX partial — table rows only)
// ---------------------------------------------------------------------------

async fn notifications_partial(State(state): State<AppState>) -> Html<String> {
    let entries = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| recent_notifications(conn, 100, 0).unwrap_or_default())
    })
    .await
    .unwrap_or_default();

    Html(render_table_rows(&entries))
}

// ---------------------------------------------------------------------------
// DELETE /admin/notifications/prune
// ---------------------------------------------------------------------------

async fn prune_handler(State(state): State<AppState>) -> Result<Html<String>, StatusCode> {
    let deleted = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::notifications::prune_old_notifications(conn, 90).unwrap_or(0)
        })
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(format!(
        r#"<div style="color:#4ade80;padding:0.5rem 0;">
          Pruned {deleted} notification(s) older than 90 days.
        </div>"#
    )))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_page(entries: &[NotifEntry], stats: (i64, i64, i64)) -> String {
    let (sent, failed, skipped) = stats;
    let rows_html = render_table_rows(entries);
    let count = entries.len();

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1.0">
  <title>Notification History - BirdNet-Behavior</title>
  <script src="/static/htmx.min.js"></script>
  <style>
    body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
    .container {{ max-width:1100px; margin:0 auto; padding:2rem 1rem; }}
    nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
    nav a:hover {{ color:#38bdf8; }}
    .card {{ background:#1e293b; border:1px solid #334155; border-radius:0.75rem;
             padding:1.5rem; margin-bottom:1.5rem; }}
    .stat {{ text-align:center; }}
    .stat .value {{ font-size:2rem; font-weight:700; }}
    .stat .label {{ font-size:0.8rem; color:#94a3b8; margin-top:0.25rem; }}
    table {{ width:100%; border-collapse:collapse; font-size:0.85rem; }}
    th {{ text-align:left; color:#64748b; font-weight:600; padding:0.5rem 0.75rem;
          border-bottom:1px solid #334155; }}
    td {{ padding:0.5rem 0.75rem; border-bottom:1px solid #1e293b; }}
    tr:hover td {{ background:#1e293b; }}
    .badge {{ display:inline-block; padding:0.15rem 0.5rem; border-radius:9999px;
              font-size:0.75rem; font-weight:600; }}
    .badge-sent {{ background:#14532d; color:#4ade80; }}
    .badge-failed {{ background:#450a0a; color:#f87171; }}
    .badge-skipped {{ background:#422006; color:#fbbf24; }}
    .btn {{ padding:0.4rem 1rem; border-radius:0.375rem; border:none;
            cursor:pointer; font-weight:600; font-size:0.8rem; }}
    .btn-danger {{ background:#7f1d1d; color:#fca5a5; }}
    .btn-danger:hover {{ background:#991b1b; }}
    .empty {{ color:#64748b; text-align:center; padding:2rem; }}
  </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/migrate">Migration</a>
    <a href="/admin/system">System</a>
    <a href="/admin/notifications" style="color:#38bdf8;">Notifications</a>
  </nav>

  <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:1.5rem;">
    <h1 style="font-size:1.5rem;font-weight:700;color:#f1f5f9;">Notification History</h1>
    <button class="btn btn-danger"
            hx-delete="/admin/notifications/prune"
            hx-target="#prune-result"
            hx-swap="innerHTML"
            hx-confirm="Prune notifications older than 90 days?">
      Prune Old Entries
    </button>
  </div>
  <div id="prune-result"></div>

  <!-- Stats cards -->
  <div style="display:grid;grid-template-columns:repeat(3,1fr);gap:1rem;margin-bottom:1.5rem;">
    <div class="card stat">
      <div class="value" style="color:#4ade80;">{sent}</div>
      <div class="label">Sent (30 days)</div>
    </div>
    <div class="card stat">
      <div class="value" style="color:#f87171;">{failed}</div>
      <div class="label">Failed (30 days)</div>
    </div>
    <div class="card stat">
      <div class="value" style="color:#fbbf24;">{skipped}</div>
      <div class="label">Skipped (30 days)</div>
    </div>
  </div>

  <div class="card" style="padding:0;overflow:hidden;">
    <div style="padding:1rem 1.5rem;border-bottom:1px solid #334155;
                display:flex;justify-content:space-between;align-items:center;">
      <span style="font-weight:600;color:#f1f5f9;">Recent Notifications</span>
      <span style="color:#64748b;font-size:0.85rem;">{count} entries</span>
    </div>
    <div id="notif-table"
         hx-get="/admin/notifications/partial"
         hx-trigger="every 30s"
         hx-swap="innerHTML">
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Channel</th>
            <th>Species</th>
            <th>Confidence</th>
            <th>Status</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody id="notif-rows">
          {rows_html}
        </tbody>
      </table>
    </div>
  </div>
</div>
</body>
</html>"##
    )
}

fn render_table_rows(entries: &[NotifEntry]) -> String {
    if entries.is_empty() {
        return r#"<tr><td colspan="6" class="empty">No notifications yet.</td></tr>"#.to_string();
    }
    entries
        .iter()
        .map(|e| {
            let badge_class = match e.status.as_str() {
                "sent" => "badge-sent",
                "failed" => "badge-failed",
                _ => "badge-skipped",
            };
            let species = e
                .species_com_name
                .as_deref()
                .unwrap_or("—")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let confidence = e
                .confidence
                .map_or("—".to_string(), |c| format!("{:.0}%", c * 100.0));
            let msg = e
                .message
                .as_deref()
                .unwrap_or("")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let error_html = if let Some(err) = &e.error {
                format!(
                    r#"<br><span style="color:#f87171;font-size:0.75rem;">{}</span>"#,
                    err.replace('<', "&lt;").replace('>', "&gt;")
                )
            } else {
                String::new()
            };
            format!(
                r#"<tr>
                  <td style="white-space:nowrap;color:#94a3b8;">{sent_at}</td>
                  <td><code style="font-size:0.8rem;">{channel}</code></td>
                  <td>{species}</td>
                  <td style="color:#94a3b8;">{confidence}</td>
                  <td><span class="badge {badge_class}">{status}</span></td>
                  <td style="color:#94a3b8;">{msg}{error_html}</td>
                </tr>"#,
                sent_at = &e.sent_at[..16], // trim seconds
                channel = e.channel,
                status = e.status,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::notifications::NotifEntry;

    fn make_entry(channel: &str, status: &str) -> NotifEntry {
        NotifEntry {
            id: 1,
            sent_at: "2026-03-13 06:15:00".into(),
            channel: channel.into(),
            species_com_name: Some("European Robin".into()),
            species_sci_name: Some("Erithacus rubecula".into()),
            confidence: Some(0.92),
            detection_date: Some("2026-03-13".into()),
            detection_time: Some("06:15:00".into()),
            status: status.into(),
            message: Some("Detected".into()),
            error: None,
        }
    }

    #[test]
    fn render_table_rows_empty() {
        let html = render_table_rows(&[]);
        assert!(html.contains("No notifications"));
    }

    #[test]
    fn render_table_rows_sent() {
        let entry = make_entry("birdweather", "sent");
        let html = render_table_rows(&[entry]);
        assert!(html.contains("badge-sent"));
        assert!(html.contains("birdweather"));
        assert!(html.contains("European Robin"));
    }

    #[test]
    fn render_table_rows_failed() {
        let entry = make_entry("apprise", "failed");
        let html = render_table_rows(&[entry]);
        assert!(html.contains("badge-failed"));
    }

    #[test]
    fn render_page_has_stats() {
        let html = render_page(&[], (5, 2, 1));
        assert!(html.contains(">5<"));
        assert!(html.contains(">2<"));
        assert!(html.contains(">1<"));
    }
}
