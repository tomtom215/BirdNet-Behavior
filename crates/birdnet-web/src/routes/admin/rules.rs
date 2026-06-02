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
use crate::routes::pages::toast::{self, Toast};
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
///
/// `confidence_min`/`confidence_max` are received as strings so the
/// handler can accept both `.` and `,` decimal separators (EU
/// operators) via `birdnet_core::config::locale::parse_decimal`.
/// Receiving them as `f64` here would let serde's default parser
/// reject any comma value with a 422.
#[derive(Debug, Deserialize)]
struct RuleForm {
    name: String,
    species_pattern: Option<String>,
    confidence_min: Option<String>,
    confidence_max: Option<String>,
    hour_start: Option<u8>,
    hour_end: Option<u8>,
    days_of_week: Option<String>,
    action_type: String,
    action_webhook_url: Option<String>,
    action_webhook_method: Option<String>,
    action_webhook_body: Option<String>,
}

/// Locale-tolerant decimal parser for an `Option<String>` form field.
/// Returns the parsed `f64` or the documented `default` when the field
/// is absent, empty, or unparseable.
fn parse_optional_decimal(raw: Option<&str>, default: f64) -> f64 {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| birdnet_core::config::locale::parse_decimal(s).ok())
        .unwrap_or(default)
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

    let rule_name = form.name.trim().to_string();
    let new_rule = NewAlertRule {
        name: rule_name.clone(),
        enabled: true,
        species_pattern,
        confidence_min: parse_optional_decimal(form.confidence_min.as_deref(), 0.0).clamp(0.0, 1.0),
        confidence_max: parse_optional_decimal(form.confidence_max.as_deref(), 1.0).clamp(0.0, 1.0),
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
    let body = Html(format!(
        "<div class=\"rule-success\">Rule created successfully.</div>\
         <div hx-get=\"/admin/rules/list\" hx-trigger=\"load\" hx-target=\"{}\" hx-swap=\"innerHTML\"></div>",
        "#rules-table-container"
    ));
    // O-18: toast the success outcome via OOB.
    Ok(toast::with(
        body,
        Toast::success(format!("Rule '{rule_name}' enabled.")),
    ))
}

