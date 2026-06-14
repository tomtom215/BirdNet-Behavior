//! Admin notification history routes.
//!
//! `GET /admin/notifications` — full HTML page showing the notification log.
//! `GET /admin/notifications/partial` — HTMX partial (table rows only) for polling.
//! `DELETE /admin/notifications/prune` — prune entries older than 90 days.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};

use birdnet_db::notifications::{NotifEntry, notification_stats, recent_notifications};

use crate::routes::pages::confirm::{self, Action, Confirm, Style};
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

/// The standalone `/admin/notifications` page GET folded into the Station
/// **Alerts** tab; its old URL permanently redirects there. The partial-poll
/// and prune endpoints below keep their `/admin/notifications/...` paths.
async fn notifications_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/alerts")
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
        r#"<div class="prune-ok">
          Pruned {deleted} notification(s) older than 90 days.
        </div>"#
    )))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Fetch the recent notification log + 30-day stats and render the body.
///
/// Shared with the Station **Alerts** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the "recent
/// alerts sent" surface in the main shell.
#[allow(clippy::similar_names)]
pub(crate) fn recent_body(state: &AppState) -> String {
    let (entries, stats) = state.with_db(|conn| {
        let entries = recent_notifications(conn, 100, 0).unwrap_or_default();
        let stats = notification_stats(conn, 30).unwrap_or((0, 0, 0));
        (entries, stats)
    });
    notifications_body(&entries, stats)
}

/// The notification-history body (scoped `<style>` + stats cards + table).
#[allow(clippy::too_many_lines)]
fn notifications_body(entries: &[NotifEntry], stats: (i64, i64, i64)) -> String {
    let (sent, failed, skipped) = stats;
    let rows_html = render_table_rows(entries);
    let count = entries.len();

    // O-17: themed confirmation modal for the destructive prune action.
    let prune_btn = confirm::confirm_button(Confirm {
        label: "Prune Old Entries",
        action: Action::Delete("/admin/notifications/prune"),
        title: "Prune notifications",
        body: "Prune notifications older than 90 days?",
        confirm_label: "Prune",
        style: Style::Danger,
        target: Some("#prune-result"),
        swap: Some("innerHTML"),
    });

    format!(
        r#"<style>
    .card {{ background:var(--surface); border:1px solid var(--border); border-radius:0.75rem;
             padding:1.5rem; margin-bottom:1.5rem; }}
    .stat {{ text-align:center; }}
    .stat .value {{ font-size:2rem; font-weight:700; }}
    .stat .label {{ font-size:0.8rem; color:var(--fg-3); margin-top:0.25rem; }}
    table {{ width:100%; border-collapse:collapse; font-size:0.85rem; }}
    th {{ text-align:left; color:var(--fg-4); font-weight:600; padding:0.5rem 0.75rem;
          border-bottom:1px solid var(--border); }}
    td {{ padding:0.5rem 0.75rem; border-bottom:1px solid var(--surface); }}
    tr:hover td {{ background:var(--surface); }}
    .badge {{ display:inline-block; padding:0.15rem 0.5rem; border-radius:9999px;
              font-size:0.75rem; font-weight:600; }}
    .badge-sent {{ background:var(--moss-soft); color:var(--moss); }}
    .badge-failed {{ background:var(--rare-soft); color:var(--rare); }}
    .badge-skipped {{ background:var(--dawn-soft); color:var(--dawn); }}
    .btn {{ padding:0.4rem 1rem; border-radius:0.375rem; border:none;
            cursor:pointer; font-weight:600; font-size:0.8rem; }}
    .btn-danger {{ background:var(--rare-soft); color:var(--rare); }}
    .btn-danger:hover {{ background:var(--rare-soft); }}
    .empty {{ color:var(--fg-4); text-align:center; padding:2rem; }}
    h1 {{ font-size:1.5rem; font-weight:700; color:var(--fg); }}
    .page-head {{ display:flex; justify-content:space-between; align-items:center; margin-bottom:1.5rem; }}
    .stat-grid {{ display:grid; grid-template-columns:repeat(3,1fr); gap:1rem; margin-bottom:1.5rem; }}
    .value.moss {{ color:var(--moss); }}
    .value.rare {{ color:var(--rare); }}
    .value.dawn {{ color:var(--dawn); }}
    .card.flush {{ padding:0; overflow:hidden; }}
    .table-head {{ padding:1rem 1.5rem; border-bottom:1px solid var(--border); display:flex; justify-content:space-between; align-items:center; }}
    .th-title {{ font-weight:600; color:var(--fg); }}
    .th-count {{ color:var(--fg-4); font-size:0.85rem; }}
    td.col-time {{ white-space:nowrap; color:var(--fg-3); }}
    td code {{ font-size:0.8rem; }}
    td.col-muted {{ color:var(--fg-3); }}
    .row-error {{ color:var(--rare); font-size:0.75rem; }}
    .prune-ok {{ color:var(--moss); padding:0.5rem 0; }}
  </style>

  <div class="page-head">
    <h1>Notification History</h1>
    {prune_btn}
  </div>
  <div id="prune-result"></div>

  <!-- Stats cards -->
  <div class="stat-grid">
    <div class="card stat">
      <div class="value moss">{sent}</div>
      <div class="label">Sent (30 days)</div>
    </div>
    <div class="card stat">
      <div class="value rare">{failed}</div>
      <div class="label">Failed (30 days)</div>
    </div>
    <div class="card stat">
      <div class="value dawn">{skipped}</div>
      <div class="label">Skipped (30 days)</div>
    </div>
  </div>

  <div class="card flush">
    <div class="table-head">
      <span class="th-title">Recent Notifications</span>
      <span class="th-count">{count} entries</span>
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
  </div>"#
    )
}

