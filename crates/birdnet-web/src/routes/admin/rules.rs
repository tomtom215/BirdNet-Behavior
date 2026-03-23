//! Admin alert-rules management routes.
//!
//! Alert rules let users define conditional actions that fire whenever a
//! detection matches a set of criteria (species, confidence, time window,
//! day of week).  Three action types are supported:
//!
//! - **webhook** — HTTP POST/GET to a user-supplied URL.
//! - **log** — emit a structured `INFO` log entry (useful with log exporters).
//! - **suppress** — block all other notifications (Apprise, email, MQTT) for
//!   this particular detection event.
//!
//! | Path | Method | Purpose |
//! |------|--------|---------|
//! | `/admin/rules` | GET | Rules list page |
//! | `/admin/rules/list` | GET | HTMX partial — rules table |
//! | `/admin/rules` | POST | Create new rule (HTMX form) |
//! | `/admin/rules/{id}/delete` | POST | Delete rule |
//! | `/admin/rules/{id}/toggle` | POST | Enable / disable rule |

use std::fmt::Write as _;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Form, Router, routing::get};
use serde::Deserialize;

use birdnet_db::alert_rules::{
    AlertAction, NewAlertRule, delete_rule, insert_rule, list_rules, toggle_rule,
};

use crate::routes::pages::escape_html;
use crate::state::AppState;

/// Mount alert-rules admin routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/rules", get(rules_page).post(create_rule))
        .route("/admin/rules/list", get(rules_list_partial))
        .route(
            "/admin/rules/{id}/delete",
            axum::routing::post(delete_rule_handler),
        )
        .route(
            "/admin/rules/{id}/toggle",
            axum::routing::post(toggle_rule_handler),
        )
}

// ---------------------------------------------------------------------------
// Form input
// ---------------------------------------------------------------------------

/// Form data for creating an alert rule.
#[derive(Debug, Deserialize)]
struct RuleForm {
    name: String,
    species_pattern: Option<String>,
    confidence_min: Option<f64>,
    confidence_max: Option<f64>,
    hour_start: Option<u8>,
    hour_end: Option<u8>,
    days_of_week: Option<String>,
    action_type: String,
    action_webhook_url: Option<String>,
    action_webhook_method: Option<String>,
    action_webhook_body: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn rules_page(State(state): State<AppState>) -> Html<String> {
    let rules_html = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| list_rules(conn).unwrap_or_default())
    })
    .await
    .unwrap_or_default();

    Html(render_page(&rules_html))
}

async fn rules_list_partial(State(state): State<AppState>) -> Html<String> {
    let rules = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| list_rules(conn).unwrap_or_default())
    })
    .await
    .unwrap_or_default();

    Html(render_rules_table(&rules))
}

async fn create_rule(
    State(state): State<AppState>,
    Form(form): Form<RuleForm>,
) -> Result<Html<String>, StatusCode> {
    // Normalise empty strings to None
    let species_pattern = form
        .species_pattern
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let days_of_week = form
        .days_of_week
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    let action = match form.action_type.as_str() {
        "webhook" => {
            let url = form
                .action_webhook_url
                .filter(|s| !s.trim().is_empty())
                .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
            AlertAction::Webhook {
                url: url.trim().to_string(),
                method: form
                    .action_webhook_method
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "POST".into()),
                body_template: form.action_webhook_body.filter(|s| !s.trim().is_empty()),
            }
        }
        "suppress" => AlertAction::Suppress,
        _ => AlertAction::Log,
    };

    let new_rule = NewAlertRule {
        name: form.name.trim().to_string(),
        enabled: true,
        species_pattern,
        confidence_min: form.confidence_min.unwrap_or(0.0).clamp(0.0, 1.0),
        confidence_max: form.confidence_max.unwrap_or(1.0).clamp(0.0, 1.0),
        hour_start: form.hour_start,
        hour_end: form.hour_end,
        days_of_week,
        action,
    };

    tokio::task::spawn_blocking(move || state.with_db(|conn| insert_rule(conn, &new_rule)))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Return a success message; HTMX will trigger a reload of the list via hx-on
    Ok(Html(format!(
        "<div style=\"color:#4ade80;padding:.5rem;border-radius:.375rem;background:#14532d33;\">Rule created successfully.</div>\
         <div hx-get=\"/admin/rules/list\" hx-trigger=\"load\" hx-target=\"{}\" hx-swap=\"innerHTML\"></div>",
        "#rules-table-container"
    )))
}