async fn delete_rule_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    tokio::task::spawn_blocking(move || state.with_db(|conn| delete_rule(conn, id)))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // O-18: HTMX removes the row via outerHTML swap (response body is empty
    // after OOB extraction); the OOB toast confirms the action separately.
    Ok(toast::oob_only(Toast::success("Rule deleted.")))
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
    let cls = if enabled { "on" } else { "off" };
    let body = Html(format!(
        r#"<span class="toggle-state {cls}">{label}</span>"#
    ));
    // O-18: toast the new enabled/disabled state.
    let msg = if enabled {
        "Rule enabled."
    } else {
        "Rule disabled."
    };
    Ok(toast::with(body, Toast::success(msg)))
}

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn render_page(_rules: &[birdnet_db::alert_rules::AlertRule]) -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head><script src="/static/theme-guard.js"></script><link rel="stylesheet" href="/static/css/app.css">
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width,initial-scale=1.0">
    <title>Alert Rules — BirdNet-Behavior Admin</title>
    <script src="/static/htmx.min.js"></script>
    <style>
      body { background:var(--bg); color:var(--fg); font-family:var(--font-ui); margin:0; }
      .container { max-width:960px; margin:0 auto; padding:2rem 1rem; }
      nav { margin-bottom:2rem; }
      nav a { color:var(--fg-3); text-decoration:none; margin-right:1.5rem; font-size:.9rem; }
      nav a:hover { color:var(--moss-ink); }
      h1 { font-size:1.5rem; font-weight:700; color:var(--fg); margin-bottom:.25rem; }
      .subtitle { color:var(--fg-4); font-size:.875rem; margin-bottom:2rem; }
      .card { background:var(--surface); border:1px solid var(--border); border-radius:.75rem; padding:1.5rem; margin-bottom:1.5rem; }
      .card h2 { font-size:1.1rem; color:var(--moss-ink); margin:0 0 1rem; }
      label { display:block; font-size:.8rem; color:var(--fg-3); margin-bottom:.25rem; margin-top:.75rem; }
      label:first-of-type { margin-top:0; }
      input,select,textarea { width:100%; background:var(--bg); border:1px solid var(--border); border-radius:.375rem;
                               color:var(--fg); padding:.5rem .75rem; font-size:.875rem; box-sizing:border-box; }
      input:focus,select:focus,textarea:focus { outline:none; border-color:var(--moss-ink); }
      .form-grid { display:grid; grid-template-columns:1fr 1fr; gap:1rem; }
      .form-grid-3 { display:grid; grid-template-columns:1fr 1fr 1fr; gap:1rem; }
      .btn { padding:.5rem 1.25rem; border-radius:.375rem; border:none; cursor:pointer; font-weight:600; font-size:.875rem; }
      .btn-primary { background:var(--moss); color:#fff; }
      .btn-primary:hover { background:var(--moss-ink); }
      .btn-danger { background:var(--rare); color:#fff; }
      .btn-danger:hover { background:var(--rare); }
      .btn-sm { padding:.25rem .75rem; font-size:.8rem; }
      table { width:100%; border-collapse:collapse; font-size:.875rem; }
      th { text-align:left; color:var(--fg-4); font-weight:600; font-size:.75rem; text-transform:uppercase;
             padding:.5rem .75rem; border-bottom:1px solid var(--border); }
      td { padding:.6rem .75rem; border-bottom:1px solid var(--surface); vertical-align:middle; }
      tr:hover td { background:var(--surface)55; }
      .badge { display:inline-block; padding:.15rem .5rem; border-radius:.25rem; font-size:.75rem; font-weight:600; }
      .badge-green { background:var(--moss-soft); color:var(--moss); }
      .badge-gray  { background:var(--surface); color:var(--fg-4); border:1px solid var(--border); }
      .badge-blue  { background:var(--surface); color:var(--moss-ink); }
      .badge-red   { background:var(--rare-soft); color:var(--rare); }
      .badge-yellow{ background:var(--dawn-soft); color:var(--dawn); }
      #webhook-fields { display:none; }
      .hint { color:var(--fg-4); font-size:.75rem; margin-top:.25rem; }
      /* O-25 sweep — faithful extraction of this page's inline styles. */
      nav a.here { color:var(--moss-ink); }
      .form-actions { margin-top:1.25rem; }
      #form-result { margin-top:.75rem; }
      .rule-success { color:var(--moss); padding:.5rem; border-radius:.375rem; background:var(--moss-soft)33; }
      .tbl-loading { color:var(--fg-4); }
      .tbl-empty { color:var(--fg-4); text-align:center; padding:2rem 0; }
      .any { color:var(--fg-4); }
      .url-frag { font-size:.75rem; color:var(--fg-3); }
      .nowrap { white-space:nowrap; }
      .toggle-cell { cursor:pointer; user-select:none; }
      .toggle-state { font-weight:600; }
      .toggle-state.on { color:var(--moss); }
      .toggle-state.off { color:var(--fg-3); }
    </style>
</head>
<body>
<div class="container">
  <nav>
    <a href="/admin/overview">Overview</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/rules" class="here">Rules</a>
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
          <input id="confidence_min" name="confidence_min" type="text"
                 inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*" value="0.70">
        </div>
        <div>
          <label for="confidence_max">Max Confidence (0.0–1.0)</label>
          <input id="confidence_max" name="confidence_max" type="text"
                 inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*" value="1.00">
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
            <div class="hint">Placeholders: {species}, {sci_name}, {confidence}, {date}, {time}</div>
          </div>
        </div>
      </div>

      <div class="form-actions">
        <button type="submit" class="btn btn-primary">Create Rule</button>
      </div>
      <div id="form-result"></div>
    </form>
  </div>

  <!-- Rules Table -->
  <div class="card">
    <h2>Active Rules</h2>
    <div id="rules-table-container"
         hx-get="/admin/rules/list"
         hx-trigger="load"
         hx-swap="innerHTML">
      <p class="tbl-loading">Loading…</p>
    </div>
  </div>
</div>
</body>
</html>"##
    .to_owned()
}

fn render_rules_table(rules: &[birdnet_db::alert_rules::AlertRule]) -> String {
    if rules.is_empty() {
        return r#"<p class="tbl-empty">
            No alert rules defined. Create one above.
        </p>"#
            .to_string();
    }

    let mut html = String::with_capacity(2048);
    html.push_str(
        r"<table>
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
<tbody>",
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
            .map_or_else(|| "<em class='any'>any</em>".to_string(), escape_html);

        let conf_display = format!(
            "{:.0}%–{:.0}%",
            rule.confidence_min * 100.0,
            rule.confidence_max * 100.0
        );

        let window_display = match (rule.hour_start, rule.hour_end) {
            (Some(s), Some(e)) => format!("{s:02}:00–{e:02}:59"),
            _ => "<em class='any'>any time</em>".to_string(),
        };

        let action_badge = match &rule.action {
            AlertAction::Webhook { url, method, .. } => {
                let url_short = if url.len() > 30 {
                    format!("{}…", &url[..30])
                } else {
                    url.clone()
                };
                format!(
                    r#"<span class="badge badge-blue">{method}</span> <span class="url-frag">{}</span>"#,
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
  <td class="nowrap">{conf_display}</td>
  <td class="nowrap">{window_display}</td>
  <td>{action_badge}</td>
  <td hx-post="/admin/rules/{id}/toggle"
      hx-swap="innerHTML"
      hx-target="this"
      class="toggle-cell"
      title="Click to toggle">{status_badge}</td>
  <td>
    <button class="btn btn-danger btn-sm"
            hx-post="/admin/rules/{id}/delete"
            hx-confirm="Delete rule '{name}'?"
            hx-target="#rule-row-{id}"
            hx-swap="outerHTML"
            data-confirm-action="hx-post"
            data-confirm-url="/admin/rules/{id}/delete"
            data-confirm-title="Delete rule"
            data-confirm-body="Delete rule '{name}'?"
            data-confirm-confirm-label="Delete"
            data-confirm-style="danger">Delete</button>
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