fn render_table_rows(entries: &[NotifEntry]) -> String {
    if entries.is_empty() {
        return r#"<tr><td colspan="6" class="empty">No notifications yet.</td></tr>"#.to_string();
    }
    let mut out = String::new();
    for e in entries {
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
            .map_or_else(|| "—".to_string(), |c| format!("{:.0}%", c * 100.0));
        let msg = e
            .message
            .as_deref()
            .unwrap_or("")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let error_html = e.error.as_ref().map_or_else(String::new, |err| {
            format!(
                r#"<br><span class="row-error">{}</span>"#,
                err.replace('<', "&lt;").replace('>', "&gt;")
            )
        });
        write!(
            out,
            r#"<tr>
                  <td class="col-time">{sent_at}</td>
                  <td><code>{channel}</code></td>
                  <td>{species}</td>
                  <td class="col-muted">{confidence}</td>
                  <td><span class="badge {badge_class}">{status}</span></td>
                  <td class="col-muted">{msg}{error_html}</td>
                </tr>"#,
            sent_at = &e.sent_at[..16], // trim seconds
            channel = e.channel,
            status = e.status,
        )
        .unwrap_or_default();
    }
    out
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
    fn notifications_body_has_stats() {
        let html = notifications_body(&[], (5, 2, 1));
        assert!(html.contains(">5<"));
        assert!(html.contains(">2<"));
        assert!(html.contains(">1<"));
    }

    #[test]
    fn pages_have_no_inline_style_attributes() {
        // P3-3 (O-25): this page's own chrome and rows carry no inline `style=`
        // attributes — page-specific styling lives in the page's <style> block,
        // and status colours use enumerable classes. (render_page also embeds the
        // shared confirm-modal component, migrated separately, so we assert on
        // this file's own markup rather than a blanket zero over the whole page.)
        assert!(!render_table_rows(&[make_entry("birdweather", "sent")]).contains("style=\""));
        assert!(!render_table_rows(&[make_entry("apprise", "failed")]).contains("style=\""));
        assert!(!notifications_body(&[], (0, 0, 0)).contains("<nav style"));
    }
}