async fn delete_rule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    tokio::task::spawn_blocking(move || state.with_db(|conn| delete_rule(conn, id)))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(String::new())) // HTMX removes the row via hx-target swap
}

async fn toggle_rule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let new_state =
        tokio::task::spawn_blocking(move || state.with_db(|conn| toggle_rule(conn, id)))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let enabled = new_state.unwrap_or(false);
    let label = if enabled { "Enabled" } else { "Disabled" };
    let color = if enabled { "#4ade80" } else { "#94a3b8" };
    Ok(Html(format!(
        r#"<span style="color:{color};font-weight:600;">{label}</span>"#
    )))
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

fn render_page(_rules: &[birdnet_db::alert_rules::AlertRule]) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width,initial-scale=1.0">
    <title>Alert Rules — BirdNet-Behavior Admin</title>
    <script src="/static/htmx.min.js"></script>
    <style>
      body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; margin:0; }}
      .container {{ max-width:960px; margin:0 auto; padding:2rem 1rem; }}
      nav {{ margin-bottom:2rem; }}
      nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; font-size:.9rem; }}
      nav a:hover {{ color:#38bdf8; }}
      h1 {{ font-size:1.5rem; font-weight:700; color:#f8fafc; margin-bottom:.25rem; }}
      .subtitle {{ color:#64748b; font-size:.875rem; margin-bottom:2rem; }}
      .card {{ background:#1e293b; border:1px solid #334155; border-radius:.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .card h2 {{ font-size:1.1rem; color:#38bdf8; margin:0 0 1rem; }}
      label {{ display:block; font-size:.8rem; color:#94a3b8; margin-bottom:.25rem; margin-top:.75rem; }}
      label:first-of-type {{ margin-top:0; }}
      input,select,textarea {{ width:100%; background:#0f172a; border:1px solid #334155; border-radius:.375rem;
                               color:#e2e8f0; padding:.5rem .75rem; font-size:.875rem; box-sizing:border-box; }}
      input:focus,select:focus,textarea:focus {{ outline:none; border-color:#38bdf8; }}
      .form-grid {{ display:grid; grid-template-columns:1fr 1fr; gap:1rem; }}
      .form-grid-3 {{ display:grid; grid-template-columns:1fr 1fr 1fr; gap:1rem; }}
      .btn {{ padding:.5rem 1.25rem; border-radius:.375rem; border:none; cursor:pointer; font-weight:600; font-size:.875rem; }}
      .btn-primary {{ background:#0ea5e9; color:#fff; }}
      .btn-primary:hover {{ background:#0284c7; }}
      .btn-danger {{ background:#dc2626; color:#fff; }}
      .btn-danger:hover {{ background:#b91c1c; }}
      .btn-sm {{ padding:.25rem .75rem; font-size:.8rem; }}
      table {{ width:100%; border-collapse:collapse; font-size:.875rem; }}
      th {{ text-align:left; color:#64748b; font-weight:600; font-size:.75rem; text-transform:uppercase;
             padding:.5rem .75rem; border-bottom:1px solid #334155; }}
      td {{ padding:.6rem .75rem; border-bottom:1px solid #1e293b; vertical-align:middle; }}
      tr:hover td {{ background:#1e293b55; }}
      .badge {{ display:inline-block; padding:.15rem .5rem; border-radius:.25rem; font-size:.75rem; font-weight:600; }}
      .badge-green {{ background:#14532d; color:#4ade80; }}
      .badge-gray  {{ background:#1e293b; color:#64748b; border:1px solid #334155; }}
      .badge-blue  {{ background:#1e3a5f; color:#60a5fa; }}
      .badge-red   {{ background:#450a0a; color:#f87171; }}
      .badge-yellow{{ background:#422006; color:#fbbf24; }}
      #webhook-fields {{ display:none; }}
      .hint {{ color:#64748b; font-size:.75rem; margin-top:.25rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav>
    <a href="/admin/overview">Overview</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/rules" style="color:#38bdf8;">Rules</a>
    <a href="/admin/notifications">Notifications</a>
    <a href="/admin/system">System</a>
  </nav>

  <h1>Alert Rules</h1>
  <p class="subtitle">
    Define conditional actions triggered by detections — webhooks, structured
    logs, or notification suppression.
  </p>

  <!-- Create Rule Form -->
  <div class="card">
    <h2>Create Rule</h2>
    <form hx-post="/admin/rules"
          hx-target="#form-result"
          hx-swap="innerHTML"
          hx-on::after-request="if(event.detail.successful) this.reset()">

      <label for="name">Rule Name</label>
      <input id="name" name="name" type="text" placeholder="e.g. Rare owl webhook" required>

      <div class="form-grid">
        <div>
          <label for="species_pattern">Species Pattern (blank = any)</label>
          <input id="species_pattern" name="species_pattern" type="text"
                 placeholder="Barn Owl, Barn*, *Owl, * ">
          <div class="hint">Wildcards: * matches any characters. Case-insensitive.</div>
        </div>
        <div>
          <label for="action_type">Action</label>
          <select id="action_type" name="action_type"
                  onchange="document.getElementById('webhook-fields').style.display=this.value==='webhook'?'block':'none'">
            <option value="log">Log (structured INFO entry)</option>
            <option value="webhook">Webhook (HTTP request)</option>
            <option value="suppress">Suppress (block all notifications)</option>
          </select>
        </div>
      </div>

      <div class="form-grid-3">
        <div>
          <label for="confidence_min">Min Confidence (0.0–1.0)</label>
          <input id="confidence_min" name="confidence_min" type="number"
                 min="0" max="1" step="0.01" value="0.70">
        </div>
        <div>
          <label for="confidence_max">Max Confidence (0.0–1.0)</label>
          <input id="confidence_max" name="confidence_max" type="number"
                 min="0" max="1" step="0.01" value="1.00">
        </div>
        <div>
          <label for="days_of_week">Days of Week (blank = any)</label>
          <input id="days_of_week" name="days_of_week" type="text"
                 placeholder="1,2,3,4,5  (1=Mon…7=Sun)">
        </div>
      </div>

      <div class="form-grid">
        <div>
          <label for="hour_start">Hour Start (0–23, blank = any)</label>
          <input id="hour_start" name="hour_start" type="number" min="0" max="23" placeholder="e.g. 5">
        </div>
        <div>
          <label for="hour_end">Hour End (0–23, blank = any)</label>
          <input id="hour_end" name="hour_end" type="number" min="0" max="23" placeholder="e.g. 9">
        </div>
      </div>

      <div id="webhook-fields">
        <label for="action_webhook_url">Webhook URL</label>
        <input id="action_webhook_url" name="action_webhook_url" type="url"
               placeholder="https://example.com/hook">

        <div class="form-grid">
          <div>
            <label for="action_webhook_method">HTTP Method</label>
            <select id="action_webhook_method" name="action_webhook_method">
              <option value="POST">POST</option>
              <option value="GET">GET</option>
            </select>
          </div>
          <div>
            <label for="action_webhook_body">Body Template (optional)</label>
            <input id="action_webhook_body" name="action_webhook_body" type="text"
                   placeholder='&#123;"bird":"&#123;&#123;species&#125;&#125;"&#125;'>
            <div class="hint">Placeholders: {{species}}, {{sci_name}}, {{confidence}}, {{date}}, {{time}}</div>
          </div>
        </div>
      </div>

      <div style="margin-top:1.25rem;">
        <button type="submit" class="btn btn-primary">Create Rule</button>
      </div>
      <div id="form-result" style="margin-top:.75rem;"></div>
    </form>
  </div>

  <!-- Rules Table -->
  <div class="card">
    <h2>Active Rules</h2>
    <div id="rules-table-container"
         hx-get="/admin/rules/list"
         hx-trigger="load"
         hx-swap="innerHTML">
      <p style="color:#64748b;">Loading…</p>
    </div>
  </div>
</div>
</body>
</html>"##
    )
}

fn render_rules_table(rules: &[birdnet_db::alert_rules::AlertRule]) -> String {
    if rules.is_empty() {
        return r#"<p style="color:#64748b;text-align:center;padding:2rem 0;">
            No alert rules defined. Create one above.
        </p>"#
            .to_string();
    }

    let mut html = String::with_capacity(2048);
    html.push_str(
        r#"<table>
<thead>
  <tr>
    <th>Name</th>
    <th>Species</th>
    <th>Confidence</th>
    <th>Window</th>
    <th>Action</th>
    <th>Status</th>
    <th>Actions</th>
  </tr>
</thead>
<tbody>"#,
    );

    for rule in rules {
        let status_badge = if rule.enabled {
            r#"<span class="badge badge-green">Enabled</span>"#
        } else {
            r#"<span class="badge badge-gray">Disabled</span>"#
        };

        let species_display = rule
            .species_pattern
            .as_deref()
            .map(escape_html)
            .unwrap_or_else(|| "<em style='color:#64748b'>any</em>".to_string());

        let conf_display = format!(
            "{:.0}%–{:.0}%",
            rule.confidence_min * 100.0,
            rule.confidence_max * 100.0
        );

        let window_display = match (rule.hour_start, rule.hour_end) {
            (Some(s), Some(e)) => format!("{s:02}:00–{e:02}:59"),
            _ => "<em style='color:#64748b'>any time</em>".to_string(),
        };

        let action_badge = match &rule.action {
            AlertAction::Webhook { url, method, .. } => {
                let url_short = if url.len() > 30 {
                    format!("{}…", &url[..30])
                } else {
                    url.clone()
                };
                format!(
                    r#"<span class="badge badge-blue">{method}</span> <span style="font-size:.75rem;color:#94a3b8;">{}</span>"#,
                    escape_html(&url_short)
                )
            }
            AlertAction::Log => r#"<span class="badge badge-yellow">Log</span>"#.to_string(),
            AlertAction::Suppress => r#"<span class="badge badge-red">Suppress</span>"#.to_string(),
        };

        let id = rule.id;
        write!(
            html,
            r##"<tr id="rule-row-{id}">
  <td><strong>{name}</strong></td>
  <td>{species_display}</td>
  <td style="white-space:nowrap">{conf_display}</td>
  <td style="white-space:nowrap">{window_display}</td>
  <td>{action_badge}</td>
  <td hx-post="/admin/rules/{id}/toggle"
      hx-swap="innerHTML"
      hx-target="this"
      style="cursor:pointer;user-select:none;"
      title="Click to toggle">{status_badge}</td>
  <td>
    <button class="btn btn-danger btn-sm"
            hx-post="/admin/rules/{id}/delete"
            hx-confirm="Delete rule '{name}'?"
            hx-target="#rule-row-{id}"
            hx-swap="outerHTML">Delete</button>
  </td>
</tr>"##,
            id = id,
            name = escape_html(&rule.name),
            species_display = species_display,
            conf_display = conf_display,
            window_display = window_display,
            action_badge = action_badge,
            status_badge = status_badge,
        )
        .unwrap_or_default();
    }

    html.push_str("</tbody></table>");
    html
}
