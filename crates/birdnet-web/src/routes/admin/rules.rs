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
//! | `/admin/rules/export` | GET | Download the rule set as JSON |
//! | `/admin/rules/import` | POST | Add rules from a pasted JSON rule set |
//! | `/admin/rules/{id}/test` | POST | Fire one rule now and report the result |

use std::fmt::Write as _;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Form, Router, routing::get};
use serde::Deserialize;

use birdnet_db::alert_rules::{
    AlertAction, EXPORT_VERSION, Imported, NewAlertRule, RuleSet, WebhookAuth, delete_rule,
    from_export, insert_rule, list_rules, to_export, toggle_rule,
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
        .route("/admin/rules/export", get(export_rules))
        .route("/admin/rules/import", axum::routing::post(import_rules))
        .route("/admin/rules/{id}/test", axum::routing::post(test_rule))
}

// ---------------------------------------------------------------------------
// Export / import
// ---------------------------------------------------------------------------

/// Query for [`export_rules`].
#[derive(Debug, Deserialize)]
struct ExportQuery {
    /// `1`/`true` to include credentials in the download.
    #[serde(default)]
    secrets: Option<String>,
}

/// `GET /admin/rules/export` — the rule set as a JSON attachment.
///
/// Credentials are replaced with `***REDACTED***` unless `?secrets=1` is
/// passed. Redacting by default is what makes the ordinary export something an
/// operator can paste into a forum thread when asking for help; the opt-in is
/// for moving a station or restoring a backup.
async fn export_rules(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ExportQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let include_secrets = matches!(q.secrets.as_deref(), Some("1" | "true" | "yes" | "on"));
    let rules = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| list_rules(conn).unwrap_or_default())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let set = RuleSet {
        version: EXPORT_VERSION,
        redacted: !include_secrets,
        rules: rules
            .iter()
            .map(|r| to_export(r, include_secrets))
            .collect(),
    };
    let body = serde_json::to_string_pretty(&set).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if include_secrets {
        tracing::warn!(
            rules = set.rules.len(),
            "alert rules exported *with* credentials"
        );
    }

    axum::response::Response::builder()
        .header("Content-Type", "application/json; charset=utf-8")
        .header(
            "Content-Disposition",
            "attachment; filename=\"alert-rules.json\"",
        )
        // The redacted form still names hosts and species; the unredacted form
        // is a credential file. Neither belongs in a shared cache.
        .header("Cache-Control", "no-store")
        .body(body.into())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Form for [`import_rules`].
#[derive(Debug, Deserialize)]
struct ImportForm {
    /// The pasted JSON rule set.
    rules_json: String,
}

/// What an import did, in a form the handler can render and a test can assert.
#[derive(Debug, PartialEq, Eq)]
struct ImportOutcome {
    /// Rules inserted.
    added: usize,
    /// Rules whose credential arrived redacted and was dropped.
    needs_credential: Vec<String>,
    /// Entries that could not be used, with the reason.
    rejected: Vec<String>,
}

/// Decide what an import would do, without touching the database.
///
/// Separated from the handler so the partial-success behaviour — some rules in,
/// some rejected, some needing their credential re-entered — is assertable
/// without a request.
fn plan_import(json: &str) -> Result<(Vec<Imported>, ImportOutcome), String> {
    let set: RuleSet = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if set.version > EXPORT_VERSION {
        return Err(format!(
            "this rule set is version {} but this station understands up to {EXPORT_VERSION}",
            set.version
        ));
    }

    let mut ready = Vec::new();
    let mut outcome = ImportOutcome {
        added: 0,
        needs_credential: Vec::new(),
        rejected: Vec::new(),
    };
    for entry in &set.rules {
        match from_export(entry) {
            Ok(imported) => {
                if imported.credential_was_redacted {
                    outcome.needs_credential.push(imported.rule.name.clone());
                }
                ready.push(imported);
            }
            // One unusable entry must not discard the rest: an operator
            // hand-edits these files, and losing nine good rules to one typo
            // is a worse outcome than importing nine and naming the tenth.
            Err(e) => outcome.rejected.push(e.to_string()),
        }
    }
    outcome.added = ready.len();
    Ok((ready, outcome))
}

/// `POST /admin/rules/import` — add the rules in a pasted rule set.
///
/// Adds rather than replaces: an import that silently deleted the operator's
/// existing rules would be unrecoverable from the same screen.
async fn import_rules(
    State(state): State<AppState>,
    Form(form): Form<ImportForm>,
) -> Result<Html<String>, StatusCode> {
    let (ready, outcome) = match plan_import(&form.rules_json) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Html(format!(
                "<div class=\"rule-error\">Could not read that rule set: {}</div>",
                escape_html(&e)
            )));
        }
    };

    let inserted = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut n = 0;
            for i in &ready {
                if insert_rule(conn, &i.rule).is_ok() {
                    n += 1;
                }
            }
            n
        })
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(render_import_outcome(inserted, &outcome)))
}

/// Render what an import did.
fn render_import_outcome(inserted: usize, outcome: &ImportOutcome) -> String {
    let mut html = format!("<div class=\"rule-success\">Imported {inserted} rule(s).</div>");
    if !outcome.needs_credential.is_empty() {
        let names = outcome
            .needs_credential
            .iter()
            .map(|n| escape_html(n))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            html,
            "<div class=\"rule-error\">These were exported with credentials \
             redacted and will fire unauthenticated until you re-enter one: {names}</div>"
        );
    }
    for reason in &outcome.rejected {
        let _ = write!(
            html,
            "<div class=\"rule-error\">Skipped: {}</div>",
            escape_html(reason)
        );
    }
    let _ = write!(
        html,
        "<div hx-get=\"/admin/rules/list\" hx-trigger=\"load\" \
         hx-target=\"#rules-table-container\" hx-swap=\"innerHTML\"></div>"
    );
    html
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
    /// `""` / `bearer` / `basic` / `header`.
    action_webhook_auth_kind: Option<String>,
    /// The credential: a token, `user:password`, or a header value.
    action_webhook_auth_value: Option<String>,
    /// Header name, for `action_webhook_auth_kind = "header"`.
    action_webhook_header_name: Option<String>,
}

/// Build the optional credential from the three form fields.
///
/// Returns `None` when no scheme is chosen or the credential is blank, so a
/// half-filled form produces an unauthenticated rule rather than a request
/// with an empty `Authorization` header — which some servers reject outright
/// and others treat as a failed login attempt.
fn webhook_auth_from_form(
    kind: Option<&str>,
    value: Option<&str>,
    header_name: Option<&str>,
) -> Option<WebhookAuth> {
    let value = value.map(str::trim).filter(|v| !v.is_empty())?;
    match kind.map(str::trim).unwrap_or_default() {
        "bearer" => Some(WebhookAuth::Bearer(value.to_string())),
        "basic" => {
            let (user, password) = value.split_once(':')?;
            Some(WebhookAuth::Basic {
                user: user.to_string(),
                password: password.to_string(),
            })
        }
        "header" => Some(WebhookAuth::Header {
            name: header_name
                .map(str::trim)
                .filter(|n| !n.is_empty())?
                .to_string(),
            value: value.to_string(),
        }),
        _ => None,
    }
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
// Test
// ---------------------------------------------------------------------------

/// The sample detection a rule test fires with.
///
/// Named so it is unmistakable in whatever the webhook lands in: an operator
/// testing a rule that posts to a shared channel should not have to wonder
/// whether a bird was really heard.
const TEST_SPECIES: &str = "Test Detection (not a real bird)";

/// Scientific name for the sample detection.
const TEST_SCI_NAME: &str = "Testus testus";

/// Confidence used for the sample detection.
const TEST_CONFIDENCE: f64 = 0.99;

/// What testing a rule did, separated from the handler so the non-network
/// branches are assertable without a request.
#[derive(Debug, PartialEq, Eq)]
enum TestPlan {
    /// A webhook to fire, with its rendered body.
    Fire {
        /// Target URL.
        url: String,
        /// HTTP method.
        method: String,
        /// Rendered body, if the rule has a template.
        body: Option<String>,
    },
    /// Nothing to send: the action has no outbound side.
    NothingToSend(&'static str),
}

/// Work out what testing `action` would send.
fn plan_test(action: &AlertAction) -> TestPlan {
    match action {
        AlertAction::Webhook {
            url,
            method,
            body_template,
            ..
        } => TestPlan::Fire {
            url: url.clone(),
            method: method.clone(),
            body: body_template.as_deref().map(|t| {
                birdnet_db::alert_rules::render_webhook_body(
                    t,
                    TEST_SPECIES,
                    TEST_SCI_NAME,
                    TEST_CONFIDENCE,
                    "2026-01-01",
                    "12:00:00",
                )
            }),
        },
        AlertAction::Log => TestPlan::NothingToSend(
            "This rule writes a log entry. There is nothing to send, so nothing to test.",
        ),
        AlertAction::Suppress => TestPlan::NothingToSend(
            "This rule suppresses other notifications. There is nothing to send, so nothing to test.",
        ),
    }
}

/// `POST /admin/rules/{id}/test` — fire this rule's action once, now.
///
/// The point is to find out an endpoint is wrong at the moment the rule is
/// written, rather than the first time an owl calls at 3 a.m. and the request
/// fails into a log nobody is reading.
async fn test_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let rule = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::alert_rules::get_rule(conn, id).ok().flatten())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    let message = match plan_test(&rule.action) {
        TestPlan::NothingToSend(msg) => {
            format!("<div class=\"rule-success\">{}</div>", escape_html(msg))
        }
        TestPlan::Fire { url, method, body } => {
            let auth = match &rule.action {
                AlertAction::Webhook { auth, .. } => auth.clone(),
                _ => None,
            };
            // The response is rendered into the admin page, so it must not
            // carry the URL: the path of a webhook URL is routinely the
            // credential, and this page can be screen-shared.
            let target = escape_html(&birdnet_integrations::webhook::redact_url(&url));
            match birdnet_integrations::webhook::dispatch_webhook(
                &url,
                &method,
                body.as_deref(),
                auth.as_ref(),
            )
            .await
            {
                Ok(status) if (200..300).contains(&status) => format!(
                    "<div class=\"rule-success\">{target} answered HTTP {status}. \
                     Look for a test detection at the other end.</div>"
                ),
                Ok(status) => format!(
                    "<div class=\"rule-error\">{target} answered HTTP {status}. \
                     The request arrived and was refused.</div>"
                ),
                Err(e) => format!(
                    "<div class=\"rule-error\">Could not reach {target}: {}</div>",
                    escape_html(&e.to_string())
                ),
            }
        }
    };

    Ok(Html(message))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// The standalone `/admin/rules` page GET folded into the Station **Alerts**
/// tab; its old URL permanently redirects there. The rule create/toggle/delete
/// endpoints (including `POST /admin/rules`) keep their `/admin/rules...` paths.
async fn rules_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/alerts#rules")
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
                auth: webhook_auth_from_form(
                    form.action_webhook_auth_kind.as_deref(),
                    form.action_webhook_auth_value.as_deref(),
                    form.action_webhook_header_name.as_deref(),
                ),
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

/// Page-specific body (scoped `<style>` + content).
///
/// Kept separate from the shared shell so the inline-style guard checks the
/// page's own markup; the `.container` / bare `nav` rules are dropped since the
/// shell owns layout + nav. Shared with the Station **Alerts** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders it in the main
/// shell — the rule list HTMX-loads from `/admin/rules/list` either way.
pub(crate) fn rules_body() -> String {
    r##"<style>
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
      .btn-primary { background:var(--moss); color:var(--on-moss); }
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
      .badge-green { background:var(--moss-soft); color:var(--moss-ink); }
      .badge-gray  { background:var(--surface); color:var(--fg-4); border:1px solid var(--border); }
      .badge-blue  { background:var(--surface); color:var(--moss-ink); }
      .badge-red   { background:var(--rare-soft); color:var(--rare); }
      .badge-yellow{ background:var(--dawn-soft); color:var(--dawn); }
      #webhook-fields { display:none; }
      .hint { color:var(--fg-4); font-size:.75rem; margin-top:.25rem; }
      /* O-25 sweep — faithful extraction of this page's inline styles. */
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
          <select id="action_type" name="action_type">
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

        <div class="form-grid">
          <div>
            <label for="action_webhook_auth_kind">Authentication</label>
            <select id="action_webhook_auth_kind" name="action_webhook_auth_kind">
              <option value="">None</option>
              <option value="bearer">Bearer token</option>
              <option value="basic">Basic (user:password)</option>
              <option value="header">Custom header</option>
            </select>
          </div>
          <div>
            <label for="action_webhook_auth_value">Credential</label>
            <input id="action_webhook_auth_value" name="action_webhook_auth_value"
                   type="password" autocomplete="off" spellcheck="false"
                   placeholder="token, or user:password">
            <div class="hint">Stored in this station's database and never shown again.</div>
          </div>
        </div>
        <div id="webhook-header-name-field">
          <label for="action_webhook_header_name">Header Name</label>
          <input id="action_webhook_header_name" name="action_webhook_header_name" type="text"
                 placeholder="X-API-Key">
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
    <div id="rule-test-result"></div>
  </div>

  <!-- Export / import -->
  <div class="card">
    <h2>Move rules between stations</h2>
    <p class="hint">
      The download replaces every credential with <code>***REDACTED***</code>,
      so it is safe to share when asking for help. Use
      <strong>with credentials</strong> only for a backup or when moving to a
      new station — that file is a secret.
    </p>
    <div class="form-actions">
      <a class="btn btn-secondary btn-sm" href="/admin/rules/export"
         download="alert-rules.json">Export rules</a>
      <a class="btn btn-secondary btn-sm" href="/admin/rules/export?secrets=1"
         download="alert-rules-with-credentials.json">Export with credentials</a>
    </div>
    <form hx-post="/admin/rules/import" hx-target="#rule-import-result" hx-swap="innerHTML">
      <label for="rules_json">Paste a rule set to add</label>
      <textarea id="rules_json" name="rules_json" rows="6" spellcheck="false"
                placeholder='&#123;"version":1,"redacted":true,"rules":[…]&#125;'></textarea>
      <div class="hint">Rules are added, never replaced — your existing rules stay.</div>
      <div class="form-actions">
        <button type="submit" class="btn btn-primary">Import rules</button>
      </div>
    </form>
    <div id="rule-import-result"></div>
  </div>
<script>
  document.getElementById('action_type').addEventListener('change', function() {
    document.getElementById('webhook-fields').style.display = this.value === 'webhook' ? 'block' : 'none';
  });
  // Set from script rather than a `style` attribute: `style-src` carries no
  // 'unsafe-inline', so an inline style would be dropped by CSP and the field
  // would render visible for every scheme. CSSOM assignment is not blocked.
  var authKind = document.getElementById('action_webhook_auth_kind');
  var headerField = document.getElementById('webhook-header-name-field');
  function syncHeaderNameField() {
    headerField.style.display = authKind.value === 'header' ? 'block' : 'none';
  }
  authKind.addEventListener('change', syncHeaderNameField);
  syncHeaderNameField();
</script>"##
    .to_owned()
}

/// Shorten a webhook URL for the rules-table badge to at most 30 characters,
/// appending an ellipsis when truncated.
///
/// Truncates by *characters*, not bytes: an operator's webhook URL can contain
/// multibyte UTF-8 (an IRI or a Unicode path), and a `&url[..30]` byte-slice
/// would panic if one straddled byte 30 — which, with `panic = "abort"` in the
/// release profile, crashes the whole process when the admin page renders.
fn truncate_url_display(url: &str) -> String {
    if url.chars().count() > 30 {
        format!("{}…", url.chars().take(30).collect::<String>())
    } else {
        url.to_string()
    }
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
                let url_short = truncate_url_display(url);
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
    <button class="btn btn-secondary btn-sm"
            hx-post="/admin/rules/{id}/test"
            hx-target="#rule-test-result"
            hx-swap="innerHTML"
            title="Fire this rule's action once, now">Test</button>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_url_display_short_url_unchanged() {
        assert_eq!(
            truncate_url_display("https://example.com/hook"),
            "https://example.com/hook"
        );
    }

    #[test]
    fn truncate_url_display_long_url_ellipsized() {
        let url = "https://example.com/very/long/webhook/path/that/exceeds";
        let out = truncate_url_display(url);
        assert_eq!(out.chars().filter(|&c| c != '…').count(), 30);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_url_display_does_not_panic_on_multibyte_boundary() {
        // Regression: byte-slicing `&url[..30]` panics when a multibyte char
        // straddles byte 30; with `panic = "abort"` that crashes the process.
        // 29 ASCII bytes then a 2-byte 'é' put the char across byte 30.
        let url = format!("{}\u{e9}tail", "a".repeat(29));
        assert!(
            !url.is_char_boundary(30),
            "test setup: byte 30 must split the multibyte char"
        );
        let out = truncate_url_display(&url);
        // No panic; 30 characters kept plus the ellipsis.
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().filter(|&c| c != '…').count(), 30);
    }

    // ── webhook authentication, from the form ───────────────────────────

    #[test]
    fn each_scheme_is_built_from_the_form_fields() {
        assert_eq!(
            webhook_auth_from_form(Some("bearer"), Some("tok_ABC"), None),
            Some(WebhookAuth::Bearer("tok_ABC".into()))
        );
        assert_eq!(
            webhook_auth_from_form(Some("basic"), Some("ada:hunter2"), None),
            Some(WebhookAuth::Basic {
                user: "ada".into(),
                password: "hunter2".into(),
            })
        );
        assert_eq!(
            webhook_auth_from_form(Some("header"), Some("k_XYZ"), Some("X-API-Key")),
            Some(WebhookAuth::Header {
                name: "X-API-Key".into(),
                value: "k_XYZ".into(),
            })
        );
    }

    #[test]
    fn a_half_filled_form_produces_no_credential_rather_than_an_empty_one() {
        // An empty or malformed `Authorization` header is not the same as
        // none: some servers reject it outright, and others count it as a
        // failed login attempt and lock the account after enough of them. The
        // form is three optional fields, so every partial combination is
        // reachable by an operator who changes the dropdown and saves.
        let cases: [(Option<&str>, Option<&str>, Option<&str>); 7] = [
            (None, None, None),                           // untouched
            (Some("bearer"), None, None),                 // scheme, no token
            (Some("bearer"), Some("   "), None),          // scheme, blank token
            (Some(""), Some("tok_ABC"), None),            // token, no scheme
            (Some("header"), Some("k_XYZ"), None),        // header value, no name
            (Some("header"), Some("k_XYZ"), Some(" ")),   // header name blank
            (Some("basic"), Some("no-colon-here"), None), // not user:password
        ];
        for (kind, value, header) in cases {
            assert_eq!(
                webhook_auth_from_form(kind, value, header),
                None,
                "{kind:?}/{value:?}/{header:?} should not produce a credential"
            );
        }
    }

    #[test]
    fn a_basic_password_containing_a_colon_is_kept_whole() {
        // Counterpart to the `no-colon-here` case above: the split is on the
        // first colon only, so a generated password with colons in it works.
        assert_eq!(
            webhook_auth_from_form(Some("basic"), Some("ada:a:b:c"), None),
            Some(WebhookAuth::Basic {
                user: "ada".into(),
                password: "a:b:c".into(),
            })
        );
    }

    #[test]
    fn an_unknown_scheme_produces_no_credential() {
        assert_eq!(
            webhook_auth_from_form(Some("oauth3"), Some("tok_ABC"), None),
            None
        );
    }

    #[test]
    fn the_rule_form_offers_every_scheme_the_dispatcher_implements() {
        // The dropdown and `webhook_auth_from_form` are edited separately; a
        // scheme in one and not the other is silently a no-op for the operator.
        let html = rules_body();
        for kind in ["bearer", "basic", "header"] {
            assert!(
                html.contains(&format!(r#"<option value="{kind}">"#)),
                "the form does not offer {kind}"
            );
            assert!(
                webhook_auth_from_form(Some(kind), Some("ada:secret"), Some("X-Api-Key")).is_some(),
                "the handler does not accept {kind}"
            );
        }
    }

    #[test]
    fn the_credential_field_is_not_a_plain_text_input() {
        // It is typed on a page an operator may well be screen-sharing while
        // asking for help, and browsers offer to save what they see in a text
        // input.
        let html = rules_body();
        let start = html
            .find(r#"<input id="action_webhook_auth_value""#)
            .expect("the credential field is rendered");
        let field = &html[start..start + html[start..].find('>').expect("unclosed tag")];
        assert!(
            field.contains(r#"type="password""#),
            "the credential field is not masked: {field}"
        );
    }

    #[test]
    fn the_header_name_field_is_hidden_by_script_not_by_a_style_attribute() {
        // `style-src` carries no 'unsafe-inline', so an inline style attribute
        // hiding this field would be dropped by CSP and the field would render
        // visible under every scheme — a silent failure, which is why
        // `inline_style_guard` exists. This pins the mechanism as well as the
        // outcome, because a reviewer adding a sibling field will copy
        // whatever is here.
        let html = rules_body();
        // Assembled rather than written out: `inline_style_guard` greps the
        // source for the literal and would flag this assertion itself.
        let inline_style = format!("{}=\"", "style");
        let tag_start = html
            .find(r#"<div id="webhook-header-name-field""#)
            .expect("the header-name field is rendered");
        let tag = &html[tag_start..tag_start + html[tag_start..].find('>').expect("unclosed tag")];
        assert!(
            !tag.contains(&inline_style),
            "the header-name field is hidden with an inline style: {tag}"
        );
        assert!(
            html.contains("syncHeaderNameField();"),
            "nothing sets the header-name field's initial visibility"
        );
    }

    // ── import planning ─────────────────────────────────────────────────

    /// A minimal rule set, as an operator would paste it.
    fn rule_set_json(rules: &str) -> String {
        format!(r#"{{"version":1,"redacted":true,"rules":[{rules}]}}"#)
    }

    #[test]
    fn one_bad_entry_does_not_discard_the_good_ones() {
        // These files are hand-edited and pasted. Losing nine working rules to
        // a tenth with a typo is a worse outcome than importing nine and
        // naming the tenth, and the operator cannot see which was which
        // afterwards if the whole paste is rejected.
        let json = rule_set_json(
            r#"{"name":"keep me","action":"log"},
               {"name":"","action":"log"},
               {"name":"broken","action":"webhook"},
               {"name":"keep me too","action":"suppress"}"#,
        );
        let (ready, outcome) = plan_import(&json).expect("the set itself parses");
        assert_eq!(outcome.added, 2);
        assert_eq!(ready.len(), 2);
        assert_eq!(outcome.rejected.len(), 2);
        assert!(
            outcome.rejected.iter().any(|r| r.contains("broken")),
            "{:?}",
            outcome.rejected
        );
    }

    #[test]
    fn a_set_that_is_not_a_rule_set_is_refused_whole() {
        // Counterpart: partial success applies to *entries*, not to a file
        // that is not a rule set at all. Importing `{}` as zero rules would
        // report "Imported 0 rule(s)" and look like a successful no-op.
        assert!(plan_import("not json at all").is_err());
        assert!(plan_import(r#"{"nope":1}"#).is_err());
    }

    #[test]
    fn a_newer_format_version_is_refused_rather_than_half_read() {
        // serde's `default` attributes mean a future format would otherwise
        // import as a set of rules with every new field silently missing.
        let json = r#"{"version":99,"redacted":true,"rules":[{"name":"x","action":"log"}]}"#;
        let err = plan_import(json).expect_err("refused");
        assert!(err.contains("99"), "{err}");
    }

    #[test]
    fn the_current_version_is_accepted() {
        // Counterpart, so "refuse everything" would not pass the gate above.
        let json = rule_set_json(r#"{"name":"x","action":"log"}"#);
        assert_eq!(plan_import(&json).expect("accepted").1.added, 1);
    }

    #[test]
    fn rules_needing_a_credential_are_named_not_just_counted() {
        // The operator has to go and re-enter them, so the message has to say
        // which. A count alone means opening every rule to find out.
        let json = rule_set_json(
            r#"{"name":"authed","action":"webhook","webhook_url":"https://x/y",
                "webhook_auth_kind":"bearer","webhook_auth_value":"***REDACTED***"},
               {"name":"open","action":"webhook","webhook_url":"https://x/z"}"#,
        );
        let (_, outcome) = plan_import(&json).expect("parses");
        assert_eq!(outcome.added, 2);
        assert_eq!(outcome.needs_credential, ["authed"]);
    }

    #[test]
    fn the_import_summary_names_every_rule_that_needs_a_credential() {
        let outcome = ImportOutcome {
            added: 2,
            needs_credential: vec!["Owls at night".into(), "Kites".into()],
            rejected: vec!["rule \"x\" has unknown action \"y\"".into()],
        };
        let html = render_import_outcome(2, &outcome);
        assert!(html.contains("Imported 2 rule(s)"), "{html}");
        assert!(html.contains("Owls at night"), "{html}");
        assert!(html.contains("Kites"), "{html}");
        assert!(html.contains("unknown action"), "{html}");
    }

    // ── test planning ───────────────────────────────────────────────────

    #[test]
    fn testing_a_webhook_rule_sends_an_unmistakably_synthetic_detection() {
        // A test firing into a shared channel must not read as a real record.
        let action = AlertAction::Webhook {
            url: "https://x/y".into(),
            method: "POST".into(),
            body_template: Some(r#"{"bird":"{{species}}","sci":"{{sci_name}}"}"#.into()),
            auth: None,
        };
        let TestPlan::Fire { url, method, body } = plan_test(&action) else {
            panic!("a webhook rule must fire");
        };
        assert_eq!(url, "https://x/y");
        assert_eq!(method, "POST");
        let body = body.expect("the template was rendered");
        assert!(body.contains(TEST_SPECIES), "{body}");
        assert!(body.contains("not a real bird"), "{body}");
        assert!(
            !body.contains("{{species}}"),
            "the template was not rendered"
        );
    }

    #[test]
    fn testing_a_log_or_suppress_rule_sends_nothing() {
        // Counterpart: a "test" that quietly did nothing and said "sent" would
        // be worse than one that says there is nothing to send.
        for action in [AlertAction::Log, AlertAction::Suppress] {
            let TestPlan::NothingToSend(msg) = plan_test(&action) else {
                panic!("{action:?} must not fire a request");
            };
            assert!(msg.contains("nothing to send"), "{msg}");
        }
    }

    #[test]
    fn a_webhook_rule_without_a_template_still_fires() {
        let action = AlertAction::Webhook {
            url: "https://x/y".into(),
            method: "POST".into(),
            body_template: None,
            auth: None,
        };
        let TestPlan::Fire { body, .. } = plan_test(&action) else {
            panic!("must fire");
        };
        assert_eq!(
            body, None,
            "an absent template must not become a rendered one"
        );
    }
}
